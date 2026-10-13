//! 测试用的迷你 HTTP 服务。
//!
//! 为什么不用 wiremock:续传的核心场景是「传到一半连接断掉」,而 wiremock 只能
//! 返回完整的预置响应,没法在响应体中途把连接掐了。这里手写一个几十行的
//! HTTP/1.1 服务,换来对 Range、ETag、If-Range 和**中断时机**的完全控制。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone, Debug)]
pub struct Behavior {
    pub supports_range: bool,
    pub etag: Option<String>,
    /// 响应体写到这么多字节就直接断开连接,用来模拟传输中断。
    pub truncate_body_at: Option<usize>,
    /// 无视一切逻辑,直接返回这个状态码。
    pub status_override: Option<u16>,
    /// 对**一切**请求回 302 并把 `Location` 指向这个地址,不返回任何响应体。
    ///
    /// 用来起一个纯转发的「源站」:真正的字节落在另一个 `TestServer` 上,于是
    /// 「哪些请求打到了源站」可以直接用 `request_count()` / `ranges()` 读出来。
    pub redirect_to: Option<String>,
}

impl Default for Behavior {
    fn default() -> Self {
        Self {
            supports_range: true,
            etag: Some("\"v1\"".to_string()),
            truncate_body_at: None,
            status_override: None,
            redirect_to: None,
        }
    }
}

struct State {
    body: Mutex<Vec<u8>>,
    behavior: Mutex<Behavior>,
    requests: AtomicUsize,
    ranges: Mutex<Vec<String>>,
    last_headers: Mutex<HashMap<String, String>>,
}

pub struct TestServer {
    addr: SocketAddr,
    state: Arc<State>,
}

impl TestServer {
    /// 在独立线程上的独立运行时里起服务。
    ///
    /// 必须独立:被测的 `download_*` 是同步接口,内部对全局运行时做 `block_on`,
    /// 在 tokio 的工作线程上调用会直接 panic。所以测试本身是普通 `#[test]`,
    /// 服务端自带一个 current-thread 运行时。
    pub fn start(body: Vec<u8>) -> Self {
        let state = Arc::new(State {
            body: Mutex::new(body),
            behavior: Mutex::new(Behavior::default()),
            requests: AtomicUsize::new(0),
            ranges: Mutex::new(Vec::new()),
            last_headers: Mutex::new(HashMap::new()),
        });

        let (tx, rx) = std::sync::mpsc::channel();
        let server_state = Arc::clone(&state);
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("创建测试服务运行时");
            runtime.block_on(async move {
                let listener = TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("监听随机端口");
                tx.send(listener.local_addr().expect("取监听地址"))
                    .expect("回传监听地址");
                while let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(handle(stream, Arc::clone(&server_state)));
                }
            });
        });

        Self {
            addr: rx.recv().expect("等待服务就绪"),
            state,
        }
    }

    pub fn url(&self) -> String {
        self.url_at("payload.bin")
    }

    /// 同一个服务换个路径的地址。服务端压根不看路径,给的永远是同一份 body ——
    /// 用来构造「两个地址在 `safe_url()` 眼里不同」的场景(端口不算,它只取 host)。
    pub fn url_at(&self, path: &str) -> String {
        format!("http://{}/{path}", self.addr)
    }

    pub fn set_behavior(&self, behavior: Behavior) {
        *self.state.behavior.lock().expect("behavior 锁") = behavior;
    }

    pub fn set_body(&self, body: Vec<u8>) {
        *self.state.body.lock().expect("body 锁") = body;
    }

    pub fn request_count(&self) -> usize {
        self.state.requests.load(Ordering::SeqCst)
    }

    /// 服务端收到过的所有 Range 请求头。
    pub fn ranges(&self) -> Vec<String> {
        self.state.ranges.lock().expect("ranges 锁").clone()
    }

    pub fn last_header(&self, name: &str) -> Option<String> {
        self.state
            .last_headers
            .lock()
            .expect("headers 锁")
            .get(name)
            .cloned()
    }
}

async fn handle(mut stream: TcpStream, state: Arc<State>) {
    let Some(request) = read_request(&mut stream).await else {
        return;
    };

    state.requests.fetch_add(1, Ordering::SeqCst);
    if let Some(range) = request.headers.get("range") {
        state.ranges.lock().expect("ranges 锁").push(range.clone());
    }
    *state.last_headers.lock().expect("headers 锁") = request.headers.clone();

    let body = state.body.lock().expect("body 锁").clone();
    let behavior = state.behavior.lock().expect("behavior 锁").clone();

    if let Some(status) = behavior.status_override {
        let head =
            format!("HTTP/1.1 {status} Forced\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let _ = stream.write_all(head.as_bytes()).await;
        return;
    }

    // 重定向早于 Range/ETag 的一切处理:源站只负责指路,方法和请求头一概不看。
    if let Some(location) = &behavior.redirect_to {
        let head = format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let _ = stream.write_all(head.as_bytes()).await;
        return;
    }

    let total = body.len();
    // If-Range 不匹配时必须无视 Range、返回完整的 200 —— 客户端据此放弃续传。
    let if_range_ok = match (request.headers.get("if-range"), behavior.etag.as_deref()) {
        (Some(given), Some(current)) => given == current,
        (Some(_), None) => false,
        (None, _) => true,
    };

    let requested = request
        .headers
        .get("range")
        .filter(|_| behavior.supports_range && if_range_ok)
        .and_then(|value| parse_range(value, total));

    let (status, start, end) = match requested {
        Some((start, end)) => (206u16, start, end),
        None => (200u16, 0, total.saturating_sub(1)),
    };

    let slice = if total == 0 {
        &body[..]
    } else {
        &body[start..=end]
    };

    let mut head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n",
        if status == 206 { "Partial Content" } else { "OK" },
        slice.len()
    );
    if status == 206 {
        head.push_str(&format!("Content-Range: bytes {start}-{end}/{total}\r\n"));
    }
    if let Some(etag) = &behavior.etag {
        head.push_str(&format!("ETag: {etag}\r\n"));
    }
    head.push_str("\r\n");

    if stream.write_all(head.as_bytes()).await.is_err() || request.method == "HEAD" {
        return;
    }

    let limit = behavior
        .truncate_body_at
        .map_or(slice.len(), |at| at.min(slice.len()));
    let _ = stream.write_all(&slice[..limit]).await;
    let _ = stream.flush().await;
    // limit < slice.len() 时直接 drop:声明的 Content-Length 没写满,
    // 客户端会把它当成传输错误 —— 这正是我们要模拟的「中断」。
}

struct Request {
    method: String,
    headers: HashMap<String, String>,
}

async fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    let text = String::from_utf8_lossy(&buffer).into_owned();
    let mut lines = text.split("\r\n");
    let method = lines
        .next()?
        .split(' ')
        .next()
        .unwrap_or_default()
        .to_string();
    let headers = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_lowercase(), value.trim().to_string()))
        .collect();

    Some(Request { method, headers })
}

/// 只认 `bytes=a-b` 和 `bytes=a-` 两种形式,够测试用了。
fn parse_range(value: &str, total: usize) -> Option<(usize, usize)> {
    if total == 0 {
        return None;
    }
    let spec = value.strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;
    let start: usize = start.trim().parse().ok()?;
    if start >= total {
        return None;
    }
    let end = match end.trim() {
        "" => total - 1,
        other => other.parse::<usize>().ok()?.min(total - 1),
    };
    if start > end {
        return None;
    }
    Some((start, end))
}

/// 可重现的伪随机负载。用固定公式而不是全 0,这样写错偏移一定会被内容比对抓到。
pub fn payload(size: usize) -> Vec<u8> {
    (0..size)
        .map(|i| (i.wrapping_mul(31) % 251) as u8)
        .collect()
}

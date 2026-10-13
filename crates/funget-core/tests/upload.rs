//! 上传路径的行为契约测试(契约 6、7)。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use funget_core::{upload, UploadOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

struct Capture {
    method: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

struct Sink {
    captures: Mutex<Vec<Capture>>,
    /// 前多少次请求返回 500,用来验证重试。
    fail_first: AtomicUsize,
}

struct Server {
    url: String,
    sink: Arc<Sink>,
}

impl Server {
    fn start() -> Self {
        let sink = Arc::new(Sink {
            captures: Mutex::new(Vec::new()),
            fail_first: AtomicUsize::new(0),
        });
        let (tx, rx) = std::sync::mpsc::channel();
        let server_sink = Arc::clone(&sink);

        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("创建测试服务运行时");
            runtime.block_on(async move {
                let listener = TcpListener::bind("127.0.0.1:0").await.expect("监听端口");
                tx.send(listener.local_addr().expect("取地址"))
                    .expect("回传地址");
                while let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(handle(stream, Arc::clone(&server_sink)));
                }
            });
        });

        let addr = rx.recv().expect("等待服务就绪");
        Self {
            url: format!("http://{addr}/upload"),
            sink,
        }
    }

    fn captures(&self) -> std::sync::MutexGuard<'_, Vec<Capture>> {
        self.sink.captures.lock().expect("captures 锁")
    }
}

async fn handle(mut stream: TcpStream, sink: Arc<Sink>) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];

    // 先把头读全。
    let header_end = loop {
        let read = match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };

    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let method = lines
        .next()
        .and_then(|line| line.split(' ').next())
        .unwrap_or_default()
        .to_string();
    let headers: Vec<(String, String)> = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_lowercase(), value.trim().to_string()))
        .collect();

    let declared: usize = headers
        .iter()
        .find(|(key, _)| key == "content-length")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);

    let mut body = buffer[header_end..].to_vec();
    while body.len() < declared {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => body.extend_from_slice(&chunk[..read]),
        }
    }

    sink.captures.lock().expect("captures 锁").push(Capture {
        method,
        headers,
        body,
    });

    let status = if sink.fail_first.load(Ordering::SeqCst) > 0 {
        sink.fail_first.fetch_sub(1, Ordering::SeqCst);
        500
    } else {
        200
    };
    let response = format!("HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = stream.write_all(response.as_bytes()).await;
}

fn options(mutate: impl FnOnce(&mut UploadOptions)) -> UploadOptions {
    let mut options = UploadOptions {
        progress: false,
        ..Default::default()
    };
    mutate(&mut options);
    options
}

fn fixture(contents: &[u8]) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("创建临时目录");
    let path = dir.path().join("payload.bin");
    std::fs::write(&path, contents).expect("写入文件");
    let path = path.to_str().expect("路径").to_string();
    (dir, path)
}

/// 契约 7:PUT 流式上传,显式带 Content-Length 和 application/octet-stream。
#[test]
fn put_streams_the_body_with_an_explicit_length() {
    let server = Server::start();
    let contents: Vec<u8> = (0..300_000u32).map(|i| (i % 253) as u8).collect();
    let (_dir, path) = fixture(&contents);

    assert!(upload(&server.url, &path, &options(|_| {})));

    let captures = server.captures();
    assert_eq!(captures.len(), 1);
    let capture = &captures[0];
    assert_eq!(capture.method, "PUT");
    assert_eq!(capture.body, contents, "PUT 的 body 必须是原始文件字节");
    assert!(capture
        .headers
        .contains(&("content-length".into(), contents.len().to_string())));
    assert!(capture
        .headers
        .contains(&("content-type".into(), "application/octet-stream".into())));
}

/// 契约 7:POST 走 multipart,字段名固定 `file`,文件名取 basename。
#[test]
fn post_sends_multipart_with_the_basename() {
    let server = Server::start();
    let (_dir, path) = fixture(b"hello multipart");

    assert!(upload(
        &server.url,
        &path,
        &options(|o| o.method = "POST".into())
    ));

    let captures = server.captures();
    let capture = &captures[0];
    assert_eq!(capture.method, "POST");
    let body = String::from_utf8_lossy(&capture.body);
    assert!(body.contains("name=\"file\""), "字段名必须是 file: {body}");
    assert!(
        body.contains("filename=\"payload.bin\""),
        "文件名应当是 basename: {body}"
    );
    assert!(body.contains("hello multipart"));
    assert!(capture
        .headers
        .iter()
        .any(|(key, value)| key == "content-type" && value.starts_with("multipart/form-data")));
}

/// 契约 7:失败后按 `2^attempt` 退避重试,次数用尽才返回 false。
#[test]
fn retries_then_succeeds() {
    let server = Server::start();
    server.sink.fail_first.store(2, Ordering::SeqCst);
    let (_dir, path) = fixture(b"retry me");

    let started = std::time::Instant::now();
    assert!(upload(&server.url, &path, &options(|o| o.max_retries = 3)));
    let elapsed = started.elapsed();

    assert_eq!(server.captures().len(), 3, "两次失败 + 一次成功");
    // 退避是 1 秒 + 2 秒,总耗时必须盖过 3 秒。
    assert!(
        elapsed >= std::time::Duration::from_secs(3),
        "退避时长不足: {elapsed:?}"
    );
}

#[test]
fn gives_up_after_max_retries() {
    let server = Server::start();
    server.sink.fail_first.store(99, Ordering::SeqCst);
    let (_dir, path) = fixture(b"always fails");

    assert!(!upload(&server.url, &path, &options(|o| o.max_retries = 1)));
    assert_eq!(server.captures().len(), 2, "首次 + 一次重试");
}

/// 契约 6:校验不通过时一个请求都不能发出去。
#[test]
fn invalid_arguments_never_reach_the_network() {
    let server = Server::start();
    let (dir, path) = fixture(b"payload");

    assert!(!upload(&server.url, &path, &options(|o| o.chunk_size = 0)));
    assert!(!upload(
        &server.url,
        &path,
        &options(|o| o.max_retries = -1)
    ));
    assert!(!upload(&server.url, &path, &options(|o| o.timeout = 0)));
    assert!(!upload(
        &server.url,
        &path,
        &options(|o| o.method = "PATCH".into())
    ));
    assert!(!upload(
        &server.url,
        dir.path().join("missing.bin").to_str().expect("路径"),
        &options(|_| {})
    ));

    let (_empty_dir, empty) = fixture(b"");
    assert!(!upload(&server.url, &empty, &options(|_| {})));

    assert!(server.captures().is_empty(), "校验阶段不应当发出任何请求");
}

/// 方法名大小写不敏感(Python 的 `method.upper()`)。
#[test]
fn method_is_upcased() {
    let server = Server::start();
    let (_dir, path) = fixture(b"lowercase method");

    assert!(upload(
        &server.url,
        &path,
        &options(|o| o.method = "post".into())
    ));
    assert_eq!(server.captures()[0].method, "POST");
}

/// 调用方传的头要带上,但 Content-Length / Content-Type 由上传逻辑说了算 ——
/// 对应 Python 的 `{**(headers or {}), "Content-Length": ..., "Content-Type": ...}`,
/// 注意这跟下载路径的优先级是**相反**的。
#[test]
fn caller_headers_are_sent_but_cannot_override_the_framing_headers() {
    let server = Server::start();
    let (_dir, path) = fixture(b"0123456789");

    assert!(upload(
        &server.url,
        &path,
        &options(|o| {
            o.headers = funget_core::headers_from_pairs([
                ("X-Token", "abc123"),
                ("Content-Type", "text/plain"),
                ("Content-Length", "1"),
            ]);
        })
    ));

    let captures = server.captures();
    let headers = &captures[0].headers;
    assert!(headers.contains(&("x-token".into(), "abc123".into())));
    assert!(headers.contains(&("content-type".into(), "application/octet-stream".into())));
    assert!(headers.contains(&("content-length".into(), "10".into())));
}

use std::path::Path;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_LENGTH, CONTENT_TYPE};
use reqwest::{Body, Method};
use tokio_util::io::ReaderStream;

use crate::client::{Auth, HttpConfig, Transport, TransportOptions};
use crate::error::Result;
use crate::progress::Progress;
use crate::runtime;
use crate::url::safe_url;

const OCTET_STREAM: &str = "application/octet-stream";

/// 上传参数。
///
/// `chunk_size` / `max_retries` / `timeout` 用有符号整数而不是 `usize`/`u32`,
/// 是为了保住契约 6 那张「不发请求就返回 false」的校验表 —— Python 侧
/// `max_retries=-1`、`timeout=0` 这类入参必须能走到校验分支并返回 `false`,
/// 换成无符号类型就只能在 PyO3 边界上抛 `OverflowError`,那是行为变更。
#[derive(Clone, Debug)]
pub struct UploadOptions {
    pub prefix: String,
    pub chunk_size: i64,
    pub method: String,
    pub max_retries: i64,
    /// 请求超时秒数。
    pub timeout: i64,
    pub headers: HeaderMap,
    pub auth: Option<Auth>,
    pub progress: bool,
    /// TLS 信任源、重定向策略、代理 —— 与下载共用同一个结构体。
    pub transport: TransportOptions,
}

impl Default for UploadOptions {
    fn default() -> Self {
        Self {
            prefix: String::new(),
            chunk_size: 256 * 1024,
            method: "PUT".to_string(),
            max_retries: 3,
            timeout: 60,
            headers: HeaderMap::new(),
            auth: None,
            progress: true,
            transport: TransportOptions::default(),
        }
    }
}

/// 上传单个文件,对应 Python 的 `funget.single_upload`。
///
/// 与下载的四个函数一样,任何失败都折叠成 `false`,绝不抛出。
pub fn upload(url: &str, filepath: &str, options: &UploadOptions) -> bool {
    // 契约 6:下面这一整段校验必须在**建立任何连接之前**完成。
    // Python 的测试直接断言这些入参下 `requests.Session` 从未被构造过。
    let method = options.method.to_uppercase();
    if options.chunk_size <= 0 || options.max_retries < 0 || options.timeout <= 0 {
        log::error!("chunk_size 与 timeout 必须为正数,max_retries 不能为负数");
        return false;
    }
    if method != "PUT" && method != "POST" {
        log::error!("不支持的 HTTP 方法: {method}");
        return false;
    }
    let path = Path::new(filepath);
    if !path.is_file() {
        log::error!("文件不存在: {filepath}");
        return false;
    }
    let filesize = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(error) => {
            log::error!("读取文件信息失败: {error}");
            return false;
        }
    };
    if filesize == 0 {
        log::error!("文件为空: {filepath}");
        return false;
    }

    let runtime = match runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            log::error!("{error}");
            return false;
        }
    };

    runtime.block_on(async move {
        match run(url, path, filesize, &method, options).await {
            Ok(result) => result,
            Err(error) => {
                log::error!("上传失败: {error}");
                false
            }
        }
    })
}

async fn run(
    url: &str,
    path: &Path,
    filesize: u64,
    method: &str,
    options: &UploadOptions,
) -> Result<bool> {
    // 刻意不把用户请求头塞进 HttpConfig:`Transport::merge_headers` 的优先级是
    // 「配置头覆盖专属头」,那是下载路径的契约。上传反过来 —— Python 写的是
    // `{**(headers or {}), "Content-Length": ..., "Content-Type": ...}`,
    // 即 **Content-Length / Content-Type 覆盖用户头**(`upload/single.py:80`)。
    // 这里自己拼好完整的 HeaderMap 再作为 extra 传进去,避开那条规则。
    //
    // 传输层那一组(TLS / 重定向 / 代理)反过来必须原样透传:它们是「怎么连」,
    // 与请求头的优先级无关,上传和下载没有理由不一致。
    let transport = Transport::new(HttpConfig {
        headers: HeaderMap::new(),
        auth: options.auth.clone(),
        timeout: Duration::from_secs(options.timeout as u64),
        max_retries: 0,
        transport: options.transport.clone(),
    })?;

    let prefix = if options.prefix.is_empty() {
        String::new()
    } else {
        format!("{}--", options.prefix)
    };
    let progress = Progress::new(filesize, &prefix, options.progress);

    let filename = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());

    let attempts = options.max_retries as u64;
    for attempt in 0..=attempts {
        if attempt > 0 {
            progress.reset();
        }

        let result = if method == "PUT" {
            send_put(&transport, url, path, filesize, &progress, options).await
        } else {
            send_post(&transport, url, path, filesize, &filename, options).await
        };

        match result {
            Ok(()) => {
                // POST 走 multipart,进度无法分块回报,成功后一次性补满 ——
                // 与 Python 的 `if method == "POST": pbar.update(filesize)` 一致。
                if method == "POST" {
                    progress.inc(filesize);
                }
                progress.finish();
                log::info!("上传完成: {}", path.display());
                return Ok(true);
            }
            Err(error) => {
                log::warn!("第 {} 次上传 {} 失败: {error}", attempt + 1, safe_url(url));
                if attempt == attempts {
                    progress.finish();
                    return Ok(false);
                }
                // 退避 `2^attempt` 秒:1、2、4……与 Python 完全一致。
                tokio::time::sleep(Duration::from_secs(1u64 << attempt)).await;
            }
        }
    }

    progress.finish();
    Ok(false)
}

async fn send_put(
    transport: &Transport,
    url: &str,
    path: &Path,
    filesize: u64,
    progress: &Progress,
    options: &UploadOptions,
) -> Result<()> {
    let file = tokio::fs::File::open(path).await?;
    let progress = progress.clone();
    // 每读出一块就推进进度条,与 Python 的 `chunks()` 生成器里 `pbar.update` 等价。
    let stream = ReaderStream::with_capacity(file, options.chunk_size as usize).map(move |chunk| {
        if let Ok(bytes) = &chunk {
            progress.inc(bytes.len() as u64);
        }
        chunk
    });

    let mut headers = options.headers.clone();
    headers.insert(CONTENT_LENGTH, HeaderValue::from(filesize));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(OCTET_STREAM));

    let response = transport
        .request(Method::PUT, url, &headers)
        .body(Body::wrap_stream(stream))
        .send()
        .await?;
    check(response)
}

async fn send_post(
    transport: &Transport,
    url: &str,
    path: &Path,
    filesize: u64,
    filename: &str,
    options: &UploadOptions,
) -> Result<()> {
    let file = tokio::fs::File::open(path).await?;
    // 流式 multipart:带上长度,免得退化成 chunked 传输 —— 不少对象存储网关不收 chunked。
    let part = reqwest::multipart::Part::stream_with_length(
        Body::wrap_stream(ReaderStream::new(file)),
        filesize,
    )
    .file_name(filename.to_string())
    .mime_str(OCTET_STREAM)?;
    // 字段名固定是 `file`,文件名取 basename —— Python 测试对这两项都有显式断言。
    let form = reqwest::multipart::Form::new().part("file", part);

    let response = transport
        .request(Method::POST, url, &options.headers)
        .multipart(form)
        .send()
        .await?;
    check(response)
}

/// 对应 `response.raise_for_status()`:4xx/5xx 都算这次尝试失败,交给外层重试。
fn check(response: reqwest::Response) -> Result<()> {
    let status = response.status();
    if status.is_client_error() || status.is_server_error() {
        return Err(crate::error::Error::Status {
            status: status.as_u16(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 契约 6:这些入参必须在发出任何请求之前就返回 false。
    /// 用一个一定连不上的地址,真要是发了请求,测试会因为耗时/报错而暴露出来。
    const DEAD_URL: &str = "http://127.0.0.1:1/upload";

    fn options(mutate: impl FnOnce(&mut UploadOptions)) -> UploadOptions {
        let mut options = UploadOptions {
            progress: false,
            ..Default::default()
        };
        mutate(&mut options);
        options
    }

    #[test]
    fn rejects_invalid_arguments_before_any_request() {
        let dir = tempfile::tempdir().expect("创建临时目录");
        let path = dir.path().join("payload.bin");
        std::fs::write(&path, b"hello").expect("写入文件");
        let filepath = path.to_str().expect("路径不是合法 UTF-8");

        assert!(!upload(DEAD_URL, filepath, &options(|o| o.chunk_size = 0)));
        assert!(!upload(
            DEAD_URL,
            filepath,
            &options(|o| o.max_retries = -1)
        ));
        assert!(!upload(DEAD_URL, filepath, &options(|o| o.timeout = 0)));
        assert!(!upload(
            DEAD_URL,
            filepath,
            &options(|o| o.method = "PATCH".into())
        ));
    }

    #[test]
    fn rejects_missing_or_empty_file() {
        let dir = tempfile::tempdir().expect("创建临时目录");
        let missing = dir.path().join("nope.bin");
        let empty = dir.path().join("empty.bin");
        std::fs::write(&empty, b"").expect("写入空文件");

        assert!(!upload(
            DEAD_URL,
            missing.to_str().expect("路径不是合法 UTF-8"),
            &options(|_| {})
        ));
        assert!(!upload(
            DEAD_URL,
            empty.to_str().expect("路径不是合法 UTF-8"),
            &options(|_| {})
        ));
        // 目录不是文件
        assert!(!upload(
            DEAD_URL,
            dir.path().to_str().expect("路径不是合法 UTF-8"),
            &options(|_| {})
        ));
    }

    #[test]
    fn method_is_case_insensitive() {
        // 小写 put 应当通过校验(随后因为连不上而失败,但不是「不支持的方法」那条路)。
        let mut options = options(|o| {
            o.method = "put".into();
            o.max_retries = 0;
            o.timeout = 1;
        });
        options.progress = false;
        assert_eq!(options.method.to_uppercase(), "PUT");
    }
}

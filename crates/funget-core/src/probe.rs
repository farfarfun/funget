use reqwest::header::{HeaderMap, HeaderValue, RANGE};
use reqwest::Method;

use crate::client::Transport;
use crate::url::safe_url;

/// 远端文件元数据,对应 Python 的 `Downloader.get_file_info()` 与
/// `MultiDownloader.supports_range`。
#[derive(Clone, Debug)]
pub struct FileInfo {
    pub url: String,
    pub filepath: String,
    pub filename: String,
    pub filesize: u64,
    pub overwrite: bool,
    pub supports_range: bool,
}

/// 一次大小探测的结果。
///
/// 额外带上 HEAD 的状态码,是为了让单线程路径复用它来完成 `validate_url()` 的判定,
/// 而不必再发一个一模一样的 HEAD —— 判定逻辑(`status < 400`)完全相同。
#[derive(Clone, Debug, Default)]
pub struct SizeProbe {
    pub size: u64,
    /// HEAD 拿到的状态码;请求本身就失败时为 `None`。
    pub head_status: Option<u16>,
    /// 远端资源的校验标识,用于断点续传时判断「还是不是同一份文件」。
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// 取出续传用的校验头。两个都可能缺失 —— 很多对象存储只给其中一个。
fn validators(response: &reqwest::Response) -> (Option<String>, Option<String>) {
    let header = |name: reqwest::header::HeaderName| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    (
        header(reqwest::header::ETAG),
        header(reqwest::header::LAST_MODIFIED),
    )
}

/// 复刻 `Downloader.__get_size()`:先 HEAD,拿不到正数大小再回退到流式 GET。
///
/// 任何失败都折叠成 `size = 0`,与 Python 一致 —— 调用方据此退回单线程。
pub async fn probe_size(transport: &Transport, url: &str) -> SizeProbe {
    let retries = transport.config().max_retries;
    let empty = HeaderMap::new();
    let mut probe = SizeProbe::default();

    match transport.send(Method::HEAD, url, retries, &empty).await {
        Ok(response) => {
            let status = response.status();
            probe.head_status = Some(status.as_u16());
            if status.is_success() {
                (probe.etag, probe.last_modified) = validators(&response);
                let size = response.content_length().unwrap_or(0);
                // 注意是 `> 0` 才认:HEAD 不带 Content-Length 时要继续试 GET,
                // 这正是 Python 那段 `if size > 0: return size` 的语义。
                if size > 0 {
                    probe.size = size;
                    return probe;
                }
            } else {
                log::warn!("HEAD {} 返回 {status},改用 GET 探测大小", safe_url(url));
            }
        }
        Err(error) => log::warn!("HEAD {} 失败: {error},改用 GET 探测大小", safe_url(url)),
    }

    // 流式 GET:只读响应头,Response 一旦 drop 连接就断,不会真的拉完整个文件。
    match transport.send(Method::GET, url, retries, &empty).await {
        Ok(response) if response.status().is_success() => {
            (probe.etag, probe.last_modified) = validators(&response);
            probe.size = response.content_length().unwrap_or(0);
        }
        Ok(response) => log::error!(
            "探测 {} 文件大小失败: 状态码 {}",
            safe_url(url),
            response.status()
        ),
        Err(error) => log::error!("探测 {} 文件大小失败: {error}", safe_url(url)),
    }

    probe
}

/// 复刻 `MultiDownloader.check_available()`:发 `Range: bytes=0-100`,仅当 206 才算支持。
pub async fn probe_range_support(transport: &Transport, url: &str) -> bool {
    let mut extra = HeaderMap::new();
    extra.insert(RANGE, HeaderValue::from_static("bytes=0-100"));

    match transport
        .send(Method::GET, url, transport.config().max_retries, &extra)
        .await
    {
        Ok(response) => {
            let status = response.status();
            if status.as_u16() != 206 {
                log::warn!("Range 探测返回状态 {status},按不支持处理");
            }
            status.as_u16() == 206
        }
        Err(error) => {
            log::warn!("Range 支持探测失败: {error}");
            false
        }
    }
}

/// 复刻 `Downloader.validate_url()`:HEAD 且状态码 `< 400`。
pub async fn validate_url(transport: &Transport, url: &str) -> bool {
    match transport
        .send(
            Method::HEAD,
            url,
            transport.config().max_retries,
            &HeaderMap::new(),
        )
        .await
    {
        Ok(response) => response.status().as_u16() < 400,
        Err(error) => {
            // 脱敏:异常信息里常常原样带着请求 URL。
            log::warn!("URL 校验失败 {}: {error}", safe_url(url));
            false
        }
    }
}

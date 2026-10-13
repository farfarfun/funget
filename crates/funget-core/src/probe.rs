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
    /// 跟随重定向之后的最终地址;没有发生重定向时为 `None`。
    pub final_url: Option<String>,
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
    /// 跟随重定向之后的最终地址,**只在流式 GET 回退分支里才会填**。
    ///
    /// 不记 HEAD 的最终地址是刻意的:有些 S3 兼容网关对 HEAD 和 GET 给的重定向
    /// 目标不同(只给 GET 签名),拿 HEAD 的结果去打分块会 404。
    pub final_url: Option<String>,
}

impl SizeProbe {
    /// 后续请求该打的地址:探测时跟随过重定向就用解析后的那个,否则用原始的。
    pub fn request_url<'a>(&'a self, url: &'a str) -> &'a str {
        self.final_url.as_deref().unwrap_or(url)
    }
}

/// Range 支持探测的结果。
///
/// 带上最终地址是这个结构体存在的全部理由:并发分块必须**全部打同一个解析后的
/// 地址**,否则 N 个 worker 就是 N 条各自独立的重定向链 —— 多余往返、签名可能
/// 中途过期,理论上还会被分流到不同后端拼出坏文件。这次探测本来就要发,而且发的
/// 是 GET(与分块请求同方法),是最合适的解析点。
#[derive(Clone, Debug, Default)]
pub struct RangeProbe {
    pub supports: bool,
    /// 跟随重定向之后的最终地址;没有发生重定向、或探测失败时为 `None`。
    pub final_url: Option<String>,
}

/// 取响应的最终地址,**仅当它与请求地址不同时**才返回 `Some`。
///
/// 只在真的发生了重定向时才返回,是为了让「没有重定向」这条路径上的后续请求用的
/// 还是调用方原本那个字符串 —— `Url` 会做归一化(补末尾斜杠、重新百分号编码),
/// 无条件覆盖等于悄悄改掉所有人的请求地址。
fn redirected_to(response: &reqwest::Response, requested: &str) -> Option<String> {
    let final_url = response.url().as_str();
    (final_url != requested).then(|| final_url.to_string())
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
            probe.final_url = redirected_to(&response, url);
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
///
/// 顺便把重定向跟随完的地址带出来给分块请求用,见 [`RangeProbe`]。
pub async fn probe_range_support(transport: &Transport, url: &str) -> RangeProbe {
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
            RangeProbe {
                supports: status.as_u16() == 206,
                // 即使不支持 Range 也要把地址带出来:那条路径会退回单块下载,
                // 同样该打解析后的地址。
                final_url: redirected_to(&response, url),
            }
        }
        Err(error) => {
            log::warn!("Range 支持探测失败: {error}");
            RangeProbe::default()
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

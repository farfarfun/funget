use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Method, RequestBuilder, Response};

use crate::error::{Error, Result};
use crate::url::safe_url;

/// Basic 认证凭据。
///
/// Python 版的参数类型标注写的是 `HTTPDigestAuth`,但全组织的调用点只传过
/// `(user, pass)` 元组 —— requests 收到元组走的就是 Basic。Digest 已确认无人使用,
/// 这一版不再支持(见 CHANGELOG 的「有意偏离」)。
#[derive(Clone, Debug)]
pub struct Auth {
    pub username: String,
    pub password: String,
}

/// 与 Python 侧 `urllib3.Retry(status_forcelist=...)` 保持一致的可重试状态码。
const RETRY_STATUS: [u16; 5] = [429, 500, 502, 503, 504];

#[derive(Clone, Debug)]
pub struct HttpConfig {
    pub headers: HeaderMap,
    pub auth: Option<Auth>,
    /// 连接超时与**单次读**超时。
    pub timeout: Duration,
    pub max_retries: u32,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            headers: HeaderMap::new(),
            auth: None,
            timeout: Duration::from_secs(30),
            max_retries: 3,
        }
    }
}

/// 把字符串键值对转成 `HeaderMap`,非法条目跳过并告警而不是整体失败 ——
/// Python 侧 `headers` 是任意 dict,下游传进来什么都有可能。
pub fn headers_from_pairs<I, K, V>(pairs: I) -> HeaderMap
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    let mut map = HeaderMap::new();
    for (key, value) in pairs {
        let (key, value) = (key.as_ref(), value.as_ref());
        match (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            (Ok(name), Ok(value)) => {
                map.insert(name, value);
            }
            // 只报键名:请求头的值里经常是 Authorization / Cookie 这类凭据。
            _ => log::warn!("忽略非法请求头: {key}"),
        }
    }
    map
}

/// 持有 `reqwest::Client` 与一次调用的公共配置。
#[derive(Clone)]
pub struct Transport {
    client: Client,
    config: HttpConfig,
}

impl Transport {
    pub fn new(config: HttpConfig) -> Result<Self> {
        // 关键:用 connect_timeout + read_timeout,**不要**用 `timeout`。
        // reqwest 的 `timeout` 是整个请求(含响应体)的总时长,而 Python 侧
        // requests 的 `timeout=30` 是连接与单次读的超时。用总时长会把一切
        // 耗时超过 30 秒的大文件下载拦腰砍断 —— 那是实打实的功能退化。
        let client = Client::builder()
            .connect_timeout(config.timeout)
            .read_timeout(config.timeout)
            .build()?;
        Ok(Self { client, config })
    }

    pub fn config(&self) -> &HttpConfig {
        &self.config
    }

    /// 合并请求头。
    ///
    /// 顺序刻意与 Python 一致:`headers = {专属头}` 再 `headers.update(self.headers)`,
    /// 即**调用方传入的 headers 覆盖 Range 这类专属头**(`download/multi.py:206`、
    /// `download/work.py:100`)。反过来写会让显式传 Range 的下游行为改变。
    pub fn merge_headers(&self, extra: &HeaderMap) -> HeaderMap {
        let mut merged = extra.clone();
        for (name, value) in self.config.headers.iter() {
            merged.insert(name.clone(), value.clone());
        }
        merged
    }

    /// 构造一个已套用请求头与认证的请求。
    pub fn request(&self, method: Method, url: &str, extra: &HeaderMap) -> RequestBuilder {
        let mut builder = self
            .client
            .request(method, url)
            .headers(self.merge_headers(extra));
        if let Some(auth) = &self.config.auth {
            builder = builder.basic_auth(&auth.username, Some(&auth.password));
        }
        builder
    }

    /// 发请求并按需重试。
    ///
    /// `retries` 由调用方给而不取 `config.max_retries`,因为 Python 侧两条路径的
    /// 重试策略本就不同:元数据探测与单线程下载挂了带 `urllib3.Retry` 的 session,
    /// 而 `Worker` 用的是**裸** session(`download/work.py:47`),靠自己的循环重试。
    ///
    /// 退避为 `2^attempt` 秒,两边一致:`urllib3` 的 `backoff_factor=1` 得到 1/2/4,
    /// `Worker` 的 `time.sleep(2**attempt)` 也是 1/2/4。
    pub async fn send(
        &self,
        method: Method,
        url: &str,
        retries: u32,
        extra: &HeaderMap,
    ) -> Result<Response> {
        let mut last: Option<Error> = None;

        for attempt in 0..=retries {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(1u64 << (attempt - 1))).await;
            }

            let result = async {
                let response = self.request(method.clone(), url, extra).send().await?;
                let status = response.status().as_u16();
                if RETRY_STATUS.contains(&status) {
                    return Err(Error::Status { status });
                }
                Ok(response)
            }
            .await;

            match result {
                Ok(response) => return Ok(response),
                Err(error) => {
                    log::warn!(
                        "{method} {} 第 {} 次失败: {error}",
                        safe_url(url),
                        attempt + 1
                    );
                    last = Some(error);
                }
            }
        }

        Err(last.unwrap_or_else(|| Error::invalid("重试次数耗尽但没有记录到错误")))
    }
}

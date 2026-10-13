use std::path::PathBuf;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Certificate, Client, Method, Proxy, RequestBuilder, Response};

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

/// 传输层设置:TLS 信任源、重定向策略、代理、User-Agent。下载与上传共用。
///
/// 单独成一个结构体,而不是把这些字段摊进 [`HttpConfig`]:`UploadOptions` 的字段是
/// 扁平的,而且它**刻意不复用** `HttpConfig`(上传的 `Content-Length` /
/// `Content-Type` 必须覆盖用户请求头,优先级与下载相反,见 `upload.rs` 里的注释)。
/// 摊开写就得在两个结构体里各列一遍,以后加新旋钮要改两处;包成一个结构体则两边
/// 各加一个字段,新旋钮只写一次。
#[derive(Clone, Debug)]
pub struct TransportOptions {
    /// 跳过证书链与主机名校验。对应 curl 的 `-k`。
    pub insecure: bool,
    /// 额外信任的 CA 证书(PEM,可以是多证书 bundle)。对应 curl 的 `--cacert`。
    ///
    /// 是**追加**而不是替换内置根:给了一张私有 CA 之后公网地址还得照常能连。
    pub ca_certs: Vec<PathBuf>,
    /// 是否使用系统证书库。
    ///
    /// 默认 `true`。留这个开关是为了兜住一种边缘情况:reqwest 在「系统证书库里有
    /// 证书但 rustls 一张都解析不了」时会让 `Client` **构造失败**(截断过的
    /// `ca-certificates` bundle、某些 musl 镜像)。这种机器关掉它就退回只用内置的
    /// webpki 根,也就是 1.1.71 的行为。
    pub native_certs: bool,
    /// 是否跟随 3xx。
    ///
    /// 默认 `true` —— 这是 1.1.71 起的既有行为(reqwest 的默认策略),与 wget 一致。
    /// curl 相反,默认**不**跟随、要显式 `-L`。
    pub follow_redirects: bool,
    /// 跟随上限,对应 curl 的 `--max-redirs`。默认 10,与 reqwest 的默认值一致。
    pub max_redirects: usize,
    /// 代理地址,支持 `http` / `https` / `socks5` / `socks5h`。
    ///
    /// 为 `None` 时沿用 reqwest 的默认行为:读 `HTTP_PROXY` / `HTTPS_PROXY` /
    /// `NO_PROXY` 环境变量 —— 与 curl 相同,所以这一项通常不用显式设。
    pub proxy: Option<String>,
    /// User-Agent。为 `None` 时**不发**这个头。
    ///
    /// 默认 `None` 而不是某个 `funget/x.y`:那四个冻结函数的线上行为必须逐字节
    /// 不变,悄悄多发一个请求头不属于「不变」。CLI 会自己设上(见 `crates/funget`)。
    pub user_agent: Option<String>,
}

impl Default for TransportOptions {
    fn default() -> Self {
        Self {
            insecure: false,
            ca_certs: Vec::new(),
            native_certs: true,
            follow_redirects: true,
            max_redirects: 10,
            proxy: None,
            user_agent: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct HttpConfig {
    pub headers: HeaderMap,
    pub auth: Option<Auth>,
    /// 连接超时与**单次读**超时。
    pub timeout: Duration,
    pub max_retries: u32,
    pub transport: TransportOptions,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            headers: HeaderMap::new(),
            auth: None,
            timeout: Duration::from_secs(30),
            max_retries: 3,
            transport: TransportOptions::default(),
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
        let options = &config.transport;

        // 关键:用 connect_timeout + read_timeout,**不要**用 `timeout`。
        // reqwest 的 `timeout` 是整个请求(含响应体)的总时长,而 Python 侧
        // requests 的 `timeout=30` 是连接与单次读的超时。用总时长会把一切
        // 耗时超过 30 秒的大文件下载拦腰砍断 —— 那是实打实的功能退化。
        let mut builder = Client::builder()
            .connect_timeout(config.timeout)
            .read_timeout(config.timeout)
            .redirect(if options.follow_redirects {
                reqwest::redirect::Policy::limited(options.max_redirects)
            } else {
                reqwest::redirect::Policy::none()
            })
            // reqwest 的 `NoVerifier` 连主机名一起放过,所以这一个调用就等于
            // curl `-k` 的全部效果,不需要再配 `danger_accept_invalid_hostnames`。
            .danger_accept_invalid_certs(options.insecure)
            .tls_built_in_native_certs(options.native_certs);

        if let Some(user_agent) = &options.user_agent {
            builder = builder.user_agent(user_agent.as_str());
        }

        for path in &options.ca_certs {
            let pem = std::fs::read(path).map_err(|error| {
                Error::invalid(format!("读取 CA 证书 {} 失败: {error}", path.display()))
            })?;
            // 用 from_pem_bundle 而不是 from_pem:`--cacert` 指过来的常常是一整条
            // 证书链(企业 CA 多半是「根 + 中间」两张),只收第一张会白白失败。
            for cert in Certificate::from_pem_bundle(&pem)? {
                builder = builder.add_root_certificate(cert);
            }
        }

        if let Some(proxy) = &options.proxy {
            builder = builder.proxy(Proxy::all(proxy.as_str())?);
        }

        Ok(Self {
            client: builder.build()?,
            config,
        })
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

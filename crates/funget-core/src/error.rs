use std::io;

/// 引擎内部错误。
///
/// 对外的四个公开函数一律把错误折叠成 `false` 并记一条日志(这是冻结 API 的契约之一),
/// 所以这个类型只服务于 crate 内部与 CLI 的诊断输出。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("HTTP 请求失败: {0}")]
    Http(reqwest::Error),

    #[error("文件操作失败: {0}")]
    Io(#[from] io::Error),

    /// 服务端返回了不可接受的状态码。
    #[error("服务端返回状态码 {status}")]
    Status { status: u16 },

    /// 参数非法或前置条件不满足,未发出任何网络请求。
    #[error("{0}")]
    Invalid(String),
}

impl Error {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Error::Invalid(message.into())
    }
}

impl From<reqwest::Error> for Error {
    /// 转换时就把 reqwest 自带的那份 URL 摘掉。
    ///
    /// `reqwest::Error` 的 Display 会在末尾拼一句 `for url (…)`。userinfo 它自己会
    /// 抹掉,但 **query 原样保留** —— 而下载链接的 query 里常常就是签名或临时 token
    /// (fundrive 的 zenodo / tianchi 驱动都这么传)。日志里的 URL 一律走
    /// [`safe_url`](crate::safe_url),所以错误自带的那份必须在入口处丢掉,否则任何一处
    /// `log::warn!("…: {error}")` 都会把 query 泄回日志。
    ///
    /// 放在 `From` 里而不是各个打日志的地方,是因为这种疏漏只会少不会多:新增一个
    /// `?` 传播点不需要记得做任何事。
    fn from(error: reqwest::Error) -> Self {
        Error::Http(error.without_url())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::Error;

    /// 回归:`{error}` 不得把 query 带进日志。
    ///
    /// 对照 Python 版 `request_error_context()` —— 上游 ada6519 修的就是同一类泄漏。
    #[tokio::test]
    async fn http_error_does_not_echo_the_query_string() {
        // 127.0.0.1:1 必定连不上,且不走 DNS,在任何环境下都是确定性的立即失败。
        let url = "http://alice:s3cret@127.0.0.1:1/file.bin?token=TOPSECRET&sig=DEADBEEF";
        let raw = reqwest::Client::new().get(url).send().await.unwrap_err();
        let rendered = Error::from(raw).to_string();

        for leaked in ["alice", "s3cret", "TOPSECRET", "DEADBEEF", "token", "sig"] {
            assert!(
                !rendered.contains(leaked),
                "错误文本里泄漏了 {leaked}: {rendered}"
            );
        }
        // 摘掉 URL 不能把错误本身摘没了,否则日志就没有诊断价值了。
        assert!(rendered.contains("HTTP 请求失败"), "{rendered}");
    }
}

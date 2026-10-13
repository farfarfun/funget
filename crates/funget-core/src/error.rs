use std::io;

/// 引擎内部错误。
///
/// 对外的四个公开函数一律把错误折叠成 `false` 并记一条日志(这是冻结 API 的契约之一),
/// 所以这个类型只服务于 crate 内部与 CLI 的诊断输出。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("HTTP 请求失败: {0}")]
    Http(#[from] reqwest::Error),

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

pub type Result<T> = std::result::Result<T, Error>;

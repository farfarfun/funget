//! funget 的下载/上传引擎。
//!
//! 这个 crate 不依赖 Python,既被 PyO3 扩展(`funget-py`)使用,也被原生 CLI
//! (`funget-cli`)使用,保证两个分发渠道背后只有一套逻辑。
//!
//! 对外的四个入口与 Python 版一一对应,且**永不 panic、永不返回错误**,
//! 一律折叠成 `bool`(契约 5)。需要细节的调用方请看日志。

pub mod client;
pub mod download;
pub mod error;
pub mod io;
pub mod probe;
pub mod progress;
pub mod resume;
pub mod upload;
pub mod url;

use std::sync::OnceLock;

use tokio::runtime::Runtime;

pub use client::{headers_from_pairs, Auth, HttpConfig};
pub use download::{
    download_auto, download_multi, download_single, DownloadOptions, MULTI_THREAD_THRESHOLD,
};
pub use error::{Error, Result};
pub use probe::FileInfo;
pub use upload::{upload, UploadOptions};
pub use url::safe_url;

/// 进程内共享的 tokio 运行时。
///
/// 必须复用而不是每次调用都建一个:下游(fundrive 等)会在循环里连续调几十次下载,
/// 每次新建运行时意味着反复创建销毁线程池,而且 `reqwest` 的连接池也跟着作废,
/// 等于把 keep-alive 彻底浪费掉。
///
/// 存的是 `Result` 而不是直接 unwrap:运行时创建失败(比如线程配额耗尽)时
/// 要能让调用方返回 `false`,而不是 panic —— PyO3 里的 panic 会变成
/// `PanicException`,直接违反「四个函数永不抛异常」的契约。
static RUNTIME: OnceLock<std::result::Result<Runtime, String>> = OnceLock::new();

/// 取全局运行时。
pub fn runtime() -> Result<&'static Runtime> {
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("funget")
                .build()
                .map_err(|error| format!("创建 tokio 运行时失败: {error}"))
        })
        .as_ref()
        .map_err(|error| Error::invalid(error.clone()))
}

/// 查询远端文件信息,供 `funget info` 使用。
///
/// 这是唯一一个会把错误交回调用方的入口 —— 它不在那四个冻结函数之列,
/// CLI 需要据此给出非零退出码。
pub fn file_info(url: &str, filepath: &str, options: &DownloadOptions) -> Result<FileInfo> {
    let runtime = runtime()?;
    let transport = client::Transport::new(options.http.clone())?;

    runtime.block_on(async move {
        let probe = probe::probe_size(&transport, url).await;
        let supports_range = probe::probe_range_support(&transport, url).await;
        Ok(FileInfo {
            url: url.to_string(),
            filepath: filepath.to_string(),
            filename: std::path::Path::new(filepath)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            filesize: probe.size,
            overwrite: options.overwrite,
            supports_range,
        })
    })
}

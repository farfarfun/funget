mod multi;
mod single;

pub use multi::{blocks_num, ranges, MIN_BLOCK_SIZE_MIB};

use crate::client::{HttpConfig, Transport};
use crate::error::Result;
use crate::probe::probe_size;
use crate::runtime;

/// 自动选择多线程的大小门槛。
///
/// 判定是**严格大于** 10 MiB(`download/common.py:59`)—— 不是 `>=`。
/// 恰好 10 MiB 的文件走单线程,Python 测试对这个边界有显式断言。
pub const MULTI_THREAD_THRESHOLD: u64 = 10 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct DownloadOptions {
    pub overwrite: bool,
    /// 进度条前缀。
    pub prefix: String,
    /// 单线程路径每次读取的字节数。
    pub chunk_size: usize,
    /// 多线程路径的并发数。
    pub worker_num: usize,
    /// 多线程路径的目标分块大小,单位 MiB。
    pub block_size: u64,
    pub max_retries: u32,
    pub http: HttpConfig,
    /// 是否绘制进度条(非终端环境下无论如何都不绘制)。
    pub progress: bool,
    /// 是否启用断点续传。
    ///
    /// 默认关闭,因为它是**纯新增**能力:Python 版没有,打开会在目标文件旁边多出一个
    /// `.funget.json` 状态文件,而下游有按目录内容做断言的代码。要用就显式开。
    pub resume: bool,
    /// 调用方已知的文件大小,对应 Python `Downloader.__init__` 的 `filesize` 参数
    /// (`self.filesize = filesize or self.__get_size()`)。
    ///
    /// 给了就完全跳过大小探测。副作用是拿不到 ETag/Last-Modified,续传的指纹
    /// 退化成「URL + 大小 + 分块方式」。要强指纹就别传这个参数。
    pub known_size: Option<u64>,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        // 默认值取自 `download/common.py` 的 `download()`。
        // 注意 `block_size` 有个历史遗留的不一致:顶层函数传 100,
        // 而 `MultiDownloader.__init__` 自己的默认值是 50。顶层函数才是公开契约。
        Self {
            overwrite: false,
            prefix: String::new(),
            chunk_size: 2048,
            worker_num: 5,
            block_size: 100,
            max_retries: 3,
            http: HttpConfig::default(),
            progress: true,
            resume: false,
            known_size: None,
        }
    }
}

/// 取文件大小:调用方给了就直接用,否则探测远端。
async fn resolve_probe(
    transport: &Transport,
    url: &str,
    options: &DownloadOptions,
) -> crate::probe::SizeProbe {
    match options.known_size {
        Some(size) => crate::probe::SizeProbe {
            size,
            ..Default::default()
        },
        None => probe_size(transport, url).await,
    }
}

/// 在全局 tokio 运行时上跑一个返回 `bool` 的任务。
///
/// 运行时创建失败同样折叠成 `false` 而不是 panic —— 四个公开函数永不抛异常是契约 5,
/// 而 PyO3 里的 panic 会变成 Python 侧的 `PanicException`,等于破坏契约。
fn blocking<F>(future: F) -> bool
where
    F: std::future::Future<Output = bool>,
{
    match runtime() {
        Ok(rt) => rt.block_on(future),
        Err(error) => {
            log::error!("{error}");
            false
        }
    }
}

fn transport_or_log(options: &DownloadOptions) -> Result<Transport> {
    Transport::new(options.http.clone())
}

/// 智能下载:`multi` 为 `None` 时按「文件大小 > 10 MiB 且支持 Range」自动选择。
///
/// 对应 Python 的 `funget.download`。
pub fn download_auto(
    url: &str,
    filepath: &str,
    multi: Option<bool>,
    options: &DownloadOptions,
) -> bool {
    blocking(async move {
        let transport = match transport_or_log(options) {
            Ok(transport) => transport,
            Err(error) => {
                log::error!("初始化 HTTP 客户端失败: {error}");
                return false;
            }
        };

        match multi {
            Some(true) => multi::run(&transport, url, filepath, options).await,
            Some(false) => single::run(&transport, url, filepath, None, options).await,
            None => {
                let probe = resolve_probe(&transport, url, options).await;
                let supports_range = crate::probe::probe_range_support(&transport, url).await;
                let use_multi = probe.size > MULTI_THREAD_THRESHOLD && supports_range;

                log::info!(
                    "自动选择{}下载(文件大小 {} 字节,Range 支持: {supports_range})",
                    if use_multi { "多线程" } else { "单线程" },
                    probe.size
                );

                if use_multi {
                    multi::run_prepared(&transport, url, filepath, &probe, supports_range, options)
                        .await
                } else {
                    single::run(&transport, url, filepath, Some(probe), options).await
                }
            }
        }
    })
}

/// 多线程分块下载,对应 Python 的 `funget.multi_thread_download`。
pub fn download_multi(url: &str, filepath: &str, options: &DownloadOptions) -> bool {
    blocking(async move {
        match transport_or_log(options) {
            Ok(transport) => multi::run(&transport, url, filepath, options).await,
            Err(error) => {
                log::error!("初始化 HTTP 客户端失败: {error}");
                false
            }
        }
    })
}

/// 单线程流式下载,对应 Python 的 `funget.simple_download`。
pub fn download_single(url: &str, filepath: &str, options: &DownloadOptions) -> bool {
    blocking(async move {
        match transport_or_log(options) {
            Ok(transport) => single::run(&transport, url, filepath, None, options).await,
            Err(error) => {
                log::error!("初始化 HTTP 客户端失败: {error}");
                false
            }
        }
    })
}

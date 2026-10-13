//! funget 的 Python 扩展模块。
//!
//! 这一层只做三件事:把 Python 的参数翻译成 `funget-core` 的 options、释放 GIL、
//! 把结果翻回 `bool`。所有真正的逻辑都在 core 里,CLI 也走同一套。
//!
//! 四个下载/上传函数的签名与 1.1.70 版逐字对齐(含默认值),下游零改动即可升级。

use std::collections::HashMap;
use std::time::Duration;

use funget_core::{
    download_auto, download_multi, download_single, headers_from_pairs, upload, Auth,
    DownloadOptions, HttpConfig, TransportOptions, UploadOptions,
};
use pyo3::prelude::*;
use pyo3::types::PyModule;

/// Python 侧 `auth` 参数的形态:`(user, password)` 元组或 `None`。
///
/// Python 版的类型标注写的是 `HTTPDigestAuth`,但全组织的调用点只传过元组,
/// requests 收到元组走的就是 Basic。Digest 已确认无人使用,这一版不再支持。
type AuthTuple = Option<(String, String)>;

/// 翻译三个新的 TLS / 代理参数。
///
/// 只开放这三个:它们是**环境决定**的,库的调用方自己说不上来默认值该是什么
/// (私有 CA 的内网 webdav、必须走代理的机器)。重定向策略和
/// `native_certs` 不开放 —— 前者的默认值就是对的,后者是 CLI 的排障逃生口。
///
/// **不设默认 User-Agent**:那四个函数的线上行为必须逐字节冻结,悄悄多发一个请求头
/// 不属于「不变」。真要设的调用方可以 `headers={"User-Agent": ...}`,它会通过
/// `merge_headers` 盖掉客户端默认值。
fn transport_options(
    insecure: bool,
    cacert: Option<String>,
    proxy: Option<String>,
) -> TransportOptions {
    TransportOptions {
        insecure,
        ca_certs: cacert.map(|path| vec![path.into()]).unwrap_or_default(),
        proxy,
        ..TransportOptions::default()
    }
}

fn http_config(
    headers: Option<HashMap<String, String>>,
    timeout: u64,
    max_retries: u32,
    auth: AuthTuple,
    transport: TransportOptions,
) -> HttpConfig {
    HttpConfig {
        headers: headers.map(headers_from_pairs).unwrap_or_default(),
        auth: auth.map(|(username, password)| Auth { username, password }),
        timeout: Duration::from_secs(timeout),
        max_retries,
        transport,
    }
}

/// 智能下载:`multi` 为 `None` 时自动在单/多线程之间选择。
#[pyfunction]
#[pyo3(signature = (
    url,
    filepath,
    multi = None,
    overwrite = false,
    prefix = "",
    chunk_size = 2048,
    worker_num = 5,
    block_size = 100,
    max_retries = 3,
    *,
    filesize = None,
    headers = None,
    timeout = 30,
    auth = None,
    resume = false,
    insecure = false,
    cacert = None,
    proxy = None,
))]
#[allow(clippy::too_many_arguments)]
fn download(
    py: Python<'_>,
    url: &str,
    filepath: &str,
    multi: Option<bool>,
    overwrite: bool,
    prefix: &str,
    chunk_size: usize,
    worker_num: usize,
    block_size: u64,
    max_retries: u32,
    filesize: Option<u64>,
    headers: Option<HashMap<String, String>>,
    timeout: u64,
    auth: AuthTuple,
    resume: bool,
    insecure: bool,
    cacert: Option<String>,
    proxy: Option<String>,
) -> bool {
    let options = DownloadOptions {
        overwrite,
        prefix: prefix.to_string(),
        chunk_size,
        worker_num,
        block_size,
        max_retries,
        http: http_config(
            headers,
            timeout,
            max_retries,
            auth,
            transport_options(insecure, cacert, proxy),
        ),
        progress: true,
        resume,
        known_size: filesize,
    };
    // 整个调用期间都放掉 GIL。这是 PyO3 版相对纯 Python 版最直接的收益来源:
    // 下载是纯 IO + 文件写,全程不碰 Python 对象,调用方的其他线程可以照常跑。
    py.detach(|| download_auto(url, filepath, multi, &options))
}

/// 多线程分块下载。
#[pyfunction]
#[pyo3(signature = (
    url,
    filepath,
    overwrite = false,
    worker_num = 5,
    block_size = 100,
    prefix = "",
    max_retries = 3,
    *,
    filesize = None,
    headers = None,
    timeout = 30,
    auth = None,
    resume = false,
    insecure = false,
    cacert = None,
    proxy = None,
))]
#[allow(clippy::too_many_arguments)]
fn multi_thread_download(
    py: Python<'_>,
    url: &str,
    filepath: &str,
    overwrite: bool,
    worker_num: usize,
    block_size: u64,
    prefix: &str,
    max_retries: u32,
    filesize: Option<u64>,
    headers: Option<HashMap<String, String>>,
    timeout: u64,
    auth: AuthTuple,
    resume: bool,
    insecure: bool,
    cacert: Option<String>,
    proxy: Option<String>,
) -> bool {
    let options = DownloadOptions {
        overwrite,
        prefix: prefix.to_string(),
        worker_num,
        block_size,
        max_retries,
        http: http_config(
            headers,
            timeout,
            max_retries,
            auth,
            transport_options(insecure, cacert, proxy),
        ),
        resume,
        known_size: filesize,
        ..DownloadOptions::default()
    };
    py.detach(|| download_multi(url, filepath, &options))
}

/// 单线程流式下载。
#[pyfunction]
#[pyo3(signature = (
    url,
    filepath,
    overwrite = false,
    prefix = "",
    chunk_size = 2048,
    *,
    filesize = None,
    headers = None,
    timeout = 30,
    max_retries = 3,
    auth = None,
    resume = false,
    insecure = false,
    cacert = None,
    proxy = None,
))]
#[allow(clippy::too_many_arguments)]
fn simple_download(
    py: Python<'_>,
    url: &str,
    filepath: &str,
    overwrite: bool,
    prefix: &str,
    chunk_size: usize,
    filesize: Option<u64>,
    headers: Option<HashMap<String, String>>,
    timeout: u64,
    max_retries: u32,
    auth: AuthTuple,
    resume: bool,
    insecure: bool,
    cacert: Option<String>,
    proxy: Option<String>,
) -> bool {
    let options = DownloadOptions {
        overwrite,
        prefix: prefix.to_string(),
        chunk_size,
        max_retries,
        http: http_config(
            headers,
            timeout,
            max_retries,
            auth,
            transport_options(insecure, cacert, proxy),
        ),
        resume,
        known_size: filesize,
        ..DownloadOptions::default()
    };
    py.detach(|| download_single(url, filepath, &options))
}

/// 上传单个文件。
///
/// `overwrite` 只为对齐下载侧的签名而保留,与 Python 版一样不起作用
/// (`upload/single.py:42` 的 `del overwrite`)。
#[pyfunction]
#[pyo3(signature = (
    url,
    filepath,
    overwrite = false,
    prefix = "",
    // 写字面量而不是 `256 * 1024`:PyO3 生成 __text_signature__ 时不会求值,
    // 表达式会被渲染成 `chunk_size=Ellipsis`,和 1.1.70 的 `inspect.signature`
    // 输出(`262144`)对不上。
    chunk_size = 262144,
    method = "PUT",
    max_retries = 3,
    timeout = 60,
    headers = None,
    auth = None,
    // `*` 放在原有参数之后:上面那些保持「位置或关键字」不变,新增的三个一律是
    // keyword-only,与 1.1.71 的 `resume` 同一先例。
    *,
    insecure = false,
    cacert = None,
    proxy = None,
))]
#[allow(clippy::too_many_arguments)]
fn single_upload(
    py: Python<'_>,
    url: &str,
    filepath: &str,
    overwrite: bool,
    prefix: &str,
    chunk_size: i64,
    method: &str,
    max_retries: i64,
    timeout: i64,
    headers: Option<HashMap<String, String>>,
    auth: AuthTuple,
    insecure: bool,
    cacert: Option<String>,
    proxy: Option<String>,
) -> bool {
    let _ = overwrite;
    let options = UploadOptions {
        prefix: prefix.to_string(),
        chunk_size,
        method: method.to_string(),
        max_retries,
        timeout,
        headers: headers.map(headers_from_pairs).unwrap_or_default(),
        auth: auth.map(|(username, password)| Auth { username, password }),
        progress: true,
        transport: transport_options(insecure, cacert, proxy),
    };
    py.detach(|| upload(url, filepath, &options))
}

/// `[project.scripts] funget` 的入口:把 `sys.argv` 转给原生 CLI。
#[pyfunction]
#[pyo3(signature = (argv))]
fn cli_main(py: Python<'_>, argv: Vec<String>) -> i32 {
    py.detach(|| funget::run(argv))
}

#[pymodule]
fn _funget(module: &Bound<'_, PyModule>) -> PyResult<()> {
    // 把 Rust 的 log 宏接到 Python 的 logging 上,farlog/loguru 都能照常拦截,
    // 下游现有的日志配置不用改。
    pyo3_log::init();

    module.add_function(wrap_pyfunction!(download, module)?)?;
    module.add_function(wrap_pyfunction!(multi_thread_download, module)?)?;
    module.add_function(wrap_pyfunction!(simple_download, module)?)?;
    module.add_function(wrap_pyfunction!(single_upload, module)?)?;
    module.add_function(wrap_pyfunction!(cli_main, module)?)?;
    Ok(())
}

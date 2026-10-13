# funget-core

HTTP 下载/上传引擎:Range 并发分块、断点续传、指数退避重试、原生定位写。

这是 [`funget`](https://crates.io/crates/funget) 命令行工具和 PyPI 上
[`funget`](https://pypi.org/project/funget/) 扩展模块共用的实现层,三条分发渠道背后
只有一套逻辑。

```toml
[dependencies]
funget-core = "0.2"
```

## 用法

```rust
use funget_core::{download_auto, DownloadOptions};

let options = DownloadOptions {
    worker_num: 8,
    block_size: 50, // MiB
    resume: true,
    ..Default::default()
};

// multi = None 时自动选择:超过 10 MiB 且服务端支持 Range 就并发,否则单线程。
let ok = download_auto("https://example.com/huge.iso", "./huge.iso", None, &options);
```

想跳过自动判断,直接用 `download_multi` 或 `download_single`。上传是 `upload` 配
`UploadOptions`,支持 PUT 流式和 multipart POST。

## 设计取舍

**下载/上传函数返回 `bool`,不返回 `Result`。** 这不是疏忽 —— 这套 API 要对 Python
侧「四个公开函数永不抛异常」的冻结契约负责,错误在内部记日志后折叠成 `false`。要拿到
错误详情就用 `file_info`,它是唯一把 `Result` 交回调用方的入口。

**并发写不走队列。** `set_len()` 预分配 + `write_all_at`(unix)/ `seek_write`
(windows),每个分块直接写到自己的偏移,没有中间缓冲、没有协调线程。

**不声明 gzip/deflate。** 本 crate 用解码后的字节数与 `Content-Length` 比对来判断
下载是否完整,压缩传输会让这个判断失效。

**日志里的 URL 一律脱敏。** `safe_url()` 抹掉 user/password/query;`reqwest::Error`
自带的那份 URL 在错误转换入口就被摘掉(它会保留 query,而下载链接的 query 里常常
就是签名或临时 token)。

## 版本

0.x,和 PyPI 上 `funget` 的 1.1.x 是两条独立的版本线。上面那些 `pub mod` 之所以
public 是为了跨 crate 复用,并不是按公共 Rust API 设计的 —— 所以在 0.x 期间它们
还会调整,等用法稳定再发 1.0.0。

MIT。

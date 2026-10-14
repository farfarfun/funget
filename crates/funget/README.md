# funget

HTTP 下载/上传命令行工具:Range 并发分块、断点续传、进度条。单文件二进制,无运行时依赖。

```bash
cargo install funget
```

也可以从 [Releases](https://github.com/farfarfun/funget/releases) 直接下载预编译二进制
(Linux x86_64/aarch64、macOS Intel/Apple Silicon、Windows x64),不需要 Rust 工具链。

## 用法

```bash
funget download https://example.com/file.bin -o file.bin
funget download https://example.com/huge.iso --worker 8 --block-size 50
funget download https://example.com/huge.iso --resume   # 中断后重跑继续
funget upload ./file.bin https://example.com/upload
funget info https://example.com/file.bin
```

`download` 默认自动选路:文件超过 10 MiB 且服务端支持 Range 就并发分块,否则单线程。
`--single` 强制走单线程。

| 选项 | 默认 | 说明 |
| --- | --- | --- |
| `-o, --output` | URL 的 basename | 输出路径 |
| `-w, --worker` | 10 | 并发数 |
| `-b, --block-size` | 100 | 分块大小,单位 MiB |
| `-r, --max-retries` | 3 | 单请求重试次数,退避 `2^n` 秒 |
| `--single` | 关 | 强制单线程 |
| `--overwrite` | 关 | 文件已完整也重新下载 |
| `--resume` | 关 | 断点续传 |

开启 `--resume` 后会在目标文件旁维护 `<文件名>.funget.json` 状态文件,下载成功后自动
删除。远端文件若发生变化(ETag、Last-Modified 或大小不匹配),续传状态会被丢弃并重新
下载,不会拼出一个损坏的文件。

## 相关

| | |
| --- | --- |
| [`funget-core`](https://crates.io/crates/funget-core) | 本工具的引擎,可独立作为 Rust 库使用 |
| [`funget` on PyPI](https://pypi.org/project/funget/) | 同一套引擎的 Python 扩展模块。只有 Python API,**不提供 `funget` 命令** |

本 crate 和 PyPI 上的 `funget` 共用同一个版本号,方便对照「这个命令对应哪个
wheel」。`funget-core` 作为库另走 0.x 线。

MIT。

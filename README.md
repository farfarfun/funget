# funget

HTTP 文件传输工具，提供单线程下载、Range 并发下载、断点续传和 PUT/POST 上传。
核心由 Rust 实现，同时提供 **Python 库**和**独立命令行二进制**两种用法。

## 安装

### 作为 Python 库

```bash
pip install funget
```

需要 Python 3.12 或更高版本。无任何运行时依赖，各平台提供预编译 wheel，
安装时不需要 Rust 工具链。

### 只要命令行

命令行是独立的原生二进制，**不需要 Python 环境**。从
[Releases](https://github.com/farfarfun/funget/releases) 下载对应平台的文件即可：

```bash
curl -L https://github.com/farfarfun/funget/releases/latest/download/funget-x86_64-unknown-linux-gnu -o funget
chmod +x funget && sudo mv funget /usr/local/bin/
```

有 Rust 工具链的话也可以 `cargo install funget`。

`pip install funget` 同样会提供 `funget` 命令，三条渠道共用同一套实现，
`funget --version` 报同一个版本号。

### 作为 Rust 库

下载/上传引擎本身发布在 crates.io 上，可以脱离 Python 和 CLI 单独使用：

```toml
[dependencies]
funget-core = "0.2"
```

crate 的版本线是独立的 0.x —— 它的公开模块当初是为跨 crate 复用而 `pub` 的，
不是按公共 Rust API 设计的，所以在 0.x 期间仍会调整。详见
[`crates/funget-core/README.md`](crates/funget-core/README.md)。

## 命令行

```bash
funget download https://httpbin.org/bytes/1024 -o file.bin
funget download https://httpbin.org/bytes/1024 --worker 8 --block-size 50
funget download https://httpbin.org/bytes/1024 -o file.bin --resume   # 中断后重跑继续
funget upload ./file.bin https://httpbin.org/put
funget info https://httpbin.org/bytes/1024
```

`download`、`upload`、`info` 三个子命令共用一组 HTTP 开关，短选项与 `curl` 对齐：

| 开关 | 说明 |
| --- | --- |
| `-H, --header LINE` | 追加请求头，格式 `Name: value`，可重复 |
| `-u, --user USER:PASSWORD` | Basic 认证。按**第一个**冒号切分，`-u user:` 是空密码 |
| `-A, --user-agent NAME` | 覆盖默认的 `funget/<版本号>` |
| `--timeout SECONDS` | 连接与读取超时。download 默认 30，upload 默认 60 |
| `-x, --proxy URL` | 支持 `http://`、`https://`、`socks5://`、`socks5h://` |
| `-k, --insecure` | 接受无效证书（含主机名不匹配） |
| `--cacert FILE` | **追加**信任的 PEM 证书，可重复；一个文件里放整条证书链也可以 |
| `--no-native-certs` | 不读系统证书库，只用内置的 webpki 根证书 |
| `--no-location` | 不跟随重定向。funget **默认跟随**，所以这里是反向开关（curl 是正向的 `-L`） |
| `--max-redirs N` | 最多跟随多少跳，默认 10 |

`download` 和 `upload` 另有 `-s, --silent` 关掉进度条（非 TTY 环境本来就不画）。
不传 `-x` 时沿用 `HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY` 环境变量，与 curl 一致。

运行 `funget --help` 或 `funget <command> --help` 查看完整参数。

### 证书

TLS 信任的是**内置 webpki 根证书与系统证书库的并集**，所以企业私有 CA、内网自签
证书这类场景只要机器上装好了就能直接用。极少数机器的系统证书库 rustls 一张都解析
不了（截断过的 `ca-certificates` bundle 等），客户端会构造失败，用
`--no-native-certs` 退回只用内置根证书。

## Python API

`download()` 会先检查文件大小和 Range 支持：超过 10 MiB 且服务器支持 Range 时并发下载，否则使用单线程。

```python
from funget import download

ok = download(
    "https://httpbin.org/bytes/1024",
    "./downloads/file.bin",
    overwrite=True,
)
```

也可以显式选择下载方式：

```python
from funget import multi_thread_download, simple_download

simple_download(
    "https://httpbin.org/bytes/1024", "./downloads/small.bin", chunk_size=64 * 1024
)
multi_thread_download(
    "https://httpbin.org/bytes/1024",
    "./downloads/large.bin",
    worker_num=8,
    block_size=50,
)
```

上传支持 PUT 和 multipart POST：

```python
from pathlib import Path
from tempfile import TemporaryDirectory

from funget import single_upload

with TemporaryDirectory() as directory:
    filepath = Path(directory) / "example-upload.txt"
    filepath.write_text("funget upload example\n", encoding="utf-8")

    single_upload("https://httpbin.org/put", str(filepath), method="PUT")
    single_upload("https://httpbin.org/post", str(filepath), method="POST")
```

该示例使用公开测试服务，运行时需要网络连接。生产环境请使用接受相应
`PUT` 或 multipart `POST` 请求的上传地址。

### 断点续传

传 `resume=True` 后，中断的下载在下次调用时从断点继续，单线程和并发两条路径都支持：

```python
from funget import download

download("https://example.com/huge.iso", "./huge.iso", resume=True)
```

续传期间会在目标文件旁维护 `huge.iso.funget.json` 状态文件，下载成功后自动删除。
远端文件若发生变化（ETag、Last-Modified 或大小不匹配），续传状态会被丢弃并重新下载，
不会拼出一个损坏的文件。默认关闭，不开启时行为与旧版完全一致。

### 参数

所有公开 API 均返回 `bool`，**不抛异常**。常用参数：

| 参数 | 单位 | 说明 |
| --- | --- | --- |
| `worker_num` | 个 | 并发下载的分块并发数 |
| `block_size` | MiB | 多线程下载分块大小 |
| `chunk_size` | 字节 | 单线程下载或上传读取块大小 |
| `max_retries` | 次 | 失败后的最大重试次数，退避 `2^n` 秒 |
| `timeout` | 秒 | 单次读取超时（不是整体下载超时） |
| `headers` | - | 自定义 HTTP 请求头 |
| `auth` | - | `(用户名, 密码)` 二元组，走 Basic 认证 |
| `resume` | - | 是否启用断点续传，默认 `False` |
| `filesize` | 字节 | 已知文件大小，传了就跳过 HEAD 探测 |
| `insecure` | - | 接受无效证书，默认 `False`。对应 CLI 的 `-k` |
| `cacert` | - | 追加信任的 PEM 证书路径，对应 CLI 的 `--cacert` |
| `proxy` | - | 代理地址，不传则读 `HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY` |

`insecure`、`cacert`、`proxy` 与 `resume`、`filesize` 一样只能按关键字传
（keyword-only）—— 原有签名是冻结的，新能力一律往后追加。完整签名见
[`src/funget/__init__.pyi`](src/funget/__init__.pyi)。

重定向默认跟随，最多 10 跳；Python 侧不开放这两个旋钮，CLI 的 `--no-location` /
`--max-redirs` 是排障用的。

---

## 关于 farfarfun

[farfarfun](https://github.com/farfarfun) 是一个专注于实用工具库的开源组织，
涵盖云存储、数据处理、AI、多媒体与开发工具链等方向。

- 🏠 组织主页：<https://github.com/farfarfun>
- 📦 PyPI：<https://pypi.org/user/niuliangtao/>
- 📧 联系：farfarfun@qq.com

本项目基于 [MIT](LICENSE) 协议开源。

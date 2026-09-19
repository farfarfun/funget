# funget

轻量的 Python HTTP 文件传输工具，提供单线程下载、Range 并发下载、PUT/POST 上传、断点续传和命令行接口。

## 安装

```bash
pip install funget
```

需要 Python 3.12 或更高版本。

## 命令行

```bash
funget download https://example.com/file.zip -o file.zip
funget download https://example.com/large.zip --worker 8 --block-size 50
funget upload ./file.zip https://upload.example.com/file.zip
funget info https://example.com/file.zip
```

运行 `funget --help` 或 `funget <command> --help` 查看完整参数。

## Python API

`download()` 会先检查文件大小和 Range 支持：超过 10 MiB 且服务器支持 Range 时并发下载，否则使用单线程。

```python
from funget import download

ok = download(
    "https://example.com/file.zip",
    "./downloads/file.zip",
    overwrite=True,
)
```

也可以显式选择下载方式：

```python
from funget import multi_thread_download, simple_download

simple_download(url, filepath, chunk_size=64 * 1024)
multi_thread_download(url, filepath, worker_num=8, block_size=50)
```

上传支持 PUT 和 multipart POST：

```python
from funget import single_upload

single_upload(upload_url, filepath, method="PUT")
single_upload(upload_url, filepath, method="POST")
```

所有公开 API 均返回 `bool`。常用参数：

| 参数 | 单位 | 说明 |
| --- | --- | --- |
| `worker_num` | 个 | 并发下载线程数 |
| `block_size` | MiB | 多线程下载分块大小 |
| `chunk_size` | 字节 | 单线程下载或上传读取块大小 |
| `max_retries` | 次 | 失败后的最大重试次数 |
| `timeout` | 秒 | HTTP 请求超时 |
| `headers` | - | 自定义 HTTP 请求头 |
| `auth` | - | `requests` 认证对象，包括 Digest 认证 |

---

## 关于 farfarfun

[farfarfun](https://github.com/farfarfun) 是一个专注于实用工具库的开源组织，
涵盖云存储、数据处理、AI、多媒体与开发工具链等方向。

- 🏠 组织主页：<https://github.com/farfarfun>
- 📦 PyPI：<https://pypi.org/user/niuliangtao/>
- 📧 联系：farfarfun@qq.com

本项目基于 [MIT](LICENSE) 协议开源。

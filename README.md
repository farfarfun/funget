# Funget

一个高效的 Python 文件下载和上传工具，支持多线程下载和断点续传功能。

## 特性

- 🚀 **多线程下载**: 支持多线程并发下载，提高下载速度
- 📁 **断点续传**: 支持断点续传，避免重复下载
- 🔧 **灵活配置**: 可自定义线程数、块大小等参数
- 📤 **文件上传**: 支持文件上传功能
- 🖥️ **命令行工具**: 提供便捷的命令行接口
- 📦 **Python API**: 提供完整的 Python API 接口

## 安装

```bash
pip install funget
```

## 快速开始

### 命令行使用

```bash
# 基本下载
funget https://example.com/file.zip

# 自定义参数
funget https://example.com/file.zip --worker 20 --block-size 200 --capacity 150
```

### Python API

#### 下载文件

```python
from funget import multi_thread_download, simple_download

# 多线程下载（推荐）
multi_thread_download(
    url="https://example.com/file.zip",
    filepath="./downloads/file.zip",
    worker_num=10,
    block_size=100,
    capacity=100
)

# 简单下载
simple_download(
    url="https://example.com/file.zip",
    filepath="./downloads/file.zip"
)
```

#### 上传文件

```python
from funget import single_upload

# 上传文件
single_upload(
    file_path="./local_file.txt",
    upload_url="https://upload.example.com/api"
)
```

## API 参考

### 下载函数

#### `multi_thread_download(url, filepath=None, worker_num=10, block_size=100, capacity=100)`

多线程下载文件

**参数:**
- `url` (str): 下载链接
- `filepath` (str, optional): 保存路径，默认为当前目录下的文件名
- `worker_num` (int): 工作线程数，默认 10
- `block_size` (int): 每个块的大小（KB），默认 100
- `capacity` (int): 队列容量，默认 100

#### `simple_download(url, filepath=None)`

单线程下载文件

**参数:**
- `url` (str): 下载链接
- `filepath` (str, optional): 保存路径

#### `download(url, filepath=None, **kwargs)`

通用下载函数，自动选择最佳下载方式

### 上传函数

#### `single_upload(file_path, upload_url, **kwargs)`

上传单个文件

**参数:**
- `file_path` (str): 本地文件路径
- `upload_url` (str): 上传接口地址

## 命令行参数

```bash
funget [URL] [OPTIONS]
```

**选项:**
- `--worker INTEGER`: 工作线程数（默认: 10）
- `--block-size INTEGER`: 块大小，单位 KB（默认: 100）
- `--capacity INTEGER`: 队列容量（默认: 100）

## 使用示例

### 下载大文件

```python
from funget import multi_thread_download

# 下载大文件，使用 20 个线程
multi_thread_download(
    url="https://releases.ubuntu.com/20.04/ubuntu-20.04.6-desktop-amd64.iso",
    filepath="./ubuntu.iso",
    worker_num=20,
    block_size=1024  # 1MB 块大小
)
```

### 批量下载

```python
from funget import multi_thread_download

urls = [
    "https://example.com/file1.zip",
    "https://example.com/file2.zip",
    "https://example.com/file3.zip"
]

for i, url in enumerate(urls):
    print(f"下载文件 {i+1}/{len(urls)}")
    multi_thread_download(url, filepath=f"./downloads/file_{i+1}.zip")
```

## 依赖

- Python >= 3.7
- funfile >= 1.0.15
- typer-slim >= 0.15.2

## 许可证

本项目基于 MIT 许可证开源。

## 作者

- **niuliangtao** - [farfarfun@qq.com](mailto:farfarfun@qq.com)

## 贡献

欢迎提交 Issue 和 Pull Request 来改进这个项目！

## 更新日志

### v1.0.46
- 当前稳定版本
- 支持多线程下载和文件上传
- 提供命令行工具和 Python API

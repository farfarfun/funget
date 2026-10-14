"""funget —— 并发 HTTP 下载与上传。

本模块只做再导出。所有实现都在 Rust 扩展 ``funget._funget`` 里,Python 侧
零运行时依赖,也没有任何业务逻辑 —— 不要在这个文件里加分支或包装,
那会让 PyO3 的签名和实际行为对不上。

这个包只提供 Python API,不提供 ``funget`` 命令。命令行走 apt / brew /
cargo install / Release 里的二进制,见 README。
"""

from ._funget import (
    download,
    multi_thread_download,
    simple_download,
    single_upload,
)

__all__ = [
    "download",
    "multi_thread_download",
    "simple_download",
    "single_upload",
]

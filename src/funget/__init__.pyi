__all__ = [
    "download",
    "multi_thread_download",
    "simple_download",
    "single_upload",
]

def download(
    url: str,
    filepath: str,
    multi: bool | None = None,
    overwrite: bool = False,
    prefix: str = "",
    chunk_size: int = 2048,
    worker_num: int = 5,
    block_size: int = 100,
    max_retries: int = 3,
    *,
    filesize: int | None = None,
    headers: dict[str, str] | None = None,
    timeout: int = 30,
    auth: tuple[str, str] | None = None,
    resume: bool = False,
) -> bool:
    """智能下载:``multi`` 为 ``None`` 时按「大小 > 10 MiB 且支持 Range」自动选择。"""

def multi_thread_download(
    url: str,
    filepath: str,
    overwrite: bool = False,
    worker_num: int = 5,
    block_size: int = 100,
    prefix: str = "",
    max_retries: int = 3,
    *,
    filesize: int | None = None,
    headers: dict[str, str] | None = None,
    timeout: int = 30,
    auth: tuple[str, str] | None = None,
    resume: bool = False,
) -> bool:
    """多线程分块下载。``block_size`` 单位是 MiB。"""

def simple_download(
    url: str,
    filepath: str,
    overwrite: bool = False,
    prefix: str = "",
    chunk_size: int = 2048,
    *,
    filesize: int | None = None,
    headers: dict[str, str] | None = None,
    timeout: int = 30,
    max_retries: int = 3,
    auth: tuple[str, str] | None = None,
    resume: bool = False,
) -> bool:
    """单线程流式下载。"""

def single_upload(
    url: str,
    filepath: str,
    overwrite: bool = False,
    prefix: str = "",
    chunk_size: int = 262144,
    method: str = "PUT",
    max_retries: int = 3,
    timeout: int = 60,
    headers: dict[str, str] | None = None,
    auth: tuple[str, str] | None = None,
) -> bool:
    """上传单个文件。``overwrite`` 仅为对齐签名保留,不起作用。"""

def cli_main(argv: list[str]) -> int:
    """执行命令行并返回退出码。"""

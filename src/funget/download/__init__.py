from .multi import MultiThreadDownloader
from .multi import download as multi_thread_download
from .single import SimpleDownloader
from .single import download as simple_download
from .work import Worker, WorkerFactory

__all__ = [
    "SimpleDownloader",
    "simple_download",
    "Worker",
    "WorkerFactory",
    "MultiThreadDownloader",
    "multi_thread_download",
]

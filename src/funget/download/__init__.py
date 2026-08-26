from .common import download
from .multi import download as multi_thread_download
from .single import download as simple_download

__all__ = [
    "download",
    "multi_thread_download",
    "simple_download",
]

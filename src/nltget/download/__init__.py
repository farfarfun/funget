from .common import download
from .multi import download as multi_download
from .multi import download as multi_thread_download
from .single import download as simple_download
from .single import download as single_download

__all__ = [
    "single_download",
    "multi_download",
    "download",
    "multi_thread_download",
    "simple_download",
]

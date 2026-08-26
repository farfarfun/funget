from typing import Optional

from nltlog import getLogger

from .multi import MultiDownloader
from .single import SingleDownloader

logger = getLogger("funget")


def download(
    url: str,
    filepath: str,
    multi: Optional[bool] = None,
    overwrite: bool = False,
    prefix: str = "",
    chunk_size: int = 2048,
    worker_num: int = 5,
    block_size: int = 100,
    max_retries: int = 3,
    **kwargs,
) -> bool:
    """智能下载函数，自动选择最佳下载方式

    Args:
        url: 下载链接
        filepath: 保存路径
        multi: 是否使用多线程下载，None表示自动选择
        overwrite: 是否覆盖已存在的文件
        prefix: 进度条前缀
        chunk_size: 数据块大小(字节)，仅用于单线程下载
        worker_num: 工作线程数，仅用于多线程下载
        block_size: 块大小(MB)，仅用于多线程下载
        max_retries: 最大重试次数

    Returns:
        bool: 下载是否成功
    """
    try:
        multi_downloader = None
        # 如果没有指定下载方式，自动选择
        if multi is None:
            # 创建一个临时的多线程下载器来检查是否支持范围请求
            multi_downloader = MultiDownloader(
                url=url,
                filepath=filepath,
                overwrite=overwrite,
                block_size=block_size,
                max_retries=max_retries,
                **kwargs,
            )

            # 根据文件大小和服务器支持情况自动选择
            file_size = multi_downloader.filesize
            supports_range = multi_downloader.supports_range

            # 文件大小大于10MB且支持范围请求时使用多线程
            multi = file_size > 10 * 1024 * 1024 and supports_range

            logger.info(
                f"Auto-selected {'multi-thread' if multi else 'single-thread'} download "
                f"(file size: {file_size:,} bytes, range support: {supports_range})"
            )

        if multi:
            loader = multi_downloader or MultiDownloader(
                url=url,
                filepath=filepath,
                overwrite=overwrite,
                block_size=block_size,
                max_retries=max_retries,
                **kwargs,
            )
            return loader.download(
                prefix=prefix,
                worker_num=worker_num,
                max_retries=max_retries,
            )

        loader = SingleDownloader(
            url=url, filepath=filepath, overwrite=overwrite, **kwargs
        )
        return loader.download(prefix=prefix, chunk_size=chunk_size)

    except Exception as e:
        logger.error(f"Download failed: {e}")
        return False

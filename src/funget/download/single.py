# -*- coding: utf-8 -*-
import os

import requests
from funfile.compress.utils import file_tqdm_bar
from funutil import getLogger

from .core import Downloader

logger = getLogger("funget")


class SingleDownloader(Downloader):
    """单线程下载器"""

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)

    def download(
        self, prefix: str = "", chunk_size: int = 2048, *args, **kwargs
    ) -> bool:
        """执行单线程下载"""
        try:
            prefix = f"{prefix}--" if prefix else ""

            # 确保目录存在
            os.makedirs(os.path.dirname(self.filepath), exist_ok=True)

            # 检查文件是否已存在且完整
            if (
                not self.overwrite
                and os.path.exists(self.filepath)
                and os.path.getsize(self.filepath) == self.filesize
            ):
                logger.info(
                    f"File {self.filepath} already exists with correct size, skipping download."
                )
                return True

            # 验证 URL
            if not self.validate_url():
                logger.error(f"Invalid URL: {self.url}")
                return False

            # 检查文件大小
            if self.filesize <= 0:
                logger.warning("File size is 0 or unknown, proceeding with download")

            pbar = None
            try:
                # 执行下载
                resp = self._session.get(
                    self.url, stream=True, headers=self.headers, timeout=self.timeout
                )
                resp.raise_for_status()

                # 验证响应
                content_length = resp.headers.get("content-length")
                if content_length and int(content_length) != self.filesize:
                    logger.warning(
                        f"Content-Length mismatch: expected {self.filesize}, got {content_length}"
                    )

                with open(self.filepath, "wb") as file:
                    pbar = file_tqdm_bar(
                        path=self.filepath,
                        total=self.filesize or int(content_length or 0),
                        prefix=f"{prefix}",
                    )

                    downloaded_bytes = 0
                    for data in resp.iter_content(chunk_size=chunk_size):
                        if data:  # 过滤空块
                            bytes_written = file.write(data)
                            downloaded_bytes += bytes_written
                            pbar.update(bytes_written)

                # 验证下载完整性
                if self.filesize > 0 and downloaded_bytes != self.filesize:
                    logger.error(
                        f"Download incomplete: expected {self.filesize} bytes, got {downloaded_bytes} bytes"
                    )
                    return False

                logger.success(f"Download completed successfully: {self.filepath}")
                return True

            except requests.exceptions.RequestException as e:
                logger.error(f"Network error during download: {e}")
                return False
            except IOError as e:
                logger.error(f"File I/O error during download: {e}")
                return False
            finally:
                if pbar:
                    pbar.close()

        except Exception as e:
            logger.error(f"Unexpected error during single-threaded download: {e}")
            return False


def download(
    url: str,
    filepath: str,
    overwrite: bool = False,
    prefix: str = "",
    chunk_size: int = 2048,
    *args,
    **kwargs,
) -> bool:
    """单线程下载文件

    Args:
        url: 下载链接
        filepath: 保存路径
        overwrite: 是否覆盖已存在的文件
        prefix: 进度条前缀
        chunk_size: 数据块大小(字节)

    Returns:
        bool: 下载是否成功
    """
    try:
        downloader = SingleDownloader(
            url=url, filepath=filepath, overwrite=overwrite, *args, **kwargs
        )
        return downloader.download(
            prefix=prefix, chunk_size=chunk_size, *args, **kwargs
        )
    except Exception as e:
        logger.error(f"Single-threaded download failed: {e}")
        return False

# -*- coding: utf-8 -*-
from typing import Optional, Generator

import requests
from funfile.compress.utils import file_tqdm_bar
from funutil import getLogger
from .core import Uploader

logger = getLogger("funget")


class SingleUploader(Uploader):
    """单文件上传器"""

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)

    def upload(
        self,
        prefix: str = "",
        chunk_size: int = 256 * 1024,
        method: str = "PUT",
        *args,
        **kwargs,
    ) -> bool:
        """执行文件上传"""
        try:
            prefix = f"{prefix}--" if prefix else ""

            # 验证文件
            if not self.validate_file():
                logger.error(f"File validation failed: {self.filepath}")
                return False

            # 验证 URL
            if not self.validate_url():
                logger.warning(f"URL validation failed, proceeding anyway: {self.url}")

            # 检查文件大小
            if self.filesize <= 0:
                logger.error("File size is 0 or invalid")
                return False

            logger.info(f"Starting upload: {self.filepath} -> {self.url}")
            logger.info(
                f"File size: {self.filesize:,} bytes ({self.filesize / (1024 * 1024):.2f} MB)"
            )

            pbar = None
            try:
                with open(self.filepath, "rb") as file:
                    pbar = file_tqdm_bar(
                        path=self.filepath,
                        total=self.filesize,
                        prefix=f"{prefix}",
                    )

                    uploaded_bytes = 0

                    def read_file_with_progress() -> Generator[bytes, None, None]:
                        """带进度更新的文件读取生成器"""
                        nonlocal uploaded_bytes
                        while True:
                            data = file.read(chunk_size)
                            if not data:
                                break
                            uploaded_bytes += len(data)
                            try:
                                pbar.update(len(data))
                            except Exception as e:
                                logger.warning(f"Progress bar update failed: {e}")
                            yield data

                    # 准备请求头
                    upload_headers = self.headers.copy()
                    upload_headers.update(
                        {
                            "Content-Length": str(self.filesize),
                            "Content-Type": "application/octet-stream",
                        }
                    )

                    # 执行上传
                    method = method.upper()
                    if method == "PUT":
                        response = self._session.put(
                            self.url,
                            data=read_file_with_progress(),
                            headers=upload_headers,
                            timeout=self.timeout,
                        )
                    elif method == "POST":
                        # 对于 POST 请求，使用 multipart/form-data
                        files = {
                            "file": (
                                self.filename,
                                read_file_with_progress(),
                                "application/octet-stream",
                            )
                        }
                        response = self._session.post(
                            self.url,
                            files=files,
                            headers=self.headers,  # 不包含 Content-Length，让 requests 自动处理
                            timeout=self.timeout,
                        )
                    else:
                        logger.error(f"Unsupported HTTP method: {method}")
                        return False

                    # 检查响应
                    response.raise_for_status()

                    # 验证上传完整性
                    if uploaded_bytes != self.filesize:
                        logger.warning(
                            f"Upload size mismatch: expected {self.filesize}, uploaded {uploaded_bytes}"
                        )

                    logger.success(
                        f"Upload completed successfully with status {response.status_code}"
                    )
                    return True

            except requests.exceptions.RequestException as e:
                logger.error(f"Network error during upload: {e}")
                return False
            except IOError as e:
                logger.error(f"File I/O error during upload: {e}")
                return False
            finally:
                if pbar:
                    pbar.close()

        except Exception as e:
            logger.error(f"Unexpected error during upload: {e}")
            return False

    def upload_with_retry(self, max_retries: Optional[int] = None, **kwargs) -> bool:
        """带重试的上传"""
        max_retries = max_retries or self.max_retries

        for attempt in range(max_retries + 1):
            try:
                if self.upload(**kwargs):
                    return True
            except Exception as e:
                logger.warning(f"Upload attempt {attempt + 1} failed: {e}")
                if attempt == max_retries:
                    logger.error(f"Upload failed after {max_retries + 1} attempts")
                    return False
                # 指数退避
                import time

                time.sleep(2**attempt)

        return False


def upload(
    url: str,
    filepath: str,
    overwrite: bool = False,
    prefix: str = "",
    chunk_size: int = 256 * 1024,
    method: str = "PUT",
    max_retries: int = 3,
    *args,
    **kwargs,
) -> bool:
    """上传文件到指定URL

    Args:
        url: 上传目标URL
        filepath: 本地文件路径
        overwrite: 是否覆盖（保留参数，兼容性）
        prefix: 进度条前缀
        chunk_size: 数据块大小(字节)
        method: HTTP方法 ("PUT" 或 "POST")
        max_retries: 最大重试次数

    Returns:
        bool: 上传是否成功
    """
    try:
        uploader = SingleUploader(
            url=url,
            filepath=filepath,
            overwrite=overwrite,
            max_retries=max_retries,
            *args,
            **kwargs,
        )
        return uploader.upload_with_retry(
            prefix=prefix, chunk_size=chunk_size, method=method, *args, **kwargs
        )
    except Exception as e:
        logger.error(f"Upload failed: {e}")
        return False

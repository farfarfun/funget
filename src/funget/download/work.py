import time
from typing import Callable, Optional

import requests
from farlog import getLogger

logger = getLogger("funget")


class Worker:
    def __init__(
        self,
        url: str,
        fileobj,
        range_start: int = 0,
        range_end: Optional[int] = None,
        update_callback: Optional[Callable] = None,
        headers: Optional[dict] = None,
        chunk_size: int = 2 * 1024 * 1024,
        max_retries: int = 3,
        timeout: int = 60,
        auth=None,
    ):
        self.url = url
        self.auth = auth
        self.fileobj = fileobj
        self.headers = headers or {}
        self.range_start = range_start
        self.range_curser = range_start
        self._session = requests.Session()
        self.range_end = range_end if range_end is not None else self._get_size()
        self.size = self.range_end - self.range_start + 1
        self.update_callback = update_callback
        self.chunk_size = chunk_size or 100 * 1024
        self.max_retries = max_retries
        self.timeout = timeout

    def _get_size(self) -> int:
        """获取文件大小"""
        try:
            resp = self._session.head(
                self.url,
                headers=self.headers,
                timeout=self.timeout,
                auth=self.auth,
            )
            resp.raise_for_status()
            return int(resp.headers.get("content-length", 0))
        except Exception as e:
            logger.warning(f"Failed to get file size via HEAD request: {e}")
            # fallback to GET request
            try:
                resp = self._session.get(
                    self.url,
                    stream=True,
                    headers=self.headers,
                    timeout=self.timeout,
                    auth=self.auth,
                )
                resp.raise_for_status()
                return int(resp.headers.get("content-length", 0))
            except Exception as e:
                logger.error(f"Failed to get file size: {e}")
                return 0

    def run(self) -> bool:
        """执行下载任务"""
        try:
            for attempt in range(self.max_retries + 1):
                try:
                    if self._download_chunk():
                        return True
                except requests.exceptions.RequestException as e:
                    logger.warning(f"Download attempt {attempt + 1} failed: {e}")
                if attempt < self.max_retries:
                    time.sleep(2**attempt)
            return False
        finally:
            self._session.close()

    def _download_chunk(self) -> bool:
        """下载数据块"""
        headers = {"Range": f"bytes={self.range_curser}-{self.range_end}"}
        headers.update(self.headers)

        try:
            with self._session.get(
                self.url,
                stream=True,
                headers=headers,
                timeout=self.timeout,
                auth=self.auth,
            ) as req:
                if req.status_code == 416:
                    return False
                req.raise_for_status()
                if req.status_code not in (200, 206):
                    logger.warning(f"Unexpected status code: {req.status_code}")
                    return False
                if req.status_code == 200:
                    content_length = int(req.headers.get("content-length", 0))
                    if self.range_start or (
                        content_length and content_length != self.size
                    ):
                        return False

                for chunk in req.iter_content(chunk_size=self.chunk_size):
                    if chunk:
                        chunk = chunk[: self.range_end - self.range_curser + 1]
                        if not chunk:
                            break
                        try:
                            _size = self.fileobj.write(
                                chunk=chunk, offset=self.range_curser
                            )
                            self.range_curser += _size
                            if self.update_callback:
                                self.update_callback(
                                    self.size, self.range_curser, _size
                                )
                        except Exception as e:
                            logger.error(f"Error writing to file: {e}")
                            raise

                        if self.range_curser > self.range_end:
                            break
                return self.range_curser > self.range_end

        except requests.exceptions.Timeout:
            logger.warning("Request timeout")
            raise
        except requests.exceptions.ConnectionError:
            logger.warning("Connection error")
            raise
        except requests.exceptions.HTTPError as e:
            logger.warning(f"HTTP error: {e}")
            raise

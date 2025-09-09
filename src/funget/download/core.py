# -*- coding: utf-8 -*-
import os
from typing import Optional, Dict, Any


import requests
from requests.adapters import HTTPAdapter
from urllib3.util.retry import Retry
from funutil import getLogger

logger = getLogger("funget")


class Downloader:
    """下载器基类"""

    def __init__(
        self,
        url: str,
        filepath: str,
        overwrite: bool = False,
        filesize: Optional[int] = None,
        headers: Optional[Dict[str, str]] = None,
        max_retries: int = 3,
        timeout: int = 30,
        *args,
        **kwargs,
    ):
        self.url = url
        self.headers = headers or {}
        self.filepath = filepath
        self.overwrite = overwrite
        self.max_retries = max_retries
        self.timeout = timeout
        self._session = self._create_session()  # 先创建 session
        self.filesize = filesize or self.__get_size()  # 然后获取文件大小
        self.filename = os.path.basename(self.filepath)

    def _create_session(self) -> requests.Session:
        """创建带有重试策略的会话"""
        session = requests.Session()
        retry_strategy = Retry(
            total=self.max_retries,
            backoff_factor=1,
            status_forcelist=[429, 500, 502, 503, 504],
            allowed_methods=["HEAD", "GET", "OPTIONS"],
        )
        adapter = HTTPAdapter(max_retries=retry_strategy)
        session.mount("http://", adapter)
        session.mount("https://", adapter)
        return session

    def download(self, *args, **kwargs) -> bool:
        """下载文件的抽象方法"""
        raise NotImplementedError("Subclasses must implement download method")

    def __get_size(self) -> int:
        """获取文件大小"""
        try:
            # 首先尝试 HEAD 请求
            resp = self._session.head(
                self.url, headers=self.headers, timeout=self.timeout
            )
            resp.raise_for_status()
            size = int(resp.headers.get("content-length", 0))
            if size > 0:
                return size
        except Exception as e:
            logger.warning(f"HEAD request failed: {e}, trying GET request")

        try:
            # 如果 HEAD 请求失败，尝试 GET 请求
            resp = self._session.get(
                self.url, stream=True, headers=self.headers, timeout=self.timeout
            )
            resp.raise_for_status()
            size = int(resp.headers.get("content-length", 0))
            return size
        except Exception as e:
            logger.error(f"Failed to get file size: {e}")
            return 0

    def get_file_info(self) -> Dict[str, Any]:
        """获取文件信息"""
        return {
            "url": self.url,
            "filepath": self.filepath,
            "filename": self.filename,
            "filesize": self.filesize,
            "overwrite": self.overwrite,
        }

    def validate_url(self) -> bool:
        """验证 URL 是否有效"""
        try:
            resp = self._session.head(
                self.url, headers=self.headers, timeout=self.timeout
            )
            return resp.status_code < 400
        except Exception:
            return False

    def __del__(self):
        """清理资源"""
        if hasattr(self, "_session"):
            self._session.close()

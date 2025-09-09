# -*- coding: utf-8 -*-
import os
from typing import Optional, Dict, Any

import requests
from requests.adapters import HTTPAdapter
from urllib3.util.retry import Retry
from funutil import getLogger

logger = getLogger("funget")


class Uploader:
    """上传器基类"""

    def __init__(
        self,
        url: str,
        filepath: str,
        overwrite: bool = False,
        filesize: Optional[int] = None,
        headers: Optional[Dict[str, str]] = None,
        max_retries: int = 3,
        timeout: int = 60,
        *args,
        **kwargs,
    ):
        self.url = url
        self.headers = headers or {}
        self.filepath = filepath
        self.overwrite = overwrite
        self.max_retries = max_retries
        self.timeout = timeout
        self.filesize = filesize or self.__get_size()
        self.filename = os.path.basename(self.filepath)
        self._session = self._create_session()

    def _create_session(self) -> requests.Session:
        """创建带有重试策略的会话"""
        session = requests.Session()
        retry_strategy = Retry(
            total=self.max_retries,
            backoff_factor=1,
            status_forcelist=[429, 500, 502, 503, 504],
            allowed_methods=["POST", "PUT", "PATCH"],
        )
        adapter = HTTPAdapter(max_retries=retry_strategy)
        session.mount("http://", adapter)
        session.mount("https://", adapter)
        return session

    def upload(self, *args, **kwargs) -> bool:
        """上传文件的抽象方法"""
        raise NotImplementedError("Subclasses must implement upload method")

    def __get_size(self) -> int:
        """获取文件大小"""
        try:
            if not os.path.exists(self.filepath):
                raise FileNotFoundError(f"File not found: {self.filepath}")
            return os.path.getsize(self.filepath)
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

    def validate_file(self) -> bool:
        """验证文件是否存在且可读"""
        try:
            if not os.path.exists(self.filepath):
                logger.error(f"File does not exist: {self.filepath}")
                return False
            if not os.path.isfile(self.filepath):
                logger.error(f"Path is not a file: {self.filepath}")
                return False
            if not os.access(self.filepath, os.R_OK):
                logger.error(f"File is not readable: {self.filepath}")
                return False
            return True
        except Exception as e:
            logger.error(f"File validation failed: {e}")
            return False

    def validate_url(self) -> bool:
        """验证 URL 是否可访问"""
        try:
            # 发送 OPTIONS 请求检查服务器是否支持上传
            resp = self._session.options(
                self.url, headers=self.headers, timeout=self.timeout
            )
            return resp.status_code < 400
        except Exception:
            # 如果 OPTIONS 失败，尝试 HEAD 请求
            try:
                resp = self._session.head(
                    self.url, headers=self.headers, timeout=self.timeout
                )
                return resp.status_code < 500  # 允许4xx错误，因为可能是认证问题
            except Exception:
                return False

    def __del__(self):
        """清理资源"""
        if hasattr(self, "_session"):
            self._session.close()

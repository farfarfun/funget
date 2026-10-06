import os
from typing import Any
from urllib.parse import urlsplit, urlunsplit

import requests
from farlog import getLogger
from requests.adapters import HTTPAdapter
from requests.auth import HTTPDigestAuth
from urllib3.util.retry import Retry

logger = getLogger("funget")


def safe_url(url: str) -> str:
    """Return a URL suitable for logs, without credentials or query data."""
    try:
        parsed = urlsplit(url)
        if not parsed.hostname:
            return "<invalid URL>"
        return urlunsplit((parsed.scheme, parsed.hostname, parsed.path, "", ""))
    except ValueError:
        return "<invalid URL>"


class Downloader:
    """管理 HTTP 会话、重试策略和远程文件元数据的下载器基类。

    Args:
        url: 下载链接。
        filepath: 本地保存路径。
        overwrite: 是否覆盖已存在文件。
        filesize: 已知文件大小；未提供时自动查询。
        headers: 附加 HTTP 请求头。
        max_retries: 请求最大重试次数。
        timeout: 请求超时秒数。
        auth: HTTP Digest 认证对象。
    """

    def __init__(
        self,
        url: str,
        filepath: str,
        overwrite: bool = False,
        filesize: int | None = None,
        headers: dict[str, str] | None = None,
        max_retries: int = 3,
        timeout: int = 30,
        auth: HTTPDigestAuth | None = None,
    ) -> None:
        self.url = url
        self.auth = auth
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

    def download(self, *args: Any, **kwargs: Any) -> bool:
        """下载文件，子类必须实现并返回是否成功。"""
        raise NotImplementedError("Subclasses must implement download method")

    def __get_size(self) -> int:
        """获取文件大小"""
        try:
            # 首先尝试 HEAD 请求
            resp = self._session.head(
                self.url,
                headers=self.headers,
                timeout=self.timeout,
                auth=self.auth,
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
                self.url,
                stream=True,
                headers=self.headers,
                timeout=self.timeout,
                auth=self.auth,
            )
            resp.raise_for_status()
            size = int(resp.headers.get("content-length", 0))
            return size
        except Exception as e:
            logger.error(f"Failed to get file size: {e}")
            return 0

    def get_file_info(self) -> dict[str, Any]:
        """返回 URL、本地路径、文件名、大小和覆盖设置。"""
        return {
            "url": self.url,
            "filepath": self.filepath,
            "filename": self.filename,
            "filesize": self.filesize,
            "overwrite": self.overwrite,
        }

    def validate_url(self) -> bool:
        """通过 HEAD 请求验证 URL 是否可访问。"""
        try:
            resp = self._session.head(
                self.url, headers=self.headers, timeout=self.timeout, auth=self.auth
            )
            return resp.status_code < 400
        except Exception as e:
            logger.warning(
                f"URL validation failed for {safe_url(self.url)}: {type(e).__name__}"
            )
            return False

    def __del__(self):
        """清理资源"""
        if hasattr(self, "_session"):
            self._session.close()

"""差分测试用的本地 HTTP 服务。

被 ``differential.py`` 拉起,也可以单独运行用来手工验证::

    python scripts/differential_server.py 8000

支持的路径(``<size>`` 单位字节):

====================  ====================================================
``/plain/<size>``     正常响应,支持 Range
``/norange/<size>``   无视 Range,永远返回完整的 200
``/flaky/<size>``     前若干次请求返回 503,之后正常(验证重试)
``/auth/<size>``      需要 Basic ``alice:s3cret``
``/nolen/<size>``     不带 Content-Length,chunked 传输
``/missing``          永远 404
``/upload``           接收 PUT/POST,回 200 并记录收到的字节数
====================  ====================================================
"""

from __future__ import annotations

import base64
import hashlib
import re
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# 可重现的伪随机负载。不用全 0:写错偏移时全 0 文件照样能对上 sha,错误会被掩盖。
def payload(size: int) -> bytes:
    return bytes((i * 31) % 251 for i in range(size))


_PATH = re.compile(r"^/(plain|norange|flaky|auth|nolen)/(\d+)$")
_CREDENTIAL = "Basic " + base64.b64encode(b"alice:s3cret").decode()

# flaky 端点的剩余失败次数,按 size 分桶,保证两个实现看到同样的失败序列。
_flaky_budget: dict[str, int] = {}
_flaky_lock = threading.Lock()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args: object) -> None:  # noqa: D102 - 保持输出干净
        pass

    def handle_one_request(self) -> None:
        # 客户端主动断开是**正常流量**,不是故障:Range 支持探测会发
        # `Range: bytes=0-100`,只读响应头就把连接丢掉。新旧实现都这么干。
        # 不拦的话 ThreadingHTTPServer 会为每次探测打一坨 BrokenPipeError 堆栈,
        # 把差分结果淹掉。
        try:
            super().handle_one_request()
        except (BrokenPipeError, ConnectionResetError):
            self.close_connection = True

    # ---- 路由 -------------------------------------------------------------

    def do_HEAD(self) -> None:  # noqa: N802
        self._serve(body=False)

    def do_GET(self) -> None:  # noqa: N802
        self._serve(body=True)

    def do_PUT(self) -> None:  # noqa: N802
        self._receive()

    def do_POST(self) -> None:  # noqa: N802
        self._receive()

    # ---- 实现 -------------------------------------------------------------

    def _receive(self) -> None:
        length = int(self.headers.get("Content-Length") or 0)
        data = b""
        remaining = length
        while remaining > 0:
            chunk = self.rfile.read(min(remaining, 65536))
            if not chunk:
                break
            data += chunk
            remaining -= len(chunk)
        digest = hashlib.sha256(data).hexdigest()
        body = f'{{"bytes": {len(data)}, "sha256": "{digest}"}}'.encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _serve(self, *, body: bool) -> None:
        match = _PATH.match(self.path)
        if match is None:
            self.send_response(404)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return

        kind, raw_size = match.group(1), int(match.group(2))
        data = payload(raw_size)

        if kind == "auth" and self.headers.get("Authorization") != _CREDENTIAL:
            self.send_response(401)
            self.send_header("WWW-Authenticate", 'Basic realm="funget"')
            self.send_header("Content-Length", "0")
            self.end_headers()
            return

        if kind == "flaky":
            key = f"{raw_size}"
            with _flaky_lock:
                budget = _flaky_budget.get(key, 3)
                if budget > 0:
                    _flaky_budget[key] = budget - 1
                    self.send_response(503)
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return

        if kind == "nolen":
            # 不给 Content-Length,走 chunked。funget 的大小探测会拿不到长度。
            self.send_response(200)
            self.send_header("Transfer-Encoding", "chunked")
            self.end_headers()
            if body:
                for start in range(0, len(data), 65536):
                    piece = data[start : start + 65536]
                    self.wfile.write(f"{len(piece):x}\r\n".encode())
                    self.wfile.write(piece)
                    self.wfile.write(b"\r\n")
                self.wfile.write(b"0\r\n\r\n")
            return

        requested = self.headers.get("Range") if kind != "norange" else None
        start, end, status = 0, len(data) - 1, 200
        if requested:
            parsed = _parse_range(requested, len(data))
            if parsed is not None:
                start, end = parsed
                status = 206

        slice_ = data[start : end + 1]
        self.send_response(status)
        self.send_header("Content-Length", str(len(slice_)))
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("ETag", f'"sz{len(data)}"')
        if status == 206:
            self.send_header("Content-Range", f"bytes {start}-{end}/{len(data)}")
        self.end_headers()
        if body:
            self.wfile.write(slice_)


def _parse_range(value: str, total: int) -> tuple[int, int] | None:
    if not value.startswith("bytes=") or total == 0:
        return None
    spec = value[len("bytes=") :]
    first, _, last = spec.partition("-")
    try:
        start = int(first)
    except ValueError:
        return None
    if start >= total:
        return None
    end = total - 1 if not last else min(int(last), total - 1)
    return (start, end) if start <= end else None


def serve(port: int = 0) -> tuple[ThreadingHTTPServer, str]:
    """起服务并返回 ``(server, base_url)``。"""
    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    host, bound = server.server_address[:2]
    return server, f"http://{host}:{bound}"


if __name__ == "__main__":
    _, url = serve(int(sys.argv[1]) if len(sys.argv) > 1 else 8000)
    print(url, flush=True)
    threading.Event().wait()

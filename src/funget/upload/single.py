import os
import time
from typing import Dict, Optional

import requests
from funfile import file_tqdm_bar
from farlog import getLogger
from requests.auth import HTTPDigestAuth

logger = getLogger("funget")


def upload(
    url: str,
    filepath: str,
    overwrite: bool = False,
    prefix: str = "",
    chunk_size: int = 256 * 1024,
    method: str = "PUT",
    max_retries: int = 3,
    timeout: int = 60,
    headers: Optional[Dict[str, str]] = None,
    auth: Optional[HTTPDigestAuth] = None,
) -> bool:
    """上传单个文件。"""
    del overwrite
    method = method.upper()
    if chunk_size <= 0 or max_retries < 0 or timeout <= 0:
        logger.error(
            "chunk_size and timeout must be positive; max_retries cannot be negative"
        )
        return False
    if method not in ("PUT", "POST"):
        logger.error(f"Unsupported HTTP method: {method}")
        return False
    if not os.path.isfile(filepath):
        logger.error(f"File not found: {filepath}")
        return False

    filesize = os.path.getsize(filepath)
    if filesize <= 0:
        logger.error(f"File is empty: {filepath}")
        return False

    session = requests.Session()
    pbar = file_tqdm_bar(
        path=filepath,
        total=filesize,
        prefix=f"{prefix}--" if prefix else "",
    )
    try:
        for attempt in range(max_retries + 1):
            try:
                if attempt:
                    pbar.reset(total=filesize)
                with open(filepath, "rb") as file:
                    if method == "PUT":

                        def chunks():
                            while data := file.read(chunk_size):
                                pbar.update(len(data))
                                yield data

                        request_headers = {
                            **(headers or {}),
                            "Content-Length": str(filesize),
                            "Content-Type": "application/octet-stream",
                        }
                        response = session.put(
                            url,
                            data=chunks(),
                            headers=request_headers,
                            timeout=timeout,
                            auth=auth,
                        )
                    else:
                        # ponytail: POST progress updates on completion; add a
                        # monitored file wrapper only if live feedback matters.
                        response = session.post(
                            url,
                            files={
                                "file": (
                                    os.path.basename(filepath),
                                    file,
                                    "application/octet-stream",
                                )
                            },
                            headers=headers,
                            timeout=timeout,
                            auth=auth,
                        )

                response.raise_for_status()
                if method == "POST":
                    pbar.update(filesize)
                logger.success(f"Upload completed: {filepath}")
                return True
            except requests.exceptions.RequestException as e:
                logger.warning(f"Upload attempt {attempt + 1} failed: {e}")
                if attempt == max_retries:
                    return False
                time.sleep(2**attempt)
    except OSError as e:
        logger.error(f"File I/O error during upload: {e}")
        return False
    finally:
        pbar.close()
        session.close()

    return False

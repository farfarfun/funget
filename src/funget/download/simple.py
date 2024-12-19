# -*- coding: utf-8 -*-
import os

import requests
from funfile.compress.utils import file_tqdm_bar
from funutil import getLogger

from .core import Downloader

logger = getLogger("funget")


class SimpleDownloader(Downloader):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)

    def download(self, prefix="", chunk_size=2048, *args, **kwargs) -> bool:
        prefix = f"{prefix}--" if prefix is not None and len(prefix) > 0 else ""
        if not os.path.exists(os.path.dirname(self.filepath)):
            os.makedirs(os.path.dirname(self.filepath))
        if (
            not self.overwrite
            and os.path.exists(self.filepath)
            and os.path.getsize(self.filepath) == self.filesize
        ):
            logger.info(f"File :{self.filepath} exists, and size is same, return.")
            return False
        with requests.Session() as sess:
            resp = sess.get(self.url, stream=True, headers=self.headers)

            with open(self.filepath, "wb") as file:
                with file_tqdm_bar(
                    path=self.filepath,
                    total=self.filesize,
                    prefix=f"{prefix}",
                ) as bar:
                    for data in resp.iter_content(chunk_size=chunk_size):
                        bar.update(file.write(data))

        logger.success(f"download success from {self.url[:20]} to {self.filepath}")


def download(
    url, filepath, overwrite=False, prefix="", chunk_size=2048, *args, **kwargs
):
    SimpleDownloader(
        url=url, filepath=filepath, overwrite=overwrite, *args, **kwargs
    ).download(prefix=prefix, chunk_size=chunk_size, *args, **kwargs)

# -*- coding: utf-8 -*-
import os
import os.path

import requests
from funfile import ConcurrentFile
from funfile.compress.utils import file_tqdm_bar
from funutil import getLogger

from .core import Downloader
from .work import Worker, WorkerFactory

logger = getLogger("funget")


class MultiDownloader(Downloader):
    def __init__(self, block_size=50, *args, **kwargs):
        super(MultiDownloader, self).__init__(*args, **kwargs)
        self.blocks_num = self.filesize // (block_size * 1024 * 1024)

        if not self.check_available():
            print(
                f"{self.filename} this url not support range requests,set blocks_num=1."
            )
            self.blocks_num = 1

    def __get_range(self):
        size = int(self.filesize) // self.blocks_num
        range_list = []
        for i in range(self.blocks_num):
            start = i * size

            if i == self.blocks_num - 1:
                end = self.filesize - 1
            else:
                end = start + size
            if i > 0:
                start += 1
            range_list.append((start, end))
        return range_list

    def download(
        self, worker_num=5, capacity=100, prefix="", overwrite=False, *args, **kwargs
    ):
        if (
            not overwrite
            and os.path.exists(self.filepath)
            and self.filesize == os.path.getsize(self.filepath)
        ):
            logger.info(f"File :{self.filepath} exists, and size is same, return.")
            return False

        prefix = prefix if prefix is not None and len(prefix) > 0 else ""
        range_list = self.__get_range()
        success_files = []
        pbar = file_tqdm_bar(
            path=self.filepath,
            total=self.filesize,
            prefix=f"{prefix}|0/{self.blocks_num}|",
        )

        def update_pbar(total, curser, current):
            pbar.update(current)
            pbar.refresh()

        with ConcurrentFile(self.filepath, "wb") as fw:
            with WorkerFactory(
                worker_num=worker_num, capacity=capacity, timeout=1
            ) as pool:
                for index, (start, end) in enumerate(range_list):
                    for record in fw._writen_data:
                        if record[0] <= start <= record[1]:
                            start = record[1] + 1
                            pbar.update(start - record[0])
                            break
                    if start >= end:
                        success_files.append("2")
                        pbar.set_description(
                            desc=f"{prefix}|{len(success_files)}/{self.blocks_num}|{os.path.basename(self.filepath)}"
                        )
                        continue

                    def finish_callback(worker: Worker, *args, **kwargs):
                        success_files.append("2")
                        pbar.set_description(
                            desc=f"{prefix}|{len(success_files)}/{self.blocks_num}|{os.path.basename(self.filepath)}"
                        )

                    worker = Worker(
                        url=self.url,
                        range_start=start,
                        range_end=end,
                        fileobj=fw,
                        update_callback=update_pbar,
                        finish_callback=finish_callback,
                        headers=self.headers,
                    )
                    pool.submit(worker=worker)

        logger.success(f"download success from {self.url[:20]} to {self.filepath}")

    def check_available(self) -> bool:
        if self.blocks_num < 1:
            return False
        headers = {"Range": "bytes=0-100"}
        headers.update(self.headers)
        with requests.get(self.url, stream=True, headers=headers) as req:
            return req.status_code == 206


def download(
    url,
    filepath,
    overwrite=False,
    worker_num=5,
    capacity=100,
    block_size=100,
    prefix="",
    *args,
    **kwargs,
):
    MultiDownloader(
        url=url, filepath=filepath, overwrite=overwrite, block_size=block_size
    ).download(worker_num=worker_num, capacity=capacity, prefix=prefix)

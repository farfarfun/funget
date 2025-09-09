# -*- coding: utf-8 -*-
import os
import os.path
from typing import Tuple, List


import requests
from funfile import ConcurrentFile
from funfile.compress.utils import file_tqdm_bar
from funutil import getLogger

from .core import Downloader
from .work import Worker, WorkerFactory

logger = getLogger("funget")


class MultiDownloader(Downloader):
    def __init__(self, block_size: int = 50, min_block_size: int = 1, *args, **kwargs):
        super(MultiDownloader, self).__init__(*args, **kwargs)

        # 确保文件大小有效
        if self.filesize <= 0:
            logger.warning(
                f"Invalid file size: {self.filesize}, falling back to single thread"
            )
            self.blocks_num = 1
        else:
            # 计算块数，但确保每个块至少有 min_block_size MB
            block_size_bytes = block_size * 1024 * 1024
            min_block_size_bytes = min_block_size * 1024 * 1024

            self.blocks_num = max(
                1,
                min(
                    self.filesize // block_size_bytes,
                    self.filesize // min_block_size_bytes,
                ),
            )

        if not self.check_available():
            logger.info(
                f"{self.filename} does not support range requests, using single thread download."
            )
            self.blocks_num = 1

    def __get_range(self) -> List[Tuple[int, int]]:
        """计算下载范围列表"""
        if self.blocks_num <= 1:
            return [(0, self.filesize - 1)]

        size = self.filesize // self.blocks_num
        range_list = []

        for i in range(self.blocks_num):
            start = i * size
            if i > 0:
                start += 1  # 避免重叠

            if i == self.blocks_num - 1:
                end = self.filesize - 1  # 最后一块包含剩余所有字节
            else:
                end = start + size

            # 确保范围有效
            if start <= end:
                range_list.append((start, end))
            else:
                logger.warning(f"Invalid range: {start}-{end}, skipping")

        return range_list

    def download(
        self,
        worker_num: int = 5,
        capacity: int = 100,
        prefix: str = "",
        overwrite: bool = False,
        max_retries: int = 3,
        *args,
        **kwargs,
    ) -> bool:
        """执行多线程下载"""
        try:
            # 检查文件是否已存在且完整
            if (
                not overwrite
                and os.path.exists(self.filepath)
                and os.path.getsize(self.filepath) == self.filesize
            ):
                logger.info(
                    f"File {self.filepath} already exists with correct size, skipping download."
                )
                return True

            # 确保目录存在
            os.makedirs(os.path.dirname(self.filepath), exist_ok=True)

            prefix = prefix if prefix else ""
            range_list = self.__get_range()

            if not range_list:
                logger.error("No valid download ranges calculated")
                return False

            success_files = []
            pbar = file_tqdm_bar(
                path=self.filepath,
                total=self.filesize,
                prefix=f"{prefix}|0/{self.blocks_num}|",
            )

            def update_pbar(total, curser, current):
                try:
                    pbar.update(current)
                    pbar.refresh()
                except Exception as e:
                    logger.warning(f"Progress bar update failed: {e}")

            try:
                with ConcurrentFile(self.filepath, "wb") as fw:
                    with WorkerFactory(
                        worker_num=worker_num, capacity=capacity, timeout=30
                    ) as pool:
                        for index, (start, end) in enumerate(range_list):
                            # 检查是否已经下载过这个范围
                            original_start = start
                            for record in fw._writen_data:
                                if record[0] <= start <= record[1]:
                                    downloaded_bytes = start - original_start
                                    start = record[1] + 1
                                    if downloaded_bytes > 0:
                                        pbar.update(downloaded_bytes)
                                    break

                            if start > end:
                                success_files.append(index)
                                pbar.set_description(
                                    desc=f"{prefix}|{len(success_files)}/{self.blocks_num}|{os.path.basename(self.filepath)}"
                                )
                                continue

                            def finish_callback(worker: Worker, *args, **kwargs):
                                success_files.append(index)
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
                                max_retries=max_retries,
                            )
                            pool.submit(worker=worker)

                        # 检查是否有失败的任务需要重试
                        failed_tasks = pool.get_failed_tasks()
                        if failed_tasks:
                            logger.warning(f"Retrying {len(failed_tasks)} failed tasks")
                            pool.retry_failed_tasks()

            except Exception as e:
                logger.error(f"Download failed: {e}")
                return False
            finally:
                try:
                    pbar.close()
                except:
                    pass

        except Exception as e:
            logger.error(f"Unexpected error during download: {e}")
            return False

    def check_available(self) -> bool:
        """检查服务器是否支持范围请求"""
        if self.blocks_num < 1:
            return False

        try:
            headers = {"Range": "bytes=0-100"}
            headers.update(self.headers)

            with requests.get(
                self.url, stream=True, headers=headers, timeout=30
            ) as req:
                # 206 表示部分内容，支持范围请求
                # 200 表示服务器忽略了范围请求，返回完整内容
                if req.status_code == 206:
                    return True
                elif req.status_code == 200:
                    # 检查响应头是否包含 Accept-Ranges
                    accept_ranges = req.headers.get("Accept-Ranges", "").lower()
                    return accept_ranges == "bytes"
                else:
                    logger.warning(f"Range request returned status {req.status_code}")
                    return False

        except Exception as e:
            logger.warning(f"Failed to check range request support: {e}")
            return False


def download(
    url: str,
    filepath: str,
    overwrite: bool = False,
    worker_num: int = 5,
    capacity: int = 100,
    block_size: int = 100,
    prefix: str = "",
    max_retries: int = 3,
    *args,
    **kwargs,
) -> bool:
    """多线程下载文件

    Args:
        url: 下载链接
        filepath: 保存路径
        overwrite: 是否覆盖已存在的文件
        worker_num: 工作线程数
        capacity: 队列容量
        block_size: 块大小(MB)
        prefix: 进度条前缀
        max_retries: 最大重试次数

    Returns:
        bool: 下载是否成功
    """
    try:
        downloader = MultiDownloader(
            url=url,
            filepath=filepath,
            overwrite=overwrite,
            block_size=block_size,
            *args,
            **kwargs,
        )
        return downloader.download(
            worker_num=worker_num,
            capacity=capacity,
            prefix=prefix,
            max_retries=max_retries,
            *args,
            **kwargs,
        )
    except Exception as e:
        logger.error(f"Multi-threaded download failed: {e}")
        return False

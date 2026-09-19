import os
from queue import Queue
from typing import Any

from farlog import getLogger
from funfile import ConcurrentFile, file_tqdm_bar
from funworker import BaseProcessor, WorkerPool

from .core import Downloader
from .work import Worker

logger = getLogger("funget")


class _DownloadProcessor(BaseProcessor):
    def process(self, item: Worker) -> Any:
        return item.run()


class MultiDownloader(Downloader):
    """将远程文件分成 Range 请求并发下载。

    Args:
        block_size: 目标分块大小，单位 MiB。
        min_block_size: 最小分块大小，单位 MiB。
        **kwargs: 传给 `Downloader` 的参数。
    """

    def __init__(
        self, block_size: int = 50, min_block_size: int = 1, **kwargs: Any
    ) -> None:
        if block_size <= 0 or min_block_size <= 0:
            raise ValueError("block sizes must be positive")
        super().__init__(**kwargs)

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

        self.supports_range = self.check_available()
        if not self.supports_range:
            logger.info(
                f"{self.filename} does not support range requests, using single thread download."
            )
            self.blocks_num = 1

    def __get_range(self) -> list[tuple[int, int]]:
        """计算下载范围列表"""
        if self.filesize <= 0:
            return []
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
        prefix: str = "",
        overwrite: bool | None = None,
        max_retries: int = 3,
    ) -> bool:
        """执行多线程下载。

        Args:
            worker_num: 并发工作线程数。
            prefix: 进度条前缀。
            overwrite: 是否覆盖已存在文件；未提供时使用实例设置。
            max_retries: 每个分块的最大重试次数。

        Returns:
            所有分块都下载成功时返回 `True`。
        """
        overwrite = self.overwrite if overwrite is None else overwrite

        try:
            if worker_num <= 0:
                raise ValueError("worker_num must be positive")
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
            os.makedirs(os.path.dirname(os.path.abspath(self.filepath)), exist_ok=True)

            prefix = prefix if prefix else ""
            range_list = self.__get_range()

            if not range_list:
                logger.error("No valid download ranges calculated")
                return False

            pbar = file_tqdm_bar(
                path=self.filepath,
                total=self.filesize,
                prefix=f"{prefix}|0/{len(range_list)}|",
            )

            def update_pbar(total: int, curser: int, current: int) -> None:
                try:
                    pbar.update(current)
                    pbar.refresh()
                except Exception as e:
                    logger.warning(f"Progress bar update failed: {e}")

            try:
                with ConcurrentFile(self.filepath, "wb") as fw:
                    results = []
                    workers = []
                    for start, end in range_list:
                        workers.append(
                            Worker(
                                url=self.url,
                                range_start=start,
                                range_end=end,
                                fileobj=fw,
                                update_callback=update_pbar,
                                headers=self.headers,
                                max_retries=max_retries,
                                timeout=self.timeout,
                                auth=self.auth,
                            )
                        )

                    input_queue: Queue[Worker] = Queue()
                    output_queue: Queue[Any] = Queue()
                    for worker in workers:
                        input_queue.put(worker)

                    pool = WorkerPool(
                        _DownloadProcessor(),
                        input_queue,
                        output_queue,
                        num_workers=worker_num,
                        name="download",
                    )
                    pool.start()
                    pool.stop(drain=True)

                    while not output_queue.empty():
                        results.append(output_queue.get_nowait())

                    completed = sum(results)
                    pbar.set_description(
                        desc=f"{prefix}|{completed}/{len(range_list)}|{self.filename}"
                    )
                    return completed == len(range_list)

            except Exception as e:
                logger.error(f"Download failed: {e}")
                return False
            finally:
                pbar.close()

        except Exception as e:
            logger.error(f"Unexpected error during download: {e}")
            return False

    def check_available(self) -> bool:
        """发送小范围请求，返回服务器是否支持 Range。"""
        if self.blocks_num < 1:
            return False

        try:
            headers = {"Range": "bytes=0-100"}
            headers.update(self.headers)

            with self._session.get(
                self.url,
                stream=True,
                headers=headers,
                timeout=self.timeout,
                auth=self.auth,
            ) as req:
                if req.status_code != 206:
                    logger.warning(f"Range request returned status {req.status_code}")
                return req.status_code == 206

        except Exception as e:
            logger.warning(f"Failed to check range request support: {e}")
            return False


def download(
    url: str,
    filepath: str,
    overwrite: bool = False,
    worker_num: int = 5,
    block_size: int = 100,
    prefix: str = "",
    max_retries: int = 3,
    **kwargs: Any,
) -> bool:
    """多线程下载文件

    Args:
        url: 下载链接
        filepath: 保存路径
        overwrite: 是否覆盖已存在的文件
        worker_num: 工作线程数
        block_size: 块大小(MB)
        prefix: 进度条前缀
        max_retries: 最大重试次数
        **kwargs: 传给 `MultiDownloader` 的其他参数

    Returns:
        bool: 下载是否成功
    """
    if worker_num <= 0:
        logger.error("worker_num must be positive")
        return False
    try:
        downloader = MultiDownloader(
            url=url,
            filepath=filepath,
            overwrite=overwrite,
            block_size=block_size,
            max_retries=max_retries,
            **kwargs,
        )
        return downloader.download(
            worker_num=worker_num,
            prefix=prefix,
            max_retries=max_retries,
        )
    except Exception as e:
        logger.error(f"Multi-threaded download failed: {e}")
        return False

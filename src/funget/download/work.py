# -*- coding: utf-8 -*-
import time
from queue import Empty, Queue
from threading import Thread
from typing import List, Optional, Callable, Any

import requests
from requests.adapters import HTTPAdapter
from urllib3.util.retry import Retry
from funutil import getLogger


logger = getLogger("funget")


def _update_callback(total, curser, current):
    """
    :param total: 总大小
    :param curser:当前下载的大小
    :param current: 最新一批次的大小
    :return:
    """
    pass


class Worker:
    def __init__(
        self,
        url: str,
        fileobj,
        range_start: int = 0,
        range_end: Optional[int] = None,
        update_callback: Optional[Callable] = None,
        finish_callback: Optional[Callable] = None,
        headers: Optional[dict] = None,
        chunk_size: int = 2 * 1024 * 1024,
        max_retries: int = 3,
        *args,
        **kwargs,
    ):
        super(Worker, self).__init__(*args, **kwargs)
        self.url = url
        self.fileobj = fileobj
        self.headers = headers or {}
        self.range_start = range_start
        self.range_curser = range_start
        self.range_end = range_end or self._get_size()
        self.size = self.range_end - self.range_start + 1
        self.update_callback = update_callback or _update_callback
        self.finish_callback = finish_callback
        self.chunk_size = chunk_size or 100 * 1024
        self.max_retries = max_retries
        self._session = self._create_session()

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

    def _get_size(self) -> int:
        """获取文件大小"""
        try:
            resp = self._session.head(self.url, headers=self.headers, timeout=30)
            resp.raise_for_status()
            return int(resp.headers.get("content-length", 0))
        except Exception as e:
            logger.warning(f"Failed to get file size via HEAD request: {e}")
            # fallback to GET request
            try:
                resp = self._session.get(
                    self.url, stream=True, headers=self.headers, timeout=30
                )
                resp.raise_for_status()
                return int(resp.headers.get("content-length", 0))
            except Exception as e:
                logger.error(f"Failed to get file size: {e}")
                return 0

    def run(self) -> bool:
        """执行下载任务"""
        for attempt in range(self.max_retries + 1):
            try:
                return self._download_chunk()
            except requests.exceptions.RequestException as e:
                logger.warning(f"Download attempt {attempt + 1} failed: {e}")
                if attempt == self.max_retries:
                    logger.error(
                        f"Download failed after {self.max_retries + 1} attempts"
                    )
                    raise
                # 指数退避
                time.sleep(2**attempt)
            except Exception as e:
                logger.error(f"Unexpected error during download: {e}")
                raise
        return False

    def _download_chunk(self) -> bool:
        """下载数据块"""
        headers = {"Range": f"bytes={self.range_curser}-{self.range_end}"}
        headers.update(self.headers)

        try:
            with self._session.get(
                self.url, stream=True, headers=headers, timeout=60
            ) as req:
                req.raise_for_status()

                # 检查状态码
                if req.status_code not in [200, 206, 416]:  # 416 表示范围请求无效
                    logger.warning(f"Unexpected status code: {req.status_code}")
                    return False

                # 如果是416错误，说明请求的范围无效，可能已经下载完成
                if req.status_code == 416:
                    logger.info(
                        "Range request invalid, chunk may be already downloaded"
                    )
                    return True

                for chunk in req.iter_content(chunk_size=self.chunk_size):
                    if chunk:  # 过滤掉空块
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

                if self.finish_callback:
                    self.finish_callback(self)
                return True

        except requests.exceptions.Timeout:
            logger.warning("Request timeout")
            raise
        except requests.exceptions.ConnectionError:
            logger.warning("Connection error")
            raise
        except requests.exceptions.HTTPError as e:
            logger.warning(f"HTTP error: {e}")
            raise

    def __lt__(self, another):
        return self.range_start < another.range_start

    def get_progress(self):
        """progress for each worker"""
        return {
            "curser": self.range_curser,
            "start": self.range_start,
            "end": self.range_end,
            "total": self.size,
        }


class WorkerFactory(object):
    def __init__(self, worker_num: int = 10, capacity: int = 100, timeout: int = 30):
        self.worker_num = worker_num
        self.timeout = timeout
        self._close = False
        self._task_queue = Queue(maxsize=capacity)
        self._threads: List[Thread] = []
        self._failed_tasks = []
        self.start()

    def submit(self, worker):
        self._task_queue.put(worker)

    def start(self):
        for i in range(self.worker_num):
            thread = Thread(target=self._worker)
            thread.start()
            self._threads.append(thread)

    def _worker(self):
        """工作线程主循环"""
        while not self._close:
            try:
                worker = self._task_queue.get(timeout=self.timeout)
                if worker is None:  # 毒丸，用于优雅关闭
                    break

                try:
                    success = worker.run()
                    if not success:
                        logger.warning(
                            f"Worker failed to download range {worker.range_start}-{worker.range_end}"
                        )
                        self._failed_tasks.append(worker)
                except Exception as e:
                    logger.error(f"Worker execution failed: {e}")
                    self._failed_tasks.append(worker)
                finally:
                    self._task_queue.task_done()

            except Empty:
                # 超时，检查是否需要关闭
                continue
            except Exception as e:
                logger.error(f"Unexpected error in worker thread: {e}")
                # 继续运行，不要因为单个错误而停止整个线程

    def close(self):
        """优雅关闭线程池"""
        self._close = True
        # 向每个线程发送毒丸
        for _ in self._threads:
            try:
                self._task_queue.put(None, timeout=1)
            except:
                pass  # 队列可能已满，忽略错误

    def wait_for_all_done(self):
        self._task_queue.join()

    def empty(self) -> bool:
        """检查任务队列是否为空"""
        return self._task_queue.empty()

    def get_failed_tasks(self) -> List[Any]:
        """获取失败的任务列表"""
        return self._failed_tasks.copy()

    def retry_failed_tasks(self):
        """重试失败的任务"""
        failed_tasks = self._failed_tasks.copy()
        self._failed_tasks.clear()

        for task in failed_tasks:
            logger.info(f"Retrying failed task: {task.range_start}-{task.range_end}")
            self.submit(task)

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        """上下文管理器退出时的清理工作"""
        try:
            # 等待所有任务完成，但设置超时
            start_time = time.time()
            timeout = 300  # 5分钟超时

            while not self.empty() and (time.time() - start_time) < timeout:
                time.sleep(0.1)

            self.wait_for_all_done()
            self.close()

            # 等待所有线程结束
            for thread in self._threads:
                thread.join(timeout=30)
                if thread.is_alive():
                    logger.warning(f"Thread {thread.name} did not terminate gracefully")

        except Exception as e:
            logger.error(f"Error during cleanup: {e}")

        return False  # 不抑制异常

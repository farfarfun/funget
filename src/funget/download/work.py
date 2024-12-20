# -*- coding: utf-8 -*-
import time
from queue import Empty, Queue
from threading import Thread
from typing import List

import requests
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
        range_start=0,
        range_end=None,
        update_callback=None,
        finish_callback=None,
        header=None,
        chunk_size=2 * 1024 * 1024,
        *args,
        **kwargs,
    ):
        super(Worker, self).__init__(*args, **kwargs)
        self.url = url
        self.fileobj = fileobj
        self.header = header or {}
        self.range_start = range_start
        self.range_curser = range_start
        self.range_end = range_end or self._get_size()
        self.size = self.range_end - self.range_start + 1
        self.update_callback = update_callback or _update_callback
        self.finish_callback = finish_callback
        self.chunk_size = chunk_size or 100 * 1024

    def _get_size(self):
        with requests.Session() as sess:
            resp = sess.get(self.url, stream=True)
            return int(resp.headers.get("content-length", 0))

    def run(self):
        header = {"Range": f"bytes={self.range_curser}-{self.range_end}"}
        header.update(self.header)
        with requests.get(self.url, stream=True, headers=header) as req:
            if 200 <= req.status_code <= 299:
                for chunk in req.iter_content(chunk_size=self.chunk_size):
                    _size = self.fileobj.write(chunk=chunk, offset=self.range_curser)
                    self.range_curser += _size
                    self.update_callback(self.size, self.range_curser, _size)
        if self.finish_callback:
            self.finish_callback(self)

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
    def __init__(self, worker_num=10, capacity=100, timeout=30):
        self.worker_num = worker_num
        self.timeout = timeout
        self._close = False
        self._task_queue = Queue(maxsize=capacity)
        self._threads: List[Thread] = []
        self.start()

    def submit(self, worker):
        self._task_queue.put(worker)

    def start(self):
        for i in range(self.worker_num):
            thread = Thread(target=self._worker)
            thread.start()
            self._threads.append(thread)

    def _worker(self):
        while True:
            try:
                worker = self._task_queue.get(timeout=self.timeout)
                worker.run()
                self._task_queue.task_done()
            except Exception as e:
                if isinstance(e, Empty):
                    pass
                logger.error(e)

            if self._close:
                break

    def close(self):
        self._close = True

    def wait_for_all_done(self):
        self._task_queue.join()

    def empty(self):
        return self._task_queue.empty()

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        while not self.empty():
            time.sleep(1)
        self.wait_for_all_done()
        self.close()
        return True

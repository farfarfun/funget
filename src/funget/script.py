import os

import typer

from funget import multi_thread_download


def download(url: str, worker: int = 10, block_size: int = 100, capacity: int = 100):
    filepath = f"./{os.path.basename(url)}"
    return multi_thread_download(
        url,
        filepath=filepath,
        worker_num=worker,
        block_size=block_size,
        capacity=capacity,
    )


def funget():
    typer.run(download)

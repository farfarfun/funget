# -*- coding: utf-8 -*-

import requests
from funfile.compress.utils import file_tqdm_bar
from funutil import getLogger
from .core import Uploader

logger = getLogger("funget")


class SingleUploader(Uploader):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)

    def upload(self, prefix="", chunk_size=256 * 1024, *args, **kwargs) -> bool:
        prefix = f"{prefix}--" if prefix is not None and len(prefix) > 0 else ""

        with open(self.filepath, "rb") as file:
            with file_tqdm_bar(
                path=self.filepath,
                total=self.filesize,
                prefix=f"{prefix}",
            ) as bar:
                # 定义一个生成器函数，用于分块读取文件并更新进度条
                def read_file():
                    while True:
                        data = file.read(chunk_size)
                        if not data:
                            break
                        yield data
                        bar.update(len(data))  # 更新进度条

                # 使用流式上传
                response = requests.put(self.url, data=read_file())

        logger.success(
            f"upload success with code:{response.status_code} from {self.url[:20]} to {self.filepath}"
        )
        return True


def upload(
    url,
    filepath,
    overwrite=False,
    prefix="",
    chunk_size=256 * 1024,
    *args,
    **kwargs,
):
    SingleUploader(
        url=url, filepath=filepath, overwrite=overwrite, *args, **kwargs
    ).upload(prefix=prefix, chunk_size=chunk_size, *args, **kwargs)

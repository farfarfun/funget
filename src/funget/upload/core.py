# -*- coding: utf-8 -*-
import os


class Uploader:
    def __init__(
        self,
        url,
        filepath,
        overwrite=False,
        filesize=None,
        headers=None,
        *args,
        **kwargs,
    ):
        self.url = url
        self.headers = headers or {}
        self.filepath = filepath
        self.overwrite = overwrite
        self.filesize = filesize or self.__get_size()
        self.filename = os.path.basename(self.filepath)

    def upload(self, *args, **kwargs):
        pass

    def __get_size(self):
        return os.path.getsize(self.filepath)

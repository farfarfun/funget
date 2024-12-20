from funget.download.multi import MultiDownloader
from funget.download.single import SingleDownloader


def download(
    url,
    filepath,
    multi=False,
    overwrite=False,
    prefix="",
    chunk_size=2048,
    *args,
    **kwargs,
):
    if multi:
        loader = MultiDownloader(
            url=url, filepath=filepath, overwrite=overwrite, *args, **kwargs
        )
    else:
        loader = SingleDownloader(
            url=url, filepath=filepath, overwrite=overwrite, *args, **kwargs
        )
    loader.download(prefix=prefix, chunk_size=chunk_size, *args, **kwargs)

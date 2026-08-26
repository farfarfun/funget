from funget import multi_thread_download, simple_download

simple_download(
    "https://httpbin.org/bytes/1024",
    "./downloads/small.bin",
    overwrite=True,
)

multi_thread_download(
    "https://httpbin.org/bytes/10485760",
    "./downloads/large.bin",
    worker_num=8,
    block_size=1,
    overwrite=True,
)

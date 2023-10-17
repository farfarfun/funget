import argparse
import os

from funget import simple_download, split_download


def download(args):
    url = args.url
    filepath = f'./{os.path.basename(url)}'
    if args.multi:
        return split_download(args.url, filepath=filepath, worker_num=args.worker,block_size=args.block_size, capacity=args.capacity)
    else:
        return simple_download(args.url, filepath=filepath)


def funget():
    parser = argparse.ArgumentParser(prog="PROG")

    # 添加子命令
    parser.add_argument("url", help="下载链接")
    parser.add_argument("--multi", default=False, action="store_true", help="build multi package")
    parser.add_argument("--worker", default=10, help="下载的多线程数量")
    parser.add_argument("--block_size", default=100, help="下载的块大小")
    parser.add_argument("--capacity", default=100, help="下载的容量")

    parser.set_defaults(func=download)  # 设置默认函数

    args = parser.parse_args()
    args.func(args)

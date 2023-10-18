import argparse
import os

from funget import simple_download, multi_thread_download


def download(args):
    url = args.url
    filepath = f'./{os.path.basename(url)}'
    return multi_thread_download(args.url, filepath=filepath, worker_num=args.worker,block_size=args.block_size, capacity=args.capacity)
    


def funget():
    parser = argparse.ArgumentParser(prog="PROG")

    # 添加子命令
    parser.add_argument("url", help="下载链接")
    parser.add_argument("--worker", default=10,type=int, help="下载的多线程数量")
    parser.add_argument("--block_size", default=100,type=int, help="下载的块大小")
    parser.add_argument("--capacity", default=100,type=int, help="下载的容量")

    parser.set_defaults(func=download)  # 设置默认函数

    args = parser.parse_known_args()
    args.func(args)

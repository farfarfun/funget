
import argparse
import logging
import os
import time

from funget import simple_download,split_download
from git import Repo


def download(args):
    if args.multi:
        return split_download(args.url,worker_num=worker,capacity=capacity)
    else:
        return simple_download(args.url)




def funget():
    parser = argparse.ArgumentParser(prog="PROG")
    subparsers = parser.add_subparsers(help="sub-command help")

    # 添加子命令
    subparsers.add_argument("--multi", default=False, action="store_true", help="build multi package")
    subparsers.add_argument("--worker", default=10, help="下载的多线程数量")
    subparsers.add_argument("--capacity", default=100, help="下载的容量")
    
    subparsers.set_defaults(func=download)  # 设置默认函数
    
    args = parser.parse_args()
    args.func(args)

import argparse
import os
from urllib.parse import urlsplit

from farlog import getLogger

from funget import multi_thread_download, simple_download
from funget.download.multi import MultiDownloader
from funget.upload import single_upload

logger = getLogger("funget")


def _download(args) -> int:
    output = args.output or os.path.basename(urlsplit(args.url).path) or "download"
    download_file = simple_download if args.single else multi_thread_download
    options = {
        "url": args.url,
        "filepath": output,
        "overwrite": args.overwrite,
        "max_retries": args.max_retries,
    }
    if not args.single:
        options.update(worker_num=args.worker, block_size=args.block_size)

    if download_file(**options):
        logger.success(f"Download completed: {output}")
        return 0
    logger.error("Download failed")
    return 1


def _upload(args) -> int:
    if single_upload(
        url=args.url,
        filepath=args.file_path,
        method=args.method,
        chunk_size=args.chunk_size,
        max_retries=args.max_retries,
    ):
        logger.success(f"Upload completed: {args.file_path}")
        return 0
    logger.error("Upload failed")
    return 1


def _info(args) -> int:
    downloader = MultiDownloader(url=args.url, filepath="/tmp/funget-info")
    info = downloader.get_file_info()
    print(f"URL: {info['url']}")
    print(f"Filename: {info['filename']}")
    print(f"Size: {info['filesize']:,} bytes")
    print(
        f"Range requests: {'supported' if downloader.supports_range else 'unsupported'}"
    )
    return 0


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="funget", description="Download and upload files"
    )
    commands = parser.add_subparsers(dest="command", required=True)

    download = commands.add_parser("download", help="download a file")
    download.add_argument("url")
    download.add_argument("-o", "--output")
    download.add_argument("-w", "--worker", type=int, default=10)
    download.add_argument("-b", "--block-size", type=int, default=100)
    download.add_argument("-r", "--max-retries", type=int, default=3)
    download.add_argument("--single", action="store_true")
    download.add_argument("--overwrite", action="store_true")
    download.set_defaults(handler=_download)

    upload = commands.add_parser("upload", help="upload a file")
    upload.add_argument("file_path")
    upload.add_argument("url")
    upload.add_argument("-m", "--method", choices=("PUT", "POST"), default="PUT")
    upload.add_argument("-c", "--chunk-size", type=int, default=256 * 1024)
    upload.add_argument("-r", "--max-retries", type=int, default=3)
    upload.set_defaults(handler=_upload)

    info = commands.add_parser("info", help="show remote file information")
    info.add_argument("url")
    info.set_defaults(handler=_info)
    return parser


def funget() -> int:
    """解析命令行参数并执行对应命令，返回进程退出码。"""
    args = _parser().parse_args()
    try:
        return args.handler(args)
    except KeyboardInterrupt:
        logger.warning("Interrupted")
        return 1
    except Exception as e:
        logger.error(f"Error: {e}")
        return 1

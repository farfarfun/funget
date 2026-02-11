import os
from typing import Optional

import typer
from funlog import getLogger

from funget import multi_thread_download, simple_download
from funget.upload import single_upload

logger = getLogger("funget")

app = typer.Typer(help="Funget - A fast and reliable file downloader")


@app.command()
def download(
    url: str = typer.Argument(..., help="URL to download"),
    output: Optional[str] = typer.Option(
        None, "-o", "--output", help="Output file path"
    ),
    worker: int = typer.Option(10, "-w", "--worker", help="Number of worker threads"),
    block_size: int = typer.Option(100, "-b", "--block-size", help="Block size in MB"),
    capacity: int = typer.Option(100, "-c", "--capacity", help="Queue capacity"),
    max_retries: int = typer.Option(
        3, "-r", "--max-retries", help="Maximum retry attempts"
    ),
    single_thread: bool = typer.Option(
        False, "--single", help="Use single-thread download"
    ),
    overwrite: bool = typer.Option(
        False, "--overwrite", help="Overwrite existing files"
    ),
    verbose: bool = typer.Option(
        False, "-v", "--verbose", help="Enable verbose logging"
    ),
):
    """Download a file from the given URL"""

    # 设置日志级别
    if verbose:
        logger.setLevel("DEBUG")

    # 确定输出路径
    if output is None:
        filename = os.path.basename(url.split("?")[0])  # 移除查询参数
        if not filename:
            filename = "download"
        output = f"./{filename}"

    # 确保输出目录存在
    output_dir = os.path.dirname(os.path.abspath(output))
    os.makedirs(output_dir, exist_ok=True)

    logger.info(f"Starting download: {url}")
    logger.info(f"Output file: {output}")

    try:
        if single_thread:
            logger.info("Using single-thread download")
            success = simple_download(
                url=url,
                filepath=output,
                overwrite=overwrite,
            )
        else:
            logger.info(f"Using multi-thread download with {worker} workers")
            success = multi_thread_download(
                url=url,
                filepath=output,
                worker_num=worker,
                block_size=block_size,
                capacity=capacity,
                overwrite=overwrite,
                max_retries=max_retries,
            )

        if success:
            logger.success(f"Download completed successfully: {output}")
            typer.echo(f"✅ Download completed: {output}")
        else:
            logger.error("Download failed")
            typer.echo("❌ Download failed", err=True)
            raise typer.Exit(1)

    except KeyboardInterrupt:
        logger.warning("Download interrupted by user")
        typer.echo("⚠️  Download interrupted", err=True)
        raise typer.Exit(1)
    except Exception as e:
        logger.error(f"Unexpected error: {e}")
        typer.echo(f"❌ Error: {e}", err=True)
        raise typer.Exit(1)


@app.command()
def upload(
    file_path: str = typer.Argument(..., help="Local file path to upload"),
    url: str = typer.Argument(..., help="Upload URL"),
    method: str = typer.Option(
        "PUT", "-m", "--method", help="HTTP method (PUT or POST)"
    ),
    chunk_size: int = typer.Option(
        256 * 1024, "-c", "--chunk-size", help="Chunk size in bytes"
    ),
    max_retries: int = typer.Option(
        3, "-r", "--max-retries", help="Maximum retry attempts"
    ),
    verbose: bool = typer.Option(
        False, "-v", "--verbose", help="Enable verbose logging"
    ),
):
    """Upload a file to the given URL"""

    # 设置日志级别
    if verbose:
        logger.setLevel("DEBUG")

    # 验证文件存在
    if not os.path.exists(file_path):
        typer.echo(f"❌ File not found: {file_path}", err=True)
        raise typer.Exit(1)

    # 获取文件信息
    file_size = os.path.getsize(file_path)
    filename = os.path.basename(file_path)

    logger.info(f"Starting upload: {file_path} -> {url}")
    logger.info(
        f"File: {filename} ({file_size:,} bytes, {file_size / (1024 * 1024):.2f} MB)"
    )

    try:
        success = single_upload(
            url=url,
            filepath=file_path,
            method=method.upper(),
            chunk_size=chunk_size,
            max_retries=max_retries,
        )

        if success:
            logger.success(f"Upload completed successfully: {filename}")
            typer.echo(f"✅ Upload completed: {filename}")
        else:
            logger.error("Upload failed")
            typer.echo("❌ Upload failed", err=True)
            raise typer.Exit(1)

    except KeyboardInterrupt:
        logger.warning("Upload interrupted by user")
        typer.echo("⚠️  Upload interrupted", err=True)
        raise typer.Exit(1)
    except Exception as e:
        logger.error(f"Unexpected error: {e}")
        typer.echo(f"❌ Error: {e}", err=True)
        raise typer.Exit(1)


@app.command()
def info(url: str = typer.Argument(..., help="URL to get information about")):
    """Get information about a downloadable file"""
    try:
        from funget.download.core import Downloader

        # 创建一个临时下载器来获取文件信息
        downloader = Downloader(url=url, filepath="/tmp/temp")
        info = downloader.get_file_info()

        typer.echo("📄 File Information:")
        typer.echo(f"   URL: {info['url']}")
        typer.echo(f"   Filename: {info['filename']}")
        typer.echo(
            f"   Size: {info['filesize']:,} bytes ({info['filesize'] / (1024 * 1024):.2f} MB)"
        )

        # 检查是否支持范围请求
        from funget.download.multi import MultiDownloader

        multi_downloader = MultiDownloader(url=url, filepath="/tmp/temp")
        supports_range = multi_downloader.check_available()
        typer.echo(
            f"   Range requests: {'✅ Supported' if supports_range else '❌ Not supported'}"
        )

    except Exception as e:
        typer.echo(f"❌ Error getting file info: {e}", err=True)
        raise typer.Exit(1)


def funget():
    """Entry point for the funget command"""
    app()

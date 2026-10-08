import os
from typing import Annotated, Literal
from urllib.parse import urlsplit

import typer
from farlog import getLogger

from funget import multi_thread_download, simple_download
from funget.download.multi import MultiDownloader
from funget.upload import single_upload

logger = getLogger("funget")
app = typer.Typer(help="Download and upload files")


def _failed(error: Exception) -> None:
    logger.error(f"Error: {error}")
    raise typer.Exit(1) from error


@app.command()
def download(
    url: str,
    output: Annotated[str | None, typer.Option("--output", "-o")] = None,
    worker: Annotated[int, typer.Option("--worker", "-w")] = 10,
    block_size: Annotated[int, typer.Option("--block-size", "-b")] = 100,
    max_retries: Annotated[int, typer.Option("--max-retries", "-r")] = 3,
    single: bool = False,
    overwrite: bool = False,
) -> None:
    """Download a file."""
    destination = output or os.path.basename(urlsplit(url).path) or "download"
    download_file = simple_download if single else multi_thread_download
    options = {
        "url": url,
        "filepath": destination,
        "overwrite": overwrite,
        "max_retries": max_retries,
    }
    if not single:
        options.update(worker_num=worker, block_size=block_size)
    try:
        succeeded = download_file(**options)
    except KeyboardInterrupt:
        logger.warning("Interrupted")
        raise typer.Exit(1) from None
    except Exception as error:
        _failed(error)
    if not succeeded:
        logger.error("Download failed")
        raise typer.Exit(1)
    logger.success(f"Download completed: {destination}")


@app.command()
def upload(
    file_path: str,
    url: str,
    method: Annotated[Literal["PUT", "POST"], typer.Option("--method", "-m")] = "PUT",
    chunk_size: Annotated[int, typer.Option("--chunk-size", "-c")] = 256 * 1024,
    max_retries: Annotated[int, typer.Option("--max-retries", "-r")] = 3,
) -> None:
    """Upload a file."""
    try:
        succeeded = single_upload(
            url=url,
            filepath=file_path,
            method=method,
            chunk_size=chunk_size,
            max_retries=max_retries,
        )
    except KeyboardInterrupt:
        logger.warning("Interrupted")
        raise typer.Exit(1) from None
    except Exception as error:
        _failed(error)
    if not succeeded:
        logger.error("Upload failed")
        raise typer.Exit(1)
    logger.success(f"Upload completed: {file_path}")


@app.command()
def info(url: str) -> None:
    """Show remote file information."""
    try:
        downloader = MultiDownloader(url=url, filepath="/tmp/funget-info")
        details = downloader.get_file_info()
    except KeyboardInterrupt:
        logger.warning("Interrupted")
        raise typer.Exit(1) from None
    except Exception as error:
        _failed(error)
    print(f"URL: {details['url']}")
    print(f"Filename: {details['filename']}")
    print(f"Size: {details['filesize']:,} bytes")
    print(
        f"Range requests: {'supported' if downloader.supports_range else 'unsupported'}"
    )


def funget() -> None:
    """Run the command-line interface."""
    app()

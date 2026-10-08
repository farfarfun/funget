from unittest.mock import patch

from typer.testing import CliRunner

from funget.script import app

runner = CliRunner()


def test_help_keeps_existing_commands():
    result = runner.invoke(app, ["--help"])

    assert result.exit_code == 0
    assert "download" in result.stdout
    assert "upload" in result.stdout
    assert "info" in result.stdout


@patch("funget.script.simple_download", return_value=True)
def test_download_accepts_existing_arguments(download):
    result = runner.invoke(
        app,
        ["download", "https://example.com/file.zip", "-o", "file.zip", "--single"],
    )

    assert result.exit_code == 0
    download.assert_called_once_with(
        url="https://example.com/file.zip",
        filepath="file.zip",
        overwrite=False,
        max_retries=3,
    )


@patch("funget.script.single_upload", return_value=False)
def test_upload_failure_returns_one(upload):
    result = runner.invoke(
        app, ["upload", "file.zip", "https://example.com/upload", "-m", "POST"]
    )

    assert result.exit_code == 1
    upload.assert_called_once()


def test_upload_rejects_unknown_method_with_usage_error():
    result = runner.invoke(
        app, ["upload", "file.zip", "https://example.com/upload", "-m", "PATCH"]
    )

    assert result.exit_code == 2

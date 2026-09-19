import os
import tempfile
import unittest
from unittest.mock import Mock, patch

import requests

from funget.upload import single_upload


class TestUpload(unittest.TestCase):
    @patch("funget.upload.single.file_tqdm_bar")
    @patch("funget.upload.single.requests.Session")
    def test_put_upload(self, mock_session, mock_pbar):
        response = Mock()
        response.raise_for_status.return_value = None
        mock_session.return_value.put.return_value = response

        with tempfile.NamedTemporaryFile(delete=False) as file:
            file.write(b"test")
            filepath = file.name

        try:
            self.assertTrue(single_upload("https://example.com/file", filepath))
            mock_session.return_value.put.assert_called_once()
            mock_pbar.return_value.close.assert_called_once()
            mock_session.return_value.close.assert_called_once()
        finally:
            os.remove(filepath)

    @patch("funget.upload.single.file_tqdm_bar")
    @patch("funget.upload.single.requests.Session")
    def test_post_upload(self, mock_session, mock_pbar):
        response = Mock()
        response.raise_for_status.return_value = None
        mock_session.return_value.post.return_value = response

        with tempfile.NamedTemporaryFile(delete=False) as file:
            file.write(b"test")
            filepath = file.name

        try:
            self.assertTrue(
                single_upload(
                    "https://example.com/file",
                    filepath,
                    method="POST",
                    headers={"X-Test": "yes"},
                )
            )
            kwargs = mock_session.return_value.post.call_args.kwargs
            self.assertEqual(kwargs["headers"], {"X-Test": "yes"})
            self.assertEqual(kwargs["files"]["file"][0], os.path.basename(filepath))
            mock_pbar.return_value.update.assert_called_once_with(4)
        finally:
            os.remove(filepath)

    @patch("funget.upload.single.time.sleep")
    @patch("funget.upload.single.file_tqdm_bar")
    @patch("funget.upload.single.requests.Session")
    def test_put_upload_retries_request_failures(
        self, mock_session, mock_pbar, mock_sleep
    ):
        response = Mock()
        response.raise_for_status.return_value = None
        mock_session.return_value.put.side_effect = [
            requests.ConnectionError("temporary"),
            response,
        ]

        with tempfile.NamedTemporaryFile(delete=False) as file:
            file.write(b"test")
            filepath = file.name

        try:
            self.assertTrue(
                single_upload("https://example.com/file", filepath, max_retries=1)
            )
            self.assertEqual(mock_session.return_value.put.call_count, 2)
            mock_sleep.assert_called_once_with(1)
            mock_pbar.return_value.reset.assert_called_once_with(total=4)
        finally:
            os.remove(filepath)

    @patch("funget.upload.single.requests.Session")
    def test_upload_rejects_invalid_inputs(self, mock_session):
        with tempfile.NamedTemporaryFile(delete=False) as file:
            nonempty = file.name
            file.write(b"test")
        with tempfile.NamedTemporaryFile(delete=False) as file:
            empty = file.name

        try:
            for kwargs in (
                {"method": "PATCH"},
                {"chunk_size": 0},
                {"max_retries": -1},
                {"timeout": 0},
            ):
                with self.subTest(kwargs=kwargs):
                    self.assertFalse(
                        single_upload("https://example.com/file", nonempty, **kwargs)
                    )
            self.assertFalse(single_upload("https://example.com/file", empty))
            self.assertFalse(
                single_upload("https://example.com/file", f"{nonempty}.missing")
            )
            mock_session.assert_not_called()
        finally:
            os.remove(nonempty)
            os.remove(empty)


if __name__ == "__main__":
    unittest.main()

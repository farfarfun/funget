import os
import tempfile
import unittest
from unittest.mock import Mock, patch

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


if __name__ == "__main__":
    unittest.main()

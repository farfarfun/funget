# -*- coding: utf-8 -*-
"""
下载器模块测试
"""

import os
import tempfile
import unittest
from unittest.mock import Mock, patch, MagicMock
from funget.download.core import Downloader
from funget.download.single import SingleDownloader
from funget.download.multi import MultiDownloader


class TestDownloader(unittest.TestCase):
    """下载器基类测试"""

    def setUp(self):
        """测试前准备"""
        self.test_url = "https://httpbin.org/bytes/1024"
        self.test_filepath = "/tmp/test_file"

    def test_downloader_init(self):
        """测试下载器初始化"""
        with patch.object(Downloader, "_Downloader__get_size", return_value=1024):
            downloader = Downloader(url=self.test_url, filepath=self.test_filepath)

            self.assertEqual(downloader.url, self.test_url)
            self.assertEqual(downloader.filepath, self.test_filepath)
            self.assertEqual(downloader.filesize, 1024)
            self.assertEqual(downloader.filename, "test_file")
            self.assertFalse(downloader.overwrite)
            self.assertEqual(downloader.max_retries, 3)
            self.assertEqual(downloader.timeout, 30)

    def test_get_file_info(self):
        """测试获取文件信息"""
        with patch.object(Downloader, "_Downloader__get_size", return_value=2048):
            downloader = Downloader(
                url=self.test_url, filepath=self.test_filepath, overwrite=True
            )

            info = downloader.get_file_info()
            expected_info = {
                "url": self.test_url,
                "filepath": self.test_filepath,
                "filename": "test_file",
                "filesize": 2048,
                "overwrite": True,
            }

            self.assertEqual(info, expected_info)

    @patch("requests.Session")
    def test_validate_url(self, mock_session):
        """测试URL验证"""
        # 模拟成功的HEAD请求
        mock_response = Mock()
        mock_response.status_code = 200
        mock_session.return_value.head.return_value = mock_response

        with patch.object(Downloader, "_Downloader__get_size", return_value=1024):
            downloader = Downloader(self.test_url, self.test_filepath)
            self.assertTrue(downloader.validate_url())

        # 模拟失败的请求
        mock_session.return_value.head.side_effect = Exception("Network error")
        mock_session.return_value.head.return_value = None

        with patch.object(Downloader, "_Downloader__get_size", return_value=1024):
            downloader = Downloader(self.test_url, self.test_filepath)
            self.assertFalse(downloader.validate_url())


class TestSingleDownloader(unittest.TestCase):
    """单线程下载器测试"""

    def setUp(self):
        """测试前准备"""
        self.test_url = "https://httpbin.org/bytes/1024"
        self.temp_dir = tempfile.mkdtemp()
        self.test_filepath = os.path.join(self.temp_dir, "test_file")

    def tearDown(self):
        """测试后清理"""
        if os.path.exists(self.test_filepath):
            os.remove(self.test_filepath)
        os.rmdir(self.temp_dir)

    @patch("requests.Session")
    def test_single_downloader_init(self, mock_session):
        """测试单线程下载器初始化"""
        with patch.object(SingleDownloader, "_Downloader__get_size", return_value=1024):
            downloader = SingleDownloader(
                url=self.test_url, filepath=self.test_filepath
            )

            self.assertIsInstance(downloader, Downloader)
            self.assertEqual(downloader.url, self.test_url)
            self.assertEqual(downloader.filepath, self.test_filepath)


class TestMultiDownloader(unittest.TestCase):
    """多线程下载器测试"""

    def setUp(self):
        """测试前准备"""
        self.test_url = "https://httpbin.org/bytes/104857600"  # 100MB
        self.temp_dir = tempfile.mkdtemp()
        self.test_filepath = os.path.join(self.temp_dir, "test_file")

    def tearDown(self):
        """测试后清理"""
        if os.path.exists(self.test_filepath):
            os.remove(self.test_filepath)
        os.rmdir(self.temp_dir)

    def test_multi_downloader_init(self):
        """测试多线程下载器初始化"""
        with patch.object(
            MultiDownloader, "_Downloader__get_size", return_value=104857600
        ):
            with patch.object(MultiDownloader, "check_available", return_value=True):
                downloader = MultiDownloader(
                    url=self.test_url, filepath=self.test_filepath, block_size=50
                )

                self.assertIsInstance(downloader, Downloader)
                self.assertEqual(downloader.url, self.test_url)
                self.assertEqual(downloader.filepath, self.test_filepath)
                self.assertGreater(downloader.blocks_num, 1)

    def test_get_range_calculation(self):
        """测试范围计算"""
        with patch.object(MultiDownloader, "_Downloader__get_size", return_value=1000):
            with patch.object(MultiDownloader, "check_available", return_value=True):
                downloader = MultiDownloader(
                    url=self.test_url,
                    filepath=self.test_filepath,
                    block_size=1,  # 1MB blocks
                )

                # 由于文件大小只有1000字节，应该只有1个块
                self.assertEqual(downloader.blocks_num, 1)

                ranges = downloader._MultiDownloader__get_range()
                self.assertEqual(len(ranges), 1)
                self.assertEqual(ranges[0], (0, 999))

    @patch("requests.get")
    def test_check_available(self, mock_get):
        """测试范围请求支持检查"""
        # 模拟支持范围请求的响应
        mock_response = Mock()
        mock_response.status_code = 206
        mock_get.return_value.__enter__.return_value = mock_response

        with patch.object(MultiDownloader, "_Downloader__get_size", return_value=1024):
            downloader = MultiDownloader(url=self.test_url, filepath=self.test_filepath)

            self.assertTrue(downloader.check_available())

        # 模拟不支持范围请求的响应
        mock_response.status_code = 200
        mock_response.headers = {}

        with patch.object(MultiDownloader, "_Downloader__get_size", return_value=1024):
            downloader = MultiDownloader(url=self.test_url, filepath=self.test_filepath)

            self.assertFalse(downloader.check_available())


if __name__ == "__main__":
    unittest.main()

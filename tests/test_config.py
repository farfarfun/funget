# -*- coding: utf-8 -*-
"""
配置管理模块测试
"""

import os
import tempfile
import unittest
from funget.config import DownloadConfig, UploadConfig, FungetConfig


class TestConfig(unittest.TestCase):
    """配置管理测试类"""

    def test_download_config_defaults(self):
        """测试下载配置默认值"""
        config = DownloadConfig()
        self.assertEqual(config.worker_num, 10)
        self.assertEqual(config.capacity, 100)
        self.assertEqual(config.block_size, 100)
        self.assertEqual(config.max_retries, 3)
        self.assertEqual(config.timeout, 30)
        self.assertFalse(config.overwrite)
        self.assertTrue(config.create_dirs)
        self.assertIsInstance(config.headers, dict)

    def test_upload_config_defaults(self):
        """测试上传配置默认值"""
        config = UploadConfig()
        self.assertEqual(config.chunk_size, 256 * 1024)
        self.assertEqual(config.max_retries, 3)
        self.assertEqual(config.timeout, 60)
        self.assertEqual(config.method, "PUT")
        self.assertFalse(config.overwrite)
        self.assertIsInstance(config.headers, dict)

    def test_funget_config_creation(self):
        """测试主配置创建"""
        config = FungetConfig()
        self.assertIsInstance(config.download, DownloadConfig)
        self.assertIsInstance(config.upload, UploadConfig)
        self.assertEqual(config.log_level, "INFO")
        self.assertTrue(config.progress_bar)

    def test_config_from_dict(self):
        """测试从字典创建配置"""
        config_dict = {
            "download": {"worker_num": 20, "max_retries": 5},
            "upload": {"method": "POST", "timeout": 120},
            "log_level": "DEBUG",
        }

        config = FungetConfig.from_dict(config_dict)
        self.assertEqual(config.download.worker_num, 20)
        self.assertEqual(config.download.max_retries, 5)
        self.assertEqual(config.upload.method, "POST")
        self.assertEqual(config.upload.timeout, 120)
        self.assertEqual(config.log_level, "DEBUG")

    def test_config_from_env(self):
        """测试从环境变量创建配置"""
        # 设置环境变量
        os.environ["FUNGET_WORKER_NUM"] = "15"
        os.environ["FUNGET_MAX_RETRIES"] = "5"
        os.environ["FUNGET_LOG_LEVEL"] = "DEBUG"

        try:
            config = FungetConfig.from_env()
            self.assertEqual(config.download.worker_num, 15)
            self.assertEqual(config.download.max_retries, 5)
            self.assertEqual(config.log_level, "DEBUG")
        finally:
            # 清理环境变量
            for key in ["FUNGET_WORKER_NUM", "FUNGET_MAX_RETRIES", "FUNGET_LOG_LEVEL"]:
                os.environ.pop(key, None)

    def test_config_to_dict(self):
        """测试配置转换为字典"""
        config = FungetConfig()
        config.download.worker_num = 15
        config.upload.method = "POST"
        config.log_level = "DEBUG"

        config_dict = config.to_dict()
        self.assertEqual(config_dict["download"]["worker_num"], 15)
        self.assertEqual(config_dict["upload"]["method"], "POST")
        self.assertEqual(config_dict["log_level"], "DEBUG")


if __name__ == "__main__":
    unittest.main()

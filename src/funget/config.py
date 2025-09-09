# -*- coding: utf-8 -*-
"""
Funget 配置管理模块
提供统一的配置管理和默认值设置
"""

import os
from dataclasses import dataclass
from typing import Optional, Dict, Any


@dataclass
class DownloadConfig:
    """下载配置类"""

    # 基础配置
    worker_num: int = 10
    capacity: int = 100
    block_size: int = 100  # MB
    chunk_size: int = 2048  # bytes
    max_retries: int = 3
    timeout: int = 30  # seconds

    # 文件配置
    overwrite: bool = False
    create_dirs: bool = True

    # 网络配置
    headers: Optional[Dict[str, str]] = None

    # 自动选择配置
    auto_multi_threshold: int = 10 * 1024 * 1024  # 10MB

    def __post_init__(self):
        if self.headers is None:
            self.headers = {}


@dataclass
class UploadConfig:
    """上传配置类"""

    # 基础配置
    chunk_size: int = 256 * 1024  # bytes
    max_retries: int = 3
    timeout: int = 60  # seconds
    method: str = "PUT"  # PUT or POST

    # 文件配置
    overwrite: bool = False

    # 网络配置
    headers: Optional[Dict[str, str]] = None

    def __post_init__(self):
        if self.headers is None:
            self.headers = {}


@dataclass
class FungetConfig:
    """Funget 主配置类"""

    download: DownloadConfig
    upload: UploadConfig

    # 全局配置
    log_level: str = "INFO"
    progress_bar: bool = True

    def __init__(
        self,
        download_config: Optional[DownloadConfig] = None,
        upload_config: Optional[UploadConfig] = None,
        **kwargs,
    ):
        self.download = download_config or DownloadConfig()
        self.upload = upload_config or UploadConfig()

        # 设置其他属性
        for key, value in kwargs.items():
            if hasattr(self, key):
                setattr(self, key, value)

    @classmethod
    def from_dict(cls, config_dict: Dict[str, Any]) -> "FungetConfig":
        """从字典创建配置"""
        download_dict = config_dict.get("download", {})
        upload_dict = config_dict.get("upload", {})

        download_config = DownloadConfig(**download_dict)
        upload_config = UploadConfig(**upload_dict)

        # 移除已处理的键
        other_config = {
            k: v for k, v in config_dict.items() if k not in ["download", "upload"]
        }

        return cls(
            download_config=download_config, upload_config=upload_config, **other_config
        )

    @classmethod
    def from_env(cls) -> "FungetConfig":
        """从环境变量创建配置"""
        config = cls()

        # 下载配置
        if os.getenv("FUNGET_WORKER_NUM"):
            config.download.worker_num = int(os.getenv("FUNGET_WORKER_NUM"))
        if os.getenv("FUNGET_BLOCK_SIZE"):
            config.download.block_size = int(os.getenv("FUNGET_BLOCK_SIZE"))
        if os.getenv("FUNGET_MAX_RETRIES"):
            config.download.max_retries = int(os.getenv("FUNGET_MAX_RETRIES"))
        if os.getenv("FUNGET_TIMEOUT"):
            config.download.timeout = int(os.getenv("FUNGET_TIMEOUT"))

        # 上传配置
        if os.getenv("FUNGET_UPLOAD_METHOD"):
            config.upload.method = os.getenv("FUNGET_UPLOAD_METHOD")
        if os.getenv("FUNGET_UPLOAD_TIMEOUT"):
            config.upload.timeout = int(os.getenv("FUNGET_UPLOAD_TIMEOUT"))

        # 全局配置
        if os.getenv("FUNGET_LOG_LEVEL"):
            config.log_level = os.getenv("FUNGET_LOG_LEVEL")

        return config

    def to_dict(self) -> Dict[str, Any]:
        """转换为字典"""
        return {
            "download": {
                "worker_num": self.download.worker_num,
                "capacity": self.download.capacity,
                "block_size": self.download.block_size,
                "chunk_size": self.download.chunk_size,
                "max_retries": self.download.max_retries,
                "timeout": self.download.timeout,
                "overwrite": self.download.overwrite,
                "create_dirs": self.download.create_dirs,
                "headers": self.download.headers,
                "auto_multi_threshold": self.download.auto_multi_threshold,
            },
            "upload": {
                "chunk_size": self.upload.chunk_size,
                "max_retries": self.upload.max_retries,
                "timeout": self.upload.timeout,
                "method": self.upload.method,
                "overwrite": self.upload.overwrite,
                "headers": self.upload.headers,
            },
            "log_level": self.log_level,
            "progress_bar": self.progress_bar,
        }


# 全局默认配置实例
default_config = FungetConfig()


def get_config() -> FungetConfig:
    """获取当前配置"""
    return default_config


def set_config(config: FungetConfig):
    """设置全局配置"""
    global default_config
    default_config = config


def load_config_from_file(filepath: str) -> FungetConfig:
    """从文件加载配置"""
    import json
    import yaml

    try:
        with open(filepath, "r", encoding="utf-8") as f:
            if filepath.endswith(".json"):
                config_dict = json.load(f)
            elif filepath.endswith((".yml", ".yaml")):
                config_dict = yaml.safe_load(f)
            else:
                raise ValueError(f"Unsupported config file format: {filepath}")

        return FungetConfig.from_dict(config_dict)
    except Exception as e:
        raise ValueError(f"Failed to load config from {filepath}: {e}")


def save_config_to_file(config: FungetConfig, filepath: str):
    """保存配置到文件"""
    import json
    import yaml

    config_dict = config.to_dict()

    try:
        os.makedirs(os.path.dirname(filepath), exist_ok=True)

        with open(filepath, "w", encoding="utf-8") as f:
            if filepath.endswith(".json"):
                json.dump(config_dict, f, indent=2, ensure_ascii=False)
            elif filepath.endswith((".yml", ".yaml")):
                yaml.dump(config_dict, f, default_flow_style=False, allow_unicode=True)
            else:
                raise ValueError(f"Unsupported config file format: {filepath}")

    except Exception as e:
        raise ValueError(f"Failed to save config to {filepath}: {e}")

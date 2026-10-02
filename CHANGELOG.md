# Changelog

## 未发布

### 修复

- README `Python API` 示例中 `simple_download`/`multi_thread_download` 调用缺少
  `url`/`filepath` 定义，复制运行会触发 `NameError`，改为可直接运行的完整示例。
- `Downloader.validate_url()` 捕获异常后直接返回 `False`，未留下任何日志或上下文，
  补充 `logger.warning` 记录失败原因，便于排查。

## 1.1.70

### 新增

- 补充公共下载、POST 上传、重试和失败边界的回归测试。

### 修复

（无）

### 变更

- 构建后端迁移至 Hatchling，并提交 `uv.lock` 以保证依赖可复现。
- 公开 API 改用 Python 3.12 类型语法并补齐中文 docstring。
- `farlog` 最低版本提升至 1.1.7。

### 废弃

（无）

## 1.1.69 及更早版本

早期版本未维护 CHANGELOG，具体变更参见 git 提交历史。

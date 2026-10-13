# Changelog

## 未发布

### 新增

- **发布到 crates.io**，Rust 用户不再需要从源码或 Release 页取用：
  - [`funget`](https://crates.io/crates/funget) —— 命令行工具，`cargo install funget`。
    版本号与 PyPI 保持一致，三条分发渠道的 `funget --version` 报同一个值。
  - [`funget-core`](https://crates.io/crates/funget-core) —— 下载/上传引擎，可作为
    Rust 库独立使用。**版本线独立，从 0.1.0 起**：它的 9 个 `pub mod` 当初是为了让
    CLI 和 PyO3 扩展跨 crate 调用才公开的，并非按公共 Rust API 设计，0.x 期间仍会
    调整。PyPI 侧的 1.1.x 不受其影响。

### 安全

- **pyo3 0.23 → 0.29**，修掉三条影响默认分支的通告：`PyList`/`PyTuple` 迭代器
  `nth`/`nth_back` 的越界读（高危）、`PyCFunction::new_closure` 缺 `Sync` 约束（中危）、
  `PyString::from_object` 的缓冲区溢出风险（低危）。funget 的扩展层没有用到这三个
  API，但依赖在版本范围内，一并升掉。`pyo3-log` 同步升到 0.13。

  升级只改了一处写法：`Python::allow_threads` 在 0.27 更名为 `Python::detach`，
  语义和签名完全一致，释放 GIL 的行为不变。

### 变更

- CLI crate 由 `funget-cli` 改名为 `funget`（目录 `crates/funget-cli/` →
  `crates/funget/`），使 `cargo install funget` 直接装出 `funget` 命令。二进制名、
  命令行参数和行为均无变化。
- 构建 Python 扩展所需的最低 Rust 版本提升至 **1.83**（pyo3 0.29 的要求）。
  只影响从源码编译 wheel 的场景；装预编译 wheel 不需要 Rust 工具链。发到
  crates.io 的 `funget` 和 `funget-core` 不依赖 pyo3，MSRV 仍是 1.82。
- **仓库布局调整**，消除「`src/` 到底归谁」的歧义。纯内部改动，wheel 与 sdist 的
  内容、Python 包的导入路径、CLI 行为均不变：
  - 根目录改为**纯虚拟工作区**（只有 `[workspace]`，不再是 crate）。
  - `src/lib.rs` → `crates/funget-py/src/lib.rs`，PyO3 扩展成为正式的工作区成员。
  - `python/funget/` → `src/funget/`，`src/` 自此只属于 Python（src layout）。
  - 所有 Rust 代码统一在 `crates/` 下。
  - 新增 `.project-structure.json` 与 `docs/project/project-overview.md`，把上面的
    归属写成机器可校验的声明。

## 1.1.71

### 重写：实现全面迁移至 Rust

`download` / `simple_download` / `multi_thread_download` / `single_upload`
四个函数的**签名与返回值完全不变**，下游无需改动任何代码即可升级。实现改为
PyO3 扩展，Python 侧只剩再导出。

- **运行时依赖清零**：不再依赖 `requests`、`urllib3`、`funfile`、`funworker`、
  `typer`、`farlog`。
- **下载期间释放 GIL**，调用方的其他线程不再被阻塞。
- 并发分块从「线程池 + `funfile.ConcurrentFile` 的后台队列」换成 tokio +
  原生定位写（`write_all_at` / `seek_write`），内存占用显著下降。
- 新增**独立 CLI 二进制**：不装 Python 环境也能用，随 GitHub Release 分发。
  `pip install funget` 提供的 `funget` 命令与它共用同一套实现。
- 发布改为 GitHub Actions 多平台构建 wheel（abi3-py312，一个 wheel 覆盖
  3.12 及以后所有版本）。

### 新增

- **断点续传**：四个下载函数新增关键字参数 `resume`（默认 `False`，保持原行为）。
  开启后会在目标文件旁维护 `<文件名>.funget.json` 状态文件，中断后重跑只补未完成
  的字节，下载成功即删除。单线程路径额外发送 `If-Range`，远端文件变化时自动重下。
  校验基于 ETag（缺失时退回 Last-Modified）、文件大小和分块布局，任一不匹配即丢弃
  续传状态重新开始。CLI 对应 `funget download --resume`。
- 四个下载函数新增关键字参数 `filesize`，已知大小时可跳过 HEAD 探测。

### 修复

- **多线程下载失败后残留的半成品文件会被误判为「已下载完成」。** 由于采用预分配 +
  定位写，中间分块失败但末块成功时文件大小仍等于远端大小，下一次 `overwrite=False`
  的调用会命中「已完整」检查，静默返回成功并留下一个带空洞的文件。现在未开启
  `resume` 时失败会把文件截断为 0；「已完整」检查也额外要求不存在续传状态文件。
- **错误日志不再回显 URL 的 query。** `reqwest` 的错误文本会自带一句
  `for url (…)`,其中 userinfo 被抹掉但 query 原样保留,而下载链接的 query 里常常
  就是签名或临时 token。现在在错误类型的转换入口统一摘掉这份 URL,日志中的地址一律
  走 `safe_url()`(与 1.1.70 的 `request_error_context()` 同一意图)。
- README `Python API` 示例中 `simple_download`/`multi_thread_download` 调用缺少
  `url`/`filepath` 定义，复制运行会触发 `NameError`，改为可直接运行的完整示例。
- `funget info` 过去把文件路径写死为 `/tmp/funget-info`，导致 `Filename:` 一行
  永远打印 `funget-info`。现在从 URL 推导，其余三行格式不变。

### 有意偏离（升级前请确认）

- **传入未知关键字参数现在抛 `TypeError`**，而不是像过去那样被 `try` 吞掉、
  静默返回 `False`。已核对全组织的调用点，无人传递未知参数。
- **不再支持 `HTTPDigestAuth`**。`auth` 只接受 `(用户名, 密码)` 二元组或 `None`，
  走 Basic 认证。已确认全组织无人使用 Digest。
- **请求不再声明 gzip/deflate**。funget 用解码后的字节数与 `Content-Length`
  比对，压缩传输会让这个判断失效。
- 进度条由 tqdm 换成 indicatif，外观变化；非 TTY 环境下自动关闭。
- `funget upload -m` 只接受 `PUT` / `POST`（大小写不敏感），其余值是用法错误、
  退出码 2 —— 与此前 Typer 的 `Literal` 标注行为一致。

## 1.1.70

### 破坏性变更

- 运行环境最低版本提升至 Python 3.12，使用 Python 3.12 类型语法的公开 API 不再支持
  Python 3.11 及更早版本。请将运行环境升级至 Python 3.12 或更高版本后重新安装；无法升级
  的环境请继续使用 1.1.69 或更早版本，并自行维护其依赖和安全更新。

### 新增

- 补充公共下载、POST 上传、重试和失败边界的回归测试。

### 修复

（无）

### 变更

- 构建后端迁移至 Hatchling；`uv.lock` 已停止跟踪并由 `.gitignore` 忽略。
- 公开 API 改用 Python 3.12 类型语法并补齐中文 docstring。
- `farlog` 最低版本提升至 1.1.7。

### 废弃

（无）

## 1.1.69 及更早版本

早期版本未维护 CHANGELOG，具体变更参见 git 提交历史。

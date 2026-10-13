# 项目说明

> 固定路径：`docs/project/project-overview.md`
>
> 本文件记录适用于整个项目的重要背景和协作约定，由项目负责人维护。

## 基本信息

- 项目名称：funget
- 项目简介：HTTP 并发下载与上传。Rust 实现，三个分发渠道共用同一套引擎 ——
  PyPI 上的 `funget`（PyO3 扩展，Python API + `funget` 命令）、crates.io 上的
  `funget-core`（Rust 库）和 `funget`（CLI），以及 GitHub Release 上的独立二进制。
- 项目负责人：farfarfun
- 代码仓库：<https://github.com/farfarfun/funget>

## 应用清单

<!-- project-structure:applications:start -->
| 应用 | 路径 | 技术 Profile | Owner | 用途 |
| --- | --- | --- | --- | --- |
| 无（单应用仓库） | - | - | - | - |
<!-- project-structure:applications:end -->

## 项目目标

- 对 16 个下游仓库保持 `download` / `simple_download` / `multi_thread_download` /
  `single_upload` 四个函数的 API **完全冻结**，零改动即获得并发提速与内存下降。
- 提供一个不依赖 Python 环境的原生 CLI，供后续 Rust 化的仓库（如 fundeploy）复用。
- 把下载/上传引擎作为 Rust 库开放，让 Rust 侧调用方不必经过 Python。

## 项目范围

### 范围内

- Range 并发分块下载、断点续传、单线程回退、自动选择策略。
- PUT 流式上传与 POST multipart 上传，含发请求前的参数校验。
- 进度条、指数退避重试、Basic 认证、日志 URL 脱敏。

### 范围外

- **纯 Python 回退已彻底删除**，不再保留。因此差分测试（`scripts/differential.py`）
  是行为漂移的唯一安全网，CI 必须保持常绿。
- Digest 认证（全组织无人使用，改造时一并去掉）。
- 断点续传之外的下载管理功能（队列、限速、镜像选择等）。

## 重要约定

- 项目文件创建前先读取根目录 `.project-structure.json` 和项目结构规范。
- 多应用仓库中的代码必须先归属到应用清单中的一个应用，再按该应用的 profile 放入 `apps/<应用名>/` 内。
- 可独立运行、构建或部署的单元必须登记为应用；跨应用共享代码放入 `packages/`。
- 应用清单以 `.project-structure.json` 为准，以上受管表格由项目结构初始化器同步。
- 代码、测试、脚本、配置、文档、资源和生成物必须放入各自负责的目录。
- 项目文档统一使用中文；代码标识符、命令、文件路径和通用技术术语可以保留英文。
- 所有项目文档和文档资产必须按照项目文档管理规范存放到固定路径。
- 在线文档或外部平台链接不能替代仓库内的规范归档。
- 新文档类型必须先明确 Owner 和固定路径。

### 本仓库特有的目录约定

本仓库同时声明 `python` 和 `rust` 两个 profile，而这两个 profile 在
`project-layout.json` 里**都把 `src` 认领为源码区**。为消除这个歧义，实际布局做了
收窄：

- **`src/` 只属于 Python**（`src/funget/`，src layout，由 `[tool.maturin].python-source`
  指向）。里面没有任何逻辑，只做 re-export 加类型存根。
- **所有 Rust 代码都在 `crates/` 下**，根目录是**纯虚拟工作区**（只有 `[workspace]`，
  没有 `[package]`）。三个 crate：
  - `crates/funget-core` —— 引擎，零 Python 依赖，发到 crates.io，维护**独立的 0.x
    版本线**。
  - `crates/funget` —— CLI（lib + bin 两个 target，clap 解析在 lib 里），跟随
    workspace 版本号，因为它的 `--version` 要在 pip 和 cargo install 两条渠道报同一个值。
  - `crates/funget-py` —— PyO3 扩展，`publish = false`，MSRV 1.83（pyo3 0.29 的要求，
    另外两个 crate 仍是 1.82）。
- 因为根目录不是 crate，`[tool.maturin].manifest-path` **必须**显式指向
  `crates/funget-py/Cargo.toml`，否则 maturin 会对着根清单报 `missing field package`。
  funbuild 的 `MaturinBuild` 同样依赖这一项来定位构建目标和回溯版本号同步链。
- `.claude/` 会被结构检查器报一条 `zone.unowned` REVISE 且**无法消除** —— 清单的
  `ZONE_RE` 不接受以点开头的路径，写进 `additional_zones` 会直接变成
  `manifest.invalid` BLOCK。所以检查器一律用 `--fail-on block`。

## 技术与运行环境

- 主要技术栈：Rust 2021（reqwest + rustls / tokio / futures-util / indicatif / clap）、
  PyO3 0.29 + maturin（abi3-py312，一个 wheel 覆盖 3.12+）、Python 侧零运行时依赖。
- 开发环境：`cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、
  `cargo test --workspace --locked`、`maturin build --release`。
- 测试环境：GitHub Actions（`ci.yml`），runner 钉死 `ubuntu-24.04`。契约测试用
  `crates/funget-core/tests/common` 里自建的本地 HTTP / HTTPS 服务（含自签证书），
  外加对 PyPI 上 `funget==1.1.69` 的差分测试。
- 生产环境：由 `v*` tag 触发 `release.yml`，构建 6 平台 wheel + 7 平台 CLI 二进制，
  经 trusted publishing（OIDC，无长期令牌）上传 PyPI 与 crates.io。同一个 release
  还带 4 个 tarball、两个架构的 `.deb` 和 `SHA256SUMS`：`apt` 与 `brew` 两条渠道由
  组织的共享包仓库 [`farfarfun/packages`](https://github.com/farfarfun/packages)
  拉取这些资产后统一发布，本仓库为此不需要任何 secret。详见 `packaging/README.md`。
- 镜像：`sync-gitee.yml` 把仓库和 **Release 附件**一起推到 Gitee。组织级的
  `action-daily` 每两小时也镜像一遍，但它只搬 git refs —— funget 的交付物全在附件
  里，所以这里单独留一份，并盯着 `Release` workflow 跑完才同步（GITHUB_TOKEN 建的
  release 不会触发 `on: release`）。用的是组织级的 `GITEE_RSA_PRIVATE_KEY` 和
  `GITEE_TOKEN`，仓库本身仍然没有自己的 secret。

## 相关文档

- 项目结构索引：`.project-structure.json`
- 变更记录：`CHANGELOG.md`
- 使用说明：`README.md`
- 库文档：`crates/funget-core/README.md`、`crates/funget/README.md`
- 分发打包：`packaging/README.md`
- 差分测试：`scripts/differential.py`

## 更新记录

| 日期 | 修改人 | 变更摘要 |
| --- | --- | --- |
| 2026-10-13 | farfarfun | 初始化项目说明；记录 src/crates 的归属收窄 |
| 2026-10-13 | farfarfun | 新增 `packaging/` 区域；记录 apt / brew 两条渠道与共享包仓库的分工 |
| 2026-10-14 | farfarfun | 记录 Gitee 镜像：连 Release 附件一起同步 |

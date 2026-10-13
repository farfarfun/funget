# Changelog

## 1.1.73

纯分发层的一版，Rust 代码和两边的 API 一个字没动。

### 新增

- **`apt install funget` 和 `brew install funget`**。两条渠道都要**先加一次源**
  （apt 加 keyring + source 行，brew `tap` 加 `farfarfun/packages` 再 `trust` 一下），之后就和系统自带的包
  一样升级。进 Debian 官方档案和 homebrew-core 都有知名度与维护者门槛，不在计划内。
  具体命令见 README 的安装一节。

  索引和 formula 都由组织的共享包仓库
  [`farfarfun/packages`](https://github.com/farfarfun/packages) 统一维护 —— 它从
  各项目的 Release 里拉 `.deb`，所以**用户只加一次源，以后新增的 farfarfun 工具
  直接就有**。funget 这边只负责产出资产，不需要任何 secret。
- **Release 新增两个 musl 静态构建的 Linux target**，以及 `.deb`、tarball 和
  `SHA256SUMS`：
  - `funget_<版本号>_{amd64,arm64}.deb` —— 共享包仓库来拉的就是它。静态链接，
    **没有 `Depends`**；`ca-certificates` 写成 `Recommends`，因为系统证书库读不到
    时内置的 webpki 根证书照样能验公网站点（认不了私有 CA 而已）。
  - `funget-{x86_64,aarch64}-{unknown-linux-musl,apple-darwin}.tar.gz` ——
    Homebrew formula 装的是这四个。
  - `SHA256SUMS` 覆盖 release 的全部附件。
- **发布后的装机验证**：在 ubuntu x86_64 / arm64 上真 `dpkg -i` 装一遍、核对版本号、
  走一次公网 HTTPS 下载并比对校验和；再在 **`debian:12-slim`** 这个连
  `ca-certificates` 都没有的最小镜像里装一遍。后者是整件事的要点 —— Debian 12 的
  glibc 是 2.36，现有的 gnu 二进制在那里根本起不来。
- 发版流程多了一道闸：tag 与根 `Cargo.toml` 的版本号对不上直接失败。

### 变更

- **Linux 的 gnu 裸二进制保留**，没有被 musl 取代。musl 静态构建用自己的解析器、
  不走 glibc 的 NSS，靠 mDNS / LDAP / `nss-*` 插件解析主机名的环境只有 gnu 版能用；
  而且 `releases/latest/download/funget-x86_64-unknown-linux-gnu` 是公开地址，
  删掉就 404。README 里的 `curl` 一行改为推荐 musl tarball，gnu 文件标注了它要
  glibc ≥ 2.39。
- 新增 `packaging/build-deb.sh` 和 `packaging/README.md`（三条渠道的分工、本地怎么
  单跑打包脚本、共享仓库那边的登记契约）。

## 1.1.72

### 新增

- **发布到 crates.io**，Rust 用户不再需要从源码或 Release 页取用：
  - [`funget`](https://crates.io/crates/funget) —— 命令行工具，`cargo install funget`。
    版本号与 PyPI 保持一致，三条分发渠道的 `funget --version` 报同一个值。
  - [`funget-core`](https://crates.io/crates/funget-core) —— 下载/上传引擎，可作为
    Rust 库独立使用。**版本线独立**（首发 0.1.0，本次随下面的 API 调整升到 0.2.0）：
    它的 9 个 `pub mod` 当初是为了让 CLI 和 PyO3 扩展跨 crate 调用才公开的，并非
    按公共 Rust API 设计，0.x 期间仍会调整。PyPI 侧的 1.1.x 不受其影响。
- **CLI 补齐 curl 的常用开关**，`download` / `upload` / `info` 三个子命令共用一组：
  `-H/--header`（可重复）、`-u/--user`、`-A/--user-agent`、`--timeout`、`-x/--proxy`
  （含 `socks5://`）、`-k/--insecure`、`--cacert`（可重复，支持整条证书链的 bundle）、
  `--no-native-certs`、`--no-location`、`--max-redirs`。短选项与 curl 对齐。
  - `download` 和 `upload` 另加 `-s/--silent` 关掉进度条 —— 过去只能靠非 TTY 环境
    自动关闭，命令行上根本没有开关。
  - `funget info` 此前用的是 `DownloadOptions::default()`，连一个需要认证的 URL 都
    查不了，现在同样吃这组开关。
  - `-H` 和 `-u` 的格式错是**用法错误、退出码 2**。curl 对畸形的 `-H` 是静默忽略的，
    这里刻意更严：一个拼错的请求头被悄悄丢掉，排查成本远高于当场报错。
    `-u user:` 是空密码（与 curl 一致），而不带冒号的 `-u user` 是用法错误。
- 四个下载/上传函数新增三个 **keyword-only** 参数，默认值精确复刻既有行为：
  `insecure`（默认 `False`）、`cacert`（单个 PEM 路径）、`proxy`。重定向旋钮和
  `--no-native-certs` 不开放给 Python —— 默认值就是对的，后者是 CLI 的排障逃生口。
- **`funget info` 在发生重定向时多打一行 `Redirected to: <url>`**，没有重定向时输出
  仍是原来的四行。

### 安全

- **pyo3 0.23 → 0.29**，修掉三条影响默认分支的通告：`PyList`/`PyTuple` 迭代器
  `nth`/`nth_back` 的越界读（高危）、`PyCFunction::new_closure` 缺 `Sync` 约束（中危）、
  `PyString::from_object` 的缓冲区溢出风险（低危）。funget 的扩展层没有用到这三个
  API，但依赖在版本范围内，一并升掉。`pyo3-log` 同步升到 0.13。

  升级只改了一处写法：`Python::allow_threads` 在 0.27 更名为 `Python::detach`，
  语义和签名完全一致，释放 GIL 的行为不变。

### 修复

- **TLS 此前完全不读系统证书库。** 只开了 reqwest 的 `rustls-tls-webpki-roots`，
  等于只认 Mozilla 那份固定的根证书列表：企业私有 CA、内网自签证书、公司 MITM 代理
  证书一律连不上，而同一台机器上的 `curl` 是通的。现在叠加
  `rustls-tls-native-roots`，两份根证书取**并集**（reqwest 的两个 `#[cfg]` 分支往同
  一个 `RootCertStore` 里塞证书），`--cacert` 则在此之上继续追加。

  极少数机器的系统证书库里有证书但 rustls 一张都解析不了（被截断过的
  `ca-certificates` bundle 等），reqwest 会让客户端构造**直接失败**，而这种情况
  `--cacert` 救不了。新增的 `--no-native-certs` 就是给它们的逃生口，退回 1.1.71
  的行为（只用内置的 webpki 根证书）。
- **并发下载会对每个分块各走一遍重定向链。** 分块请求过去打的是调用方给的原始地址，
  于是 `-w 10` 下一个 302 到预签名地址的链接就是 **10 条各自独立的重定向链**：10 次
  多余的往返、签名可能在下载中途过期、理论上还可能被分流到不同后端拼出一个坏文件。
  现在重定向只在 Range 探测那一次 GET 里解析一次，之后所有分块直接打解析后的地址 ——
  源站每次下载固定只收到 2 个请求（大小探测 + Range 探测），与并发数无关。
  `funget info` 据此多打的那行 `Redirected to:` 就是这次解析的结果。

  最终地址只从 **GET** 探测里取、不用 HEAD 的：有些 S3 兼容网关对 HEAD 和 GET 给出
  的重定向目标不同（只给 GET 签名），拿 HEAD 的结果去打分块会 404。

  续传指纹仍然按**原始**地址计算。解析后的地址按定义是跨运行不稳定的（CDN 每次可能
  给不同的边缘节点，有些网关还会重定向到一条一次性路径），拿它做指纹就会每次都判定
  「不是同一份文件」、让续传永久失效。

### 变更

- **CLI 默认发送 `funget/<版本号>` 的 User-Agent**（取 CLI crate 自己的版本号，与
  PyPI 同号），`-A` 可以覆盖。curl / wget 都发 UA，有些 WAF 会直接 403 掉不带 UA 的
  请求。**Python 侧的四个函数不设默认 UA，仍然不发这个请求头** —— 它们的线上行为要
  逐字节冻结，为一个推测性收益改请求头不值得冒险。这条不对称是刻意的。
- **`funget-core` 升到 0.2.0**，是一次破坏性的 API 调整：新增 `TransportOptions`
  承载 TLS / 重定向 / 代理 / UA 这组旋钮，`HttpConfig` 与 `UploadOptions` 各加一个
  `transport` 字段，`probe_range_support` 的返回类型由 `bool` 改为 `RangeProbe`
  （多带一个解析后的地址）。crates.io 上的 0.1.0 仍是改动前的 API；0.x 期间允许
  这样调整，PyPI 侧的 1.1.x 和 CLI 的用法都不受影响。
- 测试：新增自签证书的本地 HTTPS 服务，给 `-k` / `--cacert` 做端到端验证；两条修复
  各自配了回归测试（都用「还原修复后测试必须变红」确认过）。顺带移除了压根没用上的
  `wiremock` dev-dependency。工作区测试从 47 项增至 61 项。
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

# 分发 funget

funget 有五条分发渠道。前两条（apt / brew）要靠组织的共享包仓库
[`farfarfun/packages`](https://github.com/farfarfun/packages)，剩下三条由本仓库的
[release.yml](../.github/workflows/release.yml) 直接产出：

| 渠道 | 谁做的 |
| --- | --- |
| `apt-get install funget` | `farfarfun/packages` 拉本仓库 Release 里的 `.deb`，重建签名索引发到 Pages |
| `brew install funget` | `farfarfun/packages` 按 `SHA256SUMS` 渲染 formula，提交回它自己的 `Formula/`（那个仓库同时就是 tap） |
| `pip install funget` | 本仓库 → PyPI（`wheels` / `sdist` / `publish`） |
| `cargo install funget` | 本仓库 → crates.io（`crates`） |
| 裸二进制 / tarball | 本仓库 → GitHub Release |

**funget 这边不需要任何 secret，也不需要跨仓库写权限。**共享仓库是拉取模型：它
读自己的 `packages.toml`，从各项目的非草稿非预发布 Release 里下 `*.deb`。funget
要做的只是把资产挂到 Release 上，并在那边的 `packages.toml` 里登记三行。

**一个 secret 都没有，这是故意的。**发完版后新版本什么时候出现在 `apt` 里，取决
于共享仓库那边每天 04:17 的兜底 cron —— 最多等一天。想立刻生效就手动触发它：

```bash
gh workflow run build.yml -R farfarfun/packages
```

曾经有个 `notify-packages` job 用 PAT 打 `repository_dispatch` 让那边即时重建，
已经去掉了。原因是跨仓库写必须带一个长期凭据：workflow 自带的 `GITHUB_TOKEN`
只对本仓库有效，同一个组织也不例外，GitHub 没有「同组织互信」这种开关。为了
「apt 里早几小时看到新版」在仓库里长期存一个 PAT 并盯着它过期，不值得；上面那行
命令用的是人的身份，要多快有多快。

## 工作流顺序

1. `check` —— 从根 `Cargo.toml` 读版本号；tag 触发时核对 `v<version>` 一致
2. `wheels` / `sdist` / `binaries` —— 6 个 wheel、sdist、7 个 target 的 CLI
3. `deb` —— 从两个 musl tarball 打 `amd64` / `arm64` 的 `.deb`
4. `publish` —— 发 PyPI（OIDC trusted publishing）、算 `SHA256SUMS`、挂 Release
5. `verify-install` —— 真 `dpkg -i` 装一遍，含 `debian:12-slim` 最小镜像
6. `crates` —— 发 crates.io（版本已存在就跳过）

## Release 上的附件

```
funget-{x86_64,aarch64}-unknown-linux-gnu          裸；动态链接，要 glibc ≥ 2.39
funget-{x86_64,aarch64}-apple-darwin               裸
funget-x86_64-pc-windows-msvc.exe                  裸
funget-{x86_64,aarch64}-unknown-linux-musl.tar.gz  静态；brew(Linux) + .deb 的料
funget-{x86_64,aarch64}-apple-darwin.tar.gz        brew(macOS)
funget_<version>_{amd64,arm64}.deb                 farfarfun/packages 来拉
SHA256SUMS                                         裸文件名，formula 据此填 sha256
```

tarball 在 `binaries` job 里就打好，不是到下游再打的：`upload-artifact` 不保留
文件权限，裸文件传到别的 job 就变成 0644，而 Homebrew 的 `bin.install` 和 `.deb`
都要可执行位。tarball 里的文件名必须是裸 `funget`。

`SHA256SUMS` 必须在所有附件都齐了之后再算 —— 提前算就漏掉 `.deb` 和 tarball。

**musl 和 gnu 两套都发。** musl 全静态让 `.deb` 可以不写 `Depends`，装上就能跑，
在 Debian 12（glibc 2.36）这种 gnu 二进制根本起不来的机器上也能用。代价是 musl
用自己的解析器、不走 glibc 的 NSS，靠 mDNS / LDAP / `nss-*` 插件解析主机名的环境
只有 gnu 版能用；README 里那条 `.../funget-x86_64-unknown-linux-gnu` 也是公开
地址，删掉就 404。

## 本地跑打包脚本

```bash
rustup target add x86_64-unknown-linux-musl
sudo apt-get install -y musl-tools dpkg-dev          # ring 的 build.rs 要 musl-gcc
cargo build --release --locked --package funget --target x86_64-unknown-linux-musl

packaging/build-deb.sh \
  target/x86_64-unknown-linux-musl/release/funget 1.1.73 amd64 dist

dpkg-deb -I dist/funget_1.1.73_amd64.deb     # 控制字段
dpkg-deb -c dist/funget_1.1.73_amd64.deb     # 只该有 /usr/bin/funget 和 copyright
```

静态性自查：`file …/funget` 要说 `statically linked`，`ldd` 要说
`not a dynamic executable`。

在最小镜像里装一遍（CI 的 `verify-install` 做的就是这件事）：

```bash
docker run --rm -v "$PWD/dist:/d" debian:12-slim \
  sh -eux -c 'dpkg -i /d/funget_*.deb && funget --version'
```

`dist/` 已被 `.gitignore` 忽略。

## `.deb` 的两个决定

**没有 `Depends`。** 二进制是 musl 静态链接的，装上就能跑。

**`ca-certificates` 是 `Recommends` 而不是 `Depends`。** 系统证书库一张都读不到时
reqwest 不报错（只在「读到了但全解析失败」时才让客户端构造失败），编进二进制的
webpki 根证书照样验得了公网站点。所以没装 `ca-certificates` 的最小镜像里 funget
依然可用，只是认不了企业私有 CA —— 那是「建议装」的语义。`verify-install` 里
`debian:12-slim` 那一步就是在守这条结论。

## 在共享仓库里登记

`farfarfun/packages` 的 `packages.toml`：

```toml
[[source]]
repo = "farfarfun/funget"
keep = 3
# 1.1.72 及更早的 release 里没有 .deb。collect-debs.sh 对「没有 .deb」是容忍的，
# 写下限只是为了别让 keep=3 把名额占在没有包的版本上。
min_version = "1.1.73"

# 有这一节才渲染 formula。desc 用英文，brew 的搜索和 `brew desc` 都读它。
[source.brew]
desc = "HTTP file transfer tool with ranged concurrency and resume"
# `brew test` 的额外断言，格式是 [参数, 输出里该出现的字串]。对 `--version` 的
# 断言那边默认就有，不用写。这一行可以整个省掉。
checks = [["--help", "download"]]
```

本仓库的 Release 满足那边 `render-formula.py` 的全部命名约定，不需要为 funget
写任何特例：

| 约定 | funget 的实际产出 |
| --- | --- |
| `<包名>-<target>.tar.gz` | `funget-{x86_64,aarch64}-{apple-darwin,unknown-linux-musl}.tar.gz` |
| `<包名>_<版本>_<架构>.deb` | `funget_1.1.73_{amd64,arm64}.deb` |
| 包名取仓库名 | 仓库叫 `funget`，二进制也叫 `funget` |
| tarball 里是单个可执行文件 | 是，裸名 `funget`，可执行位已保住 |
| 校验和 | `SHA256SUMS`，一行一个裸文件名 |

formula 名就是 `funget`，homebrew-core 里没有同名 formula（查过
`formulae.brew.sh/api/formula/funget.json` → 404），不会被遮挡，可以裸名字装。

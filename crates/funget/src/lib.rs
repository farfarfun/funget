//! funget 命令行的全部逻辑。
//!
//! 做成 lib + bin 两个 target。原来是为了让 PyO3 扩展导出的 `cli_main` 和独立
//! 二进制共用同一个 [`run`];wheel 不再带 `funget` 命令之后,那个调用方没有了,
//! 现在 `main.rs` 是唯一的消费者。仍然保留 lib,是因为下面那 12 个 clap 解析的
//! 单元测试要能 `use` 到它 —— bin-only 的 crate 测起来只能靠 `assert_cmd` 之类
//! 真的去 fork 进程。

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use funget_core::{
    headers_from_pairs, Auth, DownloadOptions, HttpConfig, TransportOptions, UploadOptions,
};

/// 本 CLI 默认发出的 User-Agent。
///
/// 取自己 crate 的版本号(与 PyPI 同号),而不是 funget-core 的 —— 后者走独立的
/// 0.x 版本线,报出去会让人困惑。curl / wget 都发 UA,有些 WAF 会直接 403 掉不带
/// UA 的请求,所以 CLI 这边设上默认值;**PyO3 扩展不设**,那四个函数的线上行为
/// 要逐字节冻结。
const USER_AGENT: &str = concat!("funget/", env!("CARGO_PKG_VERSION"));

// 注意:凡是会出现在 `--help` 里的文案一律用英文,和 1.1.70 的 typer CLI 保持一致。
// 中文的设计说明走普通 `//` 注释,不要写成 `///` 文档注释 —— clap 会把文档注释
// 直接当帮助文本打给用户。
#[derive(Parser, Debug)]
#[command(name = "funget", version, about = "Download and upload files")]
pub struct Cli {
    /// Print more detailed logs.
    #[arg(short = 'v', long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Download a file.
    Download(DownloadArgs),
    /// Upload a file.
    Upload(UploadArgs),
    /// Show remote file information.
    Info(InfoArgs),
}

// 三个子命令共用的 HTTP 开关,用 `#[command(flatten)]` 摊进各自的 Args。
// 连 `info` 也给:它过去用的是 `DownloadOptions::default()`,连一个需要认证的
// URL 都查不了。
#[derive(Args, Debug)]
pub struct HttpArgs {
    /// Extra request header, as "Name: value". Repeatable.
    #[arg(short = 'H', long = "header", value_name = "LINE", value_parser = parse_header)]
    pub headers: Vec<(String, String)>,

    /// Credentials for Basic auth, as "user:password".
    #[arg(short = 'u', long = "user", value_name = "USER:PASSWORD", value_parser = parse_user)]
    pub user: Option<Auth>,

    /// User-Agent to send.
    #[arg(short = 'A', long, value_name = "NAME", default_value = USER_AGENT)]
    pub user_agent: String,

    // 刻意不写 default_value_t:download 的默认超时是 30 秒、upload 是 60 秒,
    // 在共享结构体里写死一个值会把 upload 的 60 悄悄改掉。各子命令自己 unwrap_or。
    /// Connect and read timeout in seconds.
    #[arg(long, value_name = "SECONDS")]
    pub timeout: Option<u64>,

    /// Proxy to use, e.g. http://host:3128 or socks5://host:1080.
    #[arg(short = 'x', long, value_name = "URL")]
    pub proxy: Option<String>,

    /// Allow TLS connections with invalid certificates.
    #[arg(short = 'k', long)]
    pub insecure: bool,

    /// Extra CA certificate in PEM format. Repeatable.
    #[arg(long, value_name = "FILE")]
    pub cacert: Vec<PathBuf>,

    // 逃生口:reqwest 在「系统证书库里有证书但一张都解析不了」时会让客户端
    // 构造失败。那种机器用这个开关退回 1.1.71 的行为(只用内置的 webpki 根)。
    /// Do not read the system certificate store.
    #[arg(long)]
    pub no_native_certs: bool,

    // funget 默认跟随重定向(与 wget 一致),所以要提供的是**反向**开关 ——
    // curl 那边是正向的 `-L`,因为它默认不跟随。
    /// Do not follow redirects.
    #[arg(long)]
    pub no_location: bool,

    /// Maximum number of redirects to follow.
    #[arg(long, value_name = "N", default_value_t = 10)]
    pub max_redirs: usize,
}

impl HttpArgs {
    fn transport(&self) -> TransportOptions {
        TransportOptions {
            insecure: self.insecure,
            ca_certs: self.cacert.clone(),
            native_certs: !self.no_native_certs,
            follow_redirects: !self.no_location,
            max_redirects: self.max_redirs,
            proxy: self.proxy.clone(),
            user_agent: Some(self.user_agent.clone()),
        }
    }

    /// `default_timeout` 由子命令给:download 30 秒、upload 60 秒,与各自的
    /// Python 版默认值一致。
    fn http_config(&self, max_retries: u32, default_timeout: u64) -> HttpConfig {
        HttpConfig {
            headers: headers_from_pairs(self.headers.iter().map(|(k, v)| (k, v))),
            auth: self.user.clone(),
            timeout: Duration::from_secs(self.timeout.unwrap_or(default_timeout)),
            max_retries,
            transport: self.transport(),
        }
    }
}

/// `-H "Name: value"`。按**第一个**冒号切分,值两侧的空格去掉。
///
/// 返回 `Result` 让 clap 自己报错:格式错于是成为**用法错误、退出码 2**,而不是
/// 跑起来之后才失败。curl 对畸形的 `-H` 是静默忽略的,这里刻意更严 —— 一个拼错的
/// 请求头被悄悄丢掉,排查成本远高于当场报错。
fn parse_header(raw: &str) -> std::result::Result<(String, String), String> {
    let (name, value) = raw
        .split_once(':')
        .ok_or_else(|| format!("expected \"Name: value\", got {raw:?}"))?;
    let name = name.trim();
    if name.is_empty() {
        return Err(format!("header name is empty in {raw:?}"));
    }
    Ok((name.to_string(), value.trim().to_string()))
}

/// `-u user:password`。同样按第一个冒号切分,所以 `-u user:` 是**空密码**
/// (curl 的写法),而不带冒号的 `-u user` 是用法错误 —— 我们没有 TTY 提示密码的
/// 能力,把它当成空密码静默通过只会让人以为认证生效了。
fn parse_user(raw: &str) -> std::result::Result<Auth, String> {
    let (username, password) = raw
        .split_once(':')
        .ok_or_else(|| format!("expected \"user:password\", got {raw:?}"))?;
    Ok(Auth {
        username: username.to_string(),
        password: password.to_string(),
    })
}

#[derive(Args, Debug)]
pub struct InfoArgs {
    pub url: String,

    #[command(flatten)]
    pub http: HttpArgs,
}

#[derive(Args, Debug)]
pub struct DownloadArgs {
    pub url: String,

    /// Output path. Defaults to the basename of the URL.
    #[arg(short = 'o', long)]
    pub output: Option<String>,

    // 默认 10 —— 注意这是 **CLI** 的默认值,库函数 `multi_thread_download` 默认是 5。
    /// Number of concurrent workers.
    #[arg(short = 'w', long, default_value_t = 10)]
    pub worker: usize,

    /// Block size in MiB.
    #[arg(short = 'b', long, default_value_t = 100)]
    pub block_size: u64,

    /// Retries per request.
    #[arg(short = 'r', long, default_value_t = 3)]
    pub max_retries: u32,

    /// Force the single-threaded path.
    #[arg(long)]
    pub single: bool,

    /// Re-download even if the file is already complete.
    #[arg(long)]
    pub overwrite: bool,

    // 会在目标文件旁生成 `<文件名>.funget.json` 状态文件,下载成功后自动删除。
    /// Resume an interrupted download instead of starting over.
    #[arg(long)]
    pub resume: bool,

    /// Do not draw the progress bar.
    #[arg(short = 's', long)]
    pub silent: bool,

    #[command(flatten)]
    pub http: HttpArgs,
}

// 做成枚举而不是自由字符串,是为了保住 Python CLI 的那条行为:typer 用
// `Literal["PUT", "POST"]` 标注,传 `-m PATCH` 是**用法错误、退出码 2**,
// 而不是跑起来再失败(退出码 1)。`tests/test_script.py` 对此有显式断言。
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum UploadMethod {
    #[value(name = "PUT")]
    Put,
    #[value(name = "POST")]
    Post,
}

impl UploadMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Put => "PUT",
            Self::Post => "POST",
        }
    }
}

#[derive(Args, Debug)]
pub struct UploadArgs {
    pub file_path: String,
    pub url: String,

    /// HTTP method used to upload.
    #[arg(
        short = 'm',
        long,
        value_enum,
        ignore_case = true,
        default_value = "PUT"
    )]
    pub method: UploadMethod,

    /// Read/stream chunk size in bytes.
    #[arg(short = 'c', long, default_value_t = 256 * 1024)]
    pub chunk_size: i64,

    /// Retries before giving up.
    #[arg(short = 'r', long, default_value_t = 3)]
    pub max_retries: i64,

    /// Do not draw the progress bar.
    #[arg(short = 's', long)]
    pub silent: bool,

    #[command(flatten)]
    pub http: HttpArgs,
}

/// 解析参数并执行,返回进程退出码。
pub fn run<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        // clap 自己会把帮助/版本信息和用法错误打到对的流上,这里只负责转成退出码。
        // 用法错误是 2(与 typer/click 一致),`--help` / `--version` 是 0。
        Err(error) => {
            let _ = error.print();
            return if error.use_stderr() { 2 } else { 0 };
        }
    };

    init_logging(cli.verbose);

    match cli.command {
        Command::Download(args) => download(args),
        Command::Upload(args) => upload(args),
        Command::Info(args) => info(args),
    }
}

fn init_logging(verbose: bool) {
    let default = if verbose { "debug" } else { "info" };
    // RUST_LOG 优先,没设就用上面的默认级别。重复初始化(同进程里多次调用
    // [`run`],比如单元测试)会返回 Err,忽略即可。
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default))
        .format_timestamp_secs()
        .try_init();
}

fn download(args: DownloadArgs) -> i32 {
    let destination = args.output.unwrap_or_else(|| default_filename(&args.url));

    let options = DownloadOptions {
        overwrite: args.overwrite,
        worker_num: args.worker,
        block_size: args.block_size,
        max_retries: args.max_retries,
        resume: args.resume,
        progress: !args.silent,
        http: args.http.http_config(args.max_retries, 30),
        ..DownloadOptions::default()
    };

    let succeeded = if args.single {
        funget_core::download_single(&args.url, &destination, &options)
    } else {
        funget_core::download_multi(&args.url, &destination, &options)
    };

    if succeeded {
        log::info!("Download completed: {destination}");
        0
    } else {
        log::error!("Download failed");
        1
    }
}

fn upload(args: UploadArgs) -> i32 {
    let options = UploadOptions {
        method: args.method.as_str().to_string(),
        chunk_size: args.chunk_size,
        max_retries: args.max_retries,
        // 上传侧的用户请求头单独存在 `UploadOptions.headers` 里,不走 HttpConfig ——
        // 那边的优先级是「配置头覆盖专属头」,而上传要求 Content-Length /
        // Content-Type 覆盖用户头,正好相反(见 funget-core/src/upload.rs)。
        headers: headers_from_pairs(args.http.headers.iter().map(|(k, v)| (k, v))),
        auth: args.http.user.clone(),
        timeout: args.http.timeout.unwrap_or(60) as i64,
        progress: !args.silent,
        transport: args.http.transport(),
        ..UploadOptions::default()
    };

    if funget_core::upload(&args.url, &args.file_path, &options) {
        log::info!("Upload completed: {}", args.file_path);
        0
    } else {
        log::error!("Upload failed");
        1
    }
}

fn info(args: InfoArgs) -> i32 {
    // 文件名从 URL 推导。Python 版这里把 filepath 写死成 `/tmp/funget-info`,
    // 于是 `Filename:` 永远打印 `funget-info` —— 那是 bug,不是契约,这版改掉了
    // (见 CHANGELOG 的「有意偏离」)。输出的四行格式保持不变。
    let url = args.url.as_str();
    let filename = default_filename(url);
    let options = DownloadOptions {
        http: args.http.http_config(3, 30),
        ..DownloadOptions::default()
    };
    match funget_core::file_info(url, &filename, &options) {
        Ok(details) => {
            println!("URL: {}", details.url);
            // 只在真的发生了重定向时多打这一行:没重定向的输出仍是原来的四行。
            // 过去这种情况下只打原始地址、对落到哪里一个字都不说,排查起来很难受。
            if let Some(final_url) = &details.final_url {
                println!("Redirected to: {final_url}");
            }
            println!("Filename: {}", details.filename);
            println!("Size: {} bytes", thousands(details.filesize));
            println!(
                "Range requests: {}",
                if details.supports_range {
                    "supported"
                } else {
                    "unsupported"
                }
            );
            0
        }
        Err(error) => {
            log::error!("Error: {error}");
            1
        }
    }
}

/// 没给 `-o` 时从 URL 路径取 basename,取不到就退回 `download`。
///
/// 等价于 Python 的 `os.path.basename(urlsplit(url).path) or "download"`。
/// 关键是先把 scheme 和 authority 切掉再取 basename:`https://a.example` 的
/// **path 是空串**,直接对整个 URL 做 basename 会得到主机名。
fn default_filename(url: &str) -> String {
    let without_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path = match without_scheme.find('/') {
        Some(index) => &without_scheme[index..],
        None => "",
    };
    let path = path.split(['?', '#']).next().unwrap_or("");
    // 末尾是 `/` 时 basename 为空 —— 和 Python 一样退回固定名,而不是取上一级目录名。
    match path.rsplit('/').next() {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => "download".to_string(),
    }
}

/// 三位一组加逗号,对应 Python 的 `f"{filesize:,}"`。
fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_size_like_python() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(thousands(1_000_000_000), "1,000,000,000");
    }

    #[test]
    fn derives_filename_from_url() {
        assert_eq!(
            default_filename("https://a.example/dir/file.bin"),
            "file.bin"
        );
        assert_eq!(
            default_filename("https://a.example/file.bin?token=x"),
            "file.bin"
        );
        assert_eq!(default_filename("https://a.example/d/f.bin#frag"), "f.bin");
        // 路径为空或以 / 结尾时退回固定名,与 Python 的 `or "download"` 一致 ——
        // 尤其不能把主机名或上级目录名当成文件名。
        assert_eq!(default_filename("https://a.example/"), "download");
        assert_eq!(default_filename("https://a.example"), "download");
        assert_eq!(default_filename("https://a.example/dir/"), "download");
    }

    #[test]
    fn parses_the_same_short_flags_as_the_python_cli() {
        let cli = Cli::try_parse_from([
            "funget",
            "download",
            "http://x/y.bin",
            "-o",
            "out.bin",
            "-w",
            "4",
            "-b",
            "7",
            "-r",
            "2",
            "--single",
            "--overwrite",
            "--resume",
        ])
        .expect("参数应当解析成功");

        let Command::Download(args) = cli.command else {
            panic!("应当解析为 download 子命令");
        };
        assert_eq!(args.output.as_deref(), Some("out.bin"));
        assert_eq!(args.worker, 4);
        assert_eq!(args.block_size, 7);
        assert_eq!(args.max_retries, 2);
        assert!(args.single && args.overwrite && args.resume);
    }

    #[test]
    fn upload_defaults_match_the_python_cli() {
        let cli = Cli::try_parse_from(["funget", "upload", "a.bin", "http://x/u"])
            .expect("参数应当解析成功");
        let Command::Upload(args) = cli.command else {
            panic!("应当解析为 upload 子命令");
        };
        assert_eq!(args.method.as_str(), "PUT");
        assert_eq!(args.chunk_size, 256 * 1024);
        assert_eq!(args.max_retries, 3);
    }

    /// 对应 `tests/test_script.py::test_upload_rejects_unknown_method_with_usage_error`。
    #[test]
    fn unknown_upload_method_is_a_usage_error() {
        assert_eq!(
            run(["funget", "upload", "a.bin", "http://x/u", "-m", "PATCH"]),
            2
        );
    }

    /// `--help` / `--version` 走 stdout 且退出码为 0。
    #[test]
    fn help_exits_zero_and_lists_every_command() {
        assert_eq!(run(["funget", "--help"]), 0);

        let error = Cli::try_parse_from(["funget", "--help"]).expect_err("--help 应当短路");
        let text = error.to_string();
        for command in ["download", "upload", "info"] {
            assert!(text.contains(command), "帮助里缺少 {command}: {text}");
        }
    }

    /// 帮助文案必须全英文,和 1.1.70 的 typer CLI 一致。
    ///
    /// 这条是防回归用的:clap 会把 `///` 文档注释原样当帮助文本打给用户,
    /// 哪天有人顺手用中文写了参数说明,这里会立刻红。
    #[test]
    fn help_text_is_ascii_only() {
        let mut command = <Cli as clap::CommandFactory>::command();
        command.build();

        let mut pending = vec![command];
        while let Some(current) = pending.pop() {
            let rendered = current.clone().render_long_help().to_string();
            assert!(
                rendered.is_ascii(),
                "`{}` 的帮助里混进了非 ASCII 文案:\n{rendered}",
                current.get_name()
            );
            pending.extend(current.get_subcommands().cloned());
        }
    }

    /// 新增的 curl 风味开关全部解析到 `TransportOptions` / `HttpConfig` 上。
    #[test]
    fn parses_the_curl_style_http_flags() {
        let cli = Cli::try_parse_from([
            "funget",
            "download",
            "http://x/y.bin",
            "-H",
            "X-Token: abc",
            "-H",
            "Accept:application/json",
            "-u",
            "alice:s3cret",
            "-A",
            "me/1.0",
            "--timeout",
            "7",
            "-x",
            "socks5://127.0.0.1:1080",
            "-k",
            "--cacert",
            "/tmp/a.pem",
            "--no-native-certs",
            "--no-location",
            "--max-redirs",
            "3",
            "-s",
        ])
        .expect("参数应当解析成功");

        let Command::Download(args) = cli.command else {
            panic!("应当解析为 download 子命令");
        };
        assert!(args.silent);
        assert_eq!(
            args.http.headers,
            vec![
                ("X-Token".to_string(), "abc".to_string()),
                // 冒号后没有空格也要认,值两侧的空格一律去掉。
                ("Accept".to_string(), "application/json".to_string()),
            ]
        );
        let user = args.http.user.as_ref().expect("应当解析出凭据");
        assert_eq!(
            (user.username.as_str(), user.password.as_str()),
            ("alice", "s3cret")
        );

        let transport = args.http.transport();
        assert!(transport.insecure);
        assert!(!transport.native_certs);
        assert!(!transport.follow_redirects);
        assert_eq!(transport.max_redirects, 3);
        assert_eq!(transport.proxy.as_deref(), Some("socks5://127.0.0.1:1080"));
        assert_eq!(transport.user_agent.as_deref(), Some("me/1.0"));
        assert_eq!(transport.ca_certs, vec![PathBuf::from("/tmp/a.pem")]);

        assert_eq!(args.http.http_config(3, 30).timeout, Duration::from_secs(7));
    }

    /// 不传 `--timeout` 时各子命令用自己的默认值 —— 共享结构体不能把 upload 的
    /// 60 秒悄悄改成 download 的 30 秒。
    #[test]
    fn timeout_defaults_stay_per_subcommand() {
        let cli = Cli::try_parse_from(["funget", "download", "http://x/y.bin"]).expect("解析");
        let Command::Download(args) = cli.command else {
            panic!("应当解析为 download 子命令");
        };
        assert!(args.http.timeout.is_none());
        assert_eq!(
            args.http.http_config(3, 30).timeout,
            Duration::from_secs(30)
        );

        let cli = Cli::try_parse_from(["funget", "upload", "a.bin", "http://x/u"]).expect("解析");
        let Command::Upload(args) = cli.command else {
            panic!("应当解析为 upload 子命令");
        };
        assert_eq!(args.http.timeout.unwrap_or(60), 60);
    }

    /// 默认不带 `-A` 时发自己的版本号,且默认跟随重定向(与 wget 一致)。
    #[test]
    fn transport_defaults_match_the_documented_behaviour() {
        let cli = Cli::try_parse_from(["funget", "info", "http://x/y.bin"]).expect("解析");
        let Command::Info(args) = cli.command else {
            panic!("应当解析为 info 子命令");
        };
        let transport = args.http.transport();
        assert_eq!(transport.user_agent.as_deref(), Some(USER_AGENT));
        assert!(transport.user_agent.expect("UA").starts_with("funget/"));
        assert!(transport.follow_redirects);
        assert_eq!(transport.max_redirects, 10);
        assert!(transport.native_certs && !transport.insecure);
        assert!(transport.proxy.is_none() && transport.ca_certs.is_empty());
    }

    /// 畸形的 `-H` / `-u` 是**用法错误、退出码 2**,而不是跑起来之后才失败。
    /// curl 对畸形的 `-H` 是静默忽略的,这里刻意更严。
    #[test]
    fn malformed_header_or_credentials_are_usage_errors() {
        for bad in [
            vec!["funget", "download", "http://x/y.bin", "-H", "no-colon"],
            // 冒号前为空 → 请求头没有名字。
            vec!["funget", "download", "http://x/y.bin", "-H", ": value"],
            vec!["funget", "download", "http://x/y.bin", "-u", "nocolon"],
            vec!["funget", "info", "http://x/y.bin", "-H", "no-colon"],
            vec!["funget", "upload", "a.bin", "http://x/u", "-u", "nocolon"],
        ] {
            assert_eq!(run(bad.clone()), 2, "{bad:?} 应当是用法错误");
        }

        // `-u user:` 是空密码(curl 的写法),不是错误。
        let cli = Cli::try_parse_from(["funget", "download", "http://x/y.bin", "-u", "bob:"])
            .expect("空密码应当被接受");
        let Command::Download(args) = cli.command else {
            panic!("应当解析为 download 子命令");
        };
        let user = args.http.user.expect("应当解析出凭据");
        assert_eq!(
            (user.username.as_str(), user.password.as_str()),
            ("bob", "")
        );
    }

    /// 小写方法名也接受 —— 比 typer 的 Literal 宽松一点,但不影响上面那条用法错误。
    #[test]
    fn method_accepts_lowercase() {
        let cli = Cli::try_parse_from(["funget", "upload", "a.bin", "http://x/u", "-m", "post"])
            .expect("小写方法名应当被接受");
        let Command::Upload(args) = cli.command else {
            panic!("应当解析为 upload 子命令");
        };
        assert_eq!(args.method.as_str(), "POST");
    }
}

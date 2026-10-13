//! funget 命令行的全部逻辑。
//!
//! 刻意做成 lib + bin 两个 target:独立二进制和 PyO3 扩展导出的 `cli_main`
//! 都调用这里的 [`run`],两条分发渠道不存在两套 CLI 实现。

use std::ffi::OsString;

use clap::{Args, Parser, Subcommand, ValueEnum};
use funget_core::{DownloadOptions, HttpConfig, UploadOptions};

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
    Info { url: String },
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
        Command::Info { url } => info(&url),
    }
}

fn init_logging(verbose: bool) {
    let default = if verbose { "debug" } else { "info" };
    // RUST_LOG 优先,没设就用上面的默认级别。重复初始化(同进程里多次调用
    // `cli_main`)会返回 Err,忽略即可。
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
        http: HttpConfig {
            max_retries: args.max_retries,
            ..HttpConfig::default()
        },
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

fn info(url: &str) -> i32 {
    // 文件名从 URL 推导。Python 版这里把 filepath 写死成 `/tmp/funget-info`,
    // 于是 `Filename:` 永远打印 `funget-info` —— 那是 bug,不是契约,这版改掉了
    // (见 CHANGELOG 的「有意偏离」)。输出的四行格式保持不变。
    let filename = default_filename(url);
    match funget_core::file_info(url, &filename, &DownloadOptions::default()) {
        Ok(details) => {
            println!("URL: {}", details.url);
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

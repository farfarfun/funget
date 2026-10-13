use std::io::IsTerminal;

use indicatif::{ProgressBar, ProgressStyle};

/// 下载/上传进度条。
///
/// 与 Python 版的区别只有外观:tqdm 换成 indicatif(已记入 CHANGELOG 的「有意偏离」)。
/// 行为上有一处是**修复**而非照搬 —— Python 的 `update_pbar` 每收到一个 chunk 就调一次
/// `pbar.refresh()`(`download/multi.py:143`),强制重绘;indicatif 自带限频,
/// 不需要也不应该手动刷。
#[derive(Clone)]
pub struct Progress {
    bar: ProgressBar,
}

impl Progress {
    /// `enabled` 为假、或 stderr 不是终端时,返回一个完全不绘制的空条。
    /// 后者很重要:funget 大量场景是被 Python 脚本或 CI 调用的,
    /// 往重定向的日志里喷转义序列只会把日志弄脏。
    pub fn new(total: u64, prefix: &str, enabled: bool) -> Self {
        if !enabled || !std::io::stderr().is_terminal() {
            return Self {
                bar: ProgressBar::hidden(),
            };
        }

        let bar = ProgressBar::new(total);
        // 样式解析失败不该让下载失败,退回默认样式即可。
        match ProgressStyle::with_template(
            "{prefix}{bar:32.cyan/blue} {bytes}/{total_bytes} {binary_bytes_per_sec} eta {eta}",
        ) {
            Ok(style) => bar.set_style(style.progress_chars("=> ")),
            Err(error) => log::debug!("进度条样式无效,使用默认样式: {error}"),
        }
        bar.set_prefix(prefix.to_string());
        Self { bar }
    }

    /// 完全禁用的进度条,用于库调用方不需要任何输出的场合。
    pub fn hidden() -> Self {
        Self {
            bar: ProgressBar::hidden(),
        }
    }

    pub fn inc(&self, delta: u64) {
        self.bar.inc(delta);
    }

    pub fn set_prefix(&self, prefix: String) {
        self.bar.set_prefix(prefix);
    }

    /// 重置到 0,用于上传重试(对应 Python 的 `pbar.reset(total=filesize)`)。
    pub fn reset(&self) {
        self.bar.set_position(0);
    }

    pub fn finish(&self) {
        self.bar.finish_and_clear();
    }
}

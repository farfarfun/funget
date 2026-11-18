use std::io::{IsTerminal, Write};
use std::time::Duration;

use indicatif::{BinaryBytes, FormattedDuration, ProgressBar, ProgressDrawTarget, ProgressStyle};

#[derive(Clone)]
pub struct Progress {
    bar: ProgressBar,
    plain: bool,
}

impl Progress {
    pub fn new(total: u64, prefix: &str, enabled: bool) -> Self {
        Self::with_output(Some(total), prefix, enabled, false)
    }

    pub(crate) fn with_output(
        total: Option<u64>,
        prefix: &str,
        enabled: bool,
        plain: bool,
    ) -> Self {
        if !enabled || (!plain && !std::io::stderr().is_terminal()) {
            return Self::hidden();
        }
        let target = if plain {
            ProgressDrawTarget::hidden()
        } else {
            ProgressDrawTarget::stderr()
        };
        let bar = ProgressBar::with_draw_target(total, target);
        bar.set_prefix(prefix.to_string());

        if plain {
            let weak = bar.downgrade();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    let Some(bar) = weak.upgrade() else { break };
                    if bar.is_finished() {
                        break;
                    }
                    write_plain(&bar);
                }
            });
        } else {
            let template = if total.is_some() {
                "{prefix}{bar:32.cyan/blue} {bytes}/{total_bytes} {binary_bytes_per_sec} eta {eta}"
            } else {
                "{prefix}{bytes} {binary_bytes_per_sec} elapsed {elapsed}"
            };
            if let Ok(style) = ProgressStyle::with_template(template) {
                bar.set_style(style.progress_chars("=> "));
            }
        }

        Self { bar, plain }
    }

    pub fn hidden() -> Self {
        Self {
            bar: ProgressBar::hidden(),
            plain: false,
        }
    }

    pub fn inc(&self, delta: u64) {
        self.bar.inc(delta);
    }

    pub(crate) fn restore(&self, offset: u64) {
        self.bar.set_position(offset);
        self.bar.reset_eta();
    }

    pub fn set_prefix(&self, prefix: String) {
        self.bar.set_prefix(prefix);
    }

    pub fn reset(&self) {
        self.bar.reset();
    }

    pub fn finish(&self) {
        if self.plain {
            write_plain(&self.bar);
            self.bar.abandon();
        } else {
            self.bar.finish_and_clear();
        }
    }
}

fn write_plain(bar: &ProgressBar) {
    let total = bar
        .length()
        .map(|n| format!("/{}", BinaryBytes(n)))
        .unwrap_or_default();
    let _ = writeln!(
        std::io::stderr().lock(),
        "Progress: {}{}{} {}/s elapsed {}",
        bar.prefix(),
        BinaryBytes(bar.position()),
        total,
        BinaryBytes(bar.per_sec() as u64),
        FormattedDuration(bar.elapsed())
    );
}

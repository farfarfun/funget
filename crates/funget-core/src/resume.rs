use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// 续传状态的落盘节流间隔。每个 chunk 都写一次 JSON 会让状态文件本身变成新瓶颈。
const FLUSH_INTERVAL: Duration = Duration::from_millis(500);

/// 状态文件后缀。放在目标文件旁边(`a.bin` → `a.bin.funget.json`)。
pub const SIDECAR_SUFFIX: &str = ".funget.json";

/// 状态文件格式版本。版本不认识就整个丢弃、重新下载 —— 续传状态是缓存,不是数据。
const STATE_VERSION: u32 = 1;

/// 单个分块的完成进度。`done` 是从 `start` 起已经写入的字节数,
/// 所以续传时该块的游标就是 `start + done`。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockState {
    pub start: u64,
    pub end: u64,
    pub done: u64,
}

impl BlockState {
    pub fn cursor(&self) -> u64 {
        self.start + self.done
    }

    pub fn is_complete(&self) -> bool {
        self.start + self.done > self.end
    }
}

/// 落盘的续传状态。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResumeState {
    pub version: u32,
    /// **脱敏后**的 URL。
    ///
    /// 刻意不存原始链接:下载地址里经常带签名、临时 token 或 Basic 凭据,
    /// 而这个文件会长期留在用户磁盘上。脱敏串足以回答「还是不是同一个资源」。
    pub url: String,
    pub filesize: u64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub blocks: Vec<BlockState>,
}

impl ResumeState {
    pub fn new(
        url: &str,
        filesize: u64,
        etag: Option<String>,
        last_modified: Option<String>,
        ranges: &[(u64, u64)],
    ) -> Self {
        Self {
            version: STATE_VERSION,
            url: crate::url::safe_url(url),
            filesize,
            etag,
            last_modified,
            blocks: ranges
                .iter()
                .map(|&(start, end)| BlockState {
                    start,
                    end,
                    done: 0,
                })
                .collect(),
        }
    }

    /// 判断这份状态能不能用在当前这次下载上。
    ///
    /// 任何一项对不上都整个作废、从头下载。宁可多下一次,也不能把两个不同版本的
    /// 文件拼在一起 —— 那种损坏是静默的,用户发现不了。
    pub fn matches(
        &self,
        url: &str,
        filesize: u64,
        etag: Option<&str>,
        last_modified: Option<&str>,
        ranges: &[(u64, u64)],
    ) -> bool {
        if self.version != STATE_VERSION {
            log::info!("续传状态版本不符,重新下载");
            return false;
        }
        if self.url != crate::url::safe_url(url) || self.filesize != filesize {
            log::info!("续传状态对应的是另一个资源,重新下载");
            return false;
        }
        // ETag 优先;服务端没给 ETag 时退而比对 Last-Modified。
        // 两者都没有就只能靠长度 —— 这是弱校验,但和不续传相比没有变差。
        match (self.etag.as_deref(), etag) {
            (Some(saved), Some(current)) if saved != current => {
                log::info!("远端 ETag 已变化,重新下载");
                return false;
            }
            (Some(_), None) | (None, Some(_)) => {
                log::info!("远端 ETag 出现或消失,重新下载");
                return false;
            }
            _ => {}
        }
        if self.etag.is_none() {
            match (self.last_modified.as_deref(), last_modified) {
                (Some(saved), Some(current)) if saved != current => {
                    log::info!("远端 Last-Modified 已变化,重新下载");
                    return false;
                }
                _ => {}
            }
        }
        if self.blocks.len() != ranges.len()
            || self
                .blocks
                .iter()
                .zip(ranges)
                .any(|(block, &(start, end))| block.start != start || block.end != end)
        {
            log::info!("分块方式已变化(block_size 不同?),重新下载");
            return false;
        }
        // 防御越界的脏状态:done 不该超过块长。
        if self
            .blocks
            .iter()
            .any(|block| block.done > block.end - block.start + 1)
        {
            log::warn!("续传状态自相矛盾,重新下载");
            return false;
        }
        true
    }

    pub fn completed_bytes(&self) -> u64 {
        self.blocks.iter().map(|block| block.done).sum()
    }
}

/// 目标文件对应的状态文件路径。
pub fn sidecar_path(filepath: &Path) -> PathBuf {
    let mut name = filepath.as_os_str().to_os_string();
    name.push(SIDECAR_SUFFIX);
    PathBuf::from(name)
}

/// 状态文件存在,说明上一次下载没有正常收尾,目标文件不可信。
pub fn sidecar_exists(filepath: &Path) -> bool {
    sidecar_path(filepath).exists()
}

/// 读取状态;文件不存在或解析不了都返回 `None`(按「没有续传信息」处理)。
pub fn load(filepath: &Path) -> Option<ResumeState> {
    let path = sidecar_path(filepath);
    let raw = std::fs::read(&path).ok()?;
    match serde_json::from_slice::<ResumeState>(&raw) {
        Ok(state) => Some(state),
        Err(error) => {
            log::warn!("续传状态文件无法解析({error}),按重新下载处理");
            None
        }
    }
}

/// 原子地写入状态:先写临时文件再 rename,避免进程被杀时留下半截 JSON。
pub fn save(filepath: &Path, state: &ResumeState) -> io::Result<()> {
    let path = sidecar_path(filepath);
    let mut temp = path.clone().into_os_string();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);

    let encoded = serde_json::to_vec(state)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    std::fs::write(&temp, encoded)?;
    std::fs::rename(&temp, &path)
}

/// 删除状态文件。下载成功收尾时调用 —— 状态文件的存在本身就代表「未完成」。
pub fn remove(filepath: &Path) {
    let path = sidecar_path(filepath);
    if path.exists() {
        if let Err(error) = std::fs::remove_file(&path) {
            log::warn!("删除续传状态文件失败: {error}");
        }
    }
}

/// 续传进度的共享记录器,负责节流落盘。
///
/// 多线程与单线程两条路径共用:前者有 N 个分块,后者就是一个覆盖整文件的分块。
pub struct Tracker {
    path: PathBuf,
    enabled: bool,
    inner: Mutex<TrackerInner>,
}

struct TrackerInner {
    state: ResumeState,
    last_flush: Instant,
}

impl Tracker {
    pub fn new(filepath: &Path, state: ResumeState, enabled: bool) -> Self {
        Self {
            path: filepath.to_path_buf(),
            enabled,
            inner: Mutex::new(TrackerInner {
                state,
                last_flush: Instant::now(),
            }),
        }
    }

    /// 锁中毒不该让下载失败:续传状态是缓存不是数据,拿到可能过期的那份也比 panic 强。
    fn lock(&self) -> std::sync::MutexGuard<'_, TrackerInner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// 记录某个分块又写入了 `delta` 字节,必要时落盘。
    pub fn advance(&self, index: usize, delta: u64) {
        if !self.enabled {
            return;
        }
        let snapshot = {
            let mut inner = self.lock();
            if let Some(block) = inner.state.blocks.get_mut(index) {
                block.done += delta;
            }
            if inner.last_flush.elapsed() < FLUSH_INTERVAL {
                return;
            }
            inner.last_flush = Instant::now();
            inner.state.clone()
        };
        self.write(&snapshot);
    }

    /// 把某个分块的进度重置为 0。单线程路径在服务端忽略 Range、返回完整 200 时用到。
    pub fn rewind(&self, index: usize) {
        if !self.enabled {
            return;
        }
        let mut inner = self.lock();
        if let Some(block) = inner.state.blocks.get_mut(index) {
            block.done = 0;
        }
    }

    /// 无视节流,立刻落盘。
    pub fn flush(&self) {
        if !self.enabled {
            return;
        }
        let snapshot = self.lock().state.clone();
        self.write(&snapshot);
    }

    fn write(&self, state: &ResumeState) {
        if let Err(error) = save(&self.path, state) {
            log::warn!("写入续传状态失败: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges() -> Vec<(u64, u64)> {
        vec![(0, 99), (100, 199)]
    }

    #[test]
    fn rejects_state_from_a_different_resource() {
        let state = ResumeState::new("https://a.example/f", 200, None, None, &ranges());
        assert!(!state.matches("https://b.example/f", 200, None, None, &ranges()));
        assert!(!state.matches("https://a.example/f", 300, None, None, &ranges()));
    }

    #[test]
    fn rejects_when_etag_changed_or_disappeared() {
        let state = ResumeState::new(
            "https://a.example/f",
            200,
            Some("\"v1\"".into()),
            None,
            &ranges(),
        );
        assert!(!state.matches("https://a.example/f", 200, Some("\"v2\""), None, &ranges()));
        assert!(!state.matches("https://a.example/f", 200, None, None, &ranges()));
        assert!(state.matches("https://a.example/f", 200, Some("\"v1\""), None, &ranges()));
    }

    #[test]
    fn rejects_when_block_layout_changed() {
        let state = ResumeState::new("https://a.example/f", 200, None, None, &ranges());
        assert!(!state.matches("https://a.example/f", 200, None, None, &[(0, 199)]));
    }

    #[test]
    fn url_is_stored_redacted() {
        let state = ResumeState::new(
            "https://user:pw@a.example/f?token=secret",
            200,
            None,
            None,
            &ranges(),
        );
        assert_eq!(state.url, "https://a.example/f");
        for leaked in ["user", "pw", "token", "secret"] {
            assert!(!state.url.contains(leaked), "状态文件里泄漏了 {leaked}");
        }
    }
}

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_LENGTH, CONTENT_RANGE, IF_RANGE, RANGE};
use reqwest::Method;

use crate::client::Transport;
use crate::error::Result;
use crate::io::{already_complete, ensure_parent_dir};
use crate::probe::{validate_url, SizeProbe};
use crate::progress::Progress;
use crate::resume::{self, ResumeState, Tracker};
use crate::url::safe_url;

use super::DownloadOptions;

/// 单线程流式下载,对应 `SingleDownloader.download()`。
///
/// `probe` 允许调用方把智能选择阶段已经做过的大小探测传进来,避免重复一次 HEAD。
pub(crate) async fn run(
    transport: &Transport,
    url: &str,
    filepath: &str,
    probe: Option<SizeProbe>,
    options: &DownloadOptions,
) -> bool {
    if options.chunk_size == 0 {
        log::error!("chunk_size 必须为正数");
        return false;
    }

    let probe = match probe {
        Some(probe) => probe,
        None => super::resolve_probe(transport, url, options).await,
    };
    let filesize = probe.size;
    let path = Path::new(filepath);

    // 顺序与 Python 一致:**先 mkdir 再做跳过检查**(`download/single.py:32`)。
    // 看着反直觉,但副作用是可观测的 —— 即便最终跳过下载,父目录也已经被创建出来了。
    if let Err(error) = ensure_parent_dir(path) {
        log::error!("创建目录失败: {error}");
        return false;
    }

    // 多一个「没有残留续传状态」的条件,理由见 multi.rs 里同一处判断。
    if !options.overwrite && already_complete(path, filesize) && !resume::sidecar_exists(path) {
        log::info!("文件 {filepath} 已存在且大小一致,跳过下载");
        return true;
    }

    let valid = match probe.head_status {
        Some(status) => status < 400,
        None => validate_url(transport, url).await,
    };
    if !valid {
        log::error!("无效的 URL: {}", safe_url(url));
        return false;
    }

    if filesize == 0 {
        log::warn!("文件大小为 0 或未知,仍继续下载");
    }

    // 前缀规则照搬 Python:非空时追加 `--`,空则保持为空。
    let prefix = if options.prefix.is_empty() {
        String::new()
    } else {
        format!("{}--", options.prefix)
    };

    match download_stream(transport, url, path, &probe, &prefix, options).await {
        Ok(result) => result,
        Err(error) => {
            log::error!("单线程下载出错: {error}");
            false
        }
    }
}

async fn download_stream(
    transport: &Transport,
    url: &str,
    path: &Path,
    probe: &SizeProbe,
    prefix: &str,
    options: &DownloadOptions,
) -> Result<bool> {
    let filesize = probe.size;

    let restored = if options.resume && filesize > 0 {
        resume::load(path).filter(|state| {
            state.matches(
                url,
                filesize,
                probe.etag.as_deref(),
                probe.last_modified.as_deref(),
                &[(0, filesize - 1)],
            )
        })
    } else {
        None
    };
    let local_len = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    // Ctrl+C 可能发生在两次状态保存之间，只信任最后一个检查点。
    let mut offset = restored
        .as_ref()
        .map(ResumeState::completed_bytes)
        .filter(|&done| done <= local_len && local_len <= filesize && done < filesize)
        .unwrap_or(0);

    let mut extra = HeaderMap::new();
    if offset > 0 {
        extra.insert(
            RANGE,
            HeaderValue::from_str(&format!("bytes={offset}-"))
                .map_err(|_| crate::error::Error::invalid("无法构造 Range 请求头"))?,
        );
        // If-Range 是第二道保险:探测与真正取数据之间资源若被换掉,服务端会忽略
        // Range 直接回 200 完整内容,我们据此从 0 重来,而不是把两个版本拼在一起。
        if let Some(validator) = probe.etag.as_deref().or(probe.last_modified.as_deref()) {
            match HeaderValue::from_str(validator) {
                Ok(value) => {
                    extra.insert(IF_RANGE, value);
                }
                Err(_) => log::debug!("校验标识不适合放进 If-Range,跳过"),
            }
        }
    }

    let response = transport
        .send(Method::GET, url, transport.config().max_retries, &extra)
        .await?;
    let status = response.status();
    if !status.is_success() {
        return Err(crate::error::Error::Status {
            status: status.as_u16(),
        });
    }

    if offset > 0 {
        match status.as_u16() {
            206 => {
                let expected = format!("bytes {offset}-{}/{filesize}", filesize - 1);
                if response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    != Some(&expected)
                {
                    return Err(crate::error::Error::invalid(
                        "续传响应的 Content-Range 与断点不符",
                    ));
                }
            }
            200 => offset = 0,
            _ => {
                return Err(crate::error::Error::Status {
                    status: status.as_u16(),
                })
            }
        }
    }

    // Python 只是告警、并不中止(`download/single.py:68`),这里保持一致。
    if let Some(value) = response.headers().get(CONTENT_LENGTH) {
        if let Some(declared) = value.to_str().ok().and_then(|s| s.parse::<u64>().ok()) {
            if filesize > 0 && declared != filesize - offset {
                log::warn!("Content-Length 不符: 期望 {filesize},实际 {declared}(偏移 {offset})");
            }
        }
    }

    let total = if filesize > 0 {
        Some(filesize)
    } else {
        response.content_length()
    };

    let mut file = open_target(path, offset)?;
    if offset > 0 {
        log::info!(
            "已从断点恢复 {}（{offset} 字节）",
            indicatif::BinaryBytes(offset)
        );
    }

    let blocks = if filesize > 0 {
        vec![(0, filesize - 1)]
    } else {
        Vec::new()
    };
    let mut state = ResumeState::new(
        url,
        filesize,
        probe.etag.clone(),
        probe.last_modified.clone(),
        &blocks,
    );
    if let Some(block) = state.blocks.first_mut() {
        block.done = offset;
    }
    let tracker = Tracker::new(path, state, options.resume && !blocks.is_empty());
    tracker.flush();

    let progress = Progress::with_output(total, prefix, options.progress, options.plain_progress);
    progress.restore(offset);

    let mut downloaded = offset;
    let mut stream = response.bytes_stream();
    let mut outcome: Result<()> = Ok(());
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                outcome = Err(error.into());
                break;
            }
        };
        if chunk.is_empty() {
            continue;
        }
        if let Err(error) = file.write_all(&chunk) {
            outcome = Err(error.into());
            break;
        }
        let written = chunk.len() as u64;
        downloaded += written;
        progress.inc(written);
        tracker.advance(0, written);
    }
    progress.finish();

    if let Err(error) = outcome {
        log::error!("下载过程中出错: {error}");
        if options.resume {
            // 把真实落盘量写回状态,下次好接着下。
            let _ = file.flush();
            tracker.flush();
        } else {
            resume::remove(path);
        }
        return Ok(false);
    }

    if let Err(error) = file.flush() {
        log::error!("刷新文件缓冲失败: {error}");
        return Ok(false);
    }

    if filesize > 0 && downloaded != filesize {
        log::error!("下载不完整: 期望 {filesize} 字节,实际 {downloaded} 字节");
        if options.resume {
            tracker.flush();
        } else {
            resume::remove(path);
        }
        return Ok(false);
    }

    resume::remove(path);
    Ok(true)
}

/// `offset > 0` 时在已有内容后面追加,否则清空重写。
fn open_target(path: &Path, offset: u64) -> std::io::Result<File> {
    if offset > 0 {
        let file = OpenOptions::new().append(true).open(path)?;
        let length = file.metadata()?.len();
        if length < offset {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "本地文件短于续传检查点",
            ));
        }
        if length > offset {
            file.set_len(offset)?;
        }
        Ok(file)
    } else {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resume::sidecar_path;

    #[test]
    fn append_mode_discards_only_the_unconfirmed_tail() {
        let dir = tempfile::tempdir().expect("创建临时目录");
        let path = dir.path().join("partial.bin");
        std::fs::write(&path, b"0123456789").expect("写入半截文件");

        assert!(open_target(&path, 10).is_ok());
        assert!(open_target(&path, 11).is_err());
        assert!(open_target(&path, 5).is_ok());
        assert_eq!(std::fs::read(path).unwrap(), b"01234");
    }

    #[test]
    fn fresh_open_truncates() {
        let dir = tempfile::tempdir().expect("创建临时目录");
        let path = dir.path().join("stale.bin");
        std::fs::write(&path, b"old content").expect("写入旧文件");

        let file = open_target(&path, 0).expect("打开目标文件");
        assert_eq!(file.metadata().expect("读取元数据").len(), 0);
    }

    #[test]
    fn sidecar_sits_next_to_the_target() {
        let path = Path::new("/tmp/a.bin");
        assert_eq!(sidecar_path(path), Path::new("/tmp/a.bin.funget.json"));
    }
}

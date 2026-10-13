use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_LENGTH, IF_RANGE, RANGE};
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

    if !validate_url(transport, url).await {
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

    // 续传偏移。单线程路径是顺序追加写,所以「已完成的字节数」就是文件的当前长度 ——
    // 但光有长度不够:谁也不能保证本地那个半截文件来自同一个 URL。所以仍然靠
    // sidecar 的指纹(URL + 大小 + ETag/Last-Modified)来决定能不能接着下。
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
    // 状态文件记的完成量必须和磁盘上真实的文件长度对得上,否则一律从头来。
    let mut offset = match &restored {
        Some(state) if state.completed_bytes() == local_len && local_len < filesize => local_len,
        Some(_) => {
            log::info!("续传状态与本地文件长度不一致,重新下载");
            0
        }
        None => 0,
    };

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
    if status.as_u16() >= 400 {
        return Err(crate::error::Error::Status {
            status: status.as_u16(),
        });
    }

    if offset > 0 && status.as_u16() != 206 {
        // 服务端没接受 Range(资源变了,或压根不支持)—— 整个重下。
        log::info!("服务端返回 {status},放弃续传并从头下载");
        offset = 0;
    }

    // Python 只是告警、并不中止(`download/single.py:68`),这里保持一致。
    if let Some(value) = response.headers().get(CONTENT_LENGTH) {
        if let Some(declared) = value.to_str().ok().and_then(|s| s.parse::<u64>().ok()) {
            if declared + offset != filesize && filesize > 0 {
                log::warn!("Content-Length 不符: 期望 {filesize},实际 {declared}(偏移 {offset})");
            }
        }
    }

    let total = if filesize > 0 {
        filesize
    } else {
        response.content_length().unwrap_or(0)
    };

    let mut file = open_target(path, offset)?;

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

    let progress = Progress::new(total, prefix, options.progress);
    progress.inc(offset);

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
        // 追加模式下写入位置由内核决定,这里只校验它确实等于我们请求的偏移。
        if file.metadata()?.len() != offset {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "续传偏移与文件长度不符",
            ));
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
    fn append_mode_refuses_mismatched_offset() {
        let dir = tempfile::tempdir().expect("创建临时目录");
        let path = dir.path().join("partial.bin");
        std::fs::write(&path, b"0123456789").expect("写入半截文件");

        assert!(open_target(&path, 10).is_ok());
        // 状态说已经下了 5 字节,磁盘上却有 10 字节 —— 必须拒绝,否则会写出错位的文件。
        assert!(open_target(&path, 5).is_err());
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

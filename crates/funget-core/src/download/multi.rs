use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{self, StreamExt};
use reqwest::header::{HeaderMap, HeaderValue, RANGE};
use reqwest::Method;

use crate::client::Transport;
use crate::error::{Error, Result};
use crate::io::{already_complete, ensure_parent_dir, write_at};
use crate::probe::{probe_range_support, SizeProbe};
use crate::progress::Progress;
use crate::resume::{self, BlockState, ResumeState, Tracker};

use super::DownloadOptions;

/// 最小分块大小,单位 MiB。对应 `MultiDownloader.__init__` 的 `min_block_size=1`。
pub const MIN_BLOCK_SIZE_MIB: u64 = 1;

/// 分块数量:`max(1, min(filesize/block, filesize/min_block))`。
///
/// 照搬 `MultiDownloader.__init__`(`download/multi.py:47`)。这个式子的效果是
/// 「块不小于 1 MiB」而不是「块大小等于 block_size」—— 看着别扭,但它是契约。
pub fn blocks_num(filesize: u64, block_size_mib: u64) -> u64 {
    if filesize == 0 || block_size_mib == 0 {
        return 1;
    }
    let block = block_size_mib.saturating_mul(1024 * 1024);
    let min_block = MIN_BLOCK_SIZE_MIB * 1024 * 1024;
    std::cmp::max(1, std::cmp::min(filesize / block, filesize / min_block))
}

/// 切分下载区间。
///
/// 逐字复刻 `MultiDownloader.__get_range`(`download/multi.py:62`),**包括**
/// `i > 0` 时那个 `start += 1` 造成的块长不齐:第 0 块是 `[0, size]` 共 size+1 字节,
/// 其后各块是 `[i*size+1, i*size+1+size]`。这不是笔误就是笔误,但它决定了
/// 每个块实际请求的字节区间;"修正"它会让新旧两版发出的 Range 请求对不上,
/// 差分验证也就失去了意义。
pub fn ranges(filesize: u64, blocks: u64) -> Vec<(u64, u64)> {
    if filesize == 0 {
        return Vec::new();
    }
    if blocks <= 1 {
        return vec![(0, filesize - 1)];
    }

    let size = filesize / blocks;
    let mut list = Vec::with_capacity(blocks as usize);
    for i in 0..blocks {
        let start = if i > 0 { i * size + 1 } else { 0 };
        let end = if i == blocks - 1 {
            filesize - 1
        } else {
            start + size
        };
        if start <= end {
            list.push((start, end));
        } else {
            log::warn!("跳过无效区间 {start}-{end}");
        }
    }
    list
}

pub(crate) async fn run(
    transport: &Transport,
    url: &str,
    filepath: &str,
    options: &DownloadOptions,
) -> bool {
    let probe = super::resolve_probe(transport, url, options).await;
    let supports_range = probe_range_support(transport, url).await;
    run_prepared(transport, url, filepath, &probe, supports_range, options).await
}

pub(crate) async fn run_prepared(
    transport: &Transport,
    url: &str,
    filepath: &str,
    probe: &SizeProbe,
    supports_range: bool,
    options: &DownloadOptions,
) -> bool {
    let path = Path::new(filepath);
    let filename = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let filesize = probe.size;

    if options.worker_num == 0 {
        log::error!("worker_num 必须为正数");
        return false;
    }
    if options.block_size == 0 {
        // Python 侧是 `MultiDownloader.__init__` 抛 ValueError、外层 try 吞掉后返回 False。
        log::error!("block_size 必须为正数");
        return false;
    }

    let blocks = if supports_range {
        blocks_num(filesize, options.block_size)
    } else {
        log::info!("{filename} 不支持 Range 请求,退回单线程下载");
        1
    };

    // 契约 4,但多了一个条件:旁边不能有续传状态文件。
    // 状态文件存在就代表上一次没有正常收尾,此时「文件大小正好等于远端大小」
    // 恰恰是 Python 版会静默接受一个带空洞的损坏文件的那个洞(预分配/seek 写都会
    // 把文件撑到完整大小)。多这一个条件就把洞堵上了。
    if !options.overwrite && already_complete(path, filesize) && !resume::sidecar_exists(path) {
        log::info!("文件 {filepath} 已存在且大小一致,跳过下载");
        return true;
    }

    if let Err(error) = ensure_parent_dir(path) {
        log::error!("创建目录失败: {error}");
        return false;
    }

    let range_list = ranges(filesize, blocks);
    if range_list.is_empty() {
        log::error!("没有计算出有效的下载区间");
        return false;
    }

    // 续传与 overwrite 是正交的:overwrite 管「已完整的文件要不要重下」,
    // 续传管「没下完的文件要不要接着下」。指纹校验保证接着下也不会拼错。
    let restored = if options.resume {
        resume::load(path).filter(|state| {
            state.matches(
                url,
                filesize,
                probe.etag.as_deref(),
                probe.last_modified.as_deref(),
                &range_list,
            )
        })
    } else {
        None
    };

    let target_intact = std::fs::metadata(path).map(|meta| meta.len()).ok() == Some(filesize);
    let (file, blocks_state) = match restored {
        Some(state) if target_intact => {
            let done = state.completed_bytes();
            log::info!("命中续传状态,已完成 {done}/{filesize} 字节");
            match OpenOptions::new().write(true).open(path) {
                Ok(file) => (file, state.blocks),
                Err(error) => {
                    log::error!("打开待续传文件失败: {error}");
                    return false;
                }
            }
        }
        other => {
            if other.is_some() {
                log::info!("目标文件与续传状态不一致,重新下载");
            }
            resume::remove(path);
            match create_target(path, filesize) {
                Ok(file) => (
                    file,
                    range_list
                        .iter()
                        .map(|&(start, end)| BlockState {
                            start,
                            end,
                            done: 0,
                        })
                        .collect(),
                ),
                Err(error) => {
                    log::error!("创建目标文件失败: {error}");
                    return false;
                }
            }
        }
    };

    let mut initial = ResumeState::new(
        url,
        filesize,
        probe.etag.clone(),
        probe.last_modified.clone(),
        &range_list,
    );
    initial.blocks = blocks_state.clone();
    let resumed_bytes = initial.completed_bytes();

    let tracker = Arc::new(Tracker::new(path, initial, options.resume));
    // 先把初始状态落盘:进程若在第一次节流窗口内就被杀,也至少知道「这个文件没下完」。
    tracker.flush();

    let total_blocks = blocks_state.len();
    let finished_blocks = blocks_state.iter().filter(|b| b.is_complete()).count();

    let progress = Progress::new(
        filesize,
        &format!("{}|0/{total_blocks}|", options.prefix),
        options.progress,
    );
    progress.inc(resumed_bytes);

    let file = Arc::new(file);
    let pending: Vec<(usize, BlockState)> = blocks_state
        .into_iter()
        .enumerate()
        .filter(|(_, block)| !block.is_complete())
        .collect();

    let completed = stream::iter(pending.into_iter().map(|(index, block)| {
        let transport = transport.clone();
        let file = Arc::clone(&file);
        let tracker = Arc::clone(&tracker);
        let progress = progress.clone();
        let url = url.to_string();
        let retries = options.max_retries;
        async move {
            let ok = fetch_block(
                &transport, &url, &file, index, block, retries, &progress, &tracker,
            )
            .await;
            usize::from(ok)
        }
    }))
    .buffer_unordered(options.worker_num)
    .fold(finished_blocks, |acc, done| async move { acc + done })
    .await;

    progress.set_prefix(format!(
        "{}|{completed}/{total_blocks}|{filename}",
        options.prefix
    ));
    progress.finish();

    if completed == total_blocks {
        resume::remove(path);
        return true;
    }

    log::error!("下载失败:{completed}/{total_blocks} 个分块完成");
    if options.resume {
        tracker.flush();
        log::info!("已保留续传状态,下次调用可从断点继续");
    } else {
        // 不开续传时必须把残缺文件截断掉。预分配让它的大小正好等于远端大小,
        // 下一次 overwrite=False 的调用会命中「已完整」检查,把损坏文件当成下载完成。
        if let Err(error) = file.set_len(0) {
            log::warn!("截断残缺文件失败: {error}");
        }
        resume::remove(path);
    }
    false
}

/// 新建目标文件并预分配到完整大小,让后续的定位写落在已分配的空间上。
fn create_target(path: &Path, filesize: u64) -> std::io::Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    file.set_len(filesize)?;
    Ok(file)
}

/// 下载一个分块,失败按 `2^attempt` 秒退避重试。
///
/// 对应 `Worker.run()`。重试时**保留游标**,从已写入的位置继续,而不是从块首重来 ——
/// Python 版也是这个语义(`self.range_curser` 是实例状态)。
#[allow(clippy::too_many_arguments)]
async fn fetch_block(
    transport: &Transport,
    url: &str,
    file: &File,
    index: usize,
    block: BlockState,
    max_retries: u32,
    progress: &Progress,
    tracker: &Tracker,
) -> bool {
    let BlockState { start, end, .. } = block;
    let size = end - start + 1;
    let mut cursor = block.cursor();

    for attempt in 0..=max_retries {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(1u64 << (attempt - 1))).await;
        }
        if cursor > end {
            return true;
        }

        match fetch_once(
            transport,
            url,
            file,
            start,
            end,
            size,
            &mut cursor,
            index,
            progress,
            tracker,
        )
        .await
        {
            Ok(true) => return true,
            Ok(false) => log::warn!("分块 {start}-{end} 第 {} 次尝试未完成", attempt + 1),
            Err(error) => log::warn!("分块 {start}-{end} 第 {} 次尝试失败: {error}", attempt + 1),
        }
    }
    false
}

/// 单次取块,逐条对应 `Worker._download_chunk`。
#[allow(clippy::too_many_arguments)]
async fn fetch_once(
    transport: &Transport,
    url: &str,
    file: &File,
    start: u64,
    end: u64,
    size: u64,
    cursor: &mut u64,
    index: usize,
    progress: &Progress,
    tracker: &Tracker,
) -> Result<bool> {
    let mut extra = HeaderMap::new();
    let range = format!("bytes={}-{end}", *cursor);
    extra.insert(
        RANGE,
        HeaderValue::from_str(&range)
            .map_err(|_| Error::invalid(format!("无法构造 Range 请求头: {range}")))?,
    );

    // retries=0 是刻意的:Python 的 Worker 用的是**裸** session(`download/work.py:47`),
    // 不挂 urllib3.Retry,重试全由外层 `fetch_block` 负责。挂两层会让退避次数翻倍。
    let response = transport.send(Method::GET, url, 0, &extra).await?;
    let status = response.status().as_u16();

    // 416 不抛异常、直接算这次没成,交给外层决定要不要重试 —— 与 Python 一致。
    if status == 416 {
        return Ok(false);
    }
    if status >= 400 {
        return Err(Error::Status { status });
    }
    if status != 200 && status != 206 {
        log::warn!("意外的状态码: {status}");
        return Ok(false);
    }
    if status == 200 {
        // 服务端无视了 Range,整个文件都发过来了。只有「这个块本来就从 0 开始
        // 且长度正好对得上」才能用,否则直接判失败,免得把整文件写进某个分块的位置。
        let content_length = response.content_length().unwrap_or(0);
        if start != 0 || (content_length != 0 && content_length != size) {
            return Ok(false);
        }
    }

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if chunk.is_empty() {
            continue;
        }
        if *cursor > end {
            break;
        }
        // 服务端可能多给,按区间右界裁掉(对应 Python 的 `chunk[: end - curser + 1]`)。
        let room = (end - *cursor + 1) as usize;
        let chunk = if chunk.len() > room {
            &chunk[..room]
        } else {
            &chunk[..]
        };

        write_at(file, chunk, *cursor)?;
        let written = chunk.len() as u64;
        *cursor += written;
        progress.inc(written);
        tracker.advance(index, written);

        if *cursor > end {
            break;
        }
    }

    Ok(*cursor > end)
}

#[cfg(test)]
mod tests {
    use super::{blocks_num, ranges};

    #[test]
    fn block_count_matches_python_formula() {
        let mib = 1024 * 1024;
        // 256 MiB / 32 MiB 块 → min(8, 256) = 8
        assert_eq!(blocks_num(256 * mib, 32), 8);
        // 块比文件还大 → min(0, N) = 0 → 被 max(1, _) 抬成 1
        assert_eq!(blocks_num(5 * mib, 100), 1);
        // 不足 1 MiB 的文件:min(0, 0) → 1
        assert_eq!(blocks_num(1000, 1), 1);
        assert_eq!(blocks_num(0, 100), 1);
        // block_size 为 0 不能把除法炸掉
        assert_eq!(blocks_num(100 * mib, 0), 1);
    }

    #[test]
    fn single_block_covers_whole_file() {
        assert_eq!(ranges(1000, 1), vec![(0, 999)]);
        assert_eq!(ranges(0, 4), Vec::new());
    }

    #[test]
    fn reproduces_the_off_by_one_split() {
        // 1000 字节切 4 块,size = 250。
        // 第 0 块 [0,250];其后 start = i*250+1,末块右界收到 999。
        assert_eq!(
            ranges(1000, 4),
            vec![(0, 250), (251, 501), (501, 751), (751, 999)]
        );
    }

    #[test]
    fn ranges_cover_every_byte() {
        // 不管切得多怪,每个字节都得被至少一个区间覆盖,否则文件会留洞。
        for (filesize, blocks) in [(1000u64, 4u64), (1024 * 1024 * 7, 3), (12345, 7)] {
            let list = ranges(filesize, blocks);
            let mut covered = vec![false; filesize as usize];
            for (start, end) in list {
                for byte in covered
                    .iter_mut()
                    .take(end as usize + 1)
                    .skip(start as usize)
                {
                    *byte = true;
                }
            }
            assert!(
                covered.iter().all(|&seen| seen),
                "{filesize} 字节切 {blocks} 块时存在未覆盖的字节"
            );
        }
    }
}

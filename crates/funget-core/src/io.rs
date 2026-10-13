use std::fs::File;
use std::io;
use std::path::Path;

/// 按绝对偏移写入,不移动文件的共享读写位置。
///
/// 这是 Rust 版相对 Python 版最实在的一处结构性简化:`funfile.ConcurrentFile`
/// 要用「后台线程 + 队列」把 seek+write 串行化才能让多个 worker 并发写同一个文件,
/// 队列上限还是按**条目数**算的(默认 200 条 × 2 MiB chunk ⇒ 可驻留 400 MiB)。
/// `pwrite`/`seek_write` 本身就是原子的定位写,这一整层直接消失。
#[cfg(unix)]
pub fn write_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

#[cfg(windows)]
pub fn write_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    // Windows 没有 write_all_at,得自己处理短写。
    let mut buf = buf;
    let mut offset = offset;
    while !buf.is_empty() {
        match file.seek_write(buf, offset) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "写入返回 0 字节,无法推进",
                ))
            }
            Ok(written) => {
                buf = &buf[written..];
                offset += written as u64;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// 确保目标文件的父目录存在,对应 Python 的
/// `os.makedirs(os.path.dirname(os.path.abspath(filepath)), exist_ok=True)`。
pub fn ensure_parent_dir(filepath: &Path) -> io::Result<()> {
    let absolute = if filepath.is_absolute() {
        filepath.to_path_buf()
    } else {
        std::env::current_dir()?.join(filepath)
    };
    match absolute.parent() {
        Some(parent) => std::fs::create_dir_all(parent),
        None => Ok(()),
    }
}

/// 本地文件已存在且大小正好等于远端大小时跳过下载。
///
/// 对应契约 4(`download/multi.py:114`、`download/single.py:35`)。
///
/// 不加 `filesize > 0` 的守卫是刻意的:远端大小探测失败时 `filesize` 为 0,
/// 此时若本地恰好有个 0 字节文件,Python 版判定为「已完整」并返回 `True`。
/// 这个判定本身很可疑,但它是冻结的契约,不在这一版顺手改。
pub fn already_complete(filepath: &Path, filesize: u64) -> bool {
    std::fs::metadata(filepath)
        .map(|meta| meta.is_file() && meta.len() == filesize)
        .unwrap_or(false)
}

//! 下载路径的行为契约测试。
//!
//! 这些测试锁的是「与 1.1.70 版 Python 实现逐条对齐」的那些行为,而不是
//! 「我觉得应该怎样」。每个测试上方注明它对应哪一条契约。

mod common;

use std::path::Path;

use common::{payload, Behavior, TestServer};
use funget_core::client::headers_from_pairs;
use funget_core::{download_auto, download_multi, download_single, DownloadOptions, HttpConfig};

const MIB: usize = 1024 * 1024;

fn options(mutate: impl FnOnce(&mut DownloadOptions)) -> DownloadOptions {
    let mut options = DownloadOptions {
        progress: false,
        ..Default::default()
    };
    mutate(&mut options);
    options
}

fn sidecar(path: &Path) -> std::path::PathBuf {
    funget_core::resume::sidecar_path(path)
}

#[test]
fn multi_download_reproduces_the_bytes_exactly() {
    let body = payload(4 * MIB);
    let server = TestServer::start(body.clone());
    let dir = tempfile::tempdir().expect("创建临时目录");
    let target = dir.path().join("out.bin");

    let ok = download_multi(
        &server.url(),
        target.to_str().expect("路径"),
        &options(|o| {
            o.worker_num = 4;
            o.block_size = 1;
        }),
    );

    assert!(ok, "下载应当成功");
    assert_eq!(std::fs::read(&target).expect("读回文件"), body);
    // 真的走了多块并发,而不是悄悄退化成单线程。
    assert!(
        server.ranges().len() > 2,
        "期望多个 Range 请求,实际: {:?}",
        server.ranges()
    );
    assert!(!sidecar(&target).exists(), "成功后状态文件应当被删除");
}

/// 契约 4:文件已存在且大小一致则跳过,不重新下载。
#[test]
fn skips_when_the_local_file_is_already_complete() {
    let body = payload(3 * MIB);
    let server = TestServer::start(body.clone());
    let dir = tempfile::tempdir().expect("创建临时目录");
    let target = dir.path().join("out.bin");

    // 大小相同但内容不同:跳过逻辑只看大小,所以内容必须原封不动。
    let placeholder = vec![0xAAu8; body.len()];
    std::fs::write(&target, &placeholder).expect("预置文件");

    let ok = download_multi(
        &server.url(),
        target.to_str().expect("路径"),
        &options(|_| {}),
    );

    assert!(ok, "已完整的文件应当直接返回成功");
    assert_eq!(std::fs::read(&target).expect("读回文件"), placeholder);

    // overwrite=true 时必须真的重下。
    let ok = download_multi(
        &server.url(),
        target.to_str().expect("路径"),
        &options(|o| o.overwrite = true),
    );
    assert!(ok);
    assert_eq!(std::fs::read(&target).expect("读回文件"), body);
}

/// 契约 1:严格大于 10 MiB 且支持 Range 才走多线程。
#[test]
fn auto_select_uses_the_ten_mib_boundary() {
    let dir = tempfile::tempdir().expect("创建临时目录");

    // 恰好 10 MiB —— 单线程。观测点:除了 Range 支持探测之外没有别的 Range 请求。
    let server = TestServer::start(payload(10 * MIB));
    let target = dir.path().join("exactly.bin");
    assert!(download_auto(
        &server.url(),
        target.to_str().expect("路径"),
        None,
        &options(|_| {})
    ));
    assert_eq!(
        server.ranges().len(),
        1,
        "恰好 10 MiB 应当走单线程,只应有一次 Range 探测"
    );

    // 10 MiB + 1 —— 多线程。
    let server = TestServer::start(payload(10 * MIB + 1));
    let target = dir.path().join("one_more.bin");
    assert!(download_auto(
        &server.url(),
        target.to_str().expect("路径"),
        None,
        &options(|_| {})
    ));
    assert!(
        server.ranges().len() > 1,
        "超过 10 MiB 应当走多线程,实际 Range 请求数: {}",
        server.ranges().len()
    );
}

/// 不支持 Range 的服务端必须退回单线程,而不是把整个文件重复写进每个分块。
#[test]
fn falls_back_to_a_single_block_without_range_support() {
    let body = payload(2 * MIB);
    let server = TestServer::start(body.clone());
    server.set_behavior(Behavior {
        supports_range: false,
        ..Default::default()
    });
    let dir = tempfile::tempdir().expect("创建临时目录");
    let target = dir.path().join("out.bin");

    let ok = download_multi(
        &server.url(),
        target.to_str().expect("路径"),
        &options(|o| {
            o.worker_num = 4;
            o.block_size = 1;
        }),
    );

    assert!(ok, "不支持 Range 时仍应下载成功");
    assert_eq!(std::fs::read(&target).expect("读回文件"), body);
}

/// 续传主路径:中断 → 保留状态 → 再跑一次补齐。
#[test]
fn resume_continues_from_the_sidecar() {
    let body = payload(4 * MIB);
    let server = TestServer::start(body.clone());
    // 每个分块只发 64 KiB 就断线。
    server.set_behavior(Behavior {
        truncate_body_at: Some(64 * 1024),
        ..Default::default()
    });

    let dir = tempfile::tempdir().expect("创建临时目录");
    let target = dir.path().join("out.bin");
    let config = |o: &mut DownloadOptions| {
        o.worker_num = 4;
        o.block_size = 1;
        o.max_retries = 0;
        o.resume = true;
        o.http = HttpConfig {
            max_retries: 0,
            ..HttpConfig::default()
        };
    };

    let ok = download_multi(
        &server.url(),
        target.to_str().expect("路径"),
        &options(config),
    );
    assert!(!ok, "传输被掐断,这一次应当失败");
    assert!(sidecar(&target).exists(), "失败后应当留下续传状态");
    let partial = std::fs::metadata(&target).expect("读取元数据").len();
    assert_eq!(partial, body.len() as u64, "预分配后文件应为完整大小");

    // 恢复正常,再跑一次。
    server.set_behavior(Behavior::default());
    let before = server.request_count();
    let ok = download_multi(
        &server.url(),
        target.to_str().expect("路径"),
        &options(config),
    );

    assert!(ok, "第二次应当补齐并成功");
    assert_eq!(std::fs::read(&target).expect("读回文件"), body);
    assert!(!sidecar(&target).exists(), "成功后状态文件应当被删除");

    // 续传的意义在于只补差额:第二次请求的 Range 必须从非 0 偏移开始。
    let resumed: Vec<String> = server.ranges().into_iter().skip(before).collect();
    assert!(
        resumed.iter().any(|range| !range.starts_with("bytes=0-")),
        "第二次应当从断点处继续,实际请求: {resumed:?}"
    );
}

/// 不开续传时失败必须把文件截断到 0。
///
/// 这是在修一个 Python 版就存在的静默损坏:分块写是定位写,只要最后一块写成功,
/// 文件大小就等于远端大小。下一次 `overwrite=False` 的调用会命中「已完整」检查,
/// 把一个中间有空洞的坏文件当成下载完成返回 `True`。
#[test]
fn failed_download_without_resume_leaves_no_usable_file() {
    let body = payload(4 * MIB);
    let server = TestServer::start(body.clone());
    server.set_behavior(Behavior {
        truncate_body_at: Some(16 * 1024),
        ..Default::default()
    });

    let dir = tempfile::tempdir().expect("创建临时目录");
    let target = dir.path().join("out.bin");
    let config = |o: &mut DownloadOptions| {
        o.worker_num = 4;
        o.block_size = 1;
        o.max_retries = 0;
        o.http = HttpConfig {
            max_retries: 0,
            ..HttpConfig::default()
        };
    };

    assert!(!download_multi(
        &server.url(),
        target.to_str().expect("路径"),
        &options(config)
    ));
    assert_eq!(
        std::fs::metadata(&target).expect("读取元数据").len(),
        0,
        "失败后残缺文件必须被截断,否则会被下一次调用误判为已完成"
    );
    assert!(!sidecar(&target).exists());

    // 关键断言:再调一次(overwrite=false)不能直接返回成功了事。
    server.set_behavior(Behavior::default());
    assert!(download_multi(
        &server.url(),
        target.to_str().expect("路径"),
        &options(config)
    ));
    assert_eq!(std::fs::read(&target).expect("读回文件"), body);
}

/// 远端内容换了,旧状态必须整个作废,而不是把两个版本拼起来。
#[test]
fn stale_sidecar_is_discarded_when_the_etag_changes() {
    let first = payload(4 * MIB);
    let server = TestServer::start(first.clone());
    server.set_behavior(Behavior {
        truncate_body_at: Some(64 * 1024),
        ..Default::default()
    });

    let dir = tempfile::tempdir().expect("创建临时目录");
    let target = dir.path().join("out.bin");
    let config = |o: &mut DownloadOptions| {
        o.worker_num = 4;
        o.block_size = 1;
        o.max_retries = 0;
        o.resume = true;
        o.http = HttpConfig {
            max_retries: 0,
            ..HttpConfig::default()
        };
    };

    assert!(!download_multi(
        &server.url(),
        target.to_str().expect("路径"),
        &options(config)
    ));
    assert!(sidecar(&target).exists());

    // 同样长度、不同内容、不同 ETag —— 只有指纹校验能发现。
    let second: Vec<u8> = first.iter().map(|byte| byte ^ 0xFF).collect();
    server.set_body(second.clone());
    server.set_behavior(Behavior {
        etag: Some("\"v2\"".to_string()),
        ..Default::default()
    });

    assert!(download_multi(
        &server.url(),
        target.to_str().expect("路径"),
        &options(config)
    ));
    assert_eq!(
        std::fs::read(&target).expect("读回文件"),
        second,
        "ETag 变了就必须整个重下,不能混入上一版的字节"
    );
}

/// 单线程路径的续传:靠 sidecar 指纹 + If-Range 两道校验。
#[test]
fn single_download_resumes_and_restarts_correctly() {
    let body = payload(512 * 1024);
    let server = TestServer::start(body.clone());
    server.set_behavior(Behavior {
        truncate_body_at: Some(32 * 1024),
        ..Default::default()
    });

    let dir = tempfile::tempdir().expect("创建临时目录");
    let target = dir.path().join("out.bin");
    let config = |o: &mut DownloadOptions| {
        o.resume = true;
        o.max_retries = 0;
        o.http = HttpConfig {
            max_retries: 0,
            ..HttpConfig::default()
        };
    };

    assert!(!download_single(
        &server.url(),
        target.to_str().expect("路径"),
        &options(config)
    ));
    let partial = std::fs::metadata(&target).expect("读取元数据").len();
    assert_eq!(partial, 32 * 1024, "单线程是顺序写,文件长度就是已完成量");
    assert!(sidecar(&target).exists());

    server.set_behavior(Behavior::default());
    let before = server.ranges().len();
    assert!(download_single(
        &server.url(),
        target.to_str().expect("路径"),
        &options(config)
    ));
    assert_eq!(std::fs::read(&target).expect("读回文件"), body);
    assert!(!sidecar(&target).exists());

    let resumed: Vec<String> = server.ranges().into_iter().skip(before).collect();
    assert!(
        resumed.iter().any(|range| range == "bytes=32768-"),
        "应当从 32768 处续传,实际: {resumed:?}"
    );
}

/// 服务端换了内容时,单线程续传要靠 If-Range 自动退回整文件重下。
#[test]
fn single_download_restarts_when_if_range_does_not_match() {
    let first = payload(256 * 1024);
    let server = TestServer::start(first.clone());
    server.set_behavior(Behavior {
        truncate_body_at: Some(16 * 1024),
        ..Default::default()
    });

    let dir = tempfile::tempdir().expect("创建临时目录");
    let target = dir.path().join("out.bin");
    let config = |o: &mut DownloadOptions| {
        o.resume = true;
        o.max_retries = 0;
        o.http = HttpConfig {
            max_retries: 0,
            ..HttpConfig::default()
        };
    };

    assert!(!download_single(
        &server.url(),
        target.to_str().expect("路径"),
        &options(config)
    ));

    // 长度不变、内容和 ETag 都变了。
    let second: Vec<u8> = first.iter().map(|byte| byte.wrapping_add(7)).collect();
    server.set_body(second.clone());
    server.set_behavior(Behavior {
        etag: Some("\"v2\"".to_string()),
        ..Default::default()
    });

    assert!(download_single(
        &server.url(),
        target.to_str().expect("路径"),
        &options(config)
    ));
    assert_eq!(std::fs::read(&target).expect("读回文件"), second);
}

/// 契约:调用方传入的 headers **覆盖** Range 等专属头。
///
/// 这个顺序来自 Python 的 `headers = {"Range": ...}; headers.update(self.headers)`。
/// 行为本身很别扭(等于让调用方能把分块下载搞坏),但它是冻结的契约,
/// 这里用测试把它钉死,免得日后被"顺手优化"掉。
#[test]
fn caller_headers_override_the_range_header() {
    let body = payload(2 * MIB);
    let server = TestServer::start(body);
    let dir = tempfile::tempdir().expect("创建临时目录");
    let target = dir.path().join("out.bin");

    let _ = download_multi(
        &server.url(),
        target.to_str().expect("路径"),
        &options(|o| {
            o.max_retries = 0;
            o.http = HttpConfig {
                headers: headers_from_pairs([("Range", "bytes=0-9")]),
                max_retries: 0,
                ..HttpConfig::default()
            };
        }),
    );

    assert!(
        server.ranges().iter().all(|range| range == "bytes=0-9"),
        "调用方的 Range 应当覆盖掉内部计算的分块区间,实际: {:?}",
        server.ranges()
    );
}

/// 契约 11:`auth` 走 Basic。
#[test]
fn basic_auth_is_sent() {
    let body = payload(64 * 1024);
    let server = TestServer::start(body.clone());
    let dir = tempfile::tempdir().expect("创建临时目录");
    let target = dir.path().join("out.bin");

    assert!(download_single(
        &server.url(),
        target.to_str().expect("路径"),
        &options(|o| {
            o.http = HttpConfig {
                auth: Some(funget_core::Auth {
                    username: "alice".into(),
                    password: "s3cret".into(),
                }),
                ..HttpConfig::default()
            };
        })
    ));

    // base64("alice:s3cret")
    assert_eq!(
        server.last_header("authorization").as_deref(),
        Some("Basic YWxpY2U6czNjcmV0")
    );
}

/// 契约 5:四个公开函数永不抛异常(Rust 侧即永不 panic),一律返回 bool。
#[test]
fn never_panics_on_hostile_input() {
    let dir = tempfile::tempdir().expect("创建临时目录");
    let target = dir.path().join("out.bin");
    let path = target.to_str().expect("路径");

    // 关掉重试:默认的 3 次指数退避会让每个坏 URL 空等约 20 秒,
    // 而这个测试要验的是「不 panic」,不是退避时长(那个由 `gives_up_after_exhausting_retries` 覆盖)。
    let fast = |o: &mut DownloadOptions| {
        o.max_retries = 0;
        o.http = HttpConfig {
            max_retries: 0,
            timeout: std::time::Duration::from_secs(2),
            ..HttpConfig::default()
        };
    };

    // 非法 URL、连不上的地址、各种零值参数
    for url in ["", "not a url", "http://", "http://127.0.0.1:1/x"] {
        assert!(!download_auto(url, path, None, &options(fast)));
        assert!(!download_multi(url, path, &options(fast)));
        assert!(!download_single(url, path, &options(fast)));
    }

    let server = TestServer::start(payload(1024));
    assert!(!download_multi(
        &server.url(),
        path,
        &options(|o| o.worker_num = 0)
    ));
    assert!(!download_multi(
        &server.url(),
        path,
        &options(|o| o.block_size = 0)
    ));
    assert!(!download_single(
        &server.url(),
        path,
        &options(|o| o.chunk_size = 0)
    ));

    // 空文件名 / 指向目录的路径
    assert!(!download_single(
        &server.url(),
        dir.path().to_str().expect("路径"),
        &options(|_| {})
    ));
}

/// 契约 8:写之前先 mkdir -p。
#[test]
fn creates_missing_parent_directories() {
    let body = payload(128 * 1024);
    let server = TestServer::start(body.clone());
    let dir = tempfile::tempdir().expect("创建临时目录");
    let target = dir.path().join("a/b/c/out.bin");

    assert!(download_single(
        &server.url(),
        target.to_str().expect("路径"),
        &options(|_| {})
    ));
    assert_eq!(std::fs::read(&target).expect("读回文件"), body);
}

/// 服务端持续返回 5xx 时要重试、最终返回 false,而不是卡死或 panic。
#[test]
fn gives_up_after_exhausting_retries() {
    let server = TestServer::start(payload(64 * 1024));
    server.set_behavior(Behavior {
        status_override: Some(503),
        ..Default::default()
    });
    let dir = tempfile::tempdir().expect("创建临时目录");
    let target = dir.path().join("out.bin");

    assert!(!download_single(
        &server.url(),
        target.to_str().expect("路径"),
        &options(|o| {
            o.max_retries = 1;
            o.http = HttpConfig {
                max_retries: 1,
                ..HttpConfig::default()
            };
        })
    ));
}

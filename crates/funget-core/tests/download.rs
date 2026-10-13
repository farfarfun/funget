//! 下载路径的行为契约测试。
//!
//! 这些测试锁的是「与 1.1.70 版 Python 实现逐条对齐」的那些行为,而不是
//! 「我觉得应该怎样」。每个测试上方注明它对应哪一条契约。

mod common;

use std::path::Path;

use common::{payload, Behavior, TestServer};
use funget_core::client::headers_from_pairs;
use funget_core::{
    download_auto, download_multi, download_single, DownloadOptions, HttpConfig, TransportOptions,
};

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

// ---------------------------------------------------------------------------
// 重定向。以下四条锁的是 1.1.72 修掉的那个缺陷:并发分块曾经让每个 worker 各自
// 从原始地址重走一遍重定向链。
// ---------------------------------------------------------------------------

/// 起一对服务:`origin` 只回 302,真正的字节在返回的那个 `TestServer` 上。
fn redirecting_pair(body: Vec<u8>) -> (TestServer, TestServer) {
    let origin = TestServer::start(Vec::new());
    let target = TestServer::start(body);
    origin.set_behavior(Behavior {
        redirect_to: Some(target.url()),
        ..Default::default()
    });
    (origin, target)
}

/// 分块请求打到源站的 Range 请求数 —— 把 Range 支持探测那一发排除掉。
///
/// 探测本身就带 `Range: bytes=0-100`(`probe.rs` 里是写死的),它是**解析**重定向的
/// 那一次请求,正是修复之后仍然应该打到源站的;要数的是分块请求有没有漏过来。
fn chunk_ranges(server: &TestServer) -> Vec<String> {
    server
        .ranges()
        .into_iter()
        .filter(|range| range != "bytes=0-100")
        .collect()
}

/// 重定向只在探测时解析一次,之后所有分块都打解析后的地址。
///
/// 这一条是修复前后的分水岭:改 `worker_num` 不能改变源站收到的请求数。修复前
/// 源站会收到 `2 + 分块数` 次请求(每个分块一条独立的重定向链)。
#[test]
fn redirect_is_resolved_once_regardless_of_concurrency() {
    let body = payload(5 * MIB);
    let (origin, target) = redirecting_pair(body.clone());
    let dir = tempfile::tempdir().expect("创建临时目录");

    for worker_num in [1usize, 8] {
        let out = dir.path().join(format!("out-{worker_num}.bin"));
        let before = origin.request_count();

        let ok = download_multi(
            &origin.url(),
            out.to_str().expect("路径"),
            &options(|o| {
                o.worker_num = worker_num;
                o.block_size = 1;
            }),
        );

        assert!(ok, "worker_num={worker_num} 时下载应当成功");
        assert_eq!(
            std::fs::read(&out).expect("读回文件"),
            body,
            "worker_num={worker_num} 时内容应当逐字节一致"
        );
        // 恒为 2:HEAD 探大小 + GET 探 Range。两者都是解析重定向的请求,与并发数无关。
        assert_eq!(
            origin.request_count() - before,
            2,
            "worker_num={worker_num} 时源站请求数应当恒为 2,实际 {}",
            origin.request_count() - before
        );
    }

    // 分块请求一个都不许落在源站上。
    assert!(
        chunk_ranges(&origin).is_empty(),
        "源站不该收到分块请求,实际: {:?}",
        chunk_ranges(&origin)
    );
    // 反面确认:分块确实发生了,只是全打在解析后的地址上。
    assert!(
        chunk_ranges(&target).len() > 2,
        "期望多个分块落在解析后的地址上,实际: {:?}",
        chunk_ranges(&target)
    );
}

/// 续传指纹必须记**原始**地址:重定向目标换了也要认出是同一份文件。
///
/// 这是「指纹不能用解析后的地址」那条注释的回归网。第二次运行时源站指向另一个
/// **路径**(而不是只换端口)—— `safe_url()` 只取 host,换端口在它眼里是同一个
/// 地址,那样这条测试就什么都证明不了了。
#[test]
fn resume_survives_a_changed_redirect_target() {
    let body = payload(4 * MIB);
    let (origin, first) = redirecting_pair(body.clone());
    // 每个分块只发 64 KiB 就断线。
    first.set_behavior(Behavior {
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
        &origin.url(),
        target.to_str().expect("路径"),
        &options(config),
    );
    assert!(!ok, "传输被掐断,这一次应当失败");
    assert!(sidecar(&target).exists(), "失败后应当留下续传状态");

    // 换成一个**完全不同**的后端、而且路径也不同,模拟 CDN 换了边缘节点。
    let second = TestServer::start(body.clone());
    origin.set_behavior(Behavior {
        redirect_to: Some(second.url_at("edge-2/payload.bin")),
        ..Default::default()
    });

    let ok = download_multi(
        &origin.url(),
        target.to_str().expect("路径"),
        &options(config),
    );
    assert!(ok, "换了重定向目标之后应当补齐并成功");
    assert_eq!(std::fs::read(&target).expect("读回文件"), body);
    assert!(!sidecar(&target).exists(), "成功后状态文件应当被删除");

    // 真的命中了续传:每个分块都从断点处继续,没有任何一块从块首重来。
    // 指纹若改用解析后的地址,这里会看到 `bytes=0-...`(第 0 块从头重下)。
    let resumed = chunk_ranges(&second);
    assert!(!resumed.is_empty(), "第二次应当发出分块请求");
    assert!(
        resumed.iter().all(|range| !range.starts_with("bytes=0-")),
        "每个分块都该从断点继续,实际请求: {resumed:?}"
    );
}

/// `follow_redirects = false` 时 302 不跟随,下载失败且不留下残缺文件。
#[test]
fn redirects_can_be_disabled() {
    let (origin, _target) = redirecting_pair(payload(5 * MIB));
    let dir = tempfile::tempdir().expect("创建临时目录");
    let out = dir.path().join("out.bin");

    assert!(!download_multi(
        &origin.url(),
        out.to_str().expect("路径"),
        &options(|o| {
            o.max_retries = 0;
            o.http = HttpConfig {
                max_retries: 0,
                transport: TransportOptions {
                    follow_redirects: false,
                    ..TransportOptions::default()
                },
                ..HttpConfig::default()
            };
        })
    ));
    assert!(!out.exists(), "探测不到大小就不该落地任何文件");
}

/// `max_redirects` 真的接到了 reqwest 上:0 跳上限连一跳都不许。
#[test]
fn redirect_limit_is_enforced() {
    let body = payload(5 * MIB);
    let (origin, _target) = redirecting_pair(body.clone());
    let dir = tempfile::tempdir().expect("创建临时目录");
    let config = |max_redirects: usize| {
        move |o: &mut DownloadOptions| {
            o.block_size = 1;
            o.max_retries = 0;
            o.http = HttpConfig {
                max_retries: 0,
                transport: TransportOptions {
                    max_redirects,
                    ..TransportOptions::default()
                },
                ..HttpConfig::default()
            };
        }
    };

    let blocked = dir.path().join("blocked.bin");
    assert!(!download_multi(
        &origin.url(),
        blocked.to_str().expect("路径"),
        &options(config(0))
    ));

    // 同一对服务、同一份配置,只把上限放到 1 就该成功 —— 失败确实来自上限本身,
    // 而不是这对服务压根连不通。
    let allowed = dir.path().join("allowed.bin");
    assert!(download_multi(
        &origin.url(),
        allowed.to_str().expect("路径"),
        &options(config(1))
    ));
    assert_eq!(std::fs::read(&allowed).expect("读回文件"), body);
}

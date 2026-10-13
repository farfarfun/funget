//! `-k` / `--cacert` 的实打测试。
//!
//! 起一个自签证书的本地 HTTPS 服务,再分别用「什么都不配」「`insecure`」「`ca_certs`」
//! 三种方式去下,看 TLS 握手到底认不认。
//!
//! 这是 `TransportOptions` 那两个 TLS 旋钮唯一的端到端验证。`rustls-tls-native-roots`
//! (读系统证书库)本地没法实打 —— 没有「装了私有 CA 的机器」这种可复现环境,
//! 那一条只能靠依赖树里有 `rustls-native-certs` + reqwest 源码里两个 `#[cfg]` 分支
//! 往同一个 `RootCertStore` 塞证书这两件事来保证。

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use funget_core::{download_single, DownloadOptions, HttpConfig, TransportOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

/// 服务端返回的固定内容。要验的是握手,不是分块逻辑,小一点就够。
const BODY: &[u8] = b"funget tls payload";

struct TlsServer {
    addr: SocketAddr,
    /// 自签证书的 PEM。写到临时文件里就是 `--cacert` 的入参。
    cert_pem: String,
}

impl TlsServer {
    fn start() -> Self {
        // SAN 写成 IP 而不是 `localhost`:rcgen 会把能解析成 IP 的字符串建成
        // IpAddress SAN,客户端连 `https://127.0.0.1:port/` 时 rustls 正好拿 IP 去比对。
        // 用 `localhost` 就得依赖 DNS,而它在不少环境里先解析到 ::1。
        let issued = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_string()])
            .expect("生成自签证书");
        let cert_pem = issued.cert.pem();
        let key =
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(issued.signing_key.serialize_der()));

        // 显式指定 crypto provider,不走 `ServerConfig::builder()` 的进程默认值 ——
        // 哪天依赖树里多出第二个 provider,那个函数会直接 panic。
        let config = ServerConfig::builder_with_provider(Arc::new(
            tokio_rustls::rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("默认 TLS 版本")
        .with_no_client_auth()
        // 不配 ALPN:于是服务端不回 ALPN 扩展,reqwest 退回 HTTP/1.1,
        // 下面那个手写的响应就够用了。
        .with_single_cert(vec![issued.cert.der().clone()], key)
        .expect("装载自签证书");

        // 与 `common::TestServer` 同样的理由:被测的 `download_*` 是同步接口,
        // 内部对全局运行时做 `block_on`,服务端必须待在另一个线程的运行时上。
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("创建测试服务运行时");
            runtime.block_on(async move {
                let listener = TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("监听随机端口");
                tx.send(listener.local_addr().expect("取监听地址"))
                    .expect("回传监听地址");
                let acceptor = TlsAcceptor::from(Arc::new(config));
                while let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(handle(acceptor.clone(), stream));
                }
            });
        });

        Self {
            addr: rx.recv().expect("等待服务就绪"),
            cert_pem,
        }
    }

    fn url(&self) -> String {
        format!("https://{}/payload.bin", self.addr)
    }
}

async fn handle(acceptor: TlsAcceptor, stream: tokio::net::TcpStream) {
    // 握手失败就是这组测试里的「预期失败」路径,安静返回即可。
    let Ok(mut tls) = acceptor.accept(stream).await else {
        return;
    };

    let mut seen = Vec::new();
    let mut buffer = [0u8; 1024];
    loop {
        match tls.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) => seen.extend_from_slice(&buffer[..read]),
        }
        if seen.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: none\r\nConnection: close\r\n\r\n",
        BODY.len()
    );
    if tls.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    // HEAD 只回头不回体 —— `probe_size` 和 `validate_url` 各发一次 HEAD。
    if !seen.starts_with(b"HEAD") {
        let _ = tls.write_all(BODY).await;
    }
    let _ = tls.shutdown().await;
}

/// 下一次,返回是否成功。重试一律关掉:失败路径该立刻失败,不该退避三轮。
fn attempt(server: &TlsServer, target: &Path, mutate: impl FnOnce(&mut TransportOptions)) -> bool {
    let mut transport = TransportOptions::default();
    mutate(&mut transport);

    download_single(
        &server.url(),
        target.to_str().expect("路径"),
        &DownloadOptions {
            progress: false,
            max_retries: 0,
            http: HttpConfig {
                max_retries: 0,
                transport,
                ..HttpConfig::default()
            },
            ..DownloadOptions::default()
        },
    )
}

fn write_pem(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).expect("写入 PEM");
    path
}

/// 自签证书默认就该被拒。这条是另外两条的对照组 —— 没有它,「成功」可能只是因为
/// 校验压根没开。
#[test]
fn self_signed_cert_is_rejected_by_default() {
    let server = TlsServer::start();
    let dir = tempfile::tempdir().expect("创建临时目录");
    let target = dir.path().join("out.bin");

    assert!(!attempt(&server, &target, |_| {}), "默认配置不该信自签证书");
}

/// `insecure`(CLI 的 `-k`)放过一切证书错误。
#[test]
fn insecure_accepts_a_self_signed_cert() {
    let server = TlsServer::start();
    let dir = tempfile::tempdir().expect("创建临时目录");
    let target = dir.path().join("out.bin");

    assert!(attempt(&server, &target, |t| t.insecure = true));
    assert_eq!(std::fs::read(&target).expect("读回文件"), BODY);
}

/// `ca_certs`(CLI 的 `--cacert`)把指定的证书**追加**到信任列表里。
#[test]
fn cacert_trusts_a_self_signed_cert() {
    let server = TlsServer::start();
    let dir = tempfile::tempdir().expect("创建临时目录");
    let pem = write_pem(dir.path(), "ca.pem", &server.cert_pem);
    let target = dir.path().join("out.bin");

    assert!(attempt(&server, &target, |t| t.ca_certs = vec![pem.clone()]));
    assert_eq!(std::fs::read(&target).expect("读回文件"), BODY);
    // 校验确实还在开着:同一个服务,不给证书就连不上。
    assert!(!attempt(&server, &target, |_| {}));
}

/// 一个文件里有多张证书时,后面几张不能被丢掉。
///
/// 这条锁的是 `Certificate::from_pem_bundle()` 而不是 `from_pem()`:企业 CA 发下来的
/// `--cacert` 文件多半是「根 + 中间」一整条链,只收第一张就会白白失败。这里刻意把
/// **无关**的那张放在最前面 —— 用 `from_pem()` 的话这个顺序必然连不上。
#[test]
fn cacert_reads_every_cert_in_a_bundle() {
    let server = TlsServer::start();
    let unrelated = rcgen::generate_simple_self_signed(vec!["example.invalid".to_string()])
        .expect("生成另一张自签证书")
        .cert
        .pem();

    let dir = tempfile::tempdir().expect("创建临时目录");
    let bundle = write_pem(
        dir.path(),
        "bundle.pem",
        &format!("{unrelated}{}", server.cert_pem),
    );
    let target = dir.path().join("out.bin");

    assert!(attempt(&server, &target, |t| t.ca_certs = vec![bundle.clone()]));
    assert_eq!(std::fs::read(&target).expect("读回文件"), BODY);
}

/// `--cacert` 指到一个不存在的文件:要返回 false,不能 panic。
///
/// 这条路径上客户端压根建不起来(`Transport::new` 直接 Err),而契约 5 要求
/// 那四个公开函数永不抛异常。
#[test]
fn a_missing_cacert_file_fails_without_panicking() {
    let server = TlsServer::start();
    let dir = tempfile::tempdir().expect("创建临时目录");
    let target = dir.path().join("out.bin");

    assert!(!attempt(&server, &target, |t| t.ca_certs =
        vec![PathBuf::from("/nonexistent/funget-test-ca.pem")]));
    assert!(!target.exists(), "客户端都没建起来,不该落地任何文件");
}

/// 一个畸形的 PEM 同样只能失败,不能 panic。
#[test]
fn a_malformed_cacert_file_fails_without_panicking() {
    let server = TlsServer::start();
    let dir = tempfile::tempdir().expect("创建临时目录");
    let pem = write_pem(dir.path(), "junk.pem", "not a certificate at all\n");
    let target = dir.path().join("out.bin");

    assert!(!attempt(&server, &target, |t| t.ca_certs = vec![pem.clone()]));
}

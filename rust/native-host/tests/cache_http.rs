//! Real native DNS and HTTP cache lifecycle contract.
use mosdns_native_host::{CacheTestClock, HostAssembly, HostOptions, compile_yaml_with_base};
use mosdns_upstream_core::TransportCancellation;
use std::net::{SocketAddr, TcpListener, UdpSocket};
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
struct Upstream {
    address: SocketAddr,
    count: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Upstream {
    fn new() -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let address = socket.local_addr().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_count = count.clone();
        let worker_stop = stop.clone();
        let worker = std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            while !worker_stop.load(Ordering::SeqCst) {
                if let Ok((size, peer)) = socket.recv_from(&mut buffer) {
                    worker_count.fetch_add(1, Ordering::SeqCst);
                    let mut response = buffer[..size].to_vec();
                    response[2] = 0x81;
                    response[3] = 0x80;
                    response[7] = 1;
                    response.extend_from_slice(&[
                        0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 30, 0, 4, 192, 0, 2, 1,
                    ]);
                    socket.send_to(&response, peer).unwrap();
                }
            }
        });
        Self {
            address,
            count,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Upstream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
async fn http(address: SocketAddr, method: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: native\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.unwrap();
    let split = bytes.windows(4).position(|s| s == b"\r\n\r\n").unwrap();
    let status = std::str::from_utf8(&bytes[..split])
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    (status, bytes[split + 4..].to_vec())
}
async fn dns(address: SocketAddr, id: u16) -> Vec<u8> {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut query = vec![
        0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 4, b'C', b'a', b's', b'e', 7, b'e', b'x', b'a', b'm',
        b'p', b'l', b'e', 0, 0, 1, 0, 1,
    ];
    query[..2].copy_from_slice(&id.to_be_bytes());
    socket.send_to(&query, address).await.unwrap();
    let mut buffer = [0; 4096];
    let (length, _) = tokio::time::timeout(Duration::from_secs(2), socket.recv_from(&mut buffer))
        .await
        .unwrap()
        .unwrap();
    buffer[..length].to_vec()
}
#[test]
#[allow(clippy::too_many_lines)] // One complete DNS/HTTP lifecycle and restart scenario.
fn named_inventory_metrics_management_and_restart_use_real_owners() {
    let upstream = Upstream::new();
    let root = std::env::temp_dir().join(format!("cache-http-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let dns_port = free_port();
    let api_port = free_port();
    let yaml = format!(
        "log: {{level: error}}\napi: {{http: '127.0.0.1:{api_port}'}}\nplugins:\n  - tag: alpha\n    type: cache\n    args: {{lazy_cache_ttl: 90, dump_file: alpha.gz}}\n  - tag: beta\n    type: cache\n    args: {{lazy_cache_ttl: 90}}\n  - tag: f\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://{}'}}]}}\n  - tag: domains\n    type: domain_set\n    args: {{exps: ['example']}}\n  - tag: main\n    type: sequence\n    args: [{{exec: $alpha}}, {{exec: $beta}}, {{exec: 'cache 64'}}, {{exec: $f}}]\n  - tag: dns\n    type: udp_server\n    args: {{listen: '127.0.0.1:{dns_port}', entry: main, enable_audit: true}}\n",
        upstream.address
    );
    let clock = CacheTestClock::new(1000);
    let host = HostAssembly::with_options(
        compile_yaml_with_base(&yaml, &root).unwrap(),
        HostOptions::default().with_cache_clock(Rc::new(clock.clone())),
    )
    .unwrap();
    host.start_audit();
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let address = bound.dns_addr();
        let stop = TransportCancellation::new();
        let scope = stop.clone();
        let serving = tokio::task::spawn_local(async move { bound.serve(scope).await });
        let (status, body) = http(api, "GET", "/api/v1/cache/inventory", &[]).await;
        assert_eq!(status, 200);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({"schema_version":1,"caches":[{"tag":"alpha"},{"tag":"beta"}]})
        );
        let quick = &host
            .config()
            .caches
            .iter()
            .find(|item| item.kind == mosdns_native_host::CacheKind::Quick)
            .unwrap()
            .tag;
        for action in ["show", "save", "dump", "load_dump", "flush"] {
            assert_eq!(
                http(api, "POST", &format!("/plugins/{quick}/{action}"), &[])
                    .await
                    .0,
                404
            );
        }
        dns(address, 1).await;
        dns(address, 2).await;
        clock.advance(31);
        dns(address, 3).await;
        for _ in 0..1000 {
            if upstream.count.load(Ordering::SeqCst) == 2
                && host
                    .cache()
                    .get(mosdns_native_host::CacheId(0))
                    .unwrap()
                    .pending_refreshes()
                    == 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert_eq!(upstream.count.load(Ordering::SeqCst), 2);
        assert_eq!(host.audit_snapshot().records.len(), 3);
        let (status, metrics) = http(api, "GET", "/metrics", &[]).await;
        assert_eq!(status, 200);
        let metrics = String::from_utf8(metrics).unwrap();
        for line in [
            "mosdns_cache_query_total{tag=\"alpha\"} 3",
            "mosdns_cache_hit_total{tag=\"alpha\"} 2",
            "mosdns_cache_lazy_hit_total{tag=\"alpha\"} 1",
            "mosdns_cache_size_current{tag=\"alpha\"} 1",
            "mosdns_cache_query_total{tag=\"beta\"} 1",
        ] {
            assert!(
                metrics.lines().any(|actual| actual == line),
                "missing {line}: {metrics}"
            );
        }
        assert!(!metrics.contains("__quick"));
        assert_eq!(
            metrics
                .lines()
                .filter(|line| !line.starts_with('#') && !line.is_empty())
                .count(),
            8
        );
        let (status, show) = http(
            api,
            "GET",
            "/plugins/alpha/show?q=CASE&offset=-1&limit=0",
            &[],
        )
        .await;
        assert_eq!(status, 200);
        let show = String::from_utf8(show).unwrap();
        for text in [
            "----- Cache Entry -----",
            "Key:",
            "StoredTime:",
            "MsgExpire:",
            "CacheExpire:",
            "DNS Message:",
            ";; QUESTION SECTION:",
            ";; ANSWER SECTION:",
            "192.0.2.1",
        ] {
            assert!(show.contains(text), "missing {text}: {show}");
        }
        assert!(
            http(api, "GET", "/plugins/alpha/show?offset=1", &[])
                .await
                .1
                .is_empty()
        );
        assert!(
            !http(api, "GET", "/plugins/alpha/show?q=192.0.2.1", &[])
                .await
                .1
                .is_empty()
        );
        for (method, path, status) in [
            ("POST", "/api/v1/cache/inventory", 405),
            ("POST", "/metrics", 405),
            ("POST", "/plugins/alpha/flush", 405),
            ("GET", "/plugins/alpha/load_dump", 405),
            ("POST", "/plugins/missing/flush", 404),
            ("POST", "/plugins/domains/flush", 404),
            ("GET", "/plugins/alpha/post", 404),
            ("GET", "/plugins/f/show", 404),
            ("POST", "/plugins/alpha/unknown", 404),
            ("GET", "/plugins/beta/save", 400),
        ] {
            assert_eq!(
                http(api, method, path, &[]).await.0,
                status,
                "{method} {path}"
            );
        }
        host.cache()
            .get(mosdns_native_host::CacheId(0))
            .unwrap()
            .inject_persist_fault(mosdns_native_host::PersistFault::Rename);
        assert_eq!(http(api, "GET", "/plugins/alpha/save", &[]).await.0, 500);
        assert!(
            !http(api, "GET", "/plugins/alpha/show", &[])
                .await
                .1
                .is_empty()
        );
        assert_eq!(http(api, "GET", "/plugins/alpha/save", &[]).await.0, 200);
        let (status, dump) = http(api, "GET", "/plugins/alpha/dump", &[]).await;
        assert_eq!(status, 200);
        assert_eq!(dump[..2], [31, 139]);
        assert_eq!(http(api, "GET", "/plugins/alpha/flush", &[]).await.0, 200);
        assert!(
            http(api, "GET", "/plugins/alpha/show", &[])
                .await
                .1
                .is_empty()
        );
        assert_eq!(
            http(api, "POST", "/plugins/alpha/load_dump", &dump).await.0,
            200
        );
        assert!(
            !http(api, "GET", "/plugins/alpha/show", &[])
                .await
                .1
                .is_empty()
        );
        assert_eq!(
            http(api, "POST", "/plugins/alpha/load_dump", b"bad")
                .await
                .0,
            400
        );
        assert!(
            !http(api, "GET", "/plugins/alpha/show", &[])
                .await
                .1
                .is_empty()
        );
        let larger = vec![0; 1024 * 1024 + 1];
        let (status, body) = http(api, "POST", "/plugins/alpha/load_dump", &larger).await;
        assert_eq!(status, 400);
        assert!(
            String::from_utf8(body).unwrap().contains("cache dump"),
            "route-specific cap permits body above 1MiB to reach codec"
        );
        stop.cancel();
        serving.await.unwrap().unwrap();
    });
    let restarted = HostAssembly::with_options(
        compile_yaml_with_base(&yaml, &root).unwrap(),
        HostOptions::default().with_cache_clock(Rc::new(clock.clone())),
    )
    .unwrap();
    restarted.block_on(async {
        let bound = restarted.bind_host().await.unwrap();
        let address = bound.dns_addr();
        let stop = TransportCancellation::new();
        let scope = stop.clone();
        let serving = tokio::task::spawn_local(async move { bound.serve(scope).await });
        dns(address, 4).await;
        assert_eq!(upstream.count.load(Ordering::SeqCst), 2);
        clock.advance(90);
        let api = restarted.config().api.as_ref().unwrap().http;
        assert!(
            http(api, "GET", "/plugins/alpha/show", &[])
                .await
                .1
                .is_empty()
        );
        let (_, metrics) = http(api, "GET", "/metrics", &[]).await;
        assert!(
            String::from_utf8(metrics)
                .unwrap()
                .contains("mosdns_cache_size_current{tag=\"alpha\"} 0")
        );
        stop.cancel();
        serving.await.unwrap().unwrap();
    });
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn native_process_sigterm_drains_owners_and_reports_all_final_save_failures_with_exit_two() {
    use std::process::{Command, Stdio};
    let root = std::env::temp_dir().join(format!("cache-signal-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let api_port = free_port();
    let dns_port = free_port();
    let yaml = format!(
        "log: {{level: error}}\napi: {{http: '127.0.0.1:{api_port}'}}\nplugins:\n  - tag: alpha\n    type: cache\n    args: {{dump_file: missing/alpha.gz}}\n  - tag: beta\n    type: cache\n    args: {{dump_file: missing/beta.gz}}\n  - tag: f\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://127.0.0.1:25999'}}]}}\n  - tag: main\n    type: sequence\n    args: [{{exec: $alpha}}, {{exec: $beta}}, {{exec: $f}}]\n  - tag: dns\n    type: udp_server\n    args: {{listen: '127.0.0.1:{dns_port}', entry: main, enable_audit: false}}\n"
    );
    let config = root.join("config.yaml");
    std::fs::write(&config, yaml).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_mosdns"))
        .args(["start", "-c"])
        .arg(&config)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut ready = false;
    for _ in 0..200 {
        if std::net::TcpStream::connect(("127.0.0.1", api_port)).is_ok() {
            ready = true;
            break;
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if !ready {
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        panic!(
            "native startup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(
        Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    for _ in 0..400 {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
    }
    let output = child.wait_with_output().unwrap();
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(2), "{diagnostic}");
    assert!(diagnostic.contains("cache 0:"));
    assert!(diagnostic.contains("cache 1:"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn inventory_tags_roundtrip_through_encoded_plugin_urls_and_prometheus_labels() {
    let tag = "odd\"\\+/中";
    let quoted = serde_json::to_string(tag).unwrap();
    let api_port = free_port();
    let dns_port = free_port();
    let yaml = format!(
        "log: {{level: error}}\napi: {{http: '127.0.0.1:{api_port}'}}\nplugins:\n  - tag: {quoted}\n    type: cache\n    args: {{}}\n  - tag: f\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://127.0.0.1:25999'}}]}}\n  - tag: main\n    type: sequence\n    args: [{{exec: $f}}]\n  - tag: dns\n    type: udp_server\n    args: {{listen: '127.0.0.1:{dns_port}', entry: main, enable_audit: false}}\n"
    );
    let host = HostAssembly::from_yaml(&yaml).unwrap();
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let stop = TransportCancellation::new();
        let scope = stop.clone();
        let serving = tokio::task::spawn_local(async move { bound.serve(scope).await });
        let (status, inventory) = http(api, "GET", "/api/v1/cache/inventory", &[]).await;
        assert_eq!(status, 200);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&inventory).unwrap()["caches"][0]["tag"],
            tag
        );
        assert_eq!(
            http(api, "GET", "/plugins/odd%22%5C%2B%2F%E4%B8%AD/show", &[])
                .await
                .0,
            200
        );
        assert_eq!(
            http(api, "POST", "/plugins/odd%22%5C%2B%2F%E4%B8%AD/show", &[])
                .await
                .0,
            405
        );
        for path in [
            "/plugins/odd%/show",
            "/plugins/odd%ZZ/show",
            "/plugins/odd%FF/show",
        ] {
            assert_eq!(http(api, "POST", path, &[]).await.0, 404);
        }
        let (_, metrics) = http(api, "GET", "/metrics", &[]).await;
        assert!(
            String::from_utf8(metrics)
                .unwrap()
                .lines()
                .any(|line| line == format!("mosdns_cache_query_total{{tag={quoted}}} 0"))
        );
        stop.cancel();
        serving.await.unwrap().unwrap();
    });
}

#[test]
fn ecs_dump_http_rejects_invalid_last_entry_without_partial_merge() {
    use std::io::{Read, Write};
    let upstream = Upstream::new();
    let up_addr = upstream.address;
    let dns_port = free_port();
    let api_port = free_port();
    let yaml = format!(
        "log: {{level: error}}\napi: {{http: '127.0.0.1:{api_port}'}}\nplugins:\n  - tag: alpha\n    type: cache\n    args: {{enable_ecs: true}}\n  - tag: beta\n    type: cache\n    args: {{}}\n  - tag: up\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://{up_addr}'}}]}}\n  - tag: main\n    type: sequence\n    args: [{{exec: $alpha}}, {{exec: reject 0}}]\n  - tag: dns\n    type: udp_server\n    args: {{listen: '127.0.0.1:{dns_port}', entry: main, enable_audit: false}}\n"
    );
    let clock = CacheTestClock::new(100);
    clock.set_wall(2_000_000_000);
    let host = HostAssembly::with_options(
        compile_yaml_with_base(&yaml, &std::env::temp_dir()).unwrap(),
        HostOptions::default().with_cache_clock(Rc::new(clock)),
    )
    .unwrap();
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let stop = TransportCancellation::new();
        let scope = stop.clone();
        let serving = tokio::task::spawn_local(async move { bound.serve(scope).await });
        let fixture = include_bytes!("fixtures/cache-go-ecs-v2.gz");
        assert_eq!(
            http(api, "POST", "/plugins/beta/load_dump", fixture)
                .await
                .0,
            400
        );
        assert!(
            http(api, "GET", "/plugins/beta/show", &[])
                .await
                .1
                .is_empty()
        );
        assert_eq!(
            http(api, "POST", "/plugins/alpha/load_dump", fixture)
                .await
                .0,
            200
        );
        let before = http(api, "GET", "/plugins/alpha/show", &[]).await.1;
        let text = String::from_utf8(before.clone()).unwrap();
        assert!(text.contains("a. A IN [ecs:192.0.2.0/24/0]"), "{text}");
        let mut raw = Vec::new();
        flate2::read::GzDecoder::new(&fixture[..])
            .read_to_end(&mut raw)
            .unwrap();
        let suffix = b"192.0.2.199/120/0";
        let at = raw
            .windows(suffix.len())
            .rposition(|s| s == suffix)
            .unwrap();
        raw[at + suffix.len() - 1] = b'1';
        let mut writer = flate2::GzBuilder::new()
            .filename("mosdns_cache_v2")
            .write(Vec::new(), flate2::Compression::default());
        writer.write_all(&raw).unwrap();
        let invalid = writer.finish().unwrap();
        assert_eq!(
            http(api, "POST", "/plugins/alpha/load_dump", &invalid)
                .await
                .0,
            400
        );
        assert_eq!(http(api, "GET", "/plugins/alpha/show", &[]).await.1, before);
        stop.cancel();
        serving.await.unwrap().unwrap();
    });
}

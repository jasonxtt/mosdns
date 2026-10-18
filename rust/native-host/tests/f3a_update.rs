//! F-3a online update, against the real `mosdns` binary and a local release server (no
//! public network): status/check with the cache, the refusal paths (sha256, manifest,
//! package, pre-flight), a full guarded update (same process id, new files, confirmed),
//! automatic rollback when the candidate fails, start-up recovery after the guard is
//! killed, configuration update from a URL, and the configuration export.
#![cfg(unix)]
#![allow(clippy::pedantic)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn sha256(data: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, data)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn wait_until(what: &str, seconds: u64, mut condition: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < end {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    panic!("timed out: {what}");
}

// ----- DNS stub and client -------------------------------------------------------------

fn dns_query(name: &str) -> Vec<u8> {
    let mut wire = vec![0x42, 0x42, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.split('.') {
        wire.push(label.len() as u8);
        wire.extend_from_slice(label.as_bytes());
    }
    wire.extend_from_slice(&[0, 0, 1, 0, 1]);
    wire
}

fn dns_answer(query: &[u8]) -> Vec<u8> {
    let mut wire = query.to_vec();
    wire[2] = 0x81;
    wire[3] = 0x80;
    wire[7] = 1;
    wire.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 192, 0, 2, 1]);
    wire
}

struct Stub {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Stub {
    fn start(port: u16) -> Self {
        let socket = UdpSocket::bind(("127.0.0.1", port)).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::spawn(move || {
            let mut buffer = [0_u8; 512];
            while !flag.load(Ordering::SeqCst) {
                if let Ok((length, peer)) = socket.recv_from(&mut buffer) {
                    let _ = socket.send_to(&dns_answer(&buffer[..length]), peer);
                }
            }
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn ask(dns: u16, name: &str) -> bool {
    let Ok(socket) = UdpSocket::bind("127.0.0.1:0") else {
        return false;
    };
    let _ = socket.set_read_timeout(Some(Duration::from_millis(600)));
    if socket
        .send_to(&dns_query(name), ("127.0.0.1", dns))
        .is_err()
    {
        return false;
    }
    let mut buffer = [0_u8; 512];
    socket
        .recv_from(&mut buffer)
        .is_ok_and(|(length, _)| length > 12 && buffer[7] == 1)
}

// ----- HTTP client ---------------------------------------------------------------------

struct Reply {
    status: u16,
    headers: String,
    body: Vec<u8>,
}

impl Reply {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|_| panic!("JSON: {:?}", String::from_utf8_lossy(&self.body)))
    }
}

fn http(port: u16, method: &str, path: &str, body: &[u8]) -> Option<Reply> {
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(500),
    )
    .ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .ok()?;
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).ok()?;
    stream.write_all(body).ok()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).ok()?;
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&response[..split]).ok()?.to_owned();
    let status = head.split(' ').nth(1)?.parse().ok()?;
    Some(Reply {
        status,
        headers: head,
        body: response[split + 4..].to_vec(),
    })
}

// ----- local release server ------------------------------------------------------------

#[derive(Clone)]
struct Route {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

struct ReleaseServer {
    port: u16,
    routes: Arc<Mutex<HashMap<String, Route>>>,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ReleaseServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes: Arc<Mutex<HashMap<String, Route>>> = Arc::default();
        let requests: Arc<Mutex<Vec<String>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (r, q, s) = (routes.clone(), requests.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            while !s.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let mut reader = BufReader::new(stream.try_clone().unwrap());
                        let mut line = String::new();
                        if reader.read_line(&mut line).is_err() {
                            continue;
                        }
                        let path = line.split(' ').nth(1).unwrap_or("/").to_owned();
                        let mut header = String::new();
                        while reader
                            .read_line(&mut header)
                            .map(|n| n > 2)
                            .unwrap_or(false)
                        {
                            header.clear();
                        }
                        q.lock().unwrap().push(path.clone());
                        let route = r.lock().unwrap().get(&path).cloned().unwrap_or(Route {
                            status: 404,
                            content_type: "text/plain",
                            body: b"not found".to_vec(),
                        });
                        let _ = write!(
                            stream,
                            "HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            route.status,
                            route.content_type,
                            route.body.len()
                        );
                        let _ = stream.write_all(&route.body);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        });
        Self {
            port,
            routes,
            requests,
            stop,
            handle: Some(handle),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn set(&self, path: &str, content_type: &'static str, body: Vec<u8>) {
        self.routes.lock().unwrap().insert(
            path.to_owned(),
            Route {
                status: 200,
                content_type,
                body,
            },
        );
    }

    fn count(&self, path: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.as_str() == path)
            .count()
    }

    /// One release entry of the listing.
    fn release(
        &self,
        tag: &str,
        draft: bool,
        prerelease: bool,
        with: &[&str],
    ) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag,
            "published_at": "2026-10-18T00:00:00Z",
            "draft": draft,
            "prerelease": prerelease,
            "assets": with.iter().map(|name| serde_json::json!({
                "name": name,
                "browser_download_url": self.url(&format!("/dl/{tag}/{name}")),
                "updated_at": "2026-10-18T00:00:00Z",
            })).collect::<Vec<_>>(),
        })
    }

    fn list(&self, releases: Vec<serde_json::Value>) {
        self.set(
            "/releases",
            "application/json",
            serde_json::to_vec(&releases).unwrap(),
        );
    }
}

impl Drop for ReleaseServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

// ----- archives ------------------------------------------------------------------------

fn tar_gz(name: &str, data: &[u8]) -> Vec<u8> {
    use flate2::{Compression, write::GzEncoder};
    let mut header = [0_u8; 512];
    header[..name.len()].copy_from_slice(name.as_bytes());
    header[100..107].copy_from_slice(b"0000755");
    header[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
    header[156] = b'0';
    header[257..262].copy_from_slice(b"ustar");
    let mut tar = header.to_vec();
    tar.extend_from_slice(data);
    tar.resize(tar.len().div_ceil(512) * 512, 0);
    tar.extend_from_slice(&[0_u8; 1024]);
    let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
    gz.write_all(&tar).unwrap();
    gz.finish().unwrap()
}

/// A stored zip (enough for the host's reader).
fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = flate2::Crc::new();
        crc.update(data);
        crc.sum()
    }
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = out.len() as u32;
        let crc = crc32(data);
        out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central.extend_from_slice(&[0x1e, 3, 20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 8]);
        central.extend_from_slice(&((0o100_644_u32) << 16).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

/// A configuration package for `config_custom.yaml` (schema 1, `rust` channel).
fn config_package(content: &str, edit: impl Fn(&mut serde_json::Value)) -> Vec<u8> {
    let mut manifest = serde_json::json!({
        "format": 1, "channel": "rust", "package_id": "rust-config-schema-1", "config_schema": 1,
        "managed_files": ["config_custom.yaml"],
        "sha256": {"config_custom.yaml": sha256(content.as_bytes())},
    });
    edit(&mut manifest);
    zip(&[
        ("manifest.json", manifest.to_string().as_bytes()),
        ("config_custom.yaml", content.as_bytes()),
    ])
}

fn manifest_json(tag: &str, archive: &[u8], config: Option<(&str, &[u8])>) -> Vec<u8> {
    let mut manifest = serde_json::json!({
        "format": 1, "channel": "rust", "version": tag,
        "required_config_schema": if config.is_some() { 1 } else { 0 },
        "config_package_id": "rust-config-schema-1",
        "artifacts": {"mosdns-rust-linux-amd64.tar.gz": {"sha256": sha256(archive)}},
    });
    if let Some((url, bytes)) = config {
        manifest["config"] = serde_json::json!({"url": url, "sha256": sha256(bytes)});
    }
    serde_json::to_vec(&manifest).unwrap()
}

// ----- the host under test -------------------------------------------------------------

/// The program under test, stripped when `strip` is available (a debug build is hundreds
/// of megabytes, far above what an update is allowed to download).
fn binary() -> &'static Vec<u8> {
    static BINARY: OnceLock<Vec<u8>> = OnceLock::new();
    BINARY.get_or_init(|| {
        let stripped = std::env::temp_dir().join(format!("f3a-stripped-{}", std::process::id()));
        let ok = Command::new("strip")
            .arg("--strip-all")
            .arg("-o")
            .arg(&stripped)
            .arg(env!("CARGO_BIN_EXE_mosdns"))
            .status()
            .is_ok_and(|status| status.success());
        let bytes = std::fs::read(if ok {
            stripped.as_path()
        } else {
            Path::new(env!("CARGO_BIN_EXE_mosdns"))
        })
        .unwrap();
        let _ = std::fs::remove_file(&stripped);
        bytes
    })
}

fn current_version() -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_mosdns"))
        .arg("version")
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn config_text(api: u16, dns: u16, upstream: u16, marker: &str) -> String {
    format!(
        "# {marker}\nlog: {{level: info}}\napi: {{http: '127.0.0.1:{api}'}}\nplugins:\n  - tag: cache_main\n    type: cache\n    args: {{}}\n  - tag: upstream\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://127.0.0.1:{upstream}'}}]}}\n  - tag: main\n    type: sequence\n    args: [{{exec: $cache_main}}, {{exec: $upstream}}]\n  - tag: dns\n    type: udp_server\n    args: {{listen: '127.0.0.1:{dns}', entry: main}}\n"
    )
}

struct Host {
    root: PathBuf,
    exe: PathBuf,
    child: Child,
    stderr: Receiver<String>,
    api: u16,
    dns: u16,
    upstream: u16,
    server: ReleaseServer,
    _stub: Stub,
}

impl Host {
    fn start(name: &str, health_timeout: Option<u64>) -> Self {
        let root = std::env::temp_dir().join(format!("f3a-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let exe = root.join("mosdns");
        std::fs::write(&exe, binary()).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (api, dns, upstream) = (free_port(), free_udp_port(), free_udp_port());
        std::fs::write(
            root.join("config_custom.yaml"),
            config_text(api, dns, upstream, "original"),
        )
        .unwrap();
        let stub = Stub::start(upstream);
        let server = ReleaseServer::start();
        let mut host = Self {
            root,
            exe,
            child: Self::spawn_process_placeholder(),
            stderr: channel().1,
            api,
            dns,
            upstream,
            server,
            _stub: stub,
        };
        host.launch(health_timeout);
        host
    }

    fn spawn_process_placeholder() -> Child {
        Command::new("sleep").arg("0").spawn().unwrap()
    }

    fn launch(&mut self, health_timeout: Option<u64>) {
        let _ = self.child.wait();
        let mut command = Command::new(&self.exe);
        command
            .args(["start", "-d"])
            .arg(&self.root)
            .arg("-c")
            .arg(self.root.join("config_custom.yaml"))
            .env("MOSDNS_UPDATE_API_URL", self.server.url("/releases"))
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(seconds) = health_timeout {
            command.env("MOSDNS_UPDATE_HEALTH_TIMEOUT_SECS", seconds.to_string());
        }
        let mut attempt = 0;
        let mut child = loop {
            match command.spawn() {
                Ok(child) => break child,
                Err(error) if error.raw_os_error() == Some(26) && attempt < 40 => {
                    attempt += 1;
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) => panic!("spawn failed: {error}"),
            }
        };
        let (sender, receiver) = channel();
        let pipe = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        self.child = child;
        self.stderr = receiver;
        self.wait_ready();
    }

    fn wait_ready(&self) {
        wait_until("host ready", 30, || {
            http(self.api, "GET", "/api/v1/system/health", b"")
                .is_some_and(|reply| reply.status == 200)
        });
    }

    fn api(&self, method: &str, path: &str, body: &[u8]) -> Reply {
        http(self.api, method, path, body).unwrap_or_else(|| panic!("{method} {path}: no answer"))
    }

    fn health(&self) -> Option<serde_json::Value> {
        http(self.api, "GET", "/api/v1/system/health", b"")
            .filter(|reply| reply.status == 200)
            .map(|reply| reply.json())
    }

    fn config(&self) -> String {
        std::fs::read_to_string(self.root.join("config_custom.yaml")).unwrap()
    }

    fn transactions(&self) -> Vec<(PathBuf, serde_json::Value)> {
        let mut found = Vec::new();
        if let Ok(entries) = std::fs::read_dir(self.root.join("update")) {
            for entry in entries.flatten() {
                if let Ok(bytes) = std::fs::read(entry.path().join("transaction.json")) {
                    found.push((entry.path(), serde_json::from_slice(&bytes).unwrap()));
                }
            }
        }
        found
    }

    fn only_transaction(&self) -> serde_json::Value {
        let mut all = self.transactions();
        assert_eq!(all.len(), 1, "{all:?}");
        all.remove(0).1
    }

    fn exe_bytes(&self) -> Vec<u8> {
        std::fs::read(&self.exe).unwrap()
    }

    /// Publishes one release `tag` with the given executable (as a tar.gz) and manifest.
    fn publish(&self, tag: &str, program: &[u8], config: Option<&[u8]>) {
        let archive = tar_gz("mosdns", program);
        let config_url = self.server.url(&format!("/dl/{tag}/config_rust_up.zip"));
        let manifest = manifest_json(
            tag,
            &archive,
            config.map(|bytes| (config_url.as_str(), bytes)),
        );
        self.server.set(
            &format!("/dl/{tag}/mosdns-rust-linux-amd64.tar.gz"),
            "application/gzip",
            archive,
        );
        self.server.set(
            &format!("/dl/{tag}/mosdns-rust-update-manifest.json"),
            "application/json",
            manifest,
        );
        if let Some(bytes) = config {
            self.server.set(
                &format!("/dl/{tag}/config_rust_up.zip"),
                "application/zip",
                bytes.to_vec(),
            );
        }
        self.server.list(vec![self.server.release(
            tag,
            false,
            false,
            &[
                "mosdns-rust-linux-amd64.tar.gz",
                "mosdns-rust-update-manifest.json",
            ],
        )]);
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // A script candidate that outlived its guard.
        let _ = Command::new("pkill")
            .args(["-f", &self.root.display().to_string()])
            .status();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn start_ticks(pid: u32) -> String {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    stat.rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap()
        .to_owned()
}

// ===== status and check ==================================================================

#[test]
fn status_and_check_use_the_cache_ignore_go_releases_and_degrade_on_failure() {
    let host = Host::start("status", None);
    let version = current_version();
    host.server.list(vec![
        host.server
            .release("v99.0.0", false, false, &["mosdns-rust-linux-amd64.tar.gz"]),
        host.server.release("lite-v99.0.0", false, false, &[]),
        host.server.release(
            "rust-v99.0.0",
            false,
            false,
            &["mosdns-rust-linux-amd64.tar.gz"],
        ),
        host.server.release("rust-v99.5.0", true, false, &[]),
        host.server.release("rust-v99.6.0", false, true, &[]),
        host.server.release("rust-v98.0.0", false, false, &[]),
    ]);
    let status = host.api("GET", "/api/v1/update/status", b"");
    assert_eq!(status.status, 200);
    let status = status.json();
    assert_eq!(status["current_version"], version);
    assert_eq!(status["latest_version"], "rust-v99.0.0");
    assert_eq!(status["update_available"], true);
    assert_eq!(status["cached"], false);
    assert_eq!(status["architecture"], "linux/amd64");
    assert_eq!(status["asset_name"], "mosdns-rust-linux-amd64.tar.gz");
    assert!(
        status["download_url"]
            .as_str()
            .unwrap()
            .ends_with("/rust-v99.0.0/mosdns-rust-linux-amd64.tar.gz")
    );
    assert_eq!(status["config_schema_required"], 1);
    assert!(status.get("pending_restart").is_none() && status.get("rollback_performed").is_none());
    assert!(
        status["release_url"]
            .as_str()
            .unwrap()
            .ends_with("/releases/tag/rust-v99.0.0")
    );
    // A second read is served from the cache; a forced check goes to the server again.
    assert_eq!(host.server.count("/releases"), 1);
    let again = host.api("GET", "/api/v1/update/status", b"").json();
    assert_eq!(again["cached"], true);
    assert_eq!(host.server.count("/releases"), 1);
    let forced = host.api("POST", "/api/v1/update/check", b"{}").json();
    assert_eq!(forced["cached"], false);
    assert_eq!(host.server.count("/releases"), 2);
    // Capabilities and health reflect the feature.
    let capabilities = host.api("GET", "/api/v1/capabilities", b"").json();
    for id in ["system.update", "system.config_management"] {
        assert_eq!(capabilities["ui_operations"][id]["supported"], true, "{id}");
    }
    let health = host.health().unwrap();
    assert_eq!(health["version"], version);
    assert_eq!(health["required_config_schema"], 1);
    assert_eq!(health["config_management_enabled"], true);
    assert_eq!(health["update"]["state"], "idle");
    // The server fails: a forced check still answers 200 with the reason.
    host.server.routes.lock().unwrap().remove("/releases");
    let failed = host.api("POST", "/api/v1/update/check", b"{}");
    assert_eq!(failed.status, 200);
    assert!(
        failed.json()["message"]
            .as_str()
            .unwrap()
            .starts_with("检查更新失败")
    );
    // Only Go releases: nothing of ours.
    host.server
        .list(vec![host.server.release("v1.2.3", false, false, &[])]);
    let none = host.api("POST", "/api/v1/update/check", b"{}").json();
    assert!(
        none["message"].as_str().unwrap().contains("rust-v*"),
        "{none}"
    );
    // Methods and bodies.
    assert_eq!(host.api("POST", "/api/v1/update/status", b"").status, 405);
    assert_eq!(host.api("GET", "/api/v1/update/apply", b"").status, 405);
    assert_eq!(
        host.api("POST", "/api/v1/update/apply", b"{not json")
            .status,
        400
    );
}

// ===== refusals ==========================================================================

#[test]
fn an_update_is_refused_without_touching_anything_when_a_check_fails() {
    let host = Host::start("refuse", None);
    let real = binary().clone();
    let apply = |host: &Host| host.api("POST", "/api/v1/update/apply", br#"{"force": true}"#);
    let nothing_changed = |host: &Host| {
        assert!(host.transactions().is_empty(), "no transaction");
        assert_eq!(host.exe_bytes(), *binary());
        assert_eq!(
            host.config(),
            config_text(host.api, host.dns, host.upstream, "original")
        );
        assert!(ask(host.dns, "still.example.com"), "DNS keeps answering");
    };

    // The archive does not match the manifest's sha256.
    host.publish("rust-v99.0.0", &real, None);
    let wrong = tar_gz("mosdns", b"tampered");
    host.server.set(
        "/dl/rust-v99.0.0/mosdns-rust-linux-amd64.tar.gz",
        "application/gzip",
        wrong,
    );
    let reply = apply(&host);
    assert_eq!(reply.status, 502);
    assert!(
        reply.json()["error"].as_str().unwrap().contains("校验失败"),
        "{}",
        reply.text()
    );
    nothing_changed(&host);

    // No manifest asset in the release.
    host.publish("rust-v99.0.0", &real, None);
    host.server.list(vec![host.server.release(
        "rust-v99.0.0",
        false,
        false,
        &["mosdns-rust-linux-amd64.tar.gz"],
    )]);
    let reply = apply(&host);
    assert_eq!(reply.status, 502);
    assert!(
        reply.json()["error"]
            .as_str()
            .unwrap()
            .contains("拒绝执行非事务更新"),
        "{}",
        reply.text()
    );
    nothing_changed(&host);

    // The manifest names another version.
    host.publish("rust-v99.0.0", &real, None);
    host.server.set(
        "/dl/rust-v99.0.0/mosdns-rust-update-manifest.json",
        "application/json",
        manifest_json("rust-v99.0.1", &tar_gz("mosdns", &real), None),
    );
    let reply = apply(&host);
    assert_eq!(reply.status, 502);
    assert!(
        reply.json()["error"].as_str().unwrap().contains("不一致"),
        "{}",
        reply.text()
    );
    nothing_changed(&host);

    // The archive has no `mosdns` file.
    let no_binary = tar_gz("README", b"text");
    host.publish("rust-v99.0.0", &real, None);
    host.server.set(
        "/dl/rust-v99.0.0/mosdns-rust-linux-amd64.tar.gz",
        "application/gzip",
        no_binary.clone(),
    );
    host.server.set(
        "/dl/rust-v99.0.0/mosdns-rust-update-manifest.json",
        "application/json",
        manifest_json("rust-v99.0.0", &no_binary, None),
    );
    let reply = apply(&host);
    assert_eq!(reply.status, 502);
    assert!(
        reply.json()["error"].as_str().unwrap().contains("解压失败"),
        "{}",
        reply.text()
    );
    nothing_changed(&host);

    // The candidate reports another version than the release tag.
    host.publish("rust-v99.0.0", &real, None);
    let reply = apply(&host);
    assert_eq!(reply.status, 502);
    assert!(
        reply.json()["error"].as_str().unwrap().contains("自报版本"),
        "{}",
        reply.text()
    );
    nothing_changed(&host);

    // Configuration package problems: wrong hash, wrong schema, and a config the new program
    // rejects (caught by the pre-flight `check`, before the service is touched).
    let version = current_version();
    let good_config = config_text(host.api, host.dns, host.upstream, "new");
    for (what, package, expect) in [
        (
            "hash",
            config_package(&good_config, |m| {
                m["sha256"]["config_custom.yaml"] = "0".repeat(64).into()
            }),
            "不适用",
        ),
        (
            "schema",
            config_package(&good_config, |m| m["config_schema"] = 7.into()),
            "不适用",
        ),
        (
            "rejected config",
            config_package("plugins:\n  - tag: x\n    type: no_such_plugin\n", |_| {}),
            "检查未通过",
        ),
    ] {
        host.publish(&version, &real, Some(&package));
        let reply = apply(&host);
        assert_eq!(reply.status, 502, "{what}: {}", reply.text());
        assert!(
            reply.json()["error"].as_str().unwrap().contains(expect),
            "{what}: {}",
            reply.text()
        );
        nothing_changed(&host);
    }
    // A package whose own sha256 differs from the manifest's.
    let package = config_package(&good_config, |_| {});
    host.publish(&version, &real, Some(&package));
    let other = config_package(&good_config, |m| m["config_schema"] = 1.into());
    host.server.set(
        &format!("/dl/{version}/config_rust_up.zip"),
        "application/zip",
        [other, vec![0]].concat(),
    );
    let reply = apply(&host);
    assert_eq!(reply.status, 502);
    assert!(
        reply.json()["error"].as_str().unwrap().contains("校验失败"),
        "{}",
        reply.text()
    );
    nothing_changed(&host);

    // Nothing newer and no force: a plain answer, nothing staged.
    host.publish(&version, &real, None);
    let reply = host.api("POST", "/api/v1/update/apply", b"");
    assert_eq!(reply.status, 200);
    let body = reply.json();
    assert_eq!(
        (body["installed"].clone(), body["restart_required"].clone()),
        (false.into(), false.into())
    );
    nothing_changed(&host);
}

// ===== a full guarded update =============================================================

fn settle_after_guard(host: &Host) {
    // The service is the same process throughout; wait for the final program to be ready.
    wait_until("the guard finished and the service is back", 60, || {
        host.health()
            .is_some_and(|health| health["update"]["state"] == "idle")
            && host
                .transactions()
                .iter()
                .all(|(_, tx)| tx["status"] != "testing" && tx["status"] != "installing")
    });
}

#[test]
fn a_full_update_replaces_program_and_config_in_the_same_process_and_is_confirmed() {
    let mut host = Host::start("full", None);
    let pid = host.child.id();
    let started = start_ticks(pid);
    let version = current_version();
    let mut program = binary().clone();
    program.push(0); // a different build of the same program
    let new_config = config_text(host.api, host.dns, host.upstream, "after the update");
    host.publish(
        &version,
        &program,
        Some(&config_package(&new_config, |_| {})),
    );
    assert!(ask(host.dns, "before.example.com"));
    let reply = host.api("POST", "/api/v1/update/apply", br#"{"force": true}"#);
    assert_eq!(reply.status, 200, "{}", reply.text());
    let body = reply.json();
    assert_eq!(body["restart_required"], true);
    assert_eq!(body["status"]["pending_restart"], true);
    // The guard takes over, runs the candidate, confirms it and execs the final program.
    wait_until("the update is confirmed", 90, || {
        host.transactions()
            .iter()
            .any(|(_, tx)| tx["status"] == "verified")
    });
    wait_until("the final program serves", 60, || host.health().is_some());
    settle_after_guard(&host);
    assert_eq!(host.child.id(), pid);
    assert_eq!(
        start_ticks(pid),
        started,
        "the same process, never restarted by anyone else"
    );
    assert!(host.child.try_wait().unwrap().is_none());
    assert_eq!(host.exe_bytes(), program, "the new program is installed");
    assert_eq!(
        host.config(),
        new_config,
        "the new configuration is installed"
    );
    assert!(
        ask(host.dns, "after.example.com"),
        "DNS answers on the new program"
    );
    let health = host.health().unwrap();
    assert_eq!(health["applied_config_schema"], 1);
    let status = host.api("GET", "/api/v1/update/status", b"").json();
    assert_eq!(status["config_schema_applied"], 1);
    assert_eq!(status["config_update_status"], "success");
    assert!(status.get("pending_restart").is_none(), "{status}");
    assert!(host.root.join("update/installed.json").exists());
    assert!(
        host.root
            .join("webinfo/rust_config_update_state.json")
            .exists()
    );
    assert!(
        !host.root.join("webinfo/config_update_state.json").exists(),
        "the Go state file is never used"
    );
    // The next start cleans the confirmed transaction up.
    let _ = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();
    let _ = host.child.wait();
    host.launch(None);
    wait_until("the transaction is cleaned up", 20, || {
        host.transactions().is_empty()
    });
    assert_eq!(host.exe_bytes(), program);
}

// ===== automatic rollback ================================================================

fn candidate_script(version: &str, on_start: &str) -> Vec<u8> {
    format!(
        "#!/bin/sh\ncase \"$1\" in\n  version) echo {version};;\n  check) exit 0;;\n  start) {on_start};;\nesac\n"
    )
    .into_bytes()
}

fn assert_rolled_back(host: &Host, old_program: &[u8], old_config: &str, reason_part: &str) {
    wait_until("the rollback", 90, || {
        host.transactions()
            .iter()
            .any(|(_, tx)| tx["status"] == "rolled_back")
    });
    wait_until("the old program serves again", 60, || {
        host.health().is_some()
    });
    let tx = host.only_transaction();
    assert!(
        tx["last_error"].as_str().unwrap().contains(reason_part),
        "{} should mention {reason_part}",
        tx["last_error"]
    );
    assert_eq!(host.exe_bytes(), old_program, "the old program is back");
    assert_eq!(host.config(), old_config, "the old configuration is back");
    assert!(
        !host
            .root
            .join("webinfo/rust_config_update_state.json")
            .exists(),
        "the package state is back"
    );
    assert!(ask(host.dns, "rolled-back.example.com"));
    let status = host.api("GET", "/api/v1/update/status", b"").json();
    assert_eq!(status["rollback_performed"], true);
    assert!(
        status["rollback_message"]
            .as_str()
            .unwrap()
            .contains(reason_part)
    );
    assert_eq!(host.health().unwrap()["update"]["state"], "rolled_back");
}

#[test]
fn a_candidate_that_exits_at_once_is_rolled_back_automatically() {
    let host = Host::start("rollback-exit", None);
    let pid = host.child.id();
    let old_config = host.config();
    let version = current_version();
    let new_config = config_text(host.api, host.dns, host.upstream, "must not stay");
    host.publish(
        &version,
        &candidate_script(&version, "exit 3"),
        Some(&config_package(&new_config, |_| {})),
    );
    let reply = host.api("POST", "/api/v1/update/apply", br#"{"force": true}"#);
    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_rolled_back(&host, binary(), &old_config, "提前退出");
    assert_eq!(host.child.id(), pid, "still the same process");
}

#[test]
fn a_candidate_that_never_becomes_healthy_is_rolled_back_at_the_time_limit() {
    let host = Host::start("rollback-timeout", Some(4));
    let old_config = host.config();
    let version = current_version();
    let new_config = config_text(host.api, host.dns, host.upstream, "must not stay");
    host.publish(
        &version,
        &candidate_script(&version, "sleep 300"),
        Some(&config_package(&new_config, |_| {})),
    );
    let started = Instant::now();
    assert_eq!(
        host.api("POST", "/api/v1/update/apply", br#"{"force": true}"#)
            .status,
        200
    );
    assert_rolled_back(&host, binary(), &old_config, "未通过健康检查");
    assert!(started.elapsed() < Duration::from_secs(60));
}

#[test]
fn a_config_update_that_breaks_at_start_is_rolled_back_with_the_old_config() {
    let mut host = Host::start("rollback-config", None);
    let old_config = host.config();
    let pid = host.child.id();
    // Passes `check` (nothing binds in a check) but cannot start: its second listener's
    // port is taken by this test.
    let taken = UdpSocket::bind("127.0.0.1:0").unwrap();
    let taken_port = taken.local_addr().unwrap().port();
    let broken = format!(
        "{}  - tag: dns2\n    type: udp_server\n    args: {{listen: '127.0.0.1:{taken_port}', entry: main}}\n",
        config_text(host.api, host.dns, host.upstream, "broken")
    );
    host.server.set(
        "/config.zip",
        "application/zip",
        config_package(&broken, |_| {}),
    );
    let body =
        serde_json::json!({"url": host.server.url("/config.zip"), "dir": host.root}).to_string();
    let reply = host.api("POST", "/api/v1/config/update_from_url", body.as_bytes());
    assert_eq!(reply.status, 200, "{}", reply.text());
    assert_eq!(reply.json()["status"], "success");
    wait_until("the rollback", 90, || {
        host.transactions()
            .iter()
            .any(|(_, tx)| tx["status"] == "rolled_back")
    });
    wait_until("the service is back", 60, || host.health().is_some());
    assert_eq!(host.config(), old_config);
    assert_eq!(
        host.exe_bytes(),
        *binary(),
        "a config update never touches the program"
    );
    assert_eq!(host.child.id(), pid);
    assert!(ask(host.dns, "config-rollback.example.com"));
    assert_eq!(host.only_transaction()["kind"], "config");
    drop(taken);
    let _ = &mut host;
}

// ===== recovery after the guard is killed ================================================

#[test]
fn killing_the_guard_mid_test_is_recovered_at_the_next_start() {
    let mut host = Host::start("recover", None);
    let old_config = host.config();
    let version = current_version();
    let mut program = binary().clone();
    program.push(0);
    let new_config = config_text(host.api, host.dns, host.upstream, "interrupted");
    host.publish(
        &version,
        &program,
        Some(&config_package(&new_config, |_| {})),
    );
    assert_eq!(
        host.api("POST", "/api/v1/update/apply", br#"{"force": true}"#)
            .status,
        200
    );
    // The guard is this very process now; kill it while the candidate is on trial.
    wait_until("the candidate is on trial", 60, || {
        host.transactions()
            .iter()
            .any(|(_, tx)| tx["status"] == "testing")
    });
    host.child.kill().unwrap();
    let _ = host.child.wait();
    // The candidate is the new program, armed to die with the guard.
    wait_until("the candidate is gone", 20, || host.health().is_none());
    let tx = host.only_transaction();
    assert!(
        tx["status"] == "testing" || tx["status"] == "verified",
        "{tx}"
    );
    if tx["status"] == "testing" {
        assert_eq!(
            host.exe_bytes(),
            program,
            "mixed state: new program, new config"
        );
        // What the service manager would do: start the installed program again.
        host.launch(None);
        wait_until("recovery finished", 30, || host.health().is_some());
        let tx = host.only_transaction();
        assert_eq!(tx["status"], "rolled_back", "{tx}");
        assert!(tx["last_error"].as_str().unwrap().contains("未完成的更新"));
        assert_eq!(host.exe_bytes(), *binary(), "the old program is back");
        assert_eq!(host.config(), old_config, "the old configuration is back");
        assert!(ask(host.dns, "recovered.example.com"));
    } else {
        // The candidate was confirmed before the kill landed: the update stands.
        assert_eq!(host.exe_bytes(), program);
    }
}

// ===== configuration update and export ===================================================

#[test]
fn a_configuration_update_from_a_url_goes_through_the_guard_and_stays() {
    let mut host = Host::start("config-url", None);
    let pid = host.child.id();
    let new_config = config_text(host.api, host.dns, host.upstream, "from the url");
    host.server.set(
        "/config.zip",
        "application/zip",
        config_package(&new_config, |_| {}),
    );
    let url = host.server.url("/config.zip");
    let post = |host: &Host, body: serde_json::Value| {
        host.api(
            "POST",
            "/api/v1/config/update_from_url",
            body.to_string().as_bytes(),
        )
    };
    // Bad requests first.
    assert_eq!(
        host.api("POST", "/api/v1/config/update_from_url", b"{bad")
            .text(),
        "Invalid request body\n"
    );
    assert_eq!(
        post(&host, serde_json::json!({"url": url})).text(),
        "url and dir are required\n"
    );
    assert_eq!(
        post(&host, serde_json::json!({"dir": host.root})).text(),
        "url and dir are required\n"
    );
    assert_eq!(
        post(&host, serde_json::json!({"url": url, "dir": "/tmp"})).status,
        400,
        "only the host's own directory"
    );
    assert_eq!(
        post(
            &host,
            serde_json::json!({"url": "ftp://x/y.zip", "dir": host.root})
        )
        .status,
        400
    );
    // A package that does not verify changes nothing.
    host.server.set(
        "/bad.zip",
        "application/zip",
        config_package(&new_config, |m| {
            m["sha256"]["config_custom.yaml"] = "1".repeat(64).into()
        }),
    );
    let reply = post(
        &host,
        serde_json::json!({"url": host.server.url("/bad.zip"), "dir": host.root}),
    );
    assert_eq!(reply.status, 400);
    assert!(reply.text().contains("配置包无效"));
    host.server
        .set("/junk.zip", "application/zip", b"not a zip".to_vec());
    assert_eq!(
        post(
            &host,
            serde_json::json!({"url": host.server.url("/junk.zip"), "dir": host.root})
        )
        .status,
        400
    );
    assert!(host.transactions().is_empty());
    assert_ne!(host.config(), new_config);
    // The good one.
    let reply = post(&host, serde_json::json!({"url": url, "dir": host.root}));
    assert_eq!(reply.status, 200, "{}", reply.text());
    assert!(
        reply.json()["message"]
            .as_str()
            .unwrap()
            .starts_with("Update successful.")
    );
    wait_until("confirmed", 90, || {
        host.transactions()
            .iter()
            .any(|(_, tx)| tx["status"] == "verified")
    });
    wait_until("serving", 60, || host.health().is_some());
    settle_after_guard(&host);
    assert_eq!(host.config(), new_config);
    assert_eq!(host.exe_bytes(), *binary());
    assert_eq!(host.child.id(), pid);
    assert!(ask(host.dns, "after-config.example.com"));
    assert_eq!(host.health().unwrap()["applied_config_schema"], 1);
    let _ = &mut host;
}

#[test]
fn the_export_is_a_zip_of_this_hosts_configuration_directory_only() {
    let host = Host::start("export", None);
    std::fs::create_dir_all(host.root.join("sub_config")).unwrap();
    std::fs::create_dir_all(host.root.join("backup/x")).unwrap();
    std::fs::write(host.root.join("sub_config/a.yaml"), "plugins: []\n").unwrap();
    std::fs::write(host.root.join("backup/x/old.yaml"), "hidden").unwrap();
    let body = serde_json::json!({"dir": host.root}).to_string();
    let reply = host.api("POST", "/api/v1/config/export", body.as_bytes());
    assert_eq!(reply.status, 200);
    assert!(
        reply
            .headers
            .to_ascii_lowercase()
            .contains("content-type: application/zip")
    );
    assert!(
        reply.headers.contains("filename=\"mosdns_backup_"),
        "{}",
        reply.headers
    );
    let names = zip_names(&reply.body);
    assert!(
        names.contains(&"config_custom.yaml".to_owned())
            && names.contains(&"sub_config/a.yaml".to_owned()),
        "{names:?}"
    );
    assert!(
        names
            .iter()
            .all(|name| !name.starts_with("backup/") && !name.starts_with("update/")),
        "{names:?}"
    );
    // The program copy is not part of the export either way (it is a file in the root).
    assert_eq!(
        host.api("POST", "/api/v1/config/export", b"{}").text(),
        "dir is required\n"
    );
    assert_eq!(
        host.api("POST", "/api/v1/config/export", b"nope").text(),
        "Invalid request body\n"
    );
    assert_eq!(
        host.api("POST", "/api/v1/config/export", br#"{"dir":"/etc"}"#)
            .status,
        400
    );
    assert_eq!(host.api("GET", "/api/v1/config/export", b"").status, 405);
}

/// The names in a stored zip's central directory.
fn zip_names(data: &[u8]) -> Vec<String> {
    let eocd = data
        .windows(4)
        .rposition(|w| w == 0x0605_4b50_u32.to_le_bytes())
        .unwrap();
    let count = u16::from_le_bytes([data[eocd + 10], data[eocd + 11]]) as usize;
    let mut at = u32::from_le_bytes(data[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
    let mut names = Vec::new();
    for _ in 0..count {
        let name_len = u16::from_le_bytes([data[at + 28], data[at + 29]]) as usize;
        let extra = u16::from_le_bytes([data[at + 30], data[at + 31]]) as usize;
        let comment = u16::from_le_bytes([data[at + 32], data[at + 33]]) as usize;
        names.push(String::from_utf8(data[at + 46..at + 46 + name_len].to_vec()).unwrap());
        at += 46 + name_len + extra + comment;
    }
    names
}

#[allow(dead_code)]
fn path_exists(path: &Path) -> bool {
    path.exists()
}

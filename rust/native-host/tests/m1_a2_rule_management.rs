//! M1-a2: rule-source management (`sd_set_light`, `si_set`, `adguard_rule`,
//! `domain_set_light`) and the hot rebuild of `domain_mapper` and direct
//! matchers, through the real management API, real downloads from a local HTTP
//! server (optionally through a local SOCKS5 proxy) and real DNS queries. No
//! test reaches the public network.

// Test fixtures encode small known values and keep each scenario in one function.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::too_many_lines
)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use mosdns_native_host::{BoundHost, HostAssembly, HostOptions, PersistGate, RuleTiming};
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

// ----- SRS fixtures -----------------------------------------------------------

fn uvarint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn srs(body: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(body).unwrap();
    let mut out = b"SRS\x01".to_vec();
    out.extend(encoder.finish().unwrap());
    out
}

/// One rule with one keyword item per keyword.
fn keyword_srs(keywords: &[&str]) -> Vec<u8> {
    let mut body = Vec::new();
    uvarint(&mut body, 1);
    body.extend([0, 3]);
    uvarint(&mut body, keywords.len() as u64);
    for keyword in keywords {
        uvarint(&mut body, keyword.len() as u64);
        body.extend(keyword.as_bytes());
    }
    body.push(0xff);
    srs(&body)
}

/// One rule with one IPv4 range.
fn ip_srs(from: [u8; 4], to: [u8; 4]) -> Vec<u8> {
    let mut body = Vec::new();
    uvarint(&mut body, 1);
    body.extend([0, 6, 1]);
    body.extend(1u64.to_be_bytes());
    for address in [from, to] {
        uvarint(&mut body, 4);
        body.extend(address);
    }
    body.push(0xff);
    srs(&body)
}

// ----- fixture tree -----------------------------------------------------------

struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("m1-a2-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn read(&self, relative: &str) -> Vec<u8> {
        std::fs::read(self.0.join(relative)).unwrap_or_default()
    }

    fn text(&self, relative: &str) -> String {
        String::from_utf8(self.read(relative)).unwrap()
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Ports {
    dns: u16,
    api: u16,
}

/// A configuration with every managed provider kind feeding one mapper; the
/// entry answers with a distinct rcode per decision.
fn config(tree: &Tree, ports: &Ports, socks5: &str) {
    let yaml = format!(
        r#"log: {{level: error}}
api: {{http: "127.0.0.1:{api}"}}
plugins:
  - tag: fwd
    type: forward
    args: {{upstreams: [{{addr: "udp://127.0.0.1:9"}}]}}
  - tag: blocklist
    type: domain_set_light
    args: {{files: ["rule/blocklist.txt"]}}
  - tag: greylist
    type: domain_set_light
    args: {{files: ["rule/greylist.txt"]}}
  - tag: srslist
    type: domain_set_light
    args: {{files: ["rule/list.srs"]}}
  - tag: geosite
    type: sd_set_light
    args: {{socks5: "{socks5}", local_config: "srs/geosite.json"}}
  - tag: geoip
    type: si_set
    args: {{local_config: "srs/geoip.json"}}
  - tag: adguard
    type: adguard_rule
    args: {{dir: "adguard/"}}
  - tag: unified
    type: domain_mapper
    args:
      default_mark: 17
      rules:
        - {{tag: blocklist, mark: 1}}
        - {{tag: greylist, mark: 3}}
        - {{tag: geosite, mark: 2}}
        - {{tag: adguard, mark: 5}}
  - tag: entry
    type: sequence
    args:
      - exec: $unified
      - matches: qname full:slow.test
        exec: sleep 600
      - matches: [client_ip $geoip, qname full:ipcheck.test]
        exec: reject 4
      - matches: qname $adguard
        exec: reject 9
      - matches: fast_mark 1
        exec: reject 3
      - matches: fast_mark 3
        exec: reject 6
      - matches: fast_mark 2
        exec: reject 2
      - matches: fast_mark 5
        exec: reject 5
      - exec: reject 0
  - tag: listener
    type: udp_server
    args: {{entry: entry, listen: "127.0.0.1:{dns}"}}
"#,
        api = ports.api,
        dns = ports.dns,
    );
    tree.write("config.yaml", yaml.as_bytes());
}

const GEOSITE: &str = r#"[
  {
    "name": "g",
    "type": "geositecn",
    "files": "srs/g.srs",
    "url": "",
    "enabled": true,
    "auto_update": false,
    "update_interval_hours": 24,
    "rule_count": 1,
    "last_updated": "2025-12-19T16:32:42.281233973+08:00"
  }
]"#;

fn base_tree(name: &str, socks5: &str) -> (Tree, Ports) {
    let tree = Tree::new(name);
    let ports = Ports {
        dns: free_port(),
        api: free_port(),
    };
    tree.write("rule/blocklist.txt", b"domain:blocked.test\n");
    tree.write("rule/greylist.txt", b"");
    tree.write("srs/geosite.json", GEOSITE.as_bytes());
    tree.write("srs/g.srs", &keyword_srs(&["old-site"]));
    tree.write("srs/geoip.json", b"[]");
    tree.write("adguard/config.json", b"[]");
    config(&tree, &ports, socks5);
    (tree, ports)
}

fn host(tree: &Tree, timing: RuleTiming) -> HostAssembly {
    let config = mosdns_native_host::load_and_compile(&tree.path("config.yaml")).expect("compiles");
    HostAssembly::with_options(config, HostOptions::default().with_rule_timing(timing))
        .expect("assembles")
}

fn fast() -> RuleTiming {
    RuleTiming {
        first_check: Duration::from_secs(3600),
        source_period: Duration::from_secs(3600),
        adguard_period: Duration::from_secs(3600),
        source_timeout: Duration::from_secs(5),
        adguard_timeout: Duration::from_secs(5),
        max_download_bytes: 64 * 1024,
    }
}

// ----- local HTTP server and SOCKS5 proxy -------------------------------------

#[derive(Clone)]
enum Answer {
    Body(Vec<u8>),
    Status(u16),
    Redirect(String),
    Stall,
    Slow(Duration, Vec<u8>),
}

#[derive(Clone, Default)]
struct Server {
    routes: Rc<RefCell<HashMap<String, Answer>>>,
    hits: Rc<RefCell<Vec<String>>>,
}

fn std_listener() -> std::net::TcpListener {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap()
}

impl Server {
    fn start(&self) -> SocketAddr {
        self.start_on(std_listener())
    }

    fn start_on(&self, listener: std::net::TcpListener) -> SocketAddr {
        listener.set_nonblocking(true).unwrap();
        let listener = TcpListener::from_std(listener).unwrap();
        let address = listener.local_addr().unwrap();
        let server = self.clone();
        tokio::task::spawn_local(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let server = server.clone();
                tokio::task::spawn_local(async move { server.answer(stream).await });
            }
        });
        address
    }

    /// Serves HTTPS with a certificate for `127.0.0.1` issued by `ca`.
    fn start_tls(&self, config: std::sync::Arc<rustls::ServerConfig>) -> SocketAddr {
        let listener = std_listener();
        listener.set_nonblocking(true).unwrap();
        let listener = TcpListener::from_std(listener).unwrap();
        let address = listener.local_addr().unwrap();
        let server = self.clone();
        tokio::task::spawn_local(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let server = server.clone();
                let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::clone(&config));
                tokio::task::spawn_local(async move {
                    if let Ok(stream) = acceptor.accept(stream).await {
                        server.answer(stream).await;
                    }
                });
            }
        });
        address
    }

    fn route(&self, path: &str, answer: Answer) {
        self.routes.borrow_mut().insert(path.to_owned(), answer);
    }

    fn hits(&self, path: &str) -> usize {
        self.hits.borrow().iter().filter(|hit| *hit == path).count()
    }

    async fn answer<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(&self, mut stream: S) {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte).await.unwrap_or(0) == 0 {
                return;
            }
            head.push(byte[0]);
        }
        let text = String::from_utf8_lossy(&head).into_owned();
        let path = text
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_owned();
        self.hits.borrow_mut().push(path.clone());
        let answer = self.routes.borrow().get(&path).cloned();
        let reply = |status: &str, extra: &str, body: &[u8]| {
            let mut out = format!(
                "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
            out.extend_from_slice(body);
            out
        };
        let bytes = match answer {
            Some(Answer::Body(body)) => reply("200 OK", "", &body),
            Some(Answer::Slow(delay, body)) => {
                tokio::time::sleep(delay).await;
                reply("200 OK", "", &body)
            }
            Some(Answer::Status(code)) => reply(&format!("{code} Status"), "", b""),
            Some(Answer::Redirect(location)) => {
                reply("302 Found", &format!("Location: {location}\r\n"), b"")
            }
            Some(Answer::Stall) => {
                tokio::time::sleep(Duration::from_secs(60)).await;
                return;
            }
            None => reply("404 Not Found", "", b""),
        };
        let _ = stream.write_all(&bytes).await;
    }
}

// ----- API and DNS clients ----------------------------------------------------

struct Http {
    status: u16,
    content_type: String,
    body: String,
}

async fn api(port: u16, method: &str, path: &str, body: &str) -> Http {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: native\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.unwrap();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").expect("http response");
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    let content_type = head
        .split("\r\n")
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-type")
                .then(|| value.trim().to_owned())
        })
        .unwrap_or_default();
    Http {
        status,
        content_type,
        body: body.to_owned(),
    }
}

fn wire(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for label in name.trim_end_matches('.').split('.') {
        out.push(u8::try_from(label.len()).unwrap());
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    out
}

async fn rcode(port: u16, name: &str) -> u8 {
    let mut query = 7u16.to_be_bytes().to_vec();
    query.extend([1, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    query.extend(wire(name));
    query.extend([0, 1, 0, 1]);
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.send_to(&query, ("127.0.0.1", port)).await.unwrap();
    let mut packet = vec![0; 1500];
    let (length, _) = tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut packet))
        .await
        .expect("response in time")
        .unwrap();
    assert!(length >= 12);
    packet[3] & 15
}

/// Polls until `name` answers `expected` (a background rebuild is published).
async fn eventually(port: u16, name: &str, expected: u8) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let got = rcode(port, name).await;
        if got == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{name}: rcode {got}, expected {expected}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn serve(host: &HostAssembly) -> (TransportCancellation, tokio::task::JoinHandle<()>) {
    let bound: BoundHost = host.bind_host().await.expect("binds");
    let shutdown = TransportCancellation::new();
    let scope = shutdown.clone();
    let task = tokio::task::spawn_local(async move {
        bound.serve(scope).await.expect("clean shutdown");
    });
    (shutdown, task)
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("{error}: {text}"))
}

// ----- tests ------------------------------------------------------------------

#[test]
fn an_sd_source_downloads_through_socks5_and_redirects_then_swaps_the_mapper() {
    let targets = Rc::new(RefCell::new(Vec::new()));
    let proxy = std_listener();
    let (tree, ports) = base_tree("sd", &proxy.local_addr().unwrap().to_string());
    let host = host(&tree, fast());
    host.block_on(async {
        proxy_loop(proxy, Rc::clone(&targets));
        let server = Server::default();
        let http = server.start();
        server.route("/redir", Answer::Redirect("/g-new.srs".into()));
        server.route("/g-new.srs", Answer::Body(keyword_srs(&["new-site", "more-site"])));
        let (shutdown, task) = serve(&host).await;

        assert_eq!(rcode(ports.dns, "old-site.test").await, 2);
        assert_eq!(rcode(ports.dns, "new-site.test").await, 0);

        let listed = api(ports.api, "GET", "/plugins/geosite/config", "").await;
        assert_eq!(listed.status, 200);
        assert_eq!(listed.content_type, "application/json; charset=utf-8");
        assert!(listed.body.ends_with("]\n"));
        assert_eq!(json(&listed.body), json(GEOSITE));

        let body = format!(
            r#"{{"name":"g","type":"geositecn","files":"srs/g.srs","url":"http://127.0.0.1:{}/redir","enabled":true,"auto_update":true,"update_interval_hours":12,"rule_count":99,"last_updated":"2025-12-19T16:32:42.281233973+08:00","__readOnly":false}}"#,
            http.port()
        );
        let put = api(ports.api, "PUT", "/plugins/geosite/config/g", &body).await;
        assert_eq!(put.status, 200, "{}", put.body);
        let stored = json(&put.body);
        assert_eq!(stored["update_interval_hours"], 12);
        assert_eq!(stored["rule_count"], 1, "an update keeps the stored count");

        eventually(ports.dns, "new-site.test", 2).await;
        assert_eq!(rcode(ports.dns, "more-site.test").await, 2);
        assert_eq!(rcode(ports.dns, "old-site.test").await, 0);
        host.config().rule_sources().settle().await;
        assert_eq!(server.hits("/redir"), 1);
        assert_eq!(server.hits("/g-new.srs"), 1);
        assert!(
            targets.borrow().iter().all(|t| t == &format!("127.0.0.1:{}", http.port())),
            "{targets:?}"
        );
        assert_eq!(targets.borrow().len(), 2, "both hops went through the proxy");
        assert_eq!(tree.read("srs/g.srs"), keyword_srs(&["new-site", "more-site"]));

        let after = json(&api(ports.api, "GET", "/plugins/geosite/config", "").await.body);
        assert_eq!(after[0]["rule_count"], 2);
        assert_eq!(after[0]["auto_update"], true);
        assert_ne!(after[0]["last_updated"], "2025-12-19T16:32:42.281233973+08:00");
        assert!(after[0].get("enable_regexp").is_none());
        let manifest = tree.text("srs/geosite.json");
        assert!(manifest.starts_with("[\n  {\n    \"name\": \"g\",\n"), "{manifest}");
        assert_eq!(json(&manifest), after);

        // New source: 201, zero count and time; unknown source: 404 JSON.
        let created = api(
            ports.api,
            "PUT",
            "/plugins/geosite/config/b",
            r#"{"files":"srs/b.srs","url":"http://127.0.0.1:1/none","enabled":false}"#,
        )
        .await;
        assert_eq!(created.status, 201, "{}", created.body);
        assert_eq!(json(&created.body)["last_updated"], "0001-01-01T00:00:00Z");
        let missing = api(ports.api, "POST", "/plugins/geosite/update/zzz", "").await;
        assert_eq!(missing.status, 404);
        assert_eq!(json(&missing.body)["error"], "source 'zzz' not found");
        let required = api(ports.api, "PUT", "/plugins/geosite/config/c", r#"{"files":" "}"#).await;
        assert_eq!(required.status, 400);
        assert_eq!(json(&required.body)["error"], "'files' and 'url' fields are required");
        let invalid = api(ports.api, "PUT", "/plugins/geosite/config/c", "{").await;
        assert_eq!(invalid.status, 400);
        assert_eq!(json(&invalid.body)["error"], "invalid request body");

        // A file another source still reads survives that source's DELETE.
        let twin = api(
            ports.api,
            "PUT",
            "/plugins/geosite/config/twin",
            r#"{"files":"srs/../srs/./g.srs","url":"http://127.0.0.1:1/twin","enabled":false}"#,
        )
        .await;
        assert_eq!(twin.status, 201, "{}", twin.body);
        host.config().rule_sources().settle().await;
        assert_eq!(api(ports.api, "DELETE", "/plugins/geosite/config/twin", "").await.status, 204);
        host.config().rule_sources().settle().await;
        assert!(tree.path("srs/g.srs").exists(), "shared file kept");
        assert_eq!(rcode(ports.dns, "new-site.test").await, 2);

        // DELETE: manifest first, then the rule file; the mapper drops it.
        let deleted = api(ports.api, "DELETE", "/plugins/geosite/config/g", "").await;
        assert_eq!(deleted.status, 204);
        assert!(deleted.body.is_empty());
        eventually(ports.dns, "new-site.test", 0).await;
        assert!(!tree.path("srs/g.srs").exists());
        let gone = api(ports.api, "DELETE", "/plugins/geosite/config/g", "").await;
        assert_eq!(gone.status, 404);
        assert_eq!(json(&gone.body)["error"], "source not found");

        shutdown.cancel();
        task.await.unwrap();
    });
}

/// A minimal SOCKS5 proxy (no auth) that records every CONNECT target.
fn proxy_loop(listener: std::net::TcpListener, targets: Rc<RefCell<Vec<String>>>) {
    listener.set_nonblocking(true).unwrap();
    let listener = TcpListener::from_std(listener).unwrap();
    tokio::task::spawn_local(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                return;
            };
            tokio::task::spawn_local(proxy_one(client, Rc::clone(&targets)));
        }
    });
}

async fn proxy_one(mut client: TcpStream, targets: Rc<RefCell<Vec<String>>>) {
    let mut greeting = [0u8; 3];
    if client.read_exact(&mut greeting).await.is_err() {
        return;
    }
    client.write_all(&[5, 0]).await.unwrap();
    let mut head = [0u8; 4];
    client.read_exact(&mut head).await.unwrap();
    let host = if head[3] == 1 {
        let mut ip = [0u8; 4];
        client.read_exact(&mut ip).await.unwrap();
        std::net::Ipv4Addr::from(ip).to_string()
    } else {
        let mut length = [0u8; 1];
        client.read_exact(&mut length).await.unwrap();
        let mut name = vec![0u8; usize::from(length[0])];
        client.read_exact(&mut name).await.unwrap();
        String::from_utf8(name).unwrap()
    };
    let mut port = [0u8; 2];
    client.read_exact(&mut port).await.unwrap();
    let port = u16::from_be_bytes(port);
    targets.borrow_mut().push(format!("{host}:{port}"));
    let mut upstream = TcpStream::connect((host.as_str(), port)).await.unwrap();
    client
        .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .unwrap();
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
}

#[test]
fn failed_downloads_keep_the_file_the_manifest_and_the_rules() {
    let (tree, ports) = base_tree("fail", "");
    let mut timing = fast();
    timing.source_timeout = Duration::from_millis(400);
    timing.max_download_bytes = 1024;
    let host = host(&tree, timing);
    host.block_on(async {
        let server = Server::default();
        let http = server.start();
        server.route("/404", Answer::Status(404));
        server.route("/junk", Answer::Body(b"not an srs file".to_vec()));
        server.route("/big", Answer::Body(vec![b'S'; 4096]));
        server.route("/stall", Answer::Stall);
        server.route(
            "/refused",
            Answer::Body({
                let mut body = Vec::new();
                uvarint(&mut body, 1);
                body.extend([0, 4]);
                uvarint(&mut body, 1);
                let pattern = b"\\p{L}+";
                uvarint(&mut body, pattern.len() as u64);
                body.extend(pattern);
                body.push(0xff);
                srs(&body)
            }),
        );
        let (shutdown, task) = serve(&host).await;
        // Turning regexps on for a file holding a rule this host refuses at
        // start is rejected before anything is stored.
        tree.write("srs/r.srs", &{
            let mut body = Vec::new();
            uvarint(&mut body, 1);
            body.extend([0, 4]);
            uvarint(&mut body, 1);
            uvarint(&mut body, 6);
            body.extend(b"\\p{L}+");
            body.push(0xff);
            srs(&body)
        });
        let manifest = tree.text("srs/geosite.json");
        let refused = api(
            ports.api,
            "PUT",
            "/plugins/geosite/config/r",
            r#"{"files":"srs/r.srs","url":"http://127.0.0.1:1/r","enabled":true,"enable_regexp":true}"#,
        )
        .await;
        assert_eq!(refused.status, 400, "{}", refused.body);
        assert_eq!(tree.text("srs/geosite.json"), manifest);
        let before_file = tree.read("srs/g.srs");
        // Point the source at each failing URL in turn (enable_regexp so the
        // refused pattern is decoded), then trigger a manual update.
        for (path, url) in [
            ("/404", format!("http://127.0.0.1:{}/404", http.port())),
            ("/junk", format!("http://127.0.0.1:{}/junk", http.port())),
            ("/big", format!("http://127.0.0.1:{}/big", http.port())),
            ("/stall", format!("http://127.0.0.1:{}/stall", http.port())),
            ("/refused", format!("http://127.0.0.1:{}/refused", http.port())),
            ("closed", "http://127.0.0.1:1/closed".to_owned()),
        ] {
            let body = format!(
                r#"{{"type":"geositecn","files":"srs/g.srs","url":"{url}","enabled":true,"enable_regexp":true,"update_interval_hours":24}}"#
            );
            assert_eq!(api(ports.api, "PUT", "/plugins/geosite/config/g", &body).await.status, 200);
            host.config().rule_sources().settle().await;
            let manifest = tree.text("srs/geosite.json");
            let update = api(ports.api, "POST", "/plugins/geosite/update/g", "").await;
            assert_eq!(update.status, 202);
            assert_eq!(
                json(&update.body)["message"],
                "update process for 'g' started in the background"
            );
            host.config().rule_sources().settle().await;
            if path != "closed" {
                assert!(server.hits(path) >= 1, "{path}");
            }
            assert_eq!(tree.read("srs/g.srs"), before_file, "{path}");
            assert_eq!(tree.text("srs/geosite.json"), manifest, "{path}");
            assert_eq!(rcode(ports.dns, "old-site.test").await, 2, "{path}");
            let leftovers: Vec<_> = std::fs::read_dir(tree.path("srs"))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
                .collect();
            assert!(leftovers.is_empty(), "{path}");
        }
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn a_proxy_failure_is_a_failed_download() {
    let closed = free_port();
    let (tree, ports) = base_tree("proxyfail", &format!("127.0.0.1:{closed}"));
    let host = host(&tree, fast());
    host.block_on(async {
        let server = Server::default();
        let http = server.start();
        server.route("/g.srs", Answer::Body(keyword_srs(&["new-site"])));
        let (shutdown, task) = serve(&host).await;
        let body = format!(
            r#"{{"files":"srs/g.srs","url":"http://127.0.0.1:{}/g.srs","enabled":true}}"#,
            http.port()
        );
        assert_eq!(
            api(ports.api, "PUT", "/plugins/geosite/config/g", &body)
                .await
                .status,
            200
        );
        host.config().rule_sources().settle().await;
        assert_eq!(server.hits("/g.srs"), 0, "nothing bypasses the proxy");
        assert_eq!(tree.read("srs/g.srs"), keyword_srs(&["old-site"]));
        assert_eq!(rcode(ports.dns, "old-site.test").await, 2);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn duplicate_update_requests_never_download_one_source_concurrently() {
    let (tree, ports) = base_tree("flight", "");
    let host = host(&tree, fast());
    host.block_on(async {
        let server = Server::default();
        let http = server.start();
        server.route(
            "/g.srs",
            Answer::Slow(Duration::from_millis(300), keyword_srs(&["new-site"])),
        );
        let (shutdown, task) = serve(&host).await;
        let body = format!(
            r#"{{"files":"srs/g.srs","url":"http://127.0.0.1:{}/g.srs","enabled":true}}"#,
            http.port()
        );
        assert_eq!(
            api(ports.api, "PUT", "/plugins/geosite/config/g", &body)
                .await
                .status,
            200
        );
        for _ in 0..5 {
            assert_eq!(
                api(ports.api, "POST", "/plugins/geosite/update/g", "")
                    .await
                    .status,
                202
            );
        }
        host.config().rule_sources().settle().await;
        // The first download plus at most one coalesced re-run.
        assert!(
            (1..=2).contains(&server.hits("/g.srs")),
            "{}",
            server.hits("/g.srs")
        );
        assert_eq!(rcode(ports.dns, "new-site.test").await, 2);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn domain_set_light_show_save_and_post_follow_the_reference() {
    let (tree, ports) = base_tree("light", "");
    tree.write(
        "rule/blocklist.txt",
        b"domain:blocked.test\n# comment\n\n  domain:a.test  \ndomain:b.test\ndomain:c.test\n",
    );
    tree.write("rule/list.srs", &keyword_srs(&["srs-rule"]));
    let host = host(&tree, fast());
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let show = api(ports.api, "GET", "/plugins/blocklist/show", "").await;
        assert_eq!(
            (show.status, show.content_type.as_str()),
            (200, "text/plain; charset=utf-8")
        );
        assert_eq!(
            show.body,
            "domain:blocked.test\ndomain:a.test\ndomain:b.test\ndomain:c.test\n"
        );
        let page = api(
            ports.api,
            "GET",
            "/plugins/blocklist/show?limit=2&offset=1",
            "",
        )
        .await;
        assert_eq!(page.body, "domain:a.test\ndomain:b.test\n");
        let search = api(
            ports.api,
            "GET",
            "/plugins/blocklist/show?q=B.T&limit=10000",
            "",
        )
        .await;
        assert_eq!(search.body, "domain:b.test\n");
        assert_eq!(
            api(ports.api, "GET", "/plugins/srslist/show", "")
                .await
                .body,
            "keyword:srs-rule\n"
        );

        // POST replaces the list; values are normalized like file lines.
        let post = api(
            ports.api,
            "POST",
            "/plugins/blocklist/post",
            r##"{"values":[" full:new.test ","","# note","domain:x.test","a\nb"]}"##,
        )
        .await;
        assert_eq!(post.status, 200);
        assert_eq!(post.body, "domain_set_light replaced with 2 entries");
        assert_eq!(
            tree.text("rule/blocklist.txt"),
            "full:new.test\ndomain:x.test\n"
        );
        eventually(ports.dns, "new.test", 3).await;
        assert_eq!(rcode(ports.dns, "y.x.test").await, 3);
        assert_eq!(rcode(ports.dns, "blocked.test").await, 0);

        // A rule this host would refuse at start is rejected before writing.
        let refused = api(
            ports.api,
            "POST",
            "/plugins/blocklist/post",
            r#"{"values":["regexp:\\p{L}+"]}"#,
        )
        .await;
        assert_eq!(refused.status, 400, "{}", refused.body);
        assert_eq!(
            tree.text("rule/blocklist.txt"),
            "full:new.test\ndomain:x.test\n"
        );
        let bad = api(ports.api, "POST", "/plugins/blocklist/post", "{").await;
        assert_eq!((bad.status, bad.body.as_str()), (400, "invalid JSON\n"));
        // Not a `.txt` file: neither post nor save writes it.
        let srs_post = api(
            ports.api,
            "POST",
            "/plugins/srslist/post",
            r#"{"values":[]}"#,
        )
        .await;
        assert_eq!(srs_post.status, 400);
        assert_eq!(srs_post.body, "no txt file configured, cannot post\n");
        assert_eq!(
            api(ports.api, "GET", "/plugins/srslist/save", "")
                .await
                .status,
            400
        );
        assert_eq!(tree.read("rule/list.srs"), keyword_srs(&["srs-rule"]));
        // Save rewrites the current list; wrong method and unknown action.
        assert_eq!(
            api(ports.api, "GET", "/plugins/blocklist/save", "")
                .await
                .status,
            200
        );
        assert_eq!(
            tree.text("rule/blocklist.txt"),
            "full:new.test\ndomain:x.test\n"
        );
        assert_eq!(
            api(ports.api, "GET", "/plugins/blocklist/post", "")
                .await
                .status,
            405
        );
        assert_eq!(
            api(ports.api, "GET", "/plugins/blocklist/config", "")
                .await
                .status,
            404
        );

        // Capabilities follow the configured providers.
        let caps = json(&api(ports.api, "GET", "/api/v1/capabilities", "").await.body);
        for id in ["rules.diversion", "rules.adguard", "rules.local.manage"] {
            assert_eq!(caps["ui_operations"][id]["supported"], true, "{id}");
        }
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn adguard_lists_are_managed_and_republish_the_direct_matcher() {
    let (tree, ports) = base_tree("adguard", "");
    tree.write(
        "adguard/config.json",
        br#"[
  {
    "id": "aaaa",
    "name": "old",
    "url": "http://127.0.0.1:1/old",
    "enabled": true,
    "auto_update": false,
    "update_interval_hours": 24,
    "rule_count": 1,
    "last_updated": "2026-03-24T13:10:52.253656114+08:00"
  }
]"#,
    );
    tree.write("adguard/aaaa.rules", b"||old-ads.test^\n");
    let host = host(&tree, fast());
    host.block_on(async {
        let server = Server::default();
        let http = server.start();
        server.route("/list", Answer::Body(b"! title\n||new-ads.test^\n@@||ok.new-ads.test^\n".to_vec()));
        let (shutdown, task) = serve(&host).await;
        assert_eq!(rcode(ports.dns, "old-ads.test").await, 9);

        let listed = api(ports.api, "GET", "/plugins/adguard/rules", "").await;
        assert_eq!((listed.status, listed.content_type.as_str()), (200, "application/json"));
        let created = api(
            ports.api,
            "POST",
            "/plugins/adguard/rules",
            &format!(
                r#"{{"name":" new ","url":"http://127.0.0.1:{}/list","enabled":true,"auto_update":true,"update_interval_hours":24}}"#,
                http.port()
            ),
        )
        .await;
        assert_eq!(created.status, 201, "{}", created.body);
        let rule = json(&created.body);
        let id = rule["id"].as_str().unwrap().to_owned();
        assert_eq!(rule["name"], "new");
        assert_eq!(rule["last_updated"], "0001-01-01T00:00:00Z");
        eventually(ports.dns, "new-ads.test", 9).await;
        // The exception rule wins for the direct matcher, the mapper still marks it.
        assert_eq!(rcode(ports.dns, "ok.new-ads.test").await, 5);
        host.config().rule_sources().settle().await;
        assert!(tree.path(&format!("adguard/{id}.rules")).exists());
        let manifest = json(&tree.text("adguard/config.json"));
        let stored = manifest
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id.as_str())
            .unwrap()
            .clone();
        assert_eq!(stored["rule_count"], 2);
        assert!(stored["last_updated"].is_string());

        // PUT disables it (no download); DELETE removes the file.
        let put = api(
            ports.api,
            "PUT",
            &format!("/plugins/adguard/rules/{id}"),
            r#"{"name":"new","url":"http://x/list","enabled":false,"update_interval_hours":24}"#,
        )
        .await;
        assert_eq!(put.status, 200, "{}", put.body);
        eventually(ports.dns, "new-ads.test", 0).await;
        for (body, message) in [
            ("{", "Invalid request body"),
            (r#"{"name":"","url":"u"}"#, "Name and URL are required"),
            (
                r#"{"name":"n","url":"u","update_interval_hours":-1}"#,
                "UpdateIntervalHours cannot be negative",
            ),
        ] {
            let reply = api(ports.api, "POST", "/plugins/adguard/rules", body).await;
            assert_eq!(reply.status, 400);
            assert_eq!(json(&reply.body)["error"], message);
        }
        let all = api(ports.api, "POST", "/plugins/adguard/update", "").await;
        assert_eq!(all.status, 202);
        assert_eq!(all.body, "Update process for enabled rules has been started in the background.\n");
        let one = api(ports.api, "POST", &format!("/plugins/adguard/update/{id}"), "").await;
        assert_eq!(one.status, 202);
        assert_eq!(one.body, "Update process for rule has been started in the background.\n");
        assert_eq!(api(ports.api, "POST", "/plugins/adguard/update/nope", "").await.status, 404);
        host.config().rule_sources().settle().await;
        let deleted = api(ports.api, "DELETE", &format!("/plugins/adguard/rules/{id}"), "").await;
        assert_eq!(deleted.status, 204);
        assert!(!tree.path(&format!("adguard/{id}.rules")).exists());
        assert_eq!(api(ports.api, "DELETE", "/plugins/adguard/rules/nope", "").await.status, 404);
        host.config().rule_sources().settle().await;
        assert_eq!(rcode(ports.dns, "old-ads.test").await, 9);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn an_si_source_update_republishes_the_ip_list() {
    let (tree, ports) = base_tree("si", "");
    let host = host(&tree, fast());
    host.block_on(async {
        let server = Server::default();
        let http = server.start();
        server.route("/ip.srs", Answer::Body(ip_srs([127, 0, 0, 0], [127, 255, 255, 255])));
        let (shutdown, task) = serve(&host).await;
        assert_eq!(rcode(ports.dns, "ipcheck.test").await, 0);
        // `si_set` accepts a source without the sd-only required fields.
        let body = format!(
            r#"{{"type":"geoipcn","files":"srs/ip.srs","url":"http://127.0.0.1:{}/ip.srs","enabled":true}}"#,
            http.port()
        );
        let put = api(ports.api, "PUT", "/plugins/geoip/config/ip", &body).await;
        assert_eq!(put.status, 201, "{}", put.body);
        eventually(ports.dns, "ipcheck.test", 4).await;
        host.config().rule_sources().settle().await;
        let listed = json(&api(ports.api, "GET", "/plugins/geoip/config", "").await.body);
        assert_eq!(listed[0]["rule_count"], 1);
        assert!(listed[0].get("enable_regexp").is_none());
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn queries_are_served_during_a_rebuild_and_a_failed_build_keeps_the_old_rules() {
    let (tree, ports) = base_tree("inflight", "");
    let host = host(&tree, fast());
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let sources = host.config().rule_sources();
        // Park the rebuild on the blocking pool: DNS keeps answering from the
        // published generation meanwhile.
        let gate = PersistGate::new();
        sources.inject_build_gate(gate.clone());
        let builds = sources.completed_builds();
        let post = api(
            ports.api,
            "POST",
            "/plugins/greylist/post",
            r#"{"values":["full:grey.test"]}"#,
        )
        .await;
        assert_eq!(post.status, 200);
        let started = Instant::now();
        while !gate.arrived() {
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "rebuild never started"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let asked = Instant::now();
        assert_eq!(rcode(ports.dns, "grey.test").await, 0);
        assert_eq!(rcode(ports.dns, "blocked.test").await, 3);
        assert!(asked.elapsed() < Duration::from_millis(500));
        // A query that ran the mapper before publication keeps its marks.
        let slow = tokio::task::spawn_local(async move { rcode(ports.dns, "slow.test").await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let slow_post = api(
            ports.api,
            "POST",
            "/plugins/blocklist/post",
            r#"{"values":["domain:blocked.test","full:slow.test"]}"#,
        )
        .await;
        assert_eq!(slow_post.status, 200);
        gate.release();
        eventually(ports.dns, "grey.test", 6).await;
        eventually(ports.dns, "slow.test", 3).await;
        assert_eq!(
            slow.await.unwrap(),
            0,
            "the in-flight query keeps the old generation"
        );
        sources.settle().await;
        assert!(sources.completed_builds() > builds);

        // A list the loader cannot read (a line over the scanner limit) fails
        // the rebuild; the published generation stays.
        let mut long = b"x".repeat(70 * 1024);
        long.push(b'\n');
        std::fs::write(tree.path("rule/blocklist.txt"), &long).unwrap();
        let post = api(
            ports.api,
            "POST",
            "/plugins/greylist/post",
            r#"{"values":["full:grey2.test"]}"#,
        )
        .await;
        assert_eq!(post.status, 200);
        sources.settle().await;
        assert_eq!(
            rcode(ports.dns, "grey2.test").await,
            0,
            "failed build is not published"
        );
        assert_eq!(rcode(ports.dns, "blocked.test").await, 3, "old rules stay");
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn the_scheduler_updates_due_sources_and_shutdown_reaps_downloads() {
    let (tree, ports) = base_tree("schedule", "");
    let http = std_listener();
    let port = http.local_addr().unwrap().port();
    let now = time_text();
    let manifest = format!(
        r#"[
 {{"name":"due","files":"srs/due.srs","url":"http://127.0.0.1:{port}/due.srs","enabled":true,"auto_update":true,"update_interval_hours":1,"last_updated":"2020-01-01T00:00:00Z"}},
 {{"name":"fresh","files":"srs/fresh.srs","url":"http://127.0.0.1:{port}/fresh.srs","enabled":true,"auto_update":true,"update_interval_hours":1,"last_updated":"{now}"}},
 {{"name":"off","files":"srs/off.srs","url":"http://127.0.0.1:{port}/fresh.srs","enabled":true,"auto_update":false,"update_interval_hours":1}}
]"#
    );
    tree.write("srs/geosite.json", manifest.as_bytes());
    let mut timing = fast();
    timing.first_check = Duration::from_millis(100);
    timing.source_period = Duration::from_millis(200);
    let host = host(&tree, timing);
    host.block_on(async {
        let server = Server::default();
        server.start_on(http);
        server.route("/due.srs", Answer::Body(keyword_srs(&["due-site"])));
        server.route("/fresh.srs", Answer::Body(keyword_srs(&["fresh-site"])));
        server.route("/stall.srs", Answer::Stall);
        let (shutdown, task) = serve(&host).await;
        eventually(ports.dns, "due-site.test", 2).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(server.hits("/fresh.srs"), 0);
        assert_eq!(
            server.hits("/due.srs"),
            1,
            "updated once, then no longer due"
        );

        // A download in flight at shutdown is canceled and joined promptly.
        let body = format!(
            r#"{{"files":"srs/s.srs","url":"http://127.0.0.1:{port}/stall.srs","enabled":true}}"#
        );
        assert_eq!(
            api(ports.api, "PUT", "/plugins/geosite/config/s", &body)
                .await
                .status,
            201
        );
        let started = Instant::now();
        while server.hits("/stall.srs") == 0 {
            assert!(started.elapsed() < Duration::from_secs(3));
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let stopping = Instant::now();
        shutdown.cancel();
        task.await.unwrap();
        assert!(
            stopping.elapsed() < Duration::from_secs(2),
            "{:?}",
            stopping.elapsed()
        );
    });
}

fn time_text() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // Civil date from Unix days (Howard Hinnant).
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// A test CA and a server certificate for `127.0.0.1`.
fn tls_fixture() -> (rustls::RootCertStore, std::sync::Arc<rustls::ServerConfig>) {
    use rcgen::{
        BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, KeyUsagePurpose,
        SanType, date_time_ymd,
    };
    let root_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let mut root_params = CertificateParams::default();
    root_params
        .distinguished_name
        .push(DnType::CommonName, "m1-a2-test-root");
    root_params.not_before = date_time_ymd(2024, 1, 1);
    root_params.not_after = date_time_ymd(2036, 1, 1);
    root_params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let root = root_params.self_signed(&root_key).unwrap();
    let issuer = Issuer::from_params(&root_params, &root_key);
    let leaf_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let mut leaf_params = CertificateParams::default();
    leaf_params.subject_alt_names = vec![SanType::IpAddress([127, 0, 0, 1].into())];
    leaf_params.not_before = date_time_ymd(2024, 1, 1);
    leaf_params.not_after = date_time_ymd(2036, 1, 1);
    let leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![leaf.der().clone()],
        rustls::pki_types::PrivateKeyDer::try_from(leaf_key.serialize_der()).unwrap(),
    )
    .unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(root.der().clone()).unwrap();
    (roots, std::sync::Arc::new(config))
}

#[test]
fn https_downloads_verify_the_server_certificate() {
    let (roots, server_config) = tls_fixture();
    for trusted in [true, false] {
        let (tree, ports) = base_tree(if trusted { "tls-ok" } else { "tls-bad" }, "");
        let config =
            mosdns_native_host::load_and_compile(&tree.path("config.yaml")).expect("compiles");
        let mut options = HostOptions::default().with_rule_timing(fast());
        if trusted {
            options = options.with_tls_roots(roots.clone());
        }
        let host = HostAssembly::with_options(config, options).expect("assembles");
        host.block_on(async {
            let server = Server::default();
            let https = server.start_tls(std::sync::Arc::clone(&server_config));
            server.route("/g.srs", Answer::Body(keyword_srs(&["tls-site"])));
            let (shutdown, task) = serve(&host).await;
            let body = format!(
                r#"{{"files":"srs/g.srs","url":"https://127.0.0.1:{}/g.srs","enabled":true}}"#,
                https.port()
            );
            assert_eq!(
                api(ports.api, "PUT", "/plugins/geosite/config/g", &body)
                    .await
                    .status,
                200
            );
            host.config().rule_sources().settle().await;
            let expected = if trusted { 2 } else { 0 };
            assert_eq!(
                rcode(ports.dns, "tls-site.test").await,
                expected,
                "trusted={trusted}"
            );
            assert_eq!(server.hits("/g.srs"), usize::from(trusted));
            shutdown.cancel();
            task.await.unwrap();
        });
    }
}

/// A manifest written by the reference `saveConfig` (Go `MarshalIndent`) is
/// read and listed unchanged, and rewritten byte-for-byte after a change that
/// does not touch it.
#[test]
fn a_reference_written_manifest_round_trips() {
    let go_written = include_str!("fixtures/m1_a2/go-sd.json");
    let (tree, ports) = base_tree("interop", "");
    tree.write("srs/geosite.json", go_written.as_bytes());
    let host = host(&tree, fast());
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let listed = api(ports.api, "GET", "/plugins/geosite/config", "").await;
        assert_eq!(json(&listed.body), json(go_written));
        // Adding then deleting a source rewrites the file; the Go-written
        // entries come back byte-identical.
        let put = api(
            ports.api,
            "PUT",
            "/plugins/geosite/config/zz",
            r#"{"files":"srs/zz.srs","url":"http://127.0.0.1:1/zz","enabled":false}"#,
        )
        .await;
        assert_eq!(put.status, 201);
        host.config().rule_sources().settle().await;
        assert_eq!(
            api(ports.api, "DELETE", "/plugins/geosite/config/zz", "")
                .await
                .status,
            204
        );
        host.config().rule_sources().settle().await;
        assert_eq!(tree.text("srs/geosite.json"), go_written.trim_end());
        shutdown.cancel();
        task.await.unwrap();
    });
}

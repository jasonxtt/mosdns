//! M1-f1: the typed settings layer — configuration objects and `$tag`
//! references, `webinfo/settings.json`, the effective report, hot apply of a
//! settings change as a whole generation, and the settings routes of the web UI.
//! Local DNS stubs and a local SOCKS5 relay stand in for the network; no test
//! reaches the public network.

#![allow(clippy::too_many_lines)]

use std::cell::Cell;
use std::net::UdpSocket as StdUdpSocket;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mosdns_native_host::{BoundHost, HostAssembly, HostOptions};
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

// ----- fixtures -----------------------------------------------------------------

struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("m1-f1-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn text(&self, relative: &str) -> String {
        std::fs::read_to_string(self.0.join(relative)).unwrap_or_default()
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

fn wire_name(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for label in name.trim_end_matches('.').split('.') {
        out.push(u8::try_from(label.len()).unwrap());
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    out
}

fn query_packet(name: &str) -> Vec<u8> {
    let mut packet = vec![0x4b, 0x21, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    packet.extend_from_slice(&wire_name(name));
    packet.extend_from_slice(&[0, 1, 0, 1]);
    packet
}

fn answer_for(query: &[u8], marker: u8) -> Vec<u8> {
    let mut end = 12;
    while query[end] != 0 {
        end += 1 + usize::from(query[end]);
    }
    end += 5;
    let mut reply = vec![query[0], query[1], 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0];
    reply.extend_from_slice(&query[12..end]);
    reply.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 192, 0, 2, marker]);
    reply
}

/// A UDP DNS stub that answers every query with `192.0.2.<marker>` after
/// `delay`; it counts what it received.
struct Stub {
    port: u16,
    seen: Arc<Mutex<usize>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Stub {
    fn start(marker: u8, delay: Duration) -> Self {
        let socket = StdUdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let port = socket.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (count, stopped) = (Arc::clone(&seen), Arc::clone(&stop));
        let thread = thread::spawn(move || {
            let mut packet = vec![0_u8; 4096];
            while !stopped.load(Ordering::SeqCst) {
                let Ok((length, client)) = socket.recv_from(&mut packet) else {
                    continue;
                };
                *count.lock().unwrap() += 1;
                let reply = answer_for(&packet[..length], marker);
                let socket = socket.try_clone().unwrap();
                if delay.is_zero() {
                    let _ = socket.send_to(&reply, client);
                } else {
                    thread::spawn(move || {
                        thread::sleep(delay);
                        let _ = socket.send_to(&reply, client);
                    });
                }
            }
        });
        Self {
            port,
            seen,
            stop,
            thread: Some(thread),
        }
    }

    fn count(&self) -> usize {
        *self.seen.lock().unwrap()
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

/// DNS over TCP on `listener`, answering `192.0.2.<marker>`.
fn serve_tcp(listener: std::net::TcpListener, marker: u8) {
    listener.set_nonblocking(true).unwrap();
    let listener = TcpListener::from_std(listener).unwrap();
    tokio::task::spawn_local(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::task::spawn_local(async move {
                loop {
                    let Ok(length) = stream.read_u16().await else {
                        return;
                    };
                    let mut packet = vec![0; usize::from(length)];
                    if stream.read_exact(&mut packet).await.is_err() {
                        return;
                    }
                    let reply = answer_for(&packet, marker);
                    if stream
                        .write_u16(u16::try_from(reply.len()).unwrap())
                        .await
                        .is_err()
                        || stream.write_all(&reply).await.is_err()
                    {
                        return;
                    }
                }
            });
        }
    });
}

/// A SOCKS5 relay (no authentication, CONNECT): counts connections and
/// relays them to the requested address.
fn serve_socks(listener: std::net::TcpListener, connects: Rc<Cell<usize>>) {
    listener.set_nonblocking(true).unwrap();
    let listener = TcpListener::from_std(listener).unwrap();
    tokio::task::spawn_local(async move {
        loop {
            let Ok((mut client, _)) = listener.accept().await else {
                return;
            };
            let connects = Rc::clone(&connects);
            tokio::task::spawn_local(async move {
                let mut head = [0_u8; 2];
                if client.read_exact(&mut head).await.is_err() {
                    return;
                }
                let mut methods = vec![0; usize::from(head[1])];
                let _ = client.read_exact(&mut methods).await;
                let _ = client.write_all(&[5, 0]).await;
                let mut request = [0_u8; 4];
                if client.read_exact(&mut request).await.is_err() {
                    return;
                }
                let host = match request[3] {
                    1 => {
                        let mut ip = [0_u8; 4];
                        let _ = client.read_exact(&mut ip).await;
                        std::net::Ipv4Addr::from(ip).to_string()
                    }
                    3 => {
                        let length = client.read_u8().await.unwrap_or(0);
                        let mut name = vec![0; usize::from(length)];
                        let _ = client.read_exact(&mut name).await;
                        String::from_utf8_lossy(&name).into_owned()
                    }
                    _ => return,
                };
                let port = client.read_u16().await.unwrap_or(0);
                let Ok(mut target) = TcpStream::connect((host.as_str(), port)).await else {
                    let _ = client.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await;
                    return;
                };
                connects.set(connects.get() + 1);
                let _ = client.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await;
                let _ = tokio::io::copy_bidirectional(&mut client, &mut target).await;
            });
        }
    });
}

struct Ports {
    dns: u16,
    api: u16,
}

fn ports() -> Ports {
    Ports {
        dns: free_port(),
        api: free_port(),
    }
}

/// A configuration with the settable objects; `extra` adds plugins before the
/// entry sequence, `entry` is the sequence's exec list.
fn config(tree: &Tree, ports: &Ports, extra: &str) {
    let yaml = format!(
        r#"log: {{level: error}}
api: {{http: "127.0.0.1:{api}"}}
plugins:
{extra}  - tag: entry
    type: sequence
    args:
      - exec: $my_realiplist
      - exec: $foreign
  - tag: listener
    type: udp_server
    args: {{entry: entry, listen: "127.0.0.1:{dns}"}}
"#,
        api = ports.api,
        dns = ports.dns,
    );
    tree.write("config.yaml", &yaml);
}

/// The objects every host test needs: a proxy object, an ECS object, an
/// `aliapi` group whose entries are `upstreams`, and one `domain_output`.
fn objects(proxy_addr: &str, upstreams: &str) -> String {
    format!(
        r#"  - tag: main_proxy
    type: socks5_proxy
    args: {{addr: "{proxy_addr}"}}
  - tag: client_ecs
    type: ecs_address
    args: {{addr: "192.0.2.77"}}
  - tag: foreign
    type: aliapi
    args:
      proxy: $main_proxy
      upstreams:
{upstreams}
  - tag: my_realiplist
    type: domain_output
    args: {{file_stat: gen/real.txt, dump_interval: 3600}}
"#
    )
}

fn host(tree: &Tree) -> HostAssembly {
    let config = mosdns_native_host::load_and_compile(&tree.path("config.yaml")).expect("compiles");
    HostAssembly::with_options(config, HostOptions::default()).expect("assembles")
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

struct Http {
    status: u16,
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
    Http {
        status,
        body: body.to_owned(),
    }
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("{error}: {text}"))
}

/// The last octet of the answer's A record, `None` when the query failed.
async fn marker(port: u16, name: &str) -> Option<u8> {
    marker_within(port, name, Duration::from_secs(5))
        .await
        .expect("response in time")
}

/// Like [`marker`]; `None` when no datagram came back within `limit`.
async fn marker_within(port: u16, name: &str, limit: Duration) -> Option<Option<u8>> {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket
        .send_to(&query_packet(name), ("127.0.0.1", port))
        .await
        .unwrap();
    let mut packet = vec![0; 1500];
    let (length, _) = tokio::time::timeout(limit, socket.recv_from(&mut packet))
        .await
        .ok()?
        .unwrap();
    let packet = &packet[..length];
    if packet[3] & 15 != 0 || length < 16 || u16::from_be_bytes([packet[6], packet[7]]) == 0 {
        return Some(None);
    }
    Some(Some(packet[length - 1]))
}

fn udp_entries(stub: &Stub) -> String {
    format!(
        "        - {{tag: a, addr: \"udp://127.0.0.1:{}\"}}",
        stub.port
    )
}

// ----- compile-level behavior -----------------------------------------------------

fn compile(tree: &Tree) -> Result<mosdns_native_host::CompiledConfig, String> {
    mosdns_native_host::load_and_compile(&tree.path("config.yaml"))
        .map_err(|error| error.to_string())
}

#[test]
fn references_overlay_and_sources_are_reported_and_resolved() {
    let tree = Tree::new("effective");
    let ports = ports();
    let stub = Stub::start(1, Duration::ZERO);
    config(
        &tree,
        &ports,
        &objects(
            "192.0.2.1:7890",
            &format!(
                "        - {{tag: a, addr: \"tcp://127.0.0.1:{}\", use_proxy: true}}\n        - {{tag: b, addr: \"udp://127.0.0.1:{}\"}}",
                stub.port, stub.port
            ),
        ),
    );
    let config = compile(&tree).unwrap();
    let item = |kind: &str, object: &str| {
        config
            .settings_report()
            .iter()
            .find(|item| item.kind == kind && item.object == object)
            .unwrap_or_else(|| panic!("{kind} {object} missing"))
            .clone()
    };
    assert_eq!(item("socks5_proxy", "main_proxy").value, "192.0.2.1:7890");
    assert_eq!(item("socks5_proxy", "main_proxy").source, "yaml");
    assert_eq!(item("ecs_address", "client_ecs").value, "192.0.2.77");
    assert_eq!(item("domain_output", "my_realiplist").value, "true");
    let upstreams = item("aliapi", "foreign");
    assert_eq!(upstreams.source, "yaml");
    assert!(upstreams.value.contains("a=tcp://"), "{}", upstreams.value);
    // The entry that opted in carries the plugin proxy; the other does not.
    let definition = config
        .forward_definitions
        .iter()
        .find(|definition| definition.tag == "foreign")
        .unwrap();
    assert_eq!(
        definition.entries[0].target.socks5.as_deref(),
        Some("192.0.2.1:7890")
    );
    assert_eq!(definition.entries[1].target.socks5, None);

    // Settings overlay: a proxy address, an ECS address, a replaced upstream
    // list and the generation switches.
    tree.write(
        "webinfo/settings.json",
        r#"{"version":1,
  "proxies":{"main_proxy":{"addr":"10.9.9.9:1080"}},
  "ecs":{"client_ecs":{"addr":"198.51.100.5"}},
  "upstreams":{"foreign":[
     {"tag":"x","enabled":true,"protocol":"tcp","addr":"127.0.0.1:5353","use_proxy":true},
     {"tag":"y","enabled":false,"protocol":"udp","addr":"127.0.0.1:5354"}]},
  "domain_generation":{"plugins":{"my_realiplist":{"enabled":false}}}}"#,
    );
    let config = compile(&tree).unwrap();
    let report = config.settings_report();
    let find = |kind: &str| {
        report
            .iter()
            .find(|item| item.kind == kind)
            .unwrap()
            .clone()
    };
    assert_eq!(
        (
            find("socks5_proxy").value.as_str(),
            find("socks5_proxy").source
        ),
        ("10.9.9.9:1080", "settings")
    );
    assert_eq!(
        (
            find("ecs_address").value.as_str(),
            find("ecs_address").source
        ),
        ("198.51.100.5", "settings")
    );
    assert_eq!(
        (
            find("domain_output").value.as_str(),
            find("domain_output").source
        ),
        ("false", "settings")
    );
    let aliapi = find("aliapi");
    assert_eq!(aliapi.source, "settings");
    assert_eq!(aliapi.value, "x=tcp://127.0.0.1:5353 [proxy]");
    let definition = config
        .forward_definitions
        .iter()
        .find(|definition| definition.tag == "foreign")
        .unwrap();
    assert_eq!(definition.entries.len(), 1, "disabled entries are not used");
    assert_eq!(
        definition.entries[0].target.socks5.as_deref(),
        Some("10.9.9.9:1080")
    );
}

#[test]
fn bad_references_and_bad_settings_are_errors_with_their_path() {
    let tree = Tree::new("errors");
    let ports = ports();
    let stub = Stub::start(1, Duration::ZERO);
    let good = udp_entries(&stub);
    let try_config = |extra: String| {
        config(&tree, &ports, &extra);
        compile(&tree)
            .err()
            .unwrap_or_else(|| panic!("must fail:\n{extra}"))
    };
    // use_proxy without a plugin proxy.
    let error = try_config(
        "  - tag: main_proxy\n    type: socks5_proxy\n    args: {addr: \"1.2.3.4:1\"}\n  - tag: foreign\n    type: aliapi\n    args:\n      upstreams:\n        - {tag: a, addr: \"tcp://127.0.0.1:1\", use_proxy: true}\n  - tag: my_realiplist\n    type: domain_output\n    args: {}\n"
            .to_owned(),
    );
    assert!(
        error.contains("use_proxy") && error.contains("proxy"),
        "{error}"
    );
    // Unknown and wrong-kind references.
    let error =
        try_config(objects("1.2.3.4:1", &good).replace("proxy: $main_proxy", "proxy: $nothing"));
    assert!(error.contains("unknown socks5_proxy `$nothing`"), "{error}");
    let error =
        try_config(objects("1.2.3.4:1", &good).replace("proxy: $main_proxy", "proxy: $client_ecs"));
    assert!(
        error.contains("unknown socks5_proxy `$client_ecs`"),
        "{error}"
    );
    // A literal proxy and a reference together.
    let error = try_config(objects("1.2.3.4:1", &good).replace(
        "proxy: $main_proxy",
        "proxy: $main_proxy\n      socks5: \"5.6.7.8:1\"",
    ));
    assert!(error.contains("literal socks5"), "{error}");
    // Invalid object values.
    let error = try_config(objects("not-an-address", &good));
    assert!(error.contains("host:port"), "{error}");
    // Quick ecs referencing nothing.
    let mut extra = objects("1.2.3.4:1", &good);
    extra.push_str("  - tag: other\n    type: sequence\n    args:\n      - exec: ecs $missing\n");
    let error = try_config(extra);
    assert!(error.contains("unknown ecs_address `$missing`"), "{error}");

    // Settings that name nothing, name the wrong kind, or are malformed.
    config(&tree, &ports, &objects("1.2.3.4:1", &good));
    for (settings, expected) in [
        (
            r#"{"version":1,"proxies":{"ghost":{"addr":"1.2.3.4:1"}}}"#,
            "no socks5_proxy `ghost`",
        ),
        (
            r#"{"version":1,"proxies":{"client_ecs":{"addr":"1.2.3.4:1"}}}"#,
            "not a socks5_proxy",
        ),
        (r#"{"version":2}"#, "unsupported settings version 2"),
        (r#"{"version":1,"surprise":true}"#, "invalid settings"),
        (
            r#"{"version":1,"proxies":{"main_proxy":{"addr":"x"}}}"#,
            "host:port",
        ),
        (
            r#"{"version":1,"upstreams":{"foreign":[{"tag":"a","enabled":true,"protocol":"aliapi"}]}}"#,
            "signed AliDNS",
        ),
        (
            r#"{"version":1,"upstreams":{"foreign":[{"tag":"a","enabled":true,"protocol":"udp","addr":"1.1.1.1","socks5":"1.2.3.4:1"}]}}"#,
            "literal socks5",
        ),
        (
            r#"{"version":1,"upstreams":{"foreign":[{"tag":"a","enabled":false,"protocol":"udp","addr":"1.1.1.1"}]}}"#,
            "must not be empty",
        ),
        (r"not json", "invalid settings"),
    ] {
        tree.write("webinfo/settings.json", settings);
        let error = compile(&tree)
            .err()
            .unwrap_or_else(|| panic!("must fail: {settings}"));
        assert!(error.contains(expected), "{settings}\n{error}");
    }
}

/// The fixed addresses of every compiled ECS policy.
fn ecs_presets(config: &mosdns_native_host::CompiledConfig) -> Vec<std::net::IpAddr> {
    config
        .response_policies
        .iter()
        .filter_map(|policy| match &policy.policy {
            mosdns_native_host::ResponsePolicy::Ecs(ecs) => ecs.preset,
            _ => None,
        })
        .collect()
}

#[test]
fn quick_ecs_references_resolve_and_follow_the_settings() {
    let tree = Tree::new("ecs");
    let ports = ports();
    let stub = Stub::start(1, Duration::ZERO);
    let mut extra = objects("1.2.3.4:1", &udp_entries(&stub));
    extra.push_str(
        "  - tag: cache_x\n    type: cache\n    args: {}\n  - tag: ecs_seq\n    type: sequence\n    args:\n      - exec: $cache_x\n      - exec: ecs $client_ecs\n      - exec: $foreign\n",
    );
    config(&tree, &ports, &extra);
    // A reference to a fixed address may follow a cache (M1-g), literal or not.
    let config = compile(&tree).unwrap();
    assert!(ecs_presets(&config).contains(&"192.0.2.77".parse().unwrap()));
    tree.write(
        "webinfo/settings.json",
        r#"{"version":1,"ecs":{"client_ecs":{"addr":"2001:db8:ec5::1"}}}"#,
    );
    let config = compile(&tree).unwrap();
    assert!(ecs_presets(&config).contains(&"2001:db8:ec5::1".parse().unwrap()));
}

#[test]
fn check_prints_the_effective_values_with_their_sources() {
    let tree = Tree::new("check");
    let ports = ports();
    let stub = Stub::start(1, Duration::ZERO);
    config(
        &tree,
        &ports,
        &objects("192.0.2.1:7890", &udp_entries(&stub)),
    );
    tree.write(
        "webinfo/settings.json",
        r#"{"version":1,"proxies":{"main_proxy":{"addr":"10.9.9.9:1080"}}}"#,
    );
    let before = tree.text("webinfo/settings.json");
    let report = mosdns_native_host::check_config(&tree.path("config.yaml")).unwrap();
    let text = report.to_string();
    assert!(
        text.contains("EFFECTIVE socks5_proxy main_proxy addr=10.9.9.9:1080 source=settings"),
        "{text}"
    );
    assert!(
        text.contains("EFFECTIVE ecs_address client_ecs addr=192.0.2.77 source=yaml"),
        "{text}"
    );
    assert!(
        text.contains("EFFECTIVE domain_output my_realiplist enabled=true source=yaml"),
        "{text}"
    );
    assert!(
        text.contains("SUMMARY gaps=0 cascades=0 globals=0"),
        "{text}"
    );
    // `check` never writes.
    assert_eq!(tree.text("webinfo/settings.json"), before);
    // A broken settings file is reported, not ignored.
    tree.write("webinfo/settings.json", r#"{"version":9}"#);
    let text = mosdns_native_host::check_config(&tree.path("config.yaml"))
        .unwrap()
        .to_string();
    assert!(
        text.contains("GLOBAL")
            && text.contains("webinfo/settings.json")
            && text.contains("unsupported settings version 9"),
        "{text}"
    );
}

// ----- the running host -----------------------------------------------------------

#[test]
fn an_upstream_change_takes_effect_for_the_next_query_and_survives_a_restart() {
    let tree = Tree::new("upstream");
    let ports = ports();
    let first = Stub::start(1, Duration::from_millis(900));
    let second = Stub::start(2, Duration::ZERO);
    config(&tree, &ports, &objects("127.0.0.1:9", &udp_entries(&first)));
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let tags = api(ports.api, "GET", "/api/v1/upstream/tags", "").await;
        assert_eq!(json(&tags.body), serde_json::json!(["foreign"]));
        let listed = json(&api(ports.api, "GET", "/api/v1/upstream/config", "").await.body);
        assert_eq!(listed["foreign"][0]["tag"], "a");
        assert_eq!(listed["foreign"][0]["use_socks_proxy"], false);
        assert_eq!(marker(ports.dns, "one.example").await, Some(1));

        // A query is in flight against the first upstream while the change applies.
        let slow = tokio::task::spawn_local({
            let dns = ports.dns;
            async move { marker(dns, "slow.example").await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let body = format!(
            r#"{{"plugin_tag":"foreign","upstreams":[{{"tag":"b","enabled":true,"protocol":"udp","addr":"127.0.0.1:{}","use_socks_proxy":false}}]}}"#,
            second.port
        );
        let saved = api(ports.api, "POST", "/api/v1/upstream/config", &body).await;
        assert_eq!(saved.status, 200, "{}", saved.body);
        assert_eq!(json(&saved.body)["message"], "Upstream configuration saved.");
        // The next query already uses the new generation; the old one finishes.
        assert_eq!(marker(ports.dns, "two.example").await, Some(2));
        assert_eq!(slow.await.unwrap(), Some(1), "an in-flight query keeps its upstream");
        // The settings file holds only the difference, the API shows it.
        let file = json(&tree.text("webinfo/settings.json"));
        assert_eq!(file["version"], 1);
        assert_eq!(file["upstreams"]["foreign"][0]["tag"], "b");
        assert!(file.get("proxies").is_none());
        let listed = json(&api(ports.api, "GET", "/api/v1/upstream/config", "").await.body);
        assert_eq!(listed["foreign"][0]["addr"], format!("127.0.0.1:{}", second.port));
        let runtime = json(&api(ports.api, "GET", "/api/v1/upstream/runtime/foreign", "").await.body);
        assert_eq!(runtime["tag"], "foreign");
        assert_eq!(runtime["override_config"][0]["tag"], "b");
        let effective = json(&api(ports.api, "GET", "/api/v1/settings/effective", "").await.body);
        assert!(effective["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["kind"] == "aliapi" && item["source"] == "settings"));
        shutdown.cancel();
        task.await.unwrap();
    });
    // A fresh process reads the file and ends up in the same state.
    let host = self::host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        assert_eq!(marker(ports.dns, "three.example").await, Some(2));
        // An empty list drops the override: back to the YAML upstream.
        let reset = api(
            ports.api,
            "POST",
            "/api/v1/upstream/config",
            r#"{"plugin_tag":"foreign","upstreams":[]}"#,
        )
        .await;
        assert_eq!(reset.status, 200, "{}", reset.body);
        assert_eq!(marker(ports.dns, "four.example").await, Some(1));
        assert!(
            json(&tree.text("webinfo/settings.json"))
                .get("upstreams")
                .is_none()
        );
        shutdown.cancel();
        task.await.unwrap();
    });
    assert!(first.count() >= 2 && second.count() >= 2);
}

#[test]
fn a_rejected_change_leaves_the_file_and_the_running_configuration_alone() {
    let tree = Tree::new("rejected");
    let ports = ports();
    let first = Stub::start(1, Duration::ZERO);
    config(&tree, &ports, &objects("127.0.0.1:9", &udp_entries(&first)));
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        for (body, status, reason) in [
            ("not json", 400, "invalid upstream configuration JSON"),
            (r#"{"upstreams":[]}"#, 400, "plugin_tag is required"),
            (r#"{"plugin_tag":"nothing","upstreams":[]}"#, 404, "upstream tag not found"),
            (
                r#"{"plugin_tag":"foreign","upstreams":[{"tag":"a","enabled":true,"protocol":"aliapi"}]}"#,
                400,
                "signed AliDNS",
            ),
            (
                r#"{"plugin_tag":"foreign","upstreams":[{"tag":"a","enabled":true,"protocol":"udp","addr":"1.1.1.1","so_mark":1}]}"#,
                400,
                "so_mark",
            ),
            (
                r#"{"plugin_tag":"foreign","upstreams":[{"tag":"a","enabled":false,"protocol":"udp","addr":"1.1.1.1"}]}"#,
                400,
                "must not be empty",
            ),
            (
                r#"{"plugin_tag":"foreign","upstreams":[{"tag":"a","enabled":true,"protocol":"tcp","addr":"127.0.0.1:1","use_socks_proxy":true}],"extra":1}"#,
                200,
                "saved",
            ),
        ] {
            let reply = api(ports.api, "POST", "/api/v1/upstream/config", body).await;
            assert_eq!(reply.status, status, "{body}: {}", reply.body);
            assert!(reply.body.contains(reason), "{body}: {}", reply.body);
            if status == 400 || status == 404 {
                assert_eq!(tree.text("webinfo/settings.json"), "", "{body}");
                assert_eq!(marker(ports.dns, "still.example").await, Some(1), "{body}");
            }
        }
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn a_failed_write_publishes_nothing() {
    let tree = Tree::new("writefail");
    let ports = ports();
    let first = Stub::start(1, Duration::ZERO);
    let second = Stub::start(2, Duration::ZERO);
    config(&tree, &ports, &objects("127.0.0.1:9", &udp_entries(&first)));
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        // `webinfo` becomes a file, so the settings file cannot be written.
        tree.write("webinfo", "not a directory");
        let body = format!(
            r#"{{"plugin_tag":"foreign","upstreams":[{{"tag":"b","enabled":true,"protocol":"udp","addr":"127.0.0.1:{}"}}]}}"#,
            second.port
        );
        let reply = api(ports.api, "POST", "/api/v1/upstream/config", &body).await;
        assert_eq!(reply.status, 500, "{}", reply.body);
        assert_eq!(marker(ports.dns, "old.example").await, Some(1));
        let listed = json(&api(ports.api, "GET", "/api/v1/upstream/config", "").await.body);
        assert_eq!(listed["foreign"][0]["tag"], "a");
        // The service recovers once the path is usable again.
        std::fs::remove_file(tree.path("webinfo")).unwrap();
        let reply = api(ports.api, "POST", "/api/v1/upstream/config", &body).await;
        assert_eq!(reply.status, 200, "{}", reply.body);
        assert_eq!(marker(ports.dns, "new.example").await, Some(2));
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn the_overrides_page_changes_the_proxy_for_the_next_query() {
    let tree = Tree::new("proxy");
    let ports = ports();
    let target = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let target_port = target.local_addr().unwrap().port();
    let relay = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let relay_port = relay.local_addr().unwrap().port();
    config(
        &tree,
        &ports,
        &objects(
            "127.0.0.1:9",
            &format!(
                "        - {{tag: a, addr: \"tcp://127.0.0.1:{target_port}\", use_proxy: true}}"
            ),
        ),
    );
    let host = host(&tree);
    host.block_on(async {
        serve_tcp(target, 7);
        let connects = Rc::new(Cell::new(0));
        serve_socks(relay, Rc::clone(&connects));
        let (shutdown, task) = serve(&host).await;
        let page = json(&api(ports.api, "GET", "/api/v1/overrides", "").await.body);
        assert_eq!(page["socks5"], "127.0.0.1:9");
        assert_eq!(page["ecs"], "192.0.2.77");
        assert_eq!(page["replacements"], serde_json::json!([]));
        // The proxy in the YAML is dead: the upstream fails, nothing connects.
        assert_eq!(marker(ports.dns, "a.example").await, None);
        assert_eq!(connects.get(), 0);

        let saved = api(
            ports.api,
            "POST",
            "/api/v1/overrides",
            &format!(r#"{{"socks5":"127.0.0.1:{relay_port}","ecs":"","replacements":[]}}"#),
        )
        .await;
        assert_eq!(saved.status, 200, "{}", saved.body);
        // TCP upstreams go through the new proxy at once.
        assert_eq!(marker(ports.dns, "b.example").await, Some(7));
        assert!(connects.get() >= 1);
        let file = json(&tree.text("webinfo/settings.json"));
        assert_eq!(
            file["proxies"]["main_proxy"]["addr"],
            format!("127.0.0.1:{relay_port}")
        );
        assert!(file.get("ecs").is_none());
        let page = json(&api(ports.api, "GET", "/api/v1/overrides", "").await.body);
        assert_eq!(page["socks5"], format!("127.0.0.1:{relay_port}"));

        // An empty value returns the field to its YAML default.
        api(
            ports.api,
            "POST",
            "/api/v1/overrides",
            r#"{"socks5":"","ecs":"","replacements":[]}"#,
        )
        .await;
        assert_eq!(marker(ports.dns, "c.example").await, None);
        assert!(
            json(&tree.text("webinfo/settings.json"))
                .get("proxies")
                .is_none()
        );

        // What this host cannot do is refused with its reason.
        let refused = api(
            ports.api,
            "POST",
            "/api/v1/overrides",
            r#"{"socks5":"","ecs":"","replacements":[{"original":"a","new":"b","comment":""}]}"#,
        )
        .await;
        assert_eq!(refused.status, 400);
        assert!(
            refused.body.contains("replacements are not supported"),
            "{}",
            refused.body
        );
        let bad = api(
            ports.api,
            "POST",
            "/api/v1/overrides",
            r#"{"socks5":"no-port","ecs":"","replacements":[]}"#,
        )
        .await;
        assert_eq!(bad.status, 400);
        assert!(bad.body.contains("host:port"), "{}", bad.body);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn several_objects_of_one_kind_make_the_overrides_page_ambiguous() {
    let tree = Tree::new("ambiguous");
    let ports = ports();
    let first = Stub::start(1, Duration::ZERO);
    let mut extra = objects("127.0.0.1:9", &udp_entries(&first));
    extra.push_str(
        "  - tag: second_proxy\n    type: socks5_proxy\n    args: {addr: \"127.0.0.1:10\"}\n",
    );
    config(&tree, &ports, &extra);
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let page = api(ports.api, "GET", "/api/v1/overrides", "").await;
        assert_eq!(page.status, 409);
        assert!(page.body.contains("ambiguous"), "{}", page.body);
        let post = api(
            ports.api,
            "POST",
            "/api/v1/overrides",
            r#"{"socks5":"127.0.0.1:11","ecs":"","replacements":[]}"#,
        )
        .await;
        assert_eq!(post.status, 409);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn domain_generation_switches_follow_the_reference_shape() {
    let tree = Tree::new("generation");
    let ports = ports();
    let first = Stub::start(1, Duration::ZERO);
    config(&tree, &ports, &objects("127.0.0.1:9", &udp_entries(&first)));
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let page = json(&api(ports.api, "GET", "/api/v1/domain-generation", "").await.body);
        assert_eq!(
            page,
            serde_json::json!({"enabled":true,"remember_direct":true,"remember_proxy":true,"no_v4":true,"no_v6":true})
        );
        // Recording works while the switch is on.
        assert_eq!(marker(ports.dns, "seen.example").await, Some(1));
        let listed = api(ports.api, "GET", "/plugins/my_realiplist/show", "").await;
        assert!(listed.body.contains("seen.example"), "{}", listed.body);

        // Turning `remember_direct` off flushes that list and stops recording.
        let reply = api(
            ports.api,
            "POST",
            "/api/v1/domain-generation",
            r#"{"remember_direct":false}"#,
        )
        .await;
        assert_eq!(reply.status, 200, "{}", reply.body);
        let reply = json(&reply.body);
        assert_eq!(reply["settings"]["remember_direct"], false);
        assert_eq!(reply["settings"]["enabled"], true);
        assert_eq!(reply["flush_targets"], serde_json::json!(["my_realiplist"]));
        assert_eq!(
            api(ports.api, "GET", "/plugins/my_realiplist/show", "").await.body,
            "",
            "the list was flushed"
        );
        assert_eq!(marker(ports.dns, "unseen.example").await, Some(1));
        assert_eq!(
            api(ports.api, "GET", "/plugins/my_realiplist/show", "").await.body,
            "",
            "nothing is recorded while off"
        );
        let file = json(&tree.text("webinfo/settings.json"));
        assert_eq!(file["domain_generation"]["plugins"]["my_realiplist"]["enabled"], false);

        // Turning it on again records again; the master switch gates everything.
        api(ports.api, "POST", "/api/v1/domain-generation", r#"{"remember_direct":true}"#).await;
        assert_eq!(marker(ports.dns, "again.example").await, Some(1));
        assert!(api(ports.api, "GET", "/plugins/my_realiplist/show", "").await.body.contains("again.example"));
        let master = json(
            &api(ports.api, "POST", "/api/v1/domain-generation", r#"{"enabled":false}"#)
                .await
                .body,
        );
        assert_eq!(master["flush_targets"], serde_json::json!(["my_realiplist"]));
        assert_eq!(
            json(&api(ports.api, "GET", "/api/v1/domain-generation", "").await.body)["enabled"],
            false
        );
        assert_eq!(marker(ports.dns, "gated.example").await, Some(1));
        assert_eq!(api(ports.api, "GET", "/plugins/my_realiplist/show", "").await.body, "");
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn the_capability_matrix_lists_what_the_configuration_can_do() {
    let tree = Tree::new("capabilities");
    let ports = ports();
    let first = Stub::start(1, Duration::ZERO);
    config(&tree, &ports, &objects("127.0.0.1:9", &udp_entries(&first)));
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let capabilities = json(&api(ports.api, "GET", "/api/v1/capabilities", "").await.body);
        let operations = &capabilities["ui_operations"];
        for id in [
            "upstreams.read",
            "upstreams.manage",
            "system.global_overrides",
            "system.domain_generation",
        ] {
            assert_eq!(operations[id]["supported"], true, "{id}");
        }
        assert_eq!(operations["groups.upstreams"]["supported"], false);
        assert_eq!(capabilities["endpoints"]["upstream"]["config_post"], true);
        let unsupported = capabilities["unsupported_features"].as_array().unwrap();
        assert!(
            !unsupported.iter().any(|name| name == "socks_proxy"),
            "the settings-backed page carries the proxy switch: {unsupported:?}"
        );
        assert!(
            unsupported
                .iter()
                .any(|name| name == "so_mark" || name == "socket_marks")
        );
        shutdown.cancel();
        task.await.unwrap();
    });
    // Without the objects the pages are not offered.
    let other = Tree::new("capabilities-bare");
    let ports = self::ports();
    other.write(
        "config.yaml",
        &format!(
            "log: {{level: error}}\napi: {{http: \"127.0.0.1:{}\"}}\nplugins:\n  - tag: f\n    type: forward\n    args: {{upstreams: [{{addr: \"udp://127.0.0.1:{}\"}}]}}\n  - tag: entry\n    type: sequence\n    args: [{{exec: $f}}]\n  - tag: listener\n    type: udp_server\n    args: {{entry: entry, listen: \"127.0.0.1:{}\"}}\n",
            ports.api, first.port, ports.dns
        ),
    );
    let host = self::host(&other);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let capabilities = json(&api(ports.api, "GET", "/api/v1/capabilities", "").await.body);
        let operations = &capabilities["ui_operations"];
        for id in [
            "upstreams.manage",
            "system.global_overrides",
            "system.domain_generation",
        ] {
            assert_eq!(operations[id]["supported"], false, "{id}");
        }
        let unsupported = capabilities["unsupported_features"].as_array().unwrap();
        assert!(unsupported.iter().any(|name| name == "socks_proxy"));
        shutdown.cancel();
        task.await.unwrap();
    });
    // A group without a plugin-level proxy cannot take `use_proxy`: the page
    // is not offered the switch.
    let noproxy = Tree::new("capabilities-noproxy");
    let ports = self::ports();
    config(
        &noproxy,
        &ports,
        &objects("127.0.0.1:9", &udp_entries(&first)).replace("      proxy: $main_proxy\n", ""),
    );
    let host = self::host(&noproxy);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let capabilities = json(&api(ports.api, "GET", "/api/v1/capabilities", "").await.body);
        assert_eq!(
            capabilities["ui_operations"]["upstreams.manage"]["supported"],
            true
        );
        let unsupported = capabilities["unsupported_features"].as_array().unwrap();
        assert!(unsupported.iter().any(|name| name == "socks_proxy"));
        shutdown.cancel();
        task.await.unwrap();
    });
    let _ = Instant::now();
}

#[test]
fn the_fixture_in_the_new_syntax_checks_clean_with_effective_values_and_sources() {
    let config =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/settings/config.yaml");
    let report = mosdns_native_host::check_config(&config).unwrap();
    let text = report.to_string();
    assert!(
        text.contains("SUMMARY gaps=0 cascades=0 globals=0"),
        "{text}"
    );
    for line in [
        "EFFECTIVE socks5_proxy main_proxy addr=192.0.2.9:7890 source=settings",
        "EFFECTIVE ecs_address client_ecs addr=2001:db8:ec5::1 source=yaml",
        "EFFECTIVE aliapi domestic upstreams=ali=udp://223.6.6.6 source=settings",
        "EFFECTIVE aliapi foreign upstreams=cf=tcp://1.1.1.1 [proxy], g=tls://dns.google dial=8.8.8.8 [proxy], direct=udp://9.9.9.9 source=yaml",
        "EFFECTIVE domain_output my_realiplist enabled=false source=settings",
    ] {
        assert!(text.contains(line), "{line}\n{text}");
    }
}

#[test]
fn rule_sources_take_the_proxy_by_reference() {
    let tree = Tree::new("sources");
    let ports = ports();
    let stub = Stub::start(1, Duration::ZERO);
    let mut extra = objects("1.2.3.4:1", &udp_entries(&stub));
    extra.push_str("  - tag: geosite\n    type: sd_set_light\n    args: {proxy: $main_proxy, local_config: srs/geosite.json}\n");
    tree.write("srs/geosite.json", "[]");
    config(&tree, &ports, &extra);
    assert!(compile(&tree).is_ok());
    let conflict = extra.replace(
        "proxy: $main_proxy, local_config",
        "proxy: $main_proxy, socks5: \"5.6.7.8:1\", local_config",
    );
    config(&tree, &ports, &conflict);
    let error = compile(&tree).err().unwrap();
    assert!(error.contains("literal socks5"), "{error}");
    let unknown = extra.replace(
        "proxy: $main_proxy, local_config",
        "proxy: $nowhere, local_config",
    );
    config(&tree, &ports, &unknown);
    assert!(
        compile(&tree)
            .err()
            .unwrap()
            .contains("unknown socks5_proxy `$nowhere`")
    );
}

#[test]
fn a_changed_ecs_address_empties_the_dumps_of_the_caches_that_depend_on_it() {
    let tree = Tree::new("dump");
    let ports = ports();
    let stub = Stub::start(1, Duration::ZERO);
    let mut extra = objects("1.2.3.4:1", &udp_entries(&stub));
    extra.push_str(
        "  - tag: cache_x\n    type: cache\n    args: {dump_file: cache/x.gz}\n  - tag: cached\n    type: sequence\n    args:\n      - exec: $cache_x\n      - exec: ecs $client_ecs\n      - exec: $foreign\n",
    );
    config(&tree, &ports, &extra);
    // The entry sequence of `config` runs `$foreign`; route the listener through `cached`.
    let yaml = tree
        .text("config.yaml")
        .replace("entry: entry", "entry: cached");
    tree.write("config.yaml", &yaml);
    std::fs::create_dir_all(tree.path("cache")).unwrap();
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        assert_eq!(marker(ports.dns, "dump.example").await, Some(1));
        assert_eq!(
            api(ports.api, "GET", "/plugins/cache_x/save", "")
                .await
                .status,
            200
        );
        let filled = std::fs::read(tree.path("cache/x.gz")).unwrap();
        let reply = api(
            ports.api,
            "POST",
            "/api/v1/overrides",
            r#"{"socks5":"","ecs":"198.51.100.9","replacements":[]}"#,
        )
        .await;
        assert_eq!(reply.status, 200, "{}", reply.body);
        // The new generation's cache is empty and so is its dump on disk.
        let emptied = std::fs::read(tree.path("cache/x.gz")).unwrap();
        assert!(
            emptied.len() < filled.len(),
            "{} !< {}",
            emptied.len(),
            filled.len()
        );
        assert_eq!(marker(ports.dns, "dump.example").await, Some(1));
        assert_eq!(
            stub.count(),
            2,
            "the answer was asked for again, not served from the old cache"
        );
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn no_query_is_lost_while_a_change_applies() {
    let tree = Tree::new("noloss");
    let ports = ports();
    let first = Stub::start(1, Duration::ZERO);
    config(&tree, &ports, &objects("127.0.0.1:9", &udp_entries(&first)));
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        for round in 0..5_u16 {
            let dns = ports.dns;
            let load = tokio::task::spawn_local(async move {
                let mut lost = Vec::new();
                for i in 0..400_u32 {
                    let name = format!("q{round}-{i}.example");
                    let sent = Instant::now();
                    if marker_within(dns, &name, Duration::from_secs(3))
                        .await
                        .is_none()
                    {
                        lost.push((name, sent.elapsed()));
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                lost
            });
            tokio::time::sleep(Duration::from_millis(100)).await;
            let body = format!(
                r#"{{"socks5":"127.0.0.1:{}","ecs":"","replacements":[]}}"#,
                30_000 + round
            );
            assert_eq!(
                api(ports.api, "POST", "/api/v1/overrides", &body)
                    .await
                    .status,
                200
            );
            let lost = load.await.unwrap();
            assert!(lost.is_empty(), "round {round}: lost {lost:?}");
        }
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn a_literal_proxy_cannot_mix_with_proxy_references_or_settings() {
    let tree = Tree::new("mixing");
    let ports = ports();
    let stub = Stub::start(1, Duration::ZERO);
    let good = udp_entries(&stub);
    let group = |plugin_args: &str, entry: &str| {
        format!(
            "  - tag: main_proxy\n    type: socks5_proxy\n    args: {{addr: \"1.2.3.4:1\"}}\n  - tag: foreign\n    type: aliapi\n    args:\n{plugin_args}      upstreams:\n        - {entry}\n  - tag: my_realiplist\n    type: domain_output\n    args: {{}}\n"
        )
    };
    let try_config = |extra: String| {
        config(&tree, &ports, &extra);
        compile(&tree)
            .err()
            .unwrap_or_else(|| panic!("must fail:\n{extra}"))
    };
    // use_proxy: false next to a literal address is still a mix.
    let error = try_config(group(
        "      socks5: \"5.6.7.8:1\"\n",
        "{tag: a, addr: \"tcp://127.0.0.1:1\", use_proxy: false}",
    ));
    assert!(error.contains("literal"), "{error}");
    let error = try_config(group(
        "",
        "{tag: a, addr: \"tcp://127.0.0.1:1\", socks5: \"5.6.7.8:1\", use_proxy: false}",
    ));
    assert!(error.contains("literal"), "{error}");
    // An entry reference next to a plugin literal address.
    let error = try_config(group(
        "      socks5: \"5.6.7.8:1\"\n",
        "{tag: a, addr: \"tcp://127.0.0.1:1\", proxy: $main_proxy}",
    ));
    assert!(error.contains("literal"), "{error}");
    // Stored upstreams cannot inherit a literal plugin address.
    config(
        &tree,
        &ports,
        &group(
            "      socks5: \"5.6.7.8:1\"\n",
            "{tag: a, addr: \"tcp://127.0.0.1:1\"}",
        ),
    );
    assert!(compile(&tree).is_ok(), "a plain literal still compiles");
    tree.write(
        "webinfo/settings.json",
        r#"{"version":1,"upstreams":{"foreign":[{"tag":"b","enabled":true,"protocol":"tcp","addr":"127.0.0.1:2","use_proxy":false}]}}"#,
    );
    let error = compile(&tree).err().expect("must fail");
    assert!(error.contains("literal"), "{error}");
    drop(good);
}

#[test]
fn only_differences_are_stored() {
    let tree = Tree::new("differences");
    let ports = ports();
    let first = Stub::start(1, Duration::ZERO);
    let second = Stub::start(2, Duration::ZERO);
    config(&tree, &ports, &objects("127.0.0.1:9", &udp_entries(&first)));
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let original = api(ports.api, "GET", "/api/v1/upstream/config", "").await.body;
        // The page's entry shape: the reference field names only.
        let first = &json(&original)["foreign"][0];
        assert!(first.get("use_proxy").is_none(), "{first}");
        assert!(first["use_socks_proxy"].is_boolean(), "{first}");
        let changed = format!(
            r#"{{"plugin_tag":"foreign","upstreams":[{{"tag":"b","enabled":true,"protocol":"udp","addr":"127.0.0.1:{}"}}]}}"#,
            second.port
        );
        assert_eq!(
            api(ports.api, "POST", "/api/v1/upstream/config", &changed).await.status,
            200
        );
        assert!(json(&tree.text("webinfo/settings.json")).get("upstreams").is_some());
        // Saving what the YAML says drops the stored list.
        let yaml_list = json(&original)["foreign"].clone();
        let restore = serde_json::json!({"plugin_tag": "foreign", "upstreams": yaml_list});
        let reply = api(ports.api, "POST", "/api/v1/upstream/config", &restore.to_string()).await;
        assert_eq!(reply.status, 200, "{}", reply.body);
        assert!(
            json(&tree.text("webinfo/settings.json")).get("upstreams").is_none(),
            "{}",
            tree.text("webinfo/settings.json")
        );
        assert_eq!(marker(ports.dns, "back.example").await, Some(1));

        // The same for the global overrides.
        api(ports.api, "POST", "/api/v1/overrides", r#"{"socks5":"127.0.0.1:10","ecs":"192.0.2.78"}"#).await;
        let file = json(&tree.text("webinfo/settings.json"));
        assert!(file.get("proxies").is_some() && file.get("ecs").is_some());
        api(ports.api, "POST", "/api/v1/overrides", r#"{"socks5":"127.0.0.1:9","ecs":"192.0.2.77"}"#).await;
        let file = json(&tree.text("webinfo/settings.json"));
        assert!(file.get("proxies").is_none() && file.get("ecs").is_none(), "{file}");
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn a_generation_list_the_yaml_disables_is_reported_off() {
    let tree = Tree::new("yaml-off");
    let ports = ports();
    let first = Stub::start(1, Duration::ZERO);
    config(
        &tree,
        &ports,
        &objects("127.0.0.1:9", &udp_entries(&first)).replace(
            "args: {file_stat: gen/real.txt, dump_interval: 3600}",
            "args: {file_stat: gen/real.txt, dump_interval: 3600, enabled: false}",
        ),
    );
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let page = json(
            &api(ports.api, "GET", "/api/v1/domain-generation", "")
                .await
                .body,
        );
        assert_eq!(page["remember_direct"], false, "{page}");
        // Nothing is recorded, and turning it on is a difference worth storing.
        assert_eq!(marker(ports.dns, "off.example").await, Some(1));
        assert_eq!(
            api(ports.api, "GET", "/plugins/my_realiplist/show", "")
                .await
                .body,
            ""
        );
        let reply = api(
            ports.api,
            "POST",
            "/api/v1/domain-generation",
            r#"{"remember_direct":true}"#,
        )
        .await;
        assert_eq!(reply.status, 200, "{}", reply.body);
        let file = json(&tree.text("webinfo/settings.json"));
        assert_eq!(
            file["domain_generation"]["plugins"]["my_realiplist"]["enabled"],
            true
        );
        assert_eq!(marker(ports.dns, "on.example").await, Some(1));
        assert!(
            api(ports.api, "GET", "/plugins/my_realiplist/show", "")
                .await
                .body
                .contains("on.example")
        );
        // Turning it off again equals the YAML and removes the setting.
        api(
            ports.api,
            "POST",
            "/api/v1/domain-generation",
            r#"{"remember_direct":false}"#,
        )
        .await;
        assert!(
            json(&tree.text("webinfo/settings.json"))
                .get("domain_generation")
                .is_none(),
            "{}",
            tree.text("webinfo/settings.json")
        );
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn an_empty_proxy_address_means_no_proxy() {
    let tree = Tree::new("empty-proxy");
    let ports = ports();
    let stub = Stub::start(1, Duration::ZERO);
    config(
        &tree,
        &ports,
        &objects(
            "",
            &format!(
                "        - {{tag: a, addr: \"tcp://127.0.0.1:{}\", use_proxy: true}}",
                stub.port
            ),
        ),
    );
    let compiled = compile(&tree).expect("an empty proxy address compiles");
    let proxy = compiled
        .settings_report()
        .iter()
        .find(|item| item.kind == "socks5_proxy")
        .unwrap()
        .clone();
    assert_eq!((proxy.value.as_str(), proxy.source), ("", "yaml"));
    let definition = compiled
        .forward_definitions
        .iter()
        .find(|definition| definition.tag == "foreign")
        .unwrap();
    assert_eq!(
        definition.entries[0].target.socks5.as_deref().unwrap_or(""),
        ""
    );
    // A settings entry may fill it in, and may go back to empty.
    tree.write(
        "webinfo/settings.json",
        r#"{"version":1,"proxies":{"main_proxy":{"addr":"127.0.0.1:9"}}}"#,
    );
    let compiled = compile(&tree).unwrap();
    let proxy = compiled
        .settings_report()
        .iter()
        .find(|item| item.kind == "socks5_proxy")
        .unwrap()
        .clone();
    assert_eq!(
        (proxy.value.as_str(), proxy.source),
        ("127.0.0.1:9", "settings")
    );
    tree.write(
        "webinfo/settings.json",
        r#"{"version":1,"proxies":{"main_proxy":{"addr":""}}}"#,
    );
    assert!(compile(&tree).is_ok());
    // A bad address is still refused; the other object kind still needs one.
    tree.write(
        "webinfo/settings.json",
        r#"{"version":1,"proxies":{"main_proxy":{"addr":"nope"}}}"#,
    );
    assert!(compile(&tree).err().unwrap().contains("host:port"));
}

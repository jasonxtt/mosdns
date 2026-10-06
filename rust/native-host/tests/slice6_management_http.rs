//! Slice 1 of the local-rule editing workflow: the scoped management HTTP
//! surface and the top-level host supervisor that owns DNS and HTTP together.
//!
//! These tests drive a real HTTP client against the host-owned listener and a
//! real DNS client against the host-owned UDP listener. No router, handler or
//! response object is mocked.

use std::fs;
use std::net::{SocketAddr, TcpListener as StdTcpListener, UdpSocket as StdUdpSocket};
use std::path::PathBuf;
use std::time::Duration;

use mosdns_native_host::{HostAssembly, HostOptions, load_and_compile};
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// One owned fixture directory holding a config and its real rule files.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .expect("canonical temporary directory")
            .join(format!("mosdns-http-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("rules")).expect("fixture directory");
        Self { root }
    }

    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("fixture parent");
        }
        fs::write(&path, contents).expect("fixture file");
        path
    }

    fn config(&self) -> PathBuf {
        self.root.join("config.yaml")
    }

    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.root.join(relative)).expect("fixture read")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A fixture with a managed `blocklist`, a query-only `exps_only` set and a
/// management listener on a probed free port. The native config rejects port
/// 0, so the harness probes a concrete port first.
fn fixture(name: &str, dns_port: u16, api_port: u16) -> Fixture {
    let fixture = Fixture::new(name);
    fixture.write("rules/blocklist.txt", "a.example\nb.example\n");
    let dns_listen = format!("127.0.0.1:{dns_port}");
    let api_listen = format!("127.0.0.1:{api_port}");
    let yaml = format!(
        r#"log:
  level: error
api:
  http: "{api_listen}"
plugins:
  - tag: sequence_main
    type: sequence
    args:
      - matches: qname $blocklist
        exec: reject 3
      - exec: $forward_main
  - tag: blocklist
    type: domain_set
    args:
      files:
        - rules/blocklist.txt
  - tag: exps_only
    type: domain_set
    args:
      exps:
        - exps.example
  - tag: forward_main
    type: forward
    args:
      upstreams:
        - addr: "udp://127.0.0.1:25999"
  - tag: listener
    type: udp_server
    args:
      entry: sequence_main
      listen: "{dns_listen}"
      enable_audit: false
"#
    );
    fixture.write("config.yaml", &yaml);
    fixture
}

fn assembly_for(fixture: &Fixture) -> HostAssembly {
    HostAssembly::from_config(load_and_compile(&fixture.config()).expect("fixture config"))
        .expect("fixture assembly")
}

fn audit_assembly_for(fixture: &Fixture) -> HostAssembly {
    let yaml = fixture
        .read("config.yaml")
        .replace("enable_audit: false", "enable_audit: true");
    fixture.write("config.yaml", &yaml);
    HostAssembly::with_options_and_state_root(
        load_and_compile(&fixture.config()).expect("audit fixture config"),
        HostOptions::default(),
        &fixture.root,
    )
    .expect("audit fixture assembly")
}

/// A minimal DNS query for one name.
fn dns_query(id: u16, labels: &[&str]) -> Vec<u8> {
    let mut packet = Vec::from([
        u8::try_from(id >> 8).expect("high ID byte"),
        u8::try_from(id & 0x00ff).expect("low ID byte"),
        0x01,
        0x00,
        0x00,
        0x01,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
    ]);
    for label in labels {
        packet.push(u8::try_from(label.len()).expect("DNS label length"));
        packet.extend_from_slice(label.as_bytes());
    }
    packet.extend_from_slice(&[0, 0, 1, 0, 1]);
    packet
}

fn udp_query(listener: SocketAddr, request: &[u8]) -> Vec<u8> {
    let socket = StdUdpSocket::bind("127.0.0.1:0").expect("client bind");
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("client timeout");
    socket.send_to(request, listener).expect("client send");
    let mut response = vec![0_u8; 65535];
    let (length, _) = socket.recv_from(&mut response).expect("client response");
    response[..length].to_vec()
}

fn rcode(response: &[u8]) -> u16 {
    u16::from_be_bytes([response[2], response[3]]) & 0x000f
}

/// One parsed HTTP response.
#[derive(Debug)]
struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Sends one complete HTTP/1.1 request and reads the response until the server
/// closes the connection.
async fn http_request(address: SocketAddr, request: &str) -> HttpResponse {
    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("http connect");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("http write");
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.expect("http read");
    parse_http_response(&bytes)
}

fn parse_http_response(bytes: &[u8]) -> HttpResponse {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let (head, body) = text
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("malformed HTTP response: {text:?}"));
    let mut lines = head.split("\r\n");
    let status_line = lines.next().expect("status line");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .expect("status code")
        .parse()
        .expect("numeric status");
    let headers = lines
        .filter_map(|line| {
            line.split_once(':')
                .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        })
        .collect();
    HttpResponse {
        status,
        headers,
        body: body.to_owned(),
    }
}

fn get(path: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: native\r\nConnection: close\r\n\r\n")
}

fn get_with_headers(path: &str, headers: &[(&str, &str)]) -> String {
    use std::fmt::Write as _;
    let mut extra = String::new();
    for (key, value) in headers {
        write!(&mut extra, "{key}: {value}\r\n").expect("write request header");
    }
    format!("GET {path} HTTP/1.1\r\nHost: native\r\n{extra}Connection: close\r\n\r\n")
}

fn raw(method: &str, path: &str) -> String {
    format!("{method} {path} HTTP/1.1\r\nHost: native\r\nConnection: close\r\n\r\n")
}

fn post(path: &str, body: &str) -> String {
    format!(
        "POST {path} HTTP/1.1\r\nHost: native\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn post_with_headers(path: &str, body: &str, headers: &[(&str, &str)]) -> String {
    use std::fmt::Write as _;
    let mut extra = String::new();
    for (key, value) in headers {
        write!(&mut extra, "{key}: {value}\r\n").expect("write request header");
    }
    format!(
        "POST {path} HTTP/1.1\r\nHost: native\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn switch_fixture(name: &str, dns_port: u16, api_port: u16) -> Fixture {
    let fixture = Fixture::new(name);
    fixture.write("state/custom switch.txt", " A \n");
    let yaml = format!(
        r#"log:
  level: error
api:
  http: "127.0.0.1:{api_port}"
plugins:
  - tag: sequence_main
    type: sequence
    args:
      - matches: switch3 A
        exec: reject 3
      - exec: $forward_main
  - tag: "custom switch"
    type: switch3
    args:
      initial_value: "state/custom switch.txt"
  - tag: forward_main
    type: forward
    args:
      upstreams:
        - addr: "udp://127.0.0.1:25999"
  - tag: listener
    type: udp_server
    args:
      entry: sequence_main
      listen: "127.0.0.1:{dns_port}"
      enable_audit: false
"#
    );
    fixture.write("config.yaml", &yaml);
    fixture
}

/// A port that was just free; used to prove a failed bind released the other
/// listener.
fn free_tcp_port() -> u16 {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("probe bind");
    let port = listener.local_addr().expect("probe address").port();
    drop(listener);
    port
}

fn free_udp_port() -> u16 {
    let socket = StdUdpSocket::bind("127.0.0.1:0").expect("probe bind");
    let port = socket.local_addr().expect("probe address").port();
    drop(socket);
    port
}

#[test]
fn management_routes_match_the_go_visible_contract() {
    let fixture = fixture("routes", free_udp_port(), free_tcp_port());
    let assembly = assembly_for(&fixture);
    let bound = assembly
        .block_on(assembly.bind_host())
        .expect("bound host listeners");
    let api = bound.api_addr().expect("management listener address");
    let shutdown = TransportCancellation::new();

    let result = assembly.block_on(async {
        let task = tokio::task::spawn_local(bound.serve(shutdown.clone()));

        // `/show` returns accepted rules, one per line, with Go's content type.
        let show = http_request(api, &get("/plugins/blocklist/show")).await;
        assert_eq!(show.status, 200, "{show:?}");
        assert_eq!(
            show.header("content-type"),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(show.body, "a.example\nb.example\n");

        // The UI sends `?limit=10000`; Go ignores it and so must the native
        // route.
        let limited = http_request(api, &get("/plugins/blocklist/show?limit=10000")).await;
        assert_eq!(limited.status, 200);
        assert_eq!(limited.body, "a.example\nb.example\n");

        // The strict fixture configures no special groups, so `[]` is the
        // true state, not a stand-in for group management.
        let groups = http_request(api, &get("/api/v1/special-groups")).await;
        assert_eq!(groups.status, 200);
        assert_eq!(groups.header("content-type"), Some("application/json"));
        assert_eq!(groups.body, "[]\n");

        // POST validates each value, skips the invalid one, persists, publishes
        // and reports the accepted count.
        let posted = http_request(
            api,
            &post(
                "/plugins/blocklist/post",
                r#"{"values":["c.example","regexp:["]}"#,
            ),
        )
        .await;
        assert_eq!(posted.status, 200, "{posted:?}");
        assert_eq!(posted.body, "domain_set replaced with 1 entries");
        let after = http_request(api, &get("/plugins/blocklist/show")).await;
        assert_eq!(after.body, "c.example\n");
        assert_eq!(fixture.read("rules/blocklist.txt"), "c.example\n");

        // GET `/save` persists the current generation with an empty 200 body.
        let saved = http_request(api, &get("/plugins/blocklist/save")).await;
        assert_eq!(saved.status, 200);
        assert_eq!(saved.body, "");

        // Malformed JSON keeps the previous file and generation.
        let invalid = http_request(api, &post("/plugins/blocklist/post", "{")).await;
        assert_eq!(invalid.status, 400);
        assert_eq!(invalid.body, "invalid JSON\n");
        assert_eq!(
            fixture.read("rules/blocklist.txt"),
            "c.example\n",
            "an invalid POST must not touch the file"
        );

        // Unknown tags and unknown paths fail explicitly.
        let unknown = http_request(api, &get("/plugins/absent/show")).await;
        assert_eq!(unknown.status, 404);
        assert_eq!(unknown.body, "404 page not found\n");
        let unknown_path = http_request(api, &get("/nope")).await;
        assert_eq!(unknown_path.status, 404);

        // A configured but ineligible profile is never a management target.
        let ineligible = http_request(api, &get("/plugins/exps_only/show")).await;
        assert_eq!(ineligible.status, 400, "{ineligible:?}");
        assert!(
            ineligible.body.contains("exps_only"),
            "the ineligible tag must be named: {}",
            ineligible.body
        );
        let ineligible_post =
            http_request(api, &post("/plugins/exps_only/post", r#"{"values":[]}"#)).await;
        assert_eq!(ineligible_post.status, 400);

        // A registered path with a wrong method is 405.
        let wrong_method = http_request(api, &post("/plugins/blocklist/show", "")).await;
        assert_eq!(wrong_method.status, 405, "{wrong_method:?}");

        shutdown.cancel();
        task.await.expect("supervisor task")
    });

    result.expect("supervisor shutdown is clean");
}

#[allow(clippy::too_many_lines)]
#[test]
fn configured_switch_http_uses_inventory_tags_and_strict_value_inputs() {
    let fixture = switch_fixture("switch-http", free_udp_port(), free_tcp_port());
    let assembly = assembly_for(&fixture);
    let bound = assembly
        .block_on(assembly.bind_host())
        .expect("bound host listeners");
    let api = bound.api_addr().expect("management listener address");
    let shutdown = TransportCancellation::new();

    let result = assembly.block_on(async {
        let task = tokio::task::spawn_local(bound.serve(shutdown.clone()));

        let capabilities = http_request(api, &get("/api/v1/capabilities")).await;
        assert_eq!(capabilities.status, 200);
        let capabilities: serde_json::Value =
            serde_json::from_str(&capabilities.body).expect("capability JSON");
        assert_eq!(capabilities["ui_operations"]["switches.manage"]["supported"], true);
        assert_eq!(capabilities["switches"]["schema_version"], 1);
        assert_eq!(capabilities["switches"]["config_generation"], "0");
        assert_eq!(capabilities["switches"]["instances"][0]["type"], "switch3");
        assert_eq!(capabilities["switches"]["instances"][0]["tag"], "custom switch");
        assert_eq!(capabilities["switches"]["instances"][0]["readable"], true);
        assert_eq!(capabilities["switches"]["instances"][0]["writable"], true);

        let encoded = http_request(api, &get("/plugins/custom%20switch/show")).await;
        assert_eq!(encoded.status, 200, "{encoded:?}");
        assert_eq!(encoded.body, "A");

        let custom = http_request(
            api,
            &post_with_headers(
                "/plugins/custom%20switch/post",
                r#"{"value":"custom value"}"#,
                &[
                    ("Content-Type", "Application/JSON; charset=UTF-8"),
                    ("X-Mosdns-Config-Generation", "0"),
                ],
            ),
        )
        .await;
        assert_eq!(custom.status, 200, "{custom:?}");
        assert_eq!(custom.body, "updated to: custom value\n");
        assert_eq!(fixture.read("state/custom switch.txt"), "custom value");
        assert_eq!(
            http_request(api, &get("/plugins/custom%20switch/show")).await.body,
            "custom value"
        );

        let form = http_request(
            api,
            &post_with_headers(
                "/plugins/custom%20switch/post",
                "value=",
                &[("Content-Type", "application/x-www-form-urlencoded; charset=utf-8")],
            ),
        )
        .await;
        assert_eq!(form.status, 200, "{form:?}");
        assert_eq!(form.body, "updated to: \n");
        assert_eq!(fixture.read("state/custom switch.txt"), "");

        let legacy_json = http_request(
            api,
            &post_with_headers(
                "/plugins/custom%20switch/post",
                r#"{"Value":"B"}"#,
                &[],
            ),
        )
        .await;
        assert_eq!(legacy_json.status, 200, "{legacy_json:?}");
        assert_eq!(fixture.read("state/custom switch.txt"), "B");

        for (body, expected) in [
            (r#"{"value":"A","Value":"B"}"#, 400),
            (r#"{"value":1}"#, 400),
        ] {
            let invalid = http_request(
                api,
                &post_with_headers(
                    "/plugins/custom%20switch/post",
                    body,
                    &[("Content-Type", "application/json")],
                ),
            )
            .await;
            assert_eq!(invalid.status, expected, "{body}: {invalid:?}");
        }
        let duplicate_form = http_request(
            api,
            &post_with_headers(
                "/plugins/custom%20switch/post",
                "value=A&value=B",
                &[("Content-Type", "application/x-www-form-urlencoded")],
            ),
        )
        .await;
        assert_eq!(duplicate_form.status, 400);
        let unsupported = http_request(
            api,
            &post_with_headers(
                "/plugins/custom%20switch/post",
                "value=A",
                &[("Content-Type", "text/plain")],
            ),
        )
        .await;
        assert_eq!(unsupported.status, 415);

        let oversized = http_request(
            api,
            &format!(
                "POST /plugins/custom%20switch/post HTTP/1.1\r\nHost: native\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                1024 * 1024 + 1
            ),
        )
        .await;
        assert_eq!(oversized.status, 413);
        assert_eq!(fixture.read("state/custom switch.txt"), "B");

        let malformed_generation = http_request(
            api,
            &post_with_headers(
                "/plugins/custom%20switch/post",
                r#"{"value":"A"}"#,
                &[("X-Mosdns-Config-Generation", "01")],
            ),
        )
        .await;
        assert_eq!(malformed_generation.status, 400);
        let stale = http_request(
            api,
            &post_with_headers(
                "/plugins/custom%20switch/post",
                r#"{"value":"A"}"#,
                &[("X-Mosdns-Config-Generation", "1")],
            ),
        )
        .await;
        assert_eq!(stale.status, 409);
        assert_eq!(fixture.read("state/custom switch.txt"), "B");

        // Generation is a precondition only after a configured switch route
        // has been identified. Unrelated plugin and non-plugin requests keep
        // their normal route semantics even with malformed input.
        let unrelated_plugin = http_request(
            api,
            &get_with_headers(
                "/plugins/absent%20switch/show",
                &[("X-Mosdns-Config-Generation", "01")],
            ),
        )
        .await;
        assert_eq!(unrelated_plugin.status, 404, "{unrelated_plugin:?}");
        let unrelated_api = http_request(
            api,
            &get_with_headers(
                "/api/v1/capabilities",
                &[("X-Mosdns-Config-Generation", "not-a-number")],
            ),
        )
        .await;
        assert_eq!(unrelated_api.status, 200, "{unrelated_api:?}");

        let wrong_method = http_request(api, &raw("DELETE", "/plugins/custom%20switch/show")).await;
        assert_eq!(wrong_method.status, 405);
        let unknown = http_request(api, &get("/plugins/absent%20switch/show")).await;
        assert_eq!(unknown.status, 404);

        shutdown.cancel();
        task.await.expect("supervisor task")
    });
    result.expect("supervisor shutdown is clean");
}

#[test]
fn audit_v1_controls_use_real_http_and_dns_and_persist_capacity() {
    let fixture = fixture("audit-v1", free_udp_port(), free_tcp_port());
    let assembly = audit_assembly_for(&fixture);
    let bound = assembly
        .block_on(assembly.bind_host())
        .expect("bound host listeners");
    let dns = bound.dns_addr();
    let api = bound.api_addr().expect("management listener address");
    let shutdown = TransportCancellation::new();

    let result = assembly.block_on(async {
        let task = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let status = http_request(api, &get("/api/v1/audit/status")).await;
        assert_eq!(status.status, 200);
        assert_eq!(status.header("content-type"), Some("application/json"));
        assert_eq!(status.body, "{\"capturing\":true}\n");

        let stopped = http_request(api, &raw("POST", "/api/v1/audit/stop")).await;
        assert_eq!(stopped.status, 200);
        assert_eq!(stopped.header("content-type"), None);
        assert_eq!(stopped.body, "Audit log collection stopped.");
        let stopped_status = http_request(api, &get("/api/v1/audit/status")).await;
        assert_eq!(stopped_status.body, "{\"capturing\":false}\n");

        let first_dns = tokio::task::spawn_blocking(move || {
            udp_query(dns, &dns_query(0x7101, &["a", "example"]))
        })
        .await
        .expect("stopped DNS client");
        assert_eq!(rcode(&first_dns), 3);
        assert!(assembly.audit_snapshot().records.is_empty());

        let started = http_request(api, &raw("POST", "/api/v1/audit/start")).await;
        assert_eq!(started.status, 200);
        assert_eq!(started.body, "Audit log collection started.");
        let second_dns = tokio::task::spawn_blocking(move || {
            udp_query(dns, &dns_query(0x7102, &["a", "example"]))
        })
        .await
        .expect("started DNS client");
        assert_eq!(rcode(&second_dns), 3);
        assert_eq!(assembly.audit_snapshot().records.len(), 1);

        let capacity = http_request(api, &get("/api/v1/audit/capacity")).await;
        assert_eq!(capacity.body, "{\"capacity\":100000}\n");
        let resized = http_request(api, &post("/api/v1/audit/capacity", r#"{"capacity":2}"#)).await;
        assert_eq!(resized.status, 200, "{resized:?}");
        assert_eq!(resized.header("content-type"), None);
        assert_eq!(
            resized.body,
            "Audit log capacity set to 2. Existing logs have been cleared."
        );
        assert_eq!(assembly.audit_capacity(), 2);
        assert!(assembly.audit_snapshot().records.is_empty());
        assert!(fixture.root.join("webinfo/audit_settings.json").is_file());

        for (capacity, expected) in [(0, 0), (400_000, 400_000), (2, 2)] {
            let response = http_request(
                api,
                &post(
                    "/api/v1/audit/capacity",
                    &format!(r#"{{"capacity":{capacity}}}"#),
                ),
            )
            .await;
            assert_eq!(response.status, 200, "{capacity}: {response:?}");
            assert_eq!(assembly.audit_capacity(), expected);
            assert!(assembly.audit_snapshot().records.is_empty());
        }

        for body in [
            "{}",
            r#"{"capacity":1.5}"#,
            r#"{"capacity":"1"}"#,
            r#"{"capacity":1,"extra":true}"#,
            r#"{"capacity":400001}"#,
        ] {
            let invalid = http_request(api, &post("/api/v1/audit/capacity", body)).await;
            assert_eq!(invalid.status, 400, "{body}: {invalid:?}");
            assert_eq!(
                invalid.header("content-type"),
                Some("text/plain; charset=utf-8")
            );
            assert_eq!(invalid.body, "invalid audit capacity request\n");
            assert_eq!(assembly.audit_capacity(), 2);
        }

        let clear = http_request(api, &raw("POST", "/api/v1/audit/clear")).await;
        assert_eq!(clear.status, 200);
        assert_eq!(clear.body, "In-memory audit logs cleared.");
        let wrong_method = http_request(api, &raw("POST", "/api/v1/audit/status")).await;
        assert_eq!(wrong_method.status, 405);
        assert_eq!(wrong_method.body, "method not allowed\n");

        shutdown.cancel();
        task.await.expect("supervisor task")
    });
    result.expect("supervisor shutdown is clean");

    let restarted = HostAssembly::with_options_and_state_root(
        load_and_compile(&fixture.config()).expect("restart config"),
        HostOptions::default(),
        &fixture.root,
    )
    .expect("restart assembly");
    assert_eq!(restarted.audit_capacity(), 2);
}

#[test]
fn audit_capacity_without_state_root_and_failed_replace_keep_old_runtime_state() {
    let audit_fixture = fixture("audit-no-root", free_udp_port(), free_tcp_port());
    let _ = audit_assembly_for(&audit_fixture);
    let no_root = HostAssembly::from_config(
        load_and_compile(&audit_fixture.config()).expect("no-root config"),
    )
    .expect("no-root assembly");
    let no_root_bound = no_root
        .block_on(no_root.bind_host())
        .expect("no-root host bind");
    let no_root_api = no_root_bound.api_addr().expect("no-root API");
    let no_root_shutdown = TransportCancellation::new();
    let result = no_root.block_on(async {
        let task = tokio::task::spawn_local(no_root_bound.serve(no_root_shutdown.clone()));
        let response = http_request(
            no_root_api,
            &post("/api/v1/audit/capacity", r#"{"capacity":2}"#),
        )
        .await;
        assert_eq!(response.status, 500);
        assert_eq!(response.body, "audit settings state root is unavailable\n");
        no_root_shutdown.cancel();
        task.await.expect("no-root fixture task")
    });
    result.expect("fixture shutdown");

    let failure_fixture = fixture("audit-write-failure", free_udp_port(), free_tcp_port());
    let canonical = failure_fixture.root.join("webinfo/audit_settings.json");
    let old_bytes = b"{\"capacity\":100000}\n";
    fs::create_dir_all(canonical.parent().expect("canonical parent")).expect("webinfo directory");
    fs::write(&canonical, old_bytes).expect("old settings");
    let failure = audit_assembly_for(&failure_fixture);
    failure.inject_audit_temp_write_failure();
    let bound = failure
        .block_on(failure.bind_host())
        .expect("failure host bind");
    let dns = bound.dns_addr();
    let api = bound.api_addr().expect("failure API");
    let shutdown = TransportCancellation::new();
    let result = failure.block_on(async {
        let task = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let dns_response = tokio::task::spawn_blocking(move || {
            udp_query(dns, &dns_query(0x7201, &["a", "example"]))
        })
        .await
        .expect("failure fixture DNS client");
        assert_eq!(rcode(&dns_response), 3);
        assert_eq!(failure.audit_snapshot().records.len(), 1);
        let response =
            http_request(api, &post("/api/v1/audit/capacity", r#"{"capacity":2}"#)).await;
        assert_eq!(response.status, 500);
        assert_eq!(response.body, "audit settings persistence failed\n");
        assert_eq!(failure.audit_capacity(), 100_000);
        assert_eq!(failure.audit_snapshot().records.len(), 1);

        failure.inject_audit_final_replace_failure();
        let response =
            http_request(api, &post("/api/v1/audit/capacity", r#"{"capacity":2}"#)).await;
        assert_eq!(response.status, 500);
        assert_eq!(response.body, "audit settings persistence failed\n");
        assert_eq!(failure.audit_capacity(), 100_000);
        assert_eq!(failure.audit_snapshot().records.len(), 1);
        shutdown.cancel();
        task.await.expect("failure fixture task")
    });
    result.expect("failure fixture shutdown");
    assert!(
        failure_fixture
            .root
            .join("webinfo/audit_settings.json")
            .is_file()
    );
    assert_eq!(
        fs::read(&canonical).expect("old settings remains"),
        old_bytes
    );
    assert_eq!(
        fs::read_dir(failure_fixture.root.join("webinfo"))
            .expect("webinfo entries")
            .count(),
        1,
        "failed writes must clean temporary settings files"
    );
}

#[test]
fn a_dns_only_configuration_has_no_management_listener() {
    let fixture = Fixture::new("dns-only");
    fixture.write("rules/unused.txt", "unused.example\n");
    let yaml = format!(
        r#"log:
  level: error
plugins:
  - tag: sequence_main
    type: sequence
    args:
      - exec: $forward_main
  - tag: forward_main
    type: forward
    args:
      upstreams:
        - addr: "udp://127.0.0.1:25999"
  - tag: listener
    type: udp_server
    args:
      entry: sequence_main
      listen: "127.0.0.1:{}"
      enable_audit: false
"#,
        free_udp_port()
    );
    fixture.write("config.yaml", &yaml);
    let assembly = assembly_for(&fixture);
    let bound = assembly
        .block_on(assembly.bind_host())
        .expect("DNS-only host binds");
    assert!(
        bound.api_addr().is_none(),
        "a DNS-only configuration must not open a management listener"
    );
    let dns = bound.dns_addr();
    let shutdown = TransportCancellation::new();
    let result = assembly.block_on(async {
        let task = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        shutdown.cancel();
        task.await.expect("supervisor task")
    });
    result.expect("DNS-only supervisor shutdown is clean");
    assert!(dns.port() > 0);
}

#[test]
fn one_shutdown_scope_releases_both_listeners_for_rebind() {
    let fixture = fixture("lifecycle", free_udp_port(), free_tcp_port());
    let assembly = assembly_for(&fixture);
    let bound = assembly
        .block_on(assembly.bind_host())
        .expect("bound host listeners");
    let dns = bound.dns_addr();
    let api = bound.api_addr().expect("management listener");
    let shutdown = TransportCancellation::new();

    let result = assembly.block_on(async {
        let task = tokio::task::spawn_local(bound.serve(shutdown.clone()));

        // The DNS side really serves: the managed name is rejected locally.
        let listener = dns;
        let response = tokio::task::spawn_blocking(move || {
            udp_query(listener, &dns_query(0x2001, &["a", "example"]))
        })
        .await
        .expect("dns client task");
        assert_eq!(rcode(&response), 3);

        let show = http_request(api, &get("/plugins/blocklist/show")).await;
        assert_eq!(show.status, 200);

        shutdown.cancel();
        task.await.expect("supervisor task")
    });
    result.expect("supervisor shutdown is clean");

    // Both sockets must be released once the supervisor returns.
    StdUdpSocket::bind(dns).expect("the DNS socket must be released");
    StdTcpListener::bind(api).expect("the management socket must be released");
}

#[test]
fn a_failed_management_bind_releases_the_dns_listener() {
    let dns_port = free_udp_port();
    let occupied = StdTcpListener::bind("127.0.0.1:0").expect("occupy management port");
    let api_port = occupied.local_addr().expect("occupied address").port();

    let fixture = fixture("api-bind-failure", dns_port, api_port);
    let assembly = assembly_for(&fixture);
    let bound = assembly.block_on(assembly.bind_host());
    assert!(
        bound.is_err(),
        "an occupied management port must fail the host bind"
    );
    drop(bound);

    // The DNS listener bound before the failure must not stay bound.
    StdUdpSocket::bind(("127.0.0.1", dns_port))
        .expect("the DNS socket must be released after a failed management bind");
}

#[test]
fn a_failed_dns_bind_releases_the_management_listener() {
    let api_port = free_tcp_port();
    let occupied = StdUdpSocket::bind("127.0.0.1:0").expect("occupy DNS port");
    let dns_port = occupied.local_addr().expect("occupied address").port();

    let fixture = fixture("dns-bind-failure", dns_port, api_port);
    let assembly = assembly_for(&fixture);
    let bound = assembly.block_on(assembly.bind_host());
    assert!(
        bound.is_err(),
        "an occupied DNS port must fail the host bind"
    );
    drop(bound);

    StdTcpListener::bind(("127.0.0.1", api_port))
        .expect("the management socket must be released after a failed DNS bind");
}

#[test]
fn a_running_side_failure_cancels_and_joins_the_other_listener() {
    let fixture = fixture("running-failure", free_udp_port(), free_tcp_port());
    let assembly = assembly_for(&fixture);
    let bound = assembly
        .block_on(assembly.bind_host())
        .expect("bound host listeners");
    let dns = bound.dns_addr();
    let api = bound.api_addr().expect("management listener");
    bound.inject_api_accept_fault_after(1);
    let shutdown = TransportCancellation::new();

    let result = assembly.block_on(async {
        let task = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        // The first request is served; the next accepted connection makes the
        // listener fail, so the supervisor must cancel and join the DNS side.
        let first = http_request(api, &get("/plugins/blocklist/show")).await;
        assert_eq!(first.status, 200);
        if let Ok(mut stream) = tokio::net::TcpStream::connect(api).await {
            let _ = stream
                .write_all(get("/plugins/blocklist/show").as_bytes())
                .await;
            let _ = stream.shutdown().await;
        }
        task.await.expect("supervisor task")
    });

    assert!(
        result.is_err(),
        "a running-side failure must surface as a host error"
    );
    // The supervisor must have joined the DNS side and released both sockets.
    StdUdpSocket::bind(dns).expect("the DNS socket must be released");
    StdTcpListener::bind(api).expect("the management socket must be released");
}

#[test]
fn a_tag_that_is_not_mounted_is_404_before_any_method_decision() {
    let fixture = fixture("unknown-tag-methods", free_udp_port(), free_tcp_port());
    let assembly = assembly_for(&fixture);
    let bound = assembly
        .block_on(assembly.bind_host())
        .expect("bound host listeners");
    let api = bound.api_addr().expect("management listener address");
    let shutdown = TransportCancellation::new();

    let result = assembly.block_on(async {
        let task = tokio::task::spawn_local(bound.serve(shutdown.clone()));

        // Go mounts `/plugins/{tag}` only for a configured tag, so a tag with no
        // mount is 404 whatever the method is.
        for (method, path) in [
            ("POST", "/plugins/absent/show"),
            ("GET", "/plugins/absent/post"),
            ("DELETE", "/plugins/absent/save"),
            ("POST", "/plugins/absent/post"),
        ] {
            let response = http_request(api, &raw(method, path)).await;
            assert_eq!(
                response.status, 404,
                "{method} {path} must be 404 because the tag is not mounted: {response:?}"
            );
            assert_eq!(response.body, "404 page not found\n");
        }

        // A mounted tag with a wrong method is 405, and so is a mounted but
        // ineligible tag: the route exists, the method does not.
        let wrong_on_managed = http_request(api, &post("/plugins/blocklist/show", "")).await;
        assert_eq!(wrong_on_managed.status, 405, "{wrong_on_managed:?}");
        let wrong_on_ineligible = http_request(api, &raw("GET", "/plugins/exps_only/post")).await;
        assert_eq!(wrong_on_ineligible.status, 405, "{wrong_on_ineligible:?}");

        // A mounted tag with an unknown action has no route either.
        let unknown_action = http_request(api, &raw("GET", "/plugins/blocklist/nope")).await;
        assert_eq!(unknown_action.status, 404);

        // The collection route accepts GET and POST, so DELETE is the wrong method.
        let group_wrong_method = http_request(api, &raw("DELETE", "/api/v1/special-groups")).await;
        assert_eq!(group_wrong_method.status, 405);

        shutdown.cancel();
        task.await.expect("supervisor task")
    });
    result.expect("supervisor shutdown is clean");
}

#[test]
fn a_writable_file_shared_by_two_tags_is_never_managed() {
    let fixture = Fixture::new("shared-file-api");
    fixture.write("rules/shared.txt", "shared.example\n");
    let yaml = format!(
        r#"log:
  level: error
api:
  http: "127.0.0.1:{}"
plugins:
  - tag: sequence_main
    type: sequence
    args:
      - matches: qname $first
        exec: reject 3
      - exec: reject 0
  - tag: first
    type: domain_set
    args:
      files:
        - rules/shared.txt
  - tag: second
    type: domain_set
    args:
      files:
        - rules/shared.txt
  - tag: forward_main
    type: forward
    args:
      upstreams:
        - addr: "udp://127.0.0.1:25999"
  - tag: listener
    type: udp_server
    args:
      entry: sequence_main
      listen: "127.0.0.1:{}"
      enable_audit: false
"#,
        free_tcp_port(),
        free_udp_port()
    );
    fixture.write("config.yaml", &yaml);
    let assembly = assembly_for(&fixture);
    let bound = assembly
        .block_on(assembly.bind_host())
        .expect("bound host listeners");
    let api = bound.api_addr().expect("management listener address");
    let shutdown = TransportCancellation::new();

    let result = assembly.block_on(async {
        let task = tokio::task::spawn_local(bound.serve(shutdown.clone()));

        // Both conflicting tags are rejected explicitly on every management
        // route, and the shared file is never written through the API.
        for tag in ["first", "second"] {
            let show = http_request(api, &get(&format!("/plugins/{tag}/show"))).await;
            assert_eq!(show.status, 400, "{show:?}");
            assert!(
                show.body.contains("shared"),
                "the rejection must name the shared file: {}",
                show.body
            );
            let save = http_request(api, &get(&format!("/plugins/{tag}/save"))).await;
            assert_eq!(save.status, 400, "{save:?}");
            let post = http_request(
                api,
                &post(
                    &format!("/plugins/{tag}/post"),
                    r#"{"values":["x.example"]}"#,
                ),
            )
            .await;
            assert_eq!(post.status, 400, "{post:?}");
        }
        assert_eq!(
            fixture.read("rules/shared.txt"),
            "shared.example\n",
            "no shared-file write may happen"
        );

        shutdown.cancel();
        task.await.expect("supervisor task")
    });
    result.expect("supervisor shutdown is clean");
}

#[test]
fn native_supervisor_health_matches_cli_identity_and_closes_with_listeners() {
    let dns_port = free_udp_port();
    let api_port = free_tcp_port();
    let fixture = fixture("health-supervisor", dns_port, api_port);
    let host = assembly_for(&fixture);
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let shutdown = TransportCancellation::new();
        let task = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let response = http_request(api, &get("/api/v1/system/health")).await;
        assert_eq!(response.status, 200);
        let health: serde_json::Value = serde_json::from_str(&response.body).unwrap();
        assert_eq!(health["ready"], true);
        assert_eq!(
            health["version"],
            mosdns_native_host::build_identity::VERSION
        );
        assert_eq!(health["runtime"], "rust");
        shutdown.cancel();
        task.await.unwrap().unwrap();
        assert!(tokio::net::TcpStream::connect(api).await.is_err());
    });
}

#[test]
fn embedded_ui_roots_assets_and_revalidation_are_real_http() {
    let fixture = fixture("embedded-ui", free_udp_port(), free_tcp_port());
    let host = assembly_for(&fixture);
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let shutdown = TransportCancellation::new();
        let task = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let before = host.metrics_snapshot();
        host.start_audit();
        for path in [
            "/",
            "/log",
            "/assets/vue-log/app.js?v=proof",
            "/assets/vue-log1/app.css",
        ] {
            let response = http_request(api, &get(path)).await;
            assert_eq!(response.status, 200, "{path}: {response:?}");
            assert!(!response.body.is_empty());
            let mime = if path.contains("app.js") {
                "text/javascript; charset=utf-8"
            } else if path == "/assets/vue-log1/app.css" {
                "text/css; charset=utf-8"
            } else {
                "text/html; charset=utf-8"
            };
            assert_eq!(response.header("content-type"), Some(mime));
            assert_eq!(response.header("cache-control"), Some("no-cache"));
            assert_eq!(response.header("x-content-type-options"), Some("nosniff"));
            let etag = response.header("etag").unwrap();
            let cached = http_request(
                api,
                &format!("GET {path} HTTP/1.1\r\nHost: test\r\nIf-None-Match: {etag}\r\n\r\n"),
            )
            .await;
            assert_eq!(cached.status, 304);
            assert!(cached.body.is_empty());
            let head =
                http_request(api, &format!("HEAD {path} HTTP/1.1\r\nHost: test\r\n\r\n")).await;
            assert_eq!(head.status, 200);
            assert!(head.body.is_empty());
            assert_eq!(
                head.header("content-length"),
                response.header("content-length")
            );
        }
        for path in [
            "/assets/no-such.js",
            "/api/missing",
            "/plugins/missing",
            "/assets/%2e%2e/log.html",
            "/assets/vue-log%2fapp.js",
        ] {
            assert_eq!(http_request(api, &get(path)).await.status, 404, "{path}");
        }
        let wrong = http_request(api, &post("/", "")).await;
        assert_eq!(wrong.status, 405);
        assert_eq!(wrong.header("allow"), Some("GET, HEAD"));
        let redirect = http_request(api, &get("/log/")).await;
        assert_eq!(redirect.status, 301);
        assert_eq!(redirect.header("location"), Some("/log"));
        let queried = http_request(api, &get("/log/?x=1&return=%2F")).await;
        assert_eq!(queried.status, 301);
        assert_eq!(queried.header("location"), Some("/log?x=1&return=%2F"));
        let unsafe_query = http_request(api, &get("/log/?x=%0d%0aInjected%3Ayes")).await;
        assert_eq!(unsafe_query.status, 404);
        assert!(unsafe_query.header("location").is_none());
        assert_eq!(host.metrics_snapshot(), before);
        assert!(host.audit_snapshot().records.is_empty());
        shutdown.cancel();
        task.await.unwrap().unwrap();
    });
}

#[cfg(target_os = "linux")]
#[test]
fn external_ui_is_live_confined_and_root_pinned() {
    use std::os::unix::fs::symlink;
    let fixture = fixture("external-ui", free_udp_port(), free_tcp_port());
    fixture.write("ui/demo/index.html", "initial");
    fixture.write("ui/demo/nested/file.js", "nested");
    fixture.write("ui/a?b%# 空/index.html", "reserved root");
    fixture.write("ui/demo/n?%# 空/index.html", "reserved nested");
    fixture.write("secret.txt", "SECRET");
    fixture.write("ui/api/index.html", "reserved");
    symlink(&fixture.root, fixture.root.join("ui/link")).unwrap();
    symlink(
        fixture.root.join("secret.txt"),
        fixture.root.join("ui/demo/leak"),
    )
    .unwrap();
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        fixture.root.join("ui/demo/fifo"),
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    let _socket =
        std::os::unix::net::UnixListener::bind(fixture.root.join("ui/demo/socket")).unwrap();
    let host = assembly_for(&fixture);
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let api = bound.api_addr().unwrap();
        let shutdown = TransportCancellation::new();
        let task = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let redirect = http_request(api, &get("/demo?x=1")).await;
        assert_eq!(redirect.status, 301);
        assert_eq!(redirect.header("location"), Some("/demo/?x=1"));
        exercise_encoded_redirects(api).await;
        let initial = http_request(api, &get("/demo/")).await;
        assert_eq!(initial.status, 200);
        assert_eq!(initial.body, "initial");
        assert_eq!(initial.header("cache-control"), Some("no-store"));
        fixture.write("ui/demo/index.html", "changed");
        assert_eq!(http_request(api, &get("/demo/")).await.body, "changed");
        assert_eq!(
            http_request(api, &get("/demo/nested/file.js?v=1"))
                .await
                .body,
            "nested"
        );
        let dir = http_request(api, &get("/demo/nested?x=1")).await;
        assert_eq!(dir.status, 301);
        assert_eq!(dir.header("location"), Some("/demo/nested/?x=1"));
        let head = http_request(api, "HEAD /demo/ HTTP/1.1\r\nHost: test\r\n\r\n").await;
        assert_eq!(head.status, 200);
        assert!(head.body.is_empty());
        assert_eq!(head.header("content-length"), Some("7"));
        for path in [
            "/demo/leak",
            "/demo/fifo",
            "/demo/socket",
            "/link/secret.txt",
            "/api/",
            "/demo/%2e%2e/secret.txt",
            "/demo/a%2fb",
            "/demo/%00",
            "/demo/%ff",
            "/demo/missing",
            "/demo/nested/",
        ] {
            assert_eq!(http_request(api, &get(path)).await.status, 404, "{path}");
        }
        assert_eq!(http_request(api, &post("/demo/", "")).await.status, 405);
        let large = fs::File::create(fixture.root.join("ui/demo/large")).unwrap();
        large.set_len(16 * 1024 * 1024 + 1).unwrap();
        assert_eq!(http_request(api, &get("/demo/large")).await.status, 413);
        fs::rename(fixture.root.join("ui/demo"), fixture.root.join("ui/old")).unwrap();
        fixture.write("ui/demo/index.html", "replacement");
        assert_eq!(http_request(api, &get("/demo/")).await.body, "changed");
        for _ in 0..20 {
            fs::rename(
                fixture.root.join("ui/old/nested"),
                fixture.root.join("ui/old/safe"),
            )
            .unwrap();
            symlink(&fixture.root, fixture.root.join("ui/old/nested")).unwrap();
            let response = http_request(api, &get("/demo/nested/secret.txt")).await;
            assert_eq!(response.status, 404);
            assert!(!response.body.contains("SECRET"));
            fs::remove_file(fixture.root.join("ui/old/nested")).unwrap();
            fs::rename(
                fixture.root.join("ui/old/safe"),
                fixture.root.join("ui/old/nested"),
            )
            .unwrap();
        }
        exercise_ui_swaps(api, &fixture).await;
        fs::remove_file(fixture.root.join("ui/old/index.html")).unwrap();
        assert_eq!(http_request(api, &get("/demo/")).await.status, 404);
        shutdown.cancel();
        task.await.unwrap().unwrap();
    });
}

#[cfg(target_os = "linux")]
async fn exercise_encoded_redirects(api: SocketAddr) {
    let get = |path: &str| format!("GET {path} HTTP/1.1\r\nHost: test\r\n\r\n");
    for (path, body) in [
        ("/a%3Fb%25%23%20%E7%A9%BA", "reserved root"),
        ("/demo/n%3F%25%23%20%E7%A9%BA", "reserved nested"),
    ] {
        let response = http_request(api, &get(&format!("{path}?safe=%E7%A9%BA&x=1"))).await;
        assert_eq!(response.status, 301);
        let expected = format!("{path}/?safe=%E7%A9%BA&x=1");
        assert_eq!(response.header("location"), Some(expected.as_str()));
        let followed = http_request(api, &get(&expected)).await;
        assert_eq!(followed.status, 200);
        assert_eq!(followed.body, body);
    }
}

#[cfg(target_os = "linux")]
async fn exercise_ui_swaps(api: SocketAddr, fixture: &Fixture) {
    use std::os::unix::fs::symlink;
    struct SwapOwner(
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        Option<std::thread::JoinHandle<()>>,
    );
    impl Drop for SwapOwner {
        fn drop(&mut self) {
            self.0.store(false, std::sync::atomic::Ordering::SeqCst);
            self.1.take().unwrap().join().unwrap();
        }
    }
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let flag = running.clone();
    let race_root = fixture.root.clone();
    let swap = SwapOwner(
        running,
        Some(std::thread::spawn(move || {
            while flag.load(std::sync::atomic::Ordering::SeqCst) {
                fs::rename(
                    race_root.join("ui/old/nested"),
                    race_root.join("ui/old/safe"),
                )
                .unwrap();
                symlink(&race_root, race_root.join("ui/old/nested")).unwrap();
                std::thread::yield_now();
                fs::remove_file(race_root.join("ui/old/nested")).unwrap();
                fs::rename(
                    race_root.join("ui/old/safe"),
                    race_root.join("ui/old/nested"),
                )
                .unwrap();
            }
        })),
    );
    for _ in 0..100 {
        let leak = http_request(api, &get("/demo/nested/secret.txt")).await;
        assert_eq!(leak.status, 404);
        assert!(!leak.body.contains("SECRET"));
        let legitimate = http_request(api, &get("/demo/nested/file.js")).await;
        assert!(matches!(legitimate.status, 200 | 404));
        if legitimate.status == 200 {
            assert_eq!(legitimate.body, "nested");
        }
    }
    drop(swap);
}

#[test]
fn capability_matrix_is_complete_and_matches_host_eligibility() {
    for (managed, local) in [(false, true), (true, true), (false, false), (true, false)] {
        let fixture = capability_fixture(managed, local);
        let host = assembly_for(&fixture);
        host.block_on(async {
            let bound = host.bind_host().await.unwrap();
            let api = bound.api_addr().unwrap();
            let shutdown = TransportCancellation::new();
            let task = tokio::task::spawn_local(bound.serve(shutdown.clone()));
            let payload: serde_json::Value =
                serde_json::from_str(&http_request(api, &get("/api/v1/capabilities")).await.body)
                    .unwrap();
            assert_eq!(payload["schema_version"], 1);
            assert_eq!(payload["runtime"], "rust");
            assert_eq!(payload["special_groups"]["enabled"], managed);
            assert_eq!(payload["endpoints"]["manual_rules"]["post"], managed);
            let ops = payload["ui_operations"]
                .as_object()
                .expect("complete operations");
            assert_eq!(ops.len(), 30);
            for id in [
                "system.health",
                "system.version",
                "audit.read",
                "audit.control",
                "audit.capacity",
                "query.rank",
                "cache.inventory",
                "cache.manage",
                "metrics.cache",
                "rules.local.read",
                "groups.read",
                "upstreams.read",
            ] {
                assert_eq!(ops[id]["supported"], true, "{id}");
                assert!(ops[id]["reason"].is_null());
            }
            assert_eq!(ops["rules.local.manage"]["supported"], local);
            if !local {
                assert!(
                    !ops["rules.local.manage"]["reason"]
                        .as_str()
                        .unwrap()
                        .is_empty()
                );
            }
            for id in ["groups.manage", "upstreams.manage", "rules.diversion"] {
                assert_eq!(ops[id]["supported"], managed, "{id}");
            }
            for id in [
                "rules.adguard",
                "capture.logs",
                "client.aliases",
                "switches.manage",
                "cache.requery",
                "lists.remembered",
                "appearance.server",
                "system.restart",
                "system.webui_port",
                "system.config_management",
                "system.update",
                "system.domain_generation",
                "system.global_overrides",
                "metrics.process",
            ] {
                assert_eq!(ops[id]["supported"], false, "{id}");
                assert!(
                    !ops[id]["reason"].as_str().unwrap().trim().is_empty(),
                    "{id}"
                );
            }
            for path in [
                "/plugins/adguard/rules",
                "/api/v1/capture/logs",
                "/plugins/clientname",
                "/plugins/switch3/show",
                "/plugins/requery/status",
                "/api/v1/appearance/text-color",
                "/api/v1/system/webui-port",
                "/api/v1/update/status",
                "/api/v1/domain-generation",
                "/api/v1/overrides",
            ] {
                assert_eq!(http_request(api, &get(path)).await.status, 404, "{path}");
            }
            assert_eq!(
                http_request(api, &post("/api/v1/system/restart", "{}"))
                    .await
                    .status,
                404
            );
            shutdown.cancel();
            task.await.unwrap().unwrap();
        });
    }
}

fn capability_fixture(managed: bool, local: bool) -> Fixture {
    if managed || !local {
        let fixture = Fixture::new("capabilities-managed");
        fixture.write("config.yaml", &format!("log: {{level: error}}\nnative_management: {{special_groups: true}}\napi: {{http: '127.0.0.1:{}'}}\nplugins:\n  - tag: default_forward\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://127.0.0.1:9'}}]}}\n  - tag: main_entry\n    type: sequence\n    args: [{{exec: $special_upstream_matcher}}, {{exec: $default_forward}}]\n  - tag: main\n    type: udp_server\n    args: {{entry: main_entry, listen: '127.0.0.1:{}', enable_audit: true}}\n",free_tcp_port(),free_udp_port()));
        if managed && local {
            fs::create_dir_all(fixture.root.join("cache")).unwrap();
            fixture.write(
                "webinfo/special_upstream_groups.json",
                r#"[{"slot":50,"name":"actual","listen_port":0,"custom_port_only":false}]"#,
            );
            fixture.write("webinfo/upstream_overrides.json", r#"{"special_upstream_50":[{"tag":"fixture","enabled":true,"protocol":"udp","addr":"127.0.0.1:9"}]}"#);
        }
        if !managed {
            let yaml = fixture
                .read("config.yaml")
                .replace("special_groups: true", "special_groups: false")
                .replace("{{exec: $special_upstream_matcher}}, ", "")
                .replace("{exec: $special_upstream_matcher}, ", "");
            fixture.write("config.yaml", &yaml);
        }
        fixture
    } else {
        fixture("capabilities-unmanaged", free_udp_port(), free_tcp_port())
    }
}

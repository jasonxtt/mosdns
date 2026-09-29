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

use mosdns_native_host::{HostAssembly, load_and_compile};
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// One owned fixture directory holding a config and its real rule files.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("mosdns-http-{name}-{}", std::process::id()));
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

fn raw(method: &str, path: &str) -> String {
    format!("{method} {path} HTTP/1.1\r\nHost: native\r\nConnection: close\r\n\r\n")
}

fn post(path: &str, body: &str) -> String {
    format!(
        "POST {path} HTTP/1.1\r\nHost: native\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
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

        // The group-list route exists, so a wrong method there is 405.
        let group_wrong_method = http_request(api, &raw("POST", "/api/v1/special-groups")).await;
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

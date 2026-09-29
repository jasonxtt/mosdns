//! Slice 2 of the local-rule editing workflow: publication, persistence
//! atomicity and restart retention.
//!
//! Every test uses a real HTTP client, a real rule file and a real UDP or TCP
//! DNS query. The only controlled seam is one narrow persistence-step failure
//! (`PersistFault`), which the production writer exposes so that old-state
//! retention and temporary-file cleanup can be proven.

use std::fs;

use std::net::{SocketAddr, UdpSocket as StdUdpSocket};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use mosdns_native_host::{HostAssembly, HostRunError, PersistFault, PersistGate, load_and_compile};
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// One owned fixture directory holding a config and its real rule files.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("mosdns-pub-{name}-{}", std::process::id()));
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

    /// Every entry under `rules/`, used to prove temporary files are cleaned.
    fn rule_entries(&self) -> Vec<String> {
        let mut entries: Vec<String> = fs::read_dir(self.root.join("rules"))
            .expect("rules directory")
            .map(|entry| {
                entry
                    .expect("rules entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        entries.sort();
        entries
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A managed profile whose sequence answers `reject 3` for a match and
/// `reject 0` otherwise, so the DNS oracle needs no upstream peer at all.
const SETS: &str = r"  - tag: blocklist
    type: domain_set
    args:
      files:
        - rules/blocklist.txt
";

fn config_yaml(listener_kind: &str, extra_args: &str, dns_port: u16, api_port: u16) -> String {
    format!(
        r#"log:
  level: error
api:
  http: "127.0.0.1:{api_port}"
plugins:
  - tag: sequence_main
    type: sequence
    args:
      - matches: qname $blocklist
        exec: reject 3
      - exec: reject 0
{SETS}  - tag: forward_main
    type: forward
    args:
      upstreams:
        - addr: "udp://127.0.0.1:25999"
  - tag: listener
    type: {listener_kind}
    args:
      entry: sequence_main
      listen: "127.0.0.1:{dns_port}"
      enable_audit: false
{extra_args}"#
    )
}

fn udp_fixture(name: &str, rules: &str, dns_port: u16, api_port: u16) -> Fixture {
    let fixture = Fixture::new(name);
    fixture.write("rules/blocklist.txt", rules);
    fixture.write(
        "config.yaml",
        &config_yaml("udp_server", "", dns_port, api_port),
    );
    fixture
}

fn tcp_fixture(name: &str, rules: &str, dns_port: u16, api_port: u16) -> Fixture {
    let fixture = Fixture::new(name);
    fixture.write("rules/blocklist.txt", rules);
    fixture.write(
        "config.yaml",
        &config_yaml("tcp_server", "      idle_timeout: 5\n", dns_port, api_port),
    );
    fixture
}

fn assembly_for(fixture: &Fixture) -> HostAssembly {
    HostAssembly::from_config(load_and_compile(&fixture.config()).expect("fixture config"))
        .expect("fixture assembly")
}

fn free_udp_port() -> u16 {
    let socket = StdUdpSocket::bind("127.0.0.1:0").expect("probe bind");
    socket.local_addr().expect("probe address").port()
}

fn free_tcp_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("probe bind");
    listener.local_addr().expect("probe address").port()
}

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

fn rcode(response: &[u8]) -> u16 {
    u16::from_be_bytes([response[2], response[3]]) & 0x000f
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

/// Sleeps without blocking the single-threaded host runtime.
async fn pause(millis: u64) {
    tokio::task::spawn_blocking(move || std::thread::sleep(Duration::from_millis(millis)))
        .await
        .expect("pause task");
}

/// Runs one blocking UDP query off the single-threaded host so the host's own
/// listener task can answer it.
async fn udp_rcode(listener: SocketAddr, id: u16, labels: &[&str]) -> u16 {
    let request = dns_query(id, labels);
    let response = tokio::task::spawn_blocking(move || udp_query(listener, &request))
        .await
        .expect("dns client task");
    rcode(&response)
}

async fn tcp_rcode(listener: SocketAddr, id: u16, labels: &[&str]) -> u16 {
    let request = dns_query(id, labels);
    let mut stream = tokio::net::TcpStream::connect(listener)
        .await
        .expect("tcp connect");
    let length = u16::try_from(request.len()).expect("framed length");
    stream
        .write_all(&length.to_be_bytes())
        .await
        .expect("tcp write prefix");
    stream.write_all(&request).await.expect("tcp write");
    let mut prefix = [0_u8; 2];
    stream
        .read_exact(&mut prefix)
        .await
        .expect("tcp read prefix");
    let mut response = vec![0_u8; usize::from(u16::from_be_bytes(prefix))];
    stream
        .read_exact(&mut response)
        .await
        .expect("tcp read body");
    rcode(&response)
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    body: String,
}

async fn get(address: SocketAddr, path: &str) -> HttpResponse {
    raw_request(
        address,
        &format!("GET {path} HTTP/1.1\r\nHost: native\r\nConnection: close\r\n\r\n"),
    )
    .await
}

async fn post_values(address: SocketAddr, tag: &str, values: &[&str]) -> HttpResponse {
    let body = serde_json::json!({ "values": values }).to_string();
    raw_request(
        address,
        &format!(
            "POST /plugins/{tag}/post HTTP/1.1\r\nHost: native\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
    .await
}

async fn raw_request(address: SocketAddr, request: &str) -> HttpResponse {
    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("http connect");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("http write");
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.expect("http read");
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let (head, body) = text
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("malformed HTTP response: {text:?}"));
    let status = head
        .split("\r\n")
        .next()
        .expect("status line")
        .split_whitespace()
        .nth(1)
        .expect("status code")
        .parse()
        .expect("numeric status");
    HttpResponse {
        status,
        body: body.to_owned(),
    }
}

/// Binds `assembly`'s listeners, awaits `body` on the host runtime while both
/// listeners serve, then cancels the one shared scope and joins them.
fn with_assembly<T>(
    assembly: &HostAssembly,
    body: impl AsyncFnOnce(&HostAssembly, SocketAddr, SocketAddr) -> T,
) -> (T, Result<(), HostRunError>) {
    let bound = assembly
        .block_on(assembly.bind_host())
        .expect("bound host listeners");
    let dns = bound.dns_addr();
    let api = bound.api_addr().expect("management listener");
    let shutdown = TransportCancellation::new();
    assembly.block_on(async {
        let task = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let value = body(assembly, dns, api).await;
        shutdown.cancel();
        (value, task.await.expect("supervisor task"))
    })
}

fn with_host<T>(
    fixture: &Fixture,
    body: impl AsyncFnOnce(&HostAssembly, SocketAddr, SocketAddr) -> T,
) -> (T, Result<(), HostRunError>) {
    let assembly = assembly_for(fixture);
    with_assembly(&assembly, body)
}

#[test]
fn a_post_changes_the_next_udp_query_the_file_and_the_show_output() {
    let fixture = udp_fixture(
        "udp-publication",
        "a.example\n",
        free_udp_port(),
        free_tcp_port(),
    );
    let ((), result) = with_host(&fixture, async |_assembly, dns, api| {
        assert_eq!(udp_rcode(dns, 0x3001, &["a", "example"]).await, 3);
        assert_eq!(udp_rcode(dns, 0x3002, &["b", "example"]).await, 0);

        let posted = post_values(api, "blocklist", &["b.example"]).await;
        assert_eq!(posted.status, 200, "{posted:?}");
        assert_eq!(posted.body, "domain_set replaced with 1 entries");

        // The very next queries see the new generation.
        assert_eq!(
            udp_rcode(dns, 0x3003, &["b", "example"]).await,
            3,
            "the published generation must be visible to the next query"
        );
        assert_eq!(udp_rcode(dns, 0x3004, &["a", "example"]).await, 0);

        assert_eq!(
            get(api, "/plugins/blocklist/show").await.body,
            "b.example\n"
        );
        // The same-directory replacement committed exactly the rules.
        assert_eq!(fixture.read("rules/blocklist.txt"), "b.example\n");
        assert_eq!(fixture.rule_entries(), vec!["blocklist.txt".to_owned()]);
    });
    result.expect("supervisor shutdown is clean");
}

#[test]
fn the_same_publication_changes_a_tcp_query() {
    let fixture = tcp_fixture(
        "tcp-publication",
        "a.example\n",
        free_udp_port(),
        free_tcp_port(),
    );
    let ((), result) = with_host(&fixture, async |_assembly, dns, api| {
        assert_eq!(tcp_rcode(dns, 0x3101, &["a", "example"]).await, 3);
        assert_eq!(tcp_rcode(dns, 0x3102, &["b", "example"]).await, 0);

        let posted = post_values(api, "blocklist", &["b.example"]).await;
        assert_eq!(posted.status, 200);

        assert_eq!(tcp_rcode(dns, 0x3103, &["b", "example"]).await, 3);
        assert_eq!(tcp_rcode(dns, 0x3104, &["a", "example"]).await, 0);
    });
    result.expect("supervisor shutdown is clean");
}

#[test]
fn two_managed_tags_stay_isolated_across_a_post() {
    let fixture = Fixture::new("two-tags");
    fs::write(fixture.root.join("rules/alpha.txt"), "alpha.example\n").expect("alpha rules");
    fs::write(fixture.root.join("rules/beta.txt"), "beta.example\n").expect("beta rules");
    let yaml = r#"log:
  level: error
api:
  http: "127.0.0.1:__API__"
plugins:
  - tag: sequence_main
    type: sequence
    args:
      - matches: qname $alpha
        exec: reject 3
      - matches: qname $beta
        exec: reject 3
      - exec: reject 0
  - tag: alpha
    type: domain_set
    args:
      files:
        - rules/alpha.txt
  - tag: beta
    type: domain_set
    args:
      files:
        - rules/beta.txt
  - tag: forward_main
    type: forward
    args:
      upstreams:
        - addr: "udp://127.0.0.1:25999"
  - tag: listener
    type: udp_server
    args:
      entry: sequence_main
      listen: "127.0.0.1:__DNS__"
      enable_audit: false
"#
    .replace("__DNS__", &free_udp_port().to_string())
    .replace("__API__", &free_tcp_port().to_string());
    fixture.write("config.yaml", &yaml);

    let ((), result) = with_host(&fixture, async |_assembly, dns, api| {
        let posted = post_values(api, "alpha", &["newalpha.example"]).await;
        assert_eq!(posted.status, 200);

        // The edited tag changed; the other tag is untouched on disk, in
        // `/show` and in the DNS answer.
        assert_eq!(fixture.read("rules/alpha.txt"), "newalpha.example\n");
        assert_eq!(fixture.read("rules/beta.txt"), "beta.example\n");
        assert_eq!(get(api, "/plugins/beta/show").await.body, "beta.example\n");
        assert_eq!(udp_rcode(dns, 0x3201, &["beta", "example"]).await, 3);
        assert_eq!(udp_rcode(dns, 0x3202, &["alpha", "example"]).await, 0);
        assert_eq!(udp_rcode(dns, 0x3203, &["newalpha", "example"]).await, 3);
    });
    result.expect("supervisor shutdown is clean");
}

/// A POST whose persistence step fails must leave the old bytes, the old
/// `/show` output, the old DNS answer and no temporary file behind.
fn assert_persistence_failure_keeps_the_old_state(name: &str, fault: PersistFault) {
    let fixture = udp_fixture(name, "a.example\n", free_udp_port(), free_tcp_port());
    let before = fixture.read("rules/blocklist.txt");
    let ((), result) = with_host(&fixture, async |assembly, dns, api| {
        // Arm the narrow fault on the very provider the route will use.
        assembly
            .config()
            .domain_set("blocklist")
            .expect("managed tag")
            .managed
            .as_ref()
            .expect("managed provider")
            .inject_persist_fault(fault);

        let posted = post_values(api, "blocklist", &["b.example"]).await;
        assert_eq!(posted.status, 500, "{posted:?}");
        assert!(
            posted.body.contains("injected"),
            "the injected failure must be reported: {}",
            posted.body
        );

        assert_eq!(
            fixture.read("rules/blocklist.txt"),
            before,
            "the old file bytes must be preserved byte for byte"
        );
        assert_eq!(
            get(api, "/plugins/blocklist/show").await.body,
            "a.example\n",
            "the published generation must be unchanged"
        );
        assert_eq!(udp_rcode(dns, 0x3301, &["a", "example"]).await, 3);
        assert_eq!(
            udp_rcode(dns, 0x3302, &["b", "example"]).await,
            0,
            "the failed candidate must not be published"
        );
        assert_eq!(
            fixture.rule_entries(),
            vec!["blocklist.txt".to_owned()],
            "no temporary file may survive a failed persistence step"
        );
    });
    result.expect("supervisor shutdown is clean");
}

#[test]
fn a_failed_temporary_write_keeps_the_old_file_and_generation() {
    assert_persistence_failure_keeps_the_old_state("write-temp", PersistFault::WriteTemp);
}

#[test]
fn a_failed_final_replace_keeps_the_old_file_and_generation() {
    assert_persistence_failure_keeps_the_old_state("final-replace", PersistFault::Rename);
}

#[test]
fn save_persists_the_current_generation_and_a_post_skips_blank_and_invalid_values() {
    let fixture = udp_fixture(
        "save-and-skip",
        "# comment only\n",
        free_udp_port(),
        free_tcp_port(),
    );
    let ((), result) = with_host(&fixture, async |_assembly, dns, api| {
        // A file with only a comment yields an empty managed generation.
        assert_eq!(get(api, "/plugins/blocklist/show").await.body, "");
        assert_eq!(udp_rcode(dns, 0x3401, &["a", "example"]).await, 0);

        // Blank, whitespace and comment values are skipped like file lines; an
        // invalid rule is skipped per rule.
        let posted = post_values(
            api,
            "blocklist",
            &["valid.example", "", "   ", "# comment", "regexp:["],
        )
        .await;
        assert_eq!(posted.status, 200);
        assert_eq!(posted.body, "domain_set replaced with 1 entries");
        assert_eq!(
            get(api, "/plugins/blocklist/show").await.body,
            "valid.example\n"
        );
        assert_eq!(fixture.read("rules/blocklist.txt"), "valid.example\n");

        // `/save` rewrites the current generation over external damage.
        fixture.write("rules/blocklist.txt", "garbage.example\n");
        let saved = get(api, "/plugins/blocklist/save").await;
        assert_eq!(saved.status, 200);
        assert_eq!(saved.body, "");
        assert_eq!(fixture.read("rules/blocklist.txt"), "valid.example\n");
        assert_eq!(udp_rcode(dns, 0x3402, &["valid", "example"]).await, 3);
    });
    result.expect("supervisor shutdown is clean");
}

#[test]
fn restart_loads_the_same_effective_rules_a_post_produced() {
    let fixture = udp_fixture("restart", "a.example\n", free_udp_port(), free_tcp_port());
    let committed = {
        let (show, result) = with_host(&fixture, async |_assembly, _dns, api| {
            let posted = post_values(
                api,
                "blocklist",
                &["one.example", "two.example", "regexp:[", "  "],
            )
            .await;
            assert_eq!(posted.status, 200);
            assert_eq!(posted.body, "domain_set replaced with 2 entries");
            get(api, "/plugins/blocklist/show").await.body
        });
        result.expect("supervisor shutdown is clean");
        show
    };
    assert_eq!(committed, "one.example\ntwo.example\n");
    assert_eq!(
        fixture.read("rules/blocklist.txt"),
        "one.example\ntwo.example\n"
    );

    // A fresh host over the same configuration and file is the restart: it must
    // load the same effective rules the POST produced.
    let restarted = assembly_for(&fixture);
    {
        let config = load_and_compile(&fixture.config()).expect("restart config");
        let set = config.domain_set("blocklist").expect("managed tag");
        let provider = set.managed.as_ref().expect("managed provider");
        assert_eq!(
            provider.rules(),
            vec!["one.example".to_owned(), "two.example".to_owned()],
            "a restart must reload the committed generation"
        );
        assert!(set.matches("one.example"));
        assert!(set.matches("two.example"));
    }

    let ((), result) = with_assembly(&restarted, async |_assembly, dns, api| {
        assert_eq!(
            get(api, "/plugins/blocklist/show").await.body,
            "one.example\ntwo.example\n"
        );
        assert_eq!(udp_rcode(dns, 0x3501, &["one", "example"]).await, 3);
        assert_eq!(udp_rcode(dns, 0x3502, &["a", "example"]).await, 0);
    });
    result.expect("supervisor shutdown is clean");
}

#[test]
fn every_observation_is_one_whole_generation() {
    let fixture = udp_fixture(
        "whole-generation",
        "alpha.example\nalpha2.example\n",
        free_udp_port(),
        free_tcp_port(),
    );
    let ((), result) = with_host(&fixture, async |_assembly, dns, api| {
        // Two generations, each with two names. Every observation must show one
        // whole generation: never a mixture and never a half state.
        let generations = [
            (["alpha.example", "alpha2.example"], [3_u16, 3_u16]),
            (["beta.example", "beta2.example"], [3_u16, 3_u16]),
        ];
        let mut id = 0x3600_u16;
        for round in 0..4_usize {
            let (names, expected) = generations[round % 2];
            let posted = post_values(api, "blocklist", &names).await;
            assert_eq!(posted.status, 200, "{posted:?}");
            assert_eq!(posted.body, "domain_set replaced with 2 entries");

            for (index, name) in names.iter().enumerate() {
                id += 1;
                let (label, _) = name.split_once('.').expect("label");
                let observed = udp_rcode(dns, id, &[label, "example"]).await;
                assert_eq!(
                    observed, expected[index],
                    "round {round}: `{name}` must belong to the whole published generation"
                );
            }
            // Both previous-generation names are gone together, proving the
            // replacement swapped the complete rule set at once.
            for name in generations[(round + 1) % 2].0 {
                id += 1;
                let (label, _) = name.split_once('.').expect("label");
                let observed = udp_rcode(dns, id, &[label, "example"]).await;
                assert_eq!(
                    observed, 0,
                    "round {round}: `{name}` belongs to the replaced generation"
                );
            }
        }
        assert_eq!(
            fixture.rule_entries(),
            vec!["blocklist.txt".to_owned()],
            "repeated publication must not leave temporary files"
        );
    });
    result.expect("supervisor shutdown is clean");
}

#[test]
fn a_paused_update_lets_a_real_dns_reader_finish_on_the_old_generation() {
    let fixture = udp_fixture(
        "publish-barrier",
        "a.example\n",
        free_udp_port(),
        free_tcp_port(),
    );
    let (observed, result) = with_host(&fixture, async |assembly, dns, api| {
        let provider = Rc::clone(
            assembly
                .config()
                .domain_set("blocklist")
                .expect("managed tag")
                .managed
                .as_ref()
                .expect("managed provider"),
        );

        // Hold the next update immediately before publication.
        let barrier = TransportCancellation::new();
        provider.inject_publish_barrier(barrier.clone());

        let posted =
            tokio::task::spawn_local(
                async move { post_values(api, "blocklist", &["b.example"]).await },
            );

        // The candidate file is committed before the gate, so waiting for it
        // proves the update reached the persist/publish boundary.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while fixture.read("rules/blocklist.txt") != "b.example\n" {
            assert!(
                std::time::Instant::now() < deadline,
                "the POST never committed its candidate file"
            );
            pause(20).await;
        }
        pause(100).await;

        // A real reader must still be served while the update is parked, and it
        // must observe the whole OLD generation.
        let old_blocked = udp_rcode(dns, 0x3701, &["a", "example"]).await;
        let new_not_yet = udp_rcode(dns, 0x3702, &["b", "example"]).await;
        let show_while_parked = get(api, "/plugins/blocklist/show").await;
        assert_eq!(show_while_parked.status, 200, "the API stays responsive");

        // Release the gate; the same update then publishes and is visible.
        barrier.cancel();
        let posted = posted.await.expect("post task");
        let new_blocked = udp_rcode(dns, 0x3703, &["b", "example"]).await;
        let old_now_free = udp_rcode(dns, 0x3704, &["a", "example"]).await;
        let show_after = get(api, "/plugins/blocklist/show").await;

        // A failing update must still never publish, even with a gate armed.
        provider.inject_persist_fault(PersistFault::Rename);
        let barrier = TransportCancellation::new();
        provider.inject_publish_barrier(barrier.clone());
        let failed = post_values(api, "blocklist", &["c.example"]).await;
        barrier.cancel();
        let after_failure = udp_rcode(dns, 0x3705, &["b", "example"]).await;
        let show_after_failure = get(api, "/plugins/blocklist/show").await;

        (
            old_blocked,
            new_not_yet,
            show_while_parked.body,
            posted.status,
            new_blocked,
            old_now_free,
            show_after.body,
            failed.status,
            after_failure,
            show_after_failure.body,
        )
    });
    result.expect("supervisor shutdown is clean");

    let (
        old_blocked,
        new_not_yet,
        show_while_parked,
        post_status,
        new_blocked,
        old_now_free,
        show_after,
        failed_status,
        after_failure,
        show_after_failure,
    ) = observed;
    assert_eq!(
        old_blocked, 3,
        "a reader during a paused update sees the old generation"
    );
    assert_eq!(
        new_not_yet, 0,
        "the parked candidate must not be visible before publication"
    );
    assert_eq!(
        show_while_parked, "a.example\n",
        "`/show` during a paused update still reports the old generation"
    );
    assert_eq!(post_status, 200, "the released update publishes");
    assert_eq!(new_blocked, 3, "the next reader sees the new generation");
    assert_eq!(old_now_free, 0);
    assert_eq!(show_after, "b.example\n");
    assert_eq!(failed_status, 500, "the injected failure is reported");
    assert_eq!(
        after_failure, 3,
        "a failed update never publishes: the last good generation stays live"
    );
    assert_eq!(show_after_failure, "b.example\n");
}

#[test]
fn a_slow_persistence_step_does_not_block_dns_readers() {
    let fixture = udp_fixture(
        "slow-persist",
        "a.example\n",
        free_udp_port(),
        free_tcp_port(),
    );
    let (observed, result) = with_host(&fixture, async |assembly, dns, api| {
        let provider = Rc::clone(
            assembly
                .config()
                .domain_set("blocklist")
                .expect("managed tag")
                .managed
                .as_ref()
                .expect("managed provider"),
        );

        // Park the blocking persistence step before it writes anything.
        let gate = PersistGate::new();
        provider.inject_persist_gate(gate.clone());
        let posted =
            tokio::task::spawn_local(
                async move { post_values(api, "blocklist", &["b.example"]).await },
            );

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !gate.arrived() {
            assert!(
                std::time::Instant::now() < deadline,
                "the update never entered the persistence step"
            );
            pause(10).await;
        }

        // The persistence step is parked on a blocking thread. The single DNS
        // runtime must still answer a real query, and it must observe the whole
        // old generation while the file is still untouched.
        let old_generation = udp_rcode(dns, 0x3801, &["a", "example"]).await;
        let not_published = udp_rcode(dns, 0x3802, &["b", "example"]).await;
        let file_while_parked = fixture.read("rules/blocklist.txt");

        gate.release();
        let posted = posted.await.expect("post task");
        let published = udp_rcode(dns, 0x3803, &["b", "example"]).await;

        (
            old_generation,
            not_published,
            file_while_parked,
            posted.status,
            published,
        )
    });
    result.expect("supervisor shutdown is clean");

    let (old_generation, not_published, file_while_parked, status, published) = observed;
    assert_eq!(
        old_generation, 3,
        "a real DNS reader must be answered while persistence is parked"
    );
    assert_eq!(not_published, 0, "nothing is published before the write");
    assert_eq!(
        file_while_parked, "a.example\n",
        "the file is untouched while the persistence step is parked"
    );
    assert_eq!(status, 200);
    assert_eq!(published, 3, "the released update publishes normally");
}

#[test]
fn an_approved_post_normalizes_values_across_http_file_show_and_restart() {
    // Approved intentional deviation (2026-09-29): a POST value is normalized
    // like a rule-file line (outer whitespace trimmed, empty and whole-line `#`
    // skipped, then matcher validation) instead of being handed to the matcher
    // verbatim as Go does. This pins the deviation on every surface at once.
    let fixture = udp_fixture(
        "post-normalization",
        "seed.example\n",
        free_udp_port(),
        free_tcp_port(),
    );
    let http_show = {
        let (show, result) = with_host(&fixture, async |_assembly, _dns, api| {
            let posted = post_values(
                api,
                "blocklist",
                &[
                    "  padded.example  ",
                    "",
                    "   ",
                    "# whole-line comment",
                    "valid-after.example",
                ],
            )
            .await;
            assert_eq!(posted.status, 200, "{posted:?}");
            assert_eq!(posted.body, "domain_set replaced with 2 entries");
            get(api, "/plugins/blocklist/show").await.body
        });
        result.expect("supervisor shutdown is clean");
        show
    };
    let expected = "padded.example\nvalid-after.example\n";
    assert_eq!(
        http_show, expected,
        "the direct HTTP response reflects the normalized rules"
    );
    assert_eq!(
        fixture.read("rules/blocklist.txt"),
        expected,
        "the persisted file holds the normalized rules"
    );

    // A restart loads exactly the same effective rules.
    let restarted = assembly_for(&fixture);
    {
        let config = load_and_compile(&fixture.config()).expect("restart config");
        let set = config.domain_set("blocklist").expect("managed tag");
        let provider = set.managed.as_ref().expect("managed provider");
        assert_eq!(
            provider.rules(),
            vec![
                "padded.example".to_owned(),
                "valid-after.example".to_owned()
            ],
            "the restart loads the same normalized rules"
        );
    }
    let ((), result) = with_assembly(&restarted, async |_assembly, dns, api| {
        assert_eq!(
            get(api, "/plugins/blocklist/show").await.body,
            expected,
            "the restarted host shows the same rules"
        );
        assert_eq!(udp_rcode(dns, 0x3901, &["padded", "example"]).await, 3);
        assert_eq!(udp_rcode(dns, 0x3902, &["seed", "example"]).await, 0);
    });
    result.expect("supervisor shutdown is clean");
}

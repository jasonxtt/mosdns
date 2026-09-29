//! Representative configuration composition for the Phase 5B native host.
//!
//! The chain is a deliberate reduction of the local configuration package:
//! top-level include, provider files/exps, qtype/reject, direct child calls,
//! a cache on a child successor, and two forwards. Unrelated plugins are
//! replaced or removed and are recorded as deferred in the coverage table.

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use mosdns_dns_core::{
    inspect_response_header, observe_answer_addresses, parse_query, validate_response,
};
use mosdns_native_host::{
    ConfigError, HostAssembly, ListenerKind, UdpServer, compile_yaml, compile_yaml_with_base,
};

/// Unwraps a configuration rejection, because `CompiledConfig` carries no
/// `Debug` impl and therefore cannot be inspected by `expect_err`.
fn expect_config_error(
    result: Result<mosdns_native_host::CompiledConfig, ConfigError>,
    message: &str,
) -> ConfigError {
    match result {
        Ok(_) => panic!("{message}"),
        Err(error) => error,
    }
}

use mosdns_upstream_core::TransportCancellation;

/// One owned phase5b fixture directory. Rules live beside the config so the
/// include and rule-file resolution can be exercised through real paths.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("phase5b-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub_config")).expect("fixture directory");
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
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const ROOT_CONFIG: &str = r#"
log:
  level: error
include:
  - sub_config/routes.yaml
plugins:
  - tag: sequence_main
    type: sequence
    args:
      - matches: qtype 65
        exec: reject 0
      - matches: qname $blocked
        exec: reject 3
      - exec: $sequence_routed
      - matches: has_resp
        exec: accept
      - exec: $sequence_default
  - tag: listener
    type: udp_server
    args:
      entry: sequence_main
      listen: "127.0.0.1:26353"
      enable_audit: true
"#;

const ROUTES_CONFIG: &str = r#"
plugins:
  - tag: sequence_routed
    type: sequence
    args:
      - matches: qname $local_domains
        exec: $sequence_local
  - tag: sequence_local
    type: sequence
    args:
      - exec: $cache_main
      - exec: $local_forward
  - tag: sequence_default
    type: sequence
    args:
      - exec: $default_forward
  - tag: cache_main
    type: cache
    args:
      size: 64
      lazy_cache_ttl: 0
  - tag: local_forward
    type: forward
    args:
      upstreams:
        - tag: local_peer
          addr: "udp://127.0.0.1:26361"
  - tag: default_forward
    type: forward
    args:
      upstreams:
        - tag: default_peer
          addr: "tcp://127.0.0.1:26362"
  - tag: blocked
    type: domain_set
    args:
      exps:
        - full:blocked.test
        - full:another-blocked.test
  - tag: local_domains
    type: domain_set
    args:
      files:
        - "RULES_PATH"
"#;

/// Writes the representative chain with a directory-relative rule file.
fn write_chain(fixture: &Fixture) {
    fixture.write(
        "sub_config/rules/local.txt",
        "domain:local.test\nfull:local.only.test\n",
    );
    // If an included definition loses its declaring directory, this invalid
    // root-relative decoy makes the regression fail visibly.
    fixture.write("rules/local.txt", "not-a-valid-rule\n");
    fixture.write(
        "sub_config/routes.yaml",
        &ROUTES_CONFIG.replace("RULES_PATH", "rules/local.txt"),
    );
    fixture.write("config.yaml", ROOT_CONFIG);
}

#[test]
fn the_representative_chain_loads_from_file_with_include_and_rule_files() {
    let fixture = Fixture::new("chain");
    write_chain(&fixture);
    let config = mosdns_native_host::load_and_compile(&fixture.config())
        .expect("representative chain must load");
    assert_eq!(config.listener.kind, ListenerKind::Udp);
    assert_eq!(config.listener.entry, "sequence_main");
    assert!(config.listener.enable_audit);
    assert_eq!(config.forwards.len(), 2);
    let cache = config.cache.as_ref().expect("one cache");
    assert_eq!(cache.capacity, 64);
    let tags = config
        .forwards
        .iter()
        .map(|forward| forward.tag.as_str())
        .collect::<Vec<_>>();
    assert!(tags.contains(&"local_forward"), "{tags:?}");
    assert!(tags.contains(&"default_forward"), "{tags:?}");
    let program = &config.program;
    for name in [
        "sequence_main",
        "sequence_routed",
        "sequence_local",
        "sequence_default",
    ] {
        assert!(program.sequence_id(name).is_some(), "{name} must exist");
    }
    // A direct `$sequence` reference is a named child call, not a jump.
    assert_eq!(
        config.sequence.tag, "sequence_main",
        "the entry is the listener's own entry sequence"
    );
}

#[test]
fn a_relative_include_resolves_against_the_declaring_file_not_the_cwd() {
    let fixture = Fixture::new("relative");
    write_chain(&fixture);
    // The same text compiles from any working directory because the base
    // directory travels with the file.
    let config = mosdns_native_host::load_and_compile(&fixture.config())
        .expect("file-backed load resolves the include");
    assert_eq!(config.forwards.len(), 2);

    // The in-memory entry point has no declaring file, so a relative include
    // cannot be resolved and must fail rather than guess a working directory.
    let yaml = fs::read_to_string(fixture.config()).expect("fixture config");
    assert!(
        compile_yaml(&yaml).is_err(),
        "an in-memory compile has no base directory for a relative include"
    );
    assert!(
        compile_yaml_with_base(&yaml, fixture.root.as_path()).is_ok(),
        "an explicit base directory resolves the same include"
    );
}

#[test]
fn a_missing_include_or_rule_file_fails_before_assembly_with_its_path() {
    let fixture = Fixture::new("missing");
    write_chain(&fixture);
    fs::remove_file(fixture.root.join("sub_config/routes.yaml")).expect("remove include");
    let error = expect_config_error(
        mosdns_native_host::load_and_compile(&fixture.config()),
        "a missing include must fail",
    );
    assert!(error.reason.contains("routes.yaml"), "{error}");
    assert!(error.path.starts_with("$."), "{error}");

    let fixture = Fixture::new("missing-rules");
    write_chain(&fixture);
    fs::remove_file(fixture.root.join("sub_config/rules/local.txt")).expect("remove rules");
    let error = expect_config_error(
        mosdns_native_host::load_and_compile(&fixture.config()),
        "a missing rule file must fail",
    );
    assert!(error.reason.contains("local.txt"), "{error}");
}

#[test]
fn included_definitions_keep_their_relative_path_and_source_context() {
    let fixture = Fixture::new("included-relative-rules");
    write_chain(&fixture);
    let config = mosdns_native_host::load_and_compile(&fixture.config())
        .expect("included rule files resolve from the included YAML directory");
    assert_eq!(config.forwards.len(), 2);

    // Go-visible text-file policy: one invalid individual rule is skipped and
    // the remaining valid rules still load instead of aborting the whole file.
    fs::write(
        fixture.root.join("sub_config/rules/local.txt"),
        "regexp:[\nstill-valid.example\n",
    )
    .expect("invalid included rule");
    let config = mosdns_native_host::load_and_compile(&fixture.config())
        .expect("an invalid file rule must be skipped, not fatal");
    assert_eq!(config.forwards.len(), 2);

    // A real file-read failure still retains the included YAML source context
    // and the offending rule-file path.
    fs::remove_file(fixture.root.join("sub_config/rules/local.txt")).expect("remove rules");
    let error = expect_config_error(
        mosdns_native_host::load_and_compile(&fixture.config()),
        "a missing included rule file must retain its included-file source context",
    );
    assert!(
        error
            .path
            .contains("sub_config/routes.yaml.plugins[7].args.files"),
        "included YAML source path was lost: {error}"
    );
    assert!(
        error.reason.contains("sub_config/rules/local.txt"),
        "rule-file path was lost: {error}"
    );
}

#[test]
fn nested_includes_and_deferred_shapes_are_rejected_before_bind() {
    let fixture = Fixture::new("nested");
    write_chain(&fixture);
    let nested = fixture.write(
        "sub_config/deeper.yaml",
        "include:\n  - rules/local.txt\nplugins: []\n",
    );
    let routes = fs::read_to_string(fixture.root.join("sub_config/routes.yaml"))
        .expect("routes")
        .replace(
            "plugins:",
            &format!("include:\n  - {}\nplugins:", nested.display()),
        );
    fixture.write("sub_config/routes.yaml", &routes);
    let error = expect_config_error(
        mosdns_native_host::load_and_compile(&fixture.config()),
        "a nested include must be rejected",
    );
    assert!(error.reason.contains("unsupported field"), "{error}");
}

#[test]
fn reject_beyond_the_supported_wire_range_fails_at_load_time() {
    let fixture = Fixture::new("reject");
    write_chain(&fixture);
    let config = fs::read_to_string(fixture.config()).expect("config");
    fixture.write("config.yaml", &config.replace("reject 3", "reject 16"));
    let error = expect_config_error(
        mosdns_native_host::load_and_compile(&fixture.config()),
        "reject 16 is outside the supported wire range",
    );
    assert!(error.reason.contains("unsupported"), "{error}");
    assert!(error.reason.contains("16"), "{error}");
}

#[test]
fn definition_order_and_multi_exec_do_not_constrain_the_graph() {
    let fixture = Fixture::new("reordered");
    write_chain(&fixture);
    // Move every listener and sequence after the plugins they reference.
    let reordered = r#"
log:
  level: error
plugins:
  - tag: sequence_main
    type: sequence
    args:
      - matches: qname $blocked
        exec: reject 3
      - exec:
          - $sequence_routed
          - $sequence_default
  - tag: sequence_routed
    type: sequence
    args:
      - matches: qname $local_domains
        exec: $local_forward
  - tag: sequence_default
    type: sequence
    args:
      - exec: $default_forward
  - tag: local_domains
    type: domain_set
    args:
      exps:
        - domain:local.test
  - tag: blocked
    type: domain_set
    args:
      exps:
        - full:blocked.test
  - tag: local_forward
    type: forward
    args:
      upstreams:
        - addr: "udp://127.0.0.1:26361"
  - tag: default_forward
    type: forward
    args:
      upstreams:
        - addr: "udp://127.0.0.1:26362"
  - tag: listener
    type: udp_server
    args:
      entry: sequence_main
      listen: "127.0.0.1:26353"
      enable_audit: false
"#;
    let config = compile_yaml(reordered).expect("a reordered graph must compile");
    assert_eq!(config.forwards.len(), 2);
    assert_eq!(
        config.forward.as_ref().expect("primary forward").tag,
        "local_forward"
    );
    assert!(
        config
            .program
            .sequences
            .iter()
            .any(|sequence| sequence.synthetic),
        "the multi-exec list lowers to a synthetic inline scope"
    );
}

#[test]
fn duplicate_tags_and_cross_type_references_fail_before_bind() {
    let fixture = Fixture::new("duplicate");
    write_chain(&fixture);
    let routes = fs::read_to_string(fixture.root.join("sub_config/routes.yaml")).expect("routes");
    // A duplicate tag declared in the included file must fail even though the
    // root file never repeats it.
    fixture.write(
        "sub_config/routes.yaml",
        &format!("{routes}\n  - tag: blocked\n    type: domain_set\n    args:\n      exps:\n        - full:other.test\n"),
    );
    let error = expect_config_error(
        mosdns_native_host::load_and_compile(&fixture.config()),
        "duplicate tag must fail",
    );
    assert!(error.reason.contains("duplicate"), "{error}");

    // A `$` reference that names a domain_set is not executable.
    let fixture = Fixture::new("cross-type");
    write_chain(&fixture);
    let config = fs::read_to_string(fixture.config())
        .expect("config")
        .replace("exec: $sequence_default", "exec: $blocked");
    fixture.write("config.yaml", &config);
    let error = expect_config_error(
        mosdns_native_host::load_and_compile(&fixture.config()),
        "a domain_set reference must not be executable",
    );
    assert!(error.reason.contains("not executable"), "{error}");
}

/// One peer fixture that answers every A query with its own fixed address and
/// counts the requests it received, so the two upstreams stay distinguishable.
struct Peer {
    address: SocketAddr,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Peer {
    fn start(answer_address: [u8; 4]) -> Self {
        let bind: SocketAddr = "127.0.0.1:0".parse().expect("bind address");
        let requests = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);
        let socket = std::net::UdpSocket::bind(bind).expect("peer bind");
        socket
            .set_read_timeout(Some(Duration::from_millis(10)))
            .expect("peer timeout");
        let address = socket.local_addr().expect("peer address");
        let thread = thread::spawn(move || {
            let mut input = vec![0_u8; 65535];
            while !thread_stop.load(Ordering::SeqCst) {
                let Ok((length, peer)) = socket.recv_from(&mut input) else {
                    continue;
                };
                thread_requests.fetch_add(1, Ordering::SeqCst);
                if let Some(response) = answer(&input[..length], answer_address) {
                    let _ = socket.send_to(&response, peer);
                }
            }
        });
        Self {
            address,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("peer thread");
        }
    }
}

fn answer(query: &[u8], address: [u8; 4]) -> Option<Vec<u8>> {
    let (header, question) = parse_query(query).ok()?;
    let mut response = Vec::new();
    response.extend_from_slice(&header.id.to_be_bytes());
    response.extend_from_slice(&0x8180_u16.to_be_bytes());
    response.extend_from_slice(&1_u16.to_be_bytes());
    response.extend_from_slice(&1_u16.to_be_bytes());
    response.extend_from_slice(&0_u16.to_be_bytes());
    response.extend_from_slice(&0_u16.to_be_bytes());
    response.extend_from_slice(&question.qname_wire);
    response.extend_from_slice(&question.qtype.to_be_bytes());
    response.extend_from_slice(&question.qclass.to_be_bytes());
    response.extend_from_slice(&[0xc0, 0x0c]);
    response.extend_from_slice(&question.qtype.to_be_bytes());
    response.extend_from_slice(&[0, 1]);
    response.extend_from_slice(&60_u32.to_be_bytes());
    response.extend_from_slice(&[0, 4]);
    response.extend_from_slice(&address);
    Some(response)
}

/// The controlled answer addresses that keep the two peers distinguishable.
const LOCAL_ANSWER: [u8; 4] = [192, 0, 2, 21];
const DEFAULT_ANSWER: [u8; 4] = [192, 0, 2, 22];

fn query(id: u16, name: &str, qtype: u16) -> Vec<u8> {
    let mut packet = Vec::from([
        u8::try_from(id >> 8).expect("id high"),
        u8::try_from(id & 0xff).expect("id low"),
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
    for label in name.trim_end_matches('.').split('.') {
        packet.push(u8::try_from(label.len()).expect("label length"));
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&qtype.to_be_bytes());
    packet.extend_from_slice(&1_u16.to_be_bytes());
    packet
}

fn rcode(response: &[u8]) -> u8 {
    response[3] & 0x0f
}

/// The A/AAAA addresses in the answer section, read through the same dns-core
/// walk the host uses rather than a hand-rolled offset.
fn answer_addresses(response: &[u8]) -> Vec<std::net::IpAddr> {
    observe_answer_addresses(response).expect("valid response")
}

fn client_request(listener: SocketAddr, request: &[u8], timeout: Duration) -> Vec<u8> {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("client bind");
    socket
        .set_read_timeout(Some(timeout))
        .expect("client timeout");
    socket.send_to(request, listener).expect("client send");
    let mut response = vec![0_u8; 65535];
    let (length, _) = socket.recv_from(&mut response).expect("client response");
    response[..length].to_vec()
}

/// Builds the representative chain with both peers pointed at live fixtures.
fn chain_assembly(fixture: &Fixture, local: SocketAddr, default: SocketAddr) -> HostAssembly {
    fixture.write(
        "sub_config/rules/local.txt",
        "domain:local.test\nfull:local.only.test\n",
    );
    fixture.write("rules/local.txt", "not-a-valid-rule\n");
    fixture.write(
        "sub_config/routes.yaml",
        &ROUTES_CONFIG
            .replace("RULES_PATH", "rules/local.txt")
            .replace("udp://127.0.0.1:26361", &format!("udp://{local}"))
            .replace("tcp://127.0.0.1:26362", &format!("udp://{default}")),
    );
    fixture.write("config.yaml", ROOT_CONFIG);
    HostAssembly::from_config_file(&fixture.config()).expect("chain assembly")
}

#[test]
fn goto_and_try_only_forward_paths_compile_and_execute() {
    for control in ["goto", "try"] {
        let peer = Peer::start([192, 0, 2, 31]);
        let yaml = format!(
            r#"
log: {{ level: error }}
plugins:
  - tag: entry
    type: sequence
    args:
      - exec: {control} $child
  - tag: child
    type: sequence
    args:
      - exec: $forward
  - tag: forward
    type: forward
    args:
      upstreams:
        - addr: "udp://{}"
  - tag: listener
    type: udp_server
    args:
      entry: entry
      listen: "127.0.0.1:26353"
      enable_audit: false
"#,
            peer.address
        );
        let assembly = HostAssembly::from_yaml(&yaml)
            .unwrap_or_else(|error| panic!("{control} composition must compile: {error}"));
        assert_eq!(
            assembly
                .config()
                .forward
                .as_ref()
                .expect("legacy forward view")
                .tag,
            "forward"
        );
        let server = assembly
            .block_on(UdpServer::bind(
                &assembly,
                "127.0.0.1:0".parse().expect("bind"),
            ))
            .expect("listener bind");
        let listener = server.local_addr().expect("listener address");
        let shutdown = TransportCancellation::new();
        let response = assembly.block_on(async {
            let task = tokio::task::spawn_local(server.serve(shutdown.clone()));
            let request = query(0x4101, "control.test.", 1);
            let response = tokio::task::spawn_blocking(move || {
                client_request(listener, &request, Duration::from_secs(2))
            })
            .await
            .expect("client");
            shutdown.cancel();
            task.await.expect("server").expect("shutdown");
            response
        });
        validate_response(&response).expect("valid response");
        assert_eq!(rcode(&response), 0);
        assert_eq!(
            answer_addresses(&response),
            vec![std::net::IpAddr::V4([192, 0, 2, 31].into())]
        );
        assert_eq!(peer.requests(), 1, "{control} must reach the forward");
        peer.stop();
    }
}

#[test]
fn the_representative_chain_routes_blocks_and_falls_through_to_default() {
    let fixture = Fixture::new("routing");
    let local = Peer::start(LOCAL_ANSWER);
    let default = Peer::start(DEFAULT_ANSWER);
    let assembly = chain_assembly(&fixture, local.address, default.address);
    let server = assembly
        .block_on(UdpServer::bind(
            &assembly,
            "127.0.0.1:0".parse().expect("bind"),
        ))
        .expect("listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let responses = assembly.block_on(async {
        let task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let mut responses = Vec::new();
        for (id, name, qtype) in [
            (0x5001_u16, "blocked.test.", 1_u16),
            (0x5002, "another-blocked.test.", 1),
            (0x5003, "other.test.", 65),
            (0x5004, "a.local.test.", 1),
            (0x5005, "other.test.", 1),
        ] {
            let request = query(id, name, qtype);
            responses.push(
                tokio::task::spawn_blocking(move || {
                    client_request(listener, &request, Duration::from_secs(2))
                })
                .await
                .expect("client"),
            );
        }
        // A repeat of the local-suffix name inside the same server session is
        // a cache hit in the child sequence: it keeps the child's cached
        // answer, skips the local peer, and never reaches the default peer.
        for id in [0x5006_u16, 0x5007] {
            let request = query(id, "a.local.test.", 1);
            responses.push(
                tokio::task::spawn_blocking(move || {
                    client_request(listener, &request, Duration::from_secs(2))
                })
                .await
                .expect("hit client"),
            );
        }
        shutdown.cancel();
        task.await.expect("server").expect("shutdown");
        responses
    });
    for (index, response) in responses.iter().enumerate() {
        let header = inspect_response_header(response).expect("header");
        assert_eq!(header.id, 0x5001 + u16::try_from(index).expect("index"));
        validate_response(response).expect("valid response");
    }
    // The block and qtype rejects answer locally without any upstream call.
    assert_eq!(rcode(&responses[0]), 3, "blocked.test is NXDOMAIN");
    assert_eq!(rcode(&responses[1]), 3, "another-blocked.test is NXDOMAIN");
    assert_eq!(rcode(&responses[2]), 0, "HTTPS is rejected with NOERROR");
    assert_eq!(rcode(&responses[3]), 0, "the local rule answers NOERROR");
    assert_eq!(
        answer_addresses(&responses[3]),
        vec![std::net::IpAddr::V4(LOCAL_ANSWER.into())]
    );
    for hit in &responses[5..7] {
        assert_eq!(rcode(hit), 0);
        assert_eq!(
            answer_addresses(hit),
            vec![std::net::IpAddr::V4(LOCAL_ANSWER.into())],
            "the child cache must serve its own successor answer"
        );
    }
    assert_eq!(
        local.requests(),
        1,
        "only the first local-rule query reaches the local peer"
    );
    assert_eq!(
        default.requests(),
        1,
        "only the routed miss reaches the default peer"
    );
    let audit = assembly.audit_snapshot();
    assert_eq!(
        audit.records.len(),
        7,
        "audit on records every admitted query"
    );
    assert_eq!(
        audit.records[3].cache_status,
        mosdns_native_host::CacheStatus::Miss
    );
    assert_eq!(
        audit.records[5].cache_status,
        mosdns_native_host::CacheStatus::Hit
    );
    assert_eq!(
        audit.records[3].final_sequence.as_deref(),
        Some("sequence_main"),
        "the audited position is the real executing sequence"
    );

    local.stop();
    default.stop();
}

/// The same representative chain behind a TCP listener with audit disabled:
/// the wire and peer counts must not change, and no audit record is retained.
#[test]
fn the_representative_chain_also_serves_tcp_with_audit_off() {
    let fixture = Fixture::new("tcp");
    let local = Peer::start(LOCAL_ANSWER);
    let default = Peer::start(DEFAULT_ANSWER);
    fixture.write(
        "sub_config/rules/local.txt",
        "domain:local.test\nfull:local.only.test\n",
    );
    fixture.write("rules/local.txt", "not-a-valid-rule\n");
    fixture.write(
        "sub_config/routes.yaml",
        &ROUTES_CONFIG
            .replace("RULES_PATH", "rules/local.txt")
            .replace("udp://127.0.0.1:26361", &format!("udp://{}", local.address))
            .replace(
                "tcp://127.0.0.1:26362",
                &format!("udp://{}", default.address),
            ),
    );
    fixture.write(
        "config.yaml",
        &ROOT_CONFIG
            .replace("type: udp_server", "type: tcp_server")
            .replace(
                "      enable_audit: true",
                "      idle_timeout: 5\n      enable_audit: false",
            ),
    );
    let assembly = HostAssembly::from_config_file(&fixture.config()).expect("TCP chain assembly");
    let server = assembly
        .block_on(mosdns_native_host::TcpServer::bind(
            &assembly,
            "127.0.0.1:0".parse().expect("bind"),
        ))
        .expect("TCP listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let responses = assembly.block_on(async {
        let task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let mut responses = Vec::new();
        for (id, name, qtype) in [
            (0x6001_u16, "blocked.test.", 1_u16),
            (0x6002, "a.local.test.", 1),
            (0x6003, "unmatched.test.", 1),
        ] {
            let request = query(id, name, qtype);
            responses.push(
                tokio::task::spawn_blocking(move || {
                    tcp_request(listener, &request, Duration::from_secs(3))
                })
                .await
                .expect("TCP client"),
            );
        }
        shutdown.cancel();
        task.await.expect("server").expect("shutdown");
        responses
    });
    for (index, response) in responses.iter().enumerate() {
        let header = inspect_response_header(response).expect("header");
        assert_eq!(header.id, 0x6001 + u16::try_from(index).expect("index"));
        validate_response(response).expect("valid TCP response");
    }
    assert_eq!(rcode(&responses[0]), 3, "blocked.test is NXDOMAIN over TCP");
    assert_eq!(
        answer_addresses(&responses[1]),
        vec![std::net::IpAddr::V4(LOCAL_ANSWER.into())]
    );
    assert_eq!(
        answer_addresses(&responses[2]),
        vec![std::net::IpAddr::V4(DEFAULT_ANSWER.into())]
    );
    assert_eq!(local.requests(), 1, "TCP local leg is the same chain");
    assert_eq!(default.requests(), 1, "TCP default leg is the same chain");
    assert!(
        assembly.audit_snapshot().records.is_empty(),
        "audit off must retain no per-query record"
    );
    local.stop();
    default.stop();
}

/// One TCP DNS exchange with the two-byte length prefix.
fn tcp_request(listener: SocketAddr, request: &[u8], timeout: Duration) -> Vec<u8> {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(listener).expect("TCP connect");
    stream
        .set_read_timeout(Some(timeout))
        .expect("TCP read timeout");
    let length = u16::try_from(request.len()).expect("request length");
    stream
        .write_all(&length.to_be_bytes())
        .expect("TCP length write");
    stream.write_all(request).expect("TCP request write");
    let mut prefix = [0_u8; 2];
    stream.read_exact(&mut prefix).expect("TCP length read");
    let mut response = vec![0_u8; usize::from(u16::from_be_bytes(prefix))];
    stream.read_exact(&mut response).expect("TCP response read");
    response
}

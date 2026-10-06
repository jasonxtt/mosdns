use mosdns_dns_core::{QueryHeader, QuestionInfo};
use mosdns_native_host::{compile_yaml_with_base, load_and_compile};
use mosdns_sequence_core::{ExecutionControl, ExecutionState, ExecutorOutcome, MachineStep};
use serde_json::json;
use std::path::Path;

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "special-profile-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        for sub in ["webinfo", "srs", "rule", "cache"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        Self(dir)
    }
    fn write(&self, path: &str, value: impl AsRef<[u8]>) {
        std::fs::write(self.0.join(path), value).unwrap();
    }
    fn groups(&self, value: &serde_json::Value) {
        self.write(
            "webinfo/special_upstream_groups.json",
            serde_json::to_vec(&value).unwrap(),
        );
    }
    fn compile(
        &self,
    ) -> Result<mosdns_native_host::CompiledConfig, mosdns_native_host::ConfigError> {
        compile_yaml_with_base(EMPTY, &self.0)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const EMPTY: &str = r#"
log: {level: error}
native_management: {special_groups: true}
plugins:
  - tag: default
    type: forward
    args: {upstreams: [{addr: "udp://127.0.0.1:15499"}]}
  - tag: entry
    type: sequence
    args:
      - exec: $special_upstream_matcher
      - exec: reject 3
  - tag: main
    type: udp_server
    args: {entry: entry, listen: "127.0.0.1:15399", enable_audit: true}
"#;

#[test]
fn empty_managed_profile_compiles_at_explicit_hook() {
    let root = std::env::temp_dir().join(format!("special-empty-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let config = compile_yaml_with_base(EMPTY, &root).unwrap();
    assert_eq!(config.listener.entry, "entry");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn opt_in_requires_a_reachable_hook() {
    for yaml in [
        EMPTY.replace("      - exec: $special_upstream_matcher\n", ""),
        EMPTY.replace(
            "      - exec: $special_upstream_matcher",
            "      - exec: exit\n      - exec: $special_upstream_matcher",
        ),
    ] {
        let error = compile_yaml_with_base(&yaml, Path::new("")).err().unwrap();
        assert!(error.reason.contains("hook"), "{error}");
    }
    let yaml = EMPTY
        .replace(
            "plugins:\n",
            "plugins:\n  - tag: stopper\n    type: sequence\n    args: [{exec: exit}]\n",
        )
        .replace(
            "      - exec: $special_upstream_matcher",
            "      - exec: $stopper\n      - exec: $special_upstream_matcher",
        );
    assert!(
        compile_yaml_with_base(&yaml, Path::new(""))
            .err()
            .unwrap()
            .reason
            .contains("hook")
    );
}

#[test]
fn legacy_aliapi_wrapper_does_not_authorize_signed_api_entries() {
    let fixture = Fixture::new();
    let ordinary = EMPTY.replace("    type: forward", "    type: aliapi");
    assert!(compile_yaml_with_base(&ordinary, &fixture.0).is_ok());
    let signed = ordinary.replace(
        "{addr: \"udp://127.0.0.1:15499\"}",
        "{addr: \"udp://127.0.0.1:15499\", account_id: \"unsupported\"}",
    );
    assert!(compile_yaml_with_base(&signed, &fixture.0).is_err());
}

#[test]
fn file_relative_managed_state_is_not_process_relative() {
    let root = std::env::temp_dir().join(format!("special-relative-{}", std::process::id()));
    std::fs::create_dir_all(root.join("webinfo")).unwrap();
    std::fs::write(root.join("config.yaml"), EMPTY).unwrap();
    std::fs::write(
        root.join("webinfo/special_upstream_groups.json"),
        "[{\"slot\":49,\"name\":\"invalid\"}]",
    )
    .unwrap();
    let error = load_and_compile(&root.join("config.yaml")).err().unwrap();
    assert!(error.reason.contains("slot"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn group_identity_priority_and_listener_inventory_are_compiled_from_generated_input() {
    let fixture = Fixture::new();
    fixture.groups(&json!([
        {"slot":51,"name":"second","listen_port":15401,"custom_port_only":true},
        {"slot":50,"name":" first ","listen_port":0,"custom_port_only":true}
    ]));
    let config = fixture.compile().unwrap();
    let profile = config.managed_profile.as_ref().unwrap();
    assert_eq!(profile.groups[0].slot, 50);
    assert_eq!(profile.groups[0].name, "first");
    assert!(!profile.groups[0].custom_port_only);
    assert_eq!(config.listeners.len(), 3);
    assert_eq!(
        config.listeners[1].kind,
        mosdns_native_host::ListenerKind::Udp
    );
    assert_eq!(
        config.listeners[2].kind,
        mosdns_native_host::ListenerKind::Tcp
    );
    assert_eq!(config.listeners[1].listen.port(), 15401);
    assert_eq!(config.listeners[2].entry, "sequence_special_51");
    let router = config.managed_router.as_ref().unwrap();
    assert_eq!(router.groups.len(), 1);
    assert_eq!(
        config.domain_sets[router.groups[0].providers[0]].tag,
        "special_route_50"
    );
    assert_eq!(
        config
            .program
            .sequence(router.groups[0].child)
            .unwrap()
            .name,
        "sequence_special_50"
    );
    assert_eq!(
        config
            .caches
            .iter()
            .find(|c| c.tag == "cache_special_50")
            .unwrap()
            .capacity,
        20_000_000
    );
    // The exported artifact is the actual compilation input, not a hidden graph.
    let generated: serde_json::Value = yaml_serde::from_str(&profile.generated_yaml).unwrap();
    assert_eq!(
        generated["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["type"] == "sequence")
            .count(),
        3
    );
}

#[test]
fn conflicting_schema_or_reserved_generated_names_fail_before_assembly() {
    let fixture = Fixture::new();
    for groups in [
        json!([{"slot":50,"name":"a"},{"slot":50,"name":"b"}]),
        json!([{"slot":50,"name":" A "},{"slot":51,"name":"a"}]),
        json!([{"slot":50,"name":"a","listen_port":53}]),
        json!([{"slot":50,"name":"a","listen_port":15400},{"slot":51,"name":"b","listen_port":15400}]),
        json!([{"slot":50,"name":"a","listen_port":65536}]),
    ] {
        fixture.groups(&groups);
        assert!(fixture.compile().is_err());
    }
    fixture.groups(&json!([]));
    let collision = EMPTY.replace("  - tag: default", "  - tag: special_upstream_50");
    assert!(
        compile_yaml_with_base(&collision, &fixture.0)
            .err()
            .unwrap()
            .reason
            .contains("collision")
    );
    for include in [
        "sub_config/special_groups.yaml",
        "./sub_config/../sub_config/special_groups.yaml",
    ] {
        let yaml = format!("include: [{include}]\n{EMPTY}");
        assert!(
            compile_yaml_with_base(&yaml, &fixture.0)
                .err()
                .unwrap()
                .reason
                .contains("explicitly included")
        );
    }
}

#[test]
fn standard_dns_overrides_preserve_disabled_unsupported_records_and_reject_activation() {
    let fixture = Fixture::new();
    fixture.groups(&json!([{"slot":50,"name":"a"}]));
    let disabled = json!({"tag":"unsupported","enabled":false,"protocol":"aliapi","account_id":"placeholder","unknown_stored_option":{"keep":true}});
    let entries = json!([{"tag":"local","enabled":true,"protocol":"doh","addr":"https://dns.invalid","dial_addr":"127.0.0.1:15443","insecure_skip_verify":true,"idle_timeout":0,"enable_pipeline":false},disabled]);
    fixture.write(
        "webinfo/upstream_overrides.json",
        serde_json::to_vec(&json!({"special_upstream_50":entries})).unwrap(),
    );
    let config = fixture.compile().unwrap();
    let definition = config
        .forward_definitions
        .iter()
        .find(|d| d.tag == "special_upstream_50")
        .unwrap();
    assert_eq!(definition.entries.len(), 1);
    assert_eq!(
        definition.entries[0].target.service,
        "https://dns.invalid/dns-query"
    );
    assert_eq!(
        config.managed_profile.unwrap().overrides["special_upstream_50"][1],
        disabled
    );
    for invalid in [
        json!([{"tag":"a","enabled":false,"protocol":"udp","addr":"127.0.0.1:15400"}]),
        json!([{"tag":"a","enabled":true,"protocol":"aliapi","addr":"127.0.0.1:15400"}]),
        json!([{"tag":"a","enabled":true,"protocol":"udp","addr":"tcp://127.0.0.1:15400"}]),
        json!([{"tag":"a","enabled":true,"protocol":"tcp","addr":"127.0.0.1:15400","enable_pipeline":true}]),
        json!([{"tag":"a","enabled":true,"protocol":"dot","addr":"127.0.0.1:15443","idle_timeout":1}]),
    ] {
        assert!(mosdns_native_host::forward_entries(invalid.as_array().unwrap(), "test").is_err());
    }
}

#[test]
fn enabled_sources_are_strict_and_disabled_unsupported_data_remains_visible() {
    let fixture = Fixture::new();
    fixture.groups(&json!([{"slot":50,"name":"a"}]));
    let disabled = json!({"name":"deferred","type":"special_50","enabled":false,"files":"binary.srs","url":"https://invalid.example/rules","auto_update":true,"opaque":7});
    fixture.write(
        "srs/special_50.json",
        serde_json::to_vec(&json!({"deferred":disabled})).unwrap(),
    );
    assert_eq!(
        fixture.compile().unwrap().managed_profile.unwrap().sources[&50]["deferred"],
        disabled
    );
    fixture.write("srs/special_50.json",serde_json::to_vec(&json!({"local":{"name":"local","type":"special_50","enabled":true,"files":"rule/local.txt","url":"","auto_update":false}})).unwrap());
    assert!(fixture.compile().is_err()); // Missing enabled file must fail.
    fixture.write("rule/local.txt", b"not_a_supported_kind:value\n");
    assert!(fixture.compile().is_err()); // Tolerant unmanaged parsing cannot mask this.
    fixture.write("rule/local.txt", b"");
    assert!(
        !fixture
            .compile()
            .unwrap()
            .domain_set("special_route_50")
            .unwrap()
            .matches("example.test")
    );
}

#[test]
fn enabled_sources_reject_unknown_fields_even_with_valid_local_text() {
    let fixture = Fixture::new();
    fixture.groups(&json!([{"slot":50,"name":"a"}]));
    fixture.write("rule/local.txt", b"full:example.test\n");
    for opaque in [json!(7), json!(false), json!(null)] {
        fixture.write(
            "srs/special_50.json",
            serde_json::to_vec(&json!({"local": {
                "name":"local", "type":"special_50", "enabled":true,
                "files":"rule/local.txt", "url":"", "auto_update":false,
                "enable_regexp":false, "opaque":opaque
            }}))
            .unwrap(),
        );
        let error = fixture
            .compile()
            .err()
            .expect("unknown enabled source field must reject");
        assert!(error.reason.contains("opaque"), "{error}");
    }
}

fn state(name: &str, qtype: u16) -> ExecutionState {
    let mut wire = Vec::new();
    for label in name.split('.') {
        wire.push(u8::try_from(label.len()).unwrap());
        wire.extend(label.as_bytes());
    }
    wire.push(0);
    ExecutionState::new(
        QueryHeader {
            id: 7,
            qr: false,
            opcode: 0,
            qdcount: 1,
            ancount: 0,
            nscount: 0,
            arcount: 0,
        },
        QuestionInfo {
            qname_wire: wire,
            qtype,
            qclass: 1,
        },
    )
}

#[test]
fn compiled_router_selects_first_provider_group_and_never_falls_through() {
    let fixture = Fixture::new();
    fixture.groups(&json!([{"slot":51,"name":"second"},{"slot":50,"name":"first"}]));
    for slot in [50, 51] {
        fixture.write(&format!("rule/special_{slot}.txt"), b"full:example.test\n");
    }
    fixture.write("rule/diversion.txt", b"full:example.test\n");
    fixture.write("srs/special_50.json",serde_json::to_vec(&json!({"local":{"name":"local","type":"special_50","enabled":true,"files":"rule/diversion.txt"}})).unwrap());
    let config = fixture.compile().unwrap();
    // Non-address queries route too. Diversion precedes overlapping manual.
    assert!(
        config
            .domain_set("special_manual_50")
            .unwrap()
            .managed
            .is_none()
    );
    let mut machine = config
        .new_machine(state("example.test", 16), ExecutionControl::with_fuel(64))
        .unwrap();
    let MachineStep::Dispatch(cache) = machine.step().unwrap() else {
        panic!("group cache must run")
    };
    assert_eq!(
        cache.executable(),
        config
            .caches
            .iter()
            .find(|c| c.tag == "cache_special_50")
            .unwrap()
            .executable
    );
    assert_eq!(
        machine.state().routing.domain_set.as_deref(),
        Some("special_route_50")
    );
    assert_eq!(
        machine.state().routing.matched_group.as_deref(),
        Some("special_50")
    );
    let MachineStep::Dispatch(forward) = machine
        .resume(cache.executable(), Ok(ExecutorOutcome::Continue))
        .unwrap()
    else {
        panic!("group forward must run")
    };
    let invocation = config
        .forward_invocations
        .iter()
        .find(|i| i.executable == forward.executable())
        .unwrap();
    assert_eq!(
        config.forward_definitions[invocation.definition].tag,
        "special_upstream_50"
    );
    machine.state_mut().set_synthesized_response(3).unwrap();
    let MachineStep::Dispatch(cname) = machine
        .resume(forward.executable(), Ok(ExecutorOutcome::Continue))
        .unwrap()
    else {
        panic!("final policy")
    };
    assert!(matches!(
        machine
            .resume(cname.executable(), Ok(ExecutorOutcome::Continue))
            .unwrap(),
        MachineStep::Complete(mosdns_sequence_core::ExecutionCompletion::Exited)
    ));
    let mut unmatched = config
        .new_machine(state("unmatched.test", 1), ExecutionControl::with_fuel(64))
        .unwrap();
    assert!(matches!(
        unmatched.step().unwrap(),
        MachineStep::Complete(_)
    ));
    assert!(unmatched.state().routing.matched_group.is_none());
}

#[test]
fn real_group_dns_normalizes_before_cache_and_reports_actual_supplier() {
    assert_group_cache_supplier(false);
}

#[test]
fn real_group_cache_keeps_supplier_when_warmed_without_audit() {
    assert_group_cache_supplier(true);
}

fn assert_group_cache_supplier(warm_without_audit: bool) {
    use hickory_proto::op::{Message, MessageType};
    use hickory_proto::rr::{Name, RData, Record, rdata};
    use mosdns_native_host::{CacheStatus, HostAssembly, HostOptions, UdpServer};
    use mosdns_upstream_core::TransportCancellation;
    use std::net::UdpSocket;
    use std::time::Duration;
    let fixture = Fixture::new();
    fixture.groups(&json!([{"slot":50,"name":"first"}]));
    fixture.write("rule/special_50.txt", b"full:example.test\n");
    let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let peer_address = peer.local_addr().unwrap();
    fixture.write("webinfo/upstream_overrides.json",serde_json::to_vec(&json!({"special_upstream_50":[{"tag":"actual_supplier","enabled":true,"protocol":"udp","addr":peer_address.to_string()}]})).unwrap());
    let worker = std::thread::spawn(move || {
        let mut buffer = vec![0; 65535];
        let (length, remote) = peer.recv_from(&mut buffer).unwrap();
        let mut response = Message::from_vec(&buffer[..length]).unwrap();
        let name = response.queries()[0].name().clone();
        let target = Name::from_ascii("target.test.").unwrap();
        response
            .set_message_type(MessageType::Response)
            .set_recursion_available(true);
        response.add_answer(Record::from_rdata(
            name,
            30,
            RData::CNAME(rdata::CNAME(target.clone())),
        ));
        response.add_answer(Record::from_rdata(
            target,
            30,
            RData::A(rdata::A("192.0.2.50".parse().unwrap())),
        ));
        peer.send_to(&response.to_vec().unwrap(), remote).unwrap();
    });
    let host =
        HostAssembly::with_options(fixture.compile().unwrap(), HostOptions::default()).unwrap();
    if warm_without_audit {
        assert!(host.stop_audit());
    }
    let server = host
        .block_on(UdpServer::bind(&host, "127.0.0.1:0".parse().unwrap()))
        .unwrap();
    let address = server.local_addr().unwrap();
    let cancel = TransportCancellation::new();
    host.block_on(async {
        let serving = tokio::task::spawn_local(server.serve(cancel.clone()));
        for id in [10, 11] {
            if id == 11 && warm_without_audit {
                assert!(host.start_audit());
            }
            let response = tokio::task::spawn_blocking(move || query_group_supplier(address, id))
                .await
                .unwrap();
            assert_eq!(response.answers().len(), 1);
            assert_eq!(response.answers()[0].name().to_ascii(), "example.test.");
            assert_eq!(response.answers()[0].data().to_string(), "192.0.2.50");
        }
        cancel.cancel();
        serving.await.unwrap().unwrap();
    });
    worker.join().unwrap();
    let audit = host.audit_snapshot();
    assert_eq!(audit.records.len(), if warm_without_audit { 1 } else { 2 });
    if !warm_without_audit {
        assert_eq!(
            audit.records[0].final_upstream.as_deref(),
            Some("actual_supplier")
        );
        assert_eq!(
            audit.records[0].matched_group.as_deref(),
            Some("special_50")
        );
    }
    let hit = audit.records.last().unwrap();
    assert_eq!(hit.cache_status, CacheStatus::Hit);
    assert_eq!(hit.final_upstream.as_deref(), Some("actual_supplier"));
    assert_eq!(
        hit.selected_upstream.as_deref(),
        Some(peer_address.to_string().as_str())
    );
    assert!(
        hit.upstream_attempts.is_empty(),
        "a hit starts no network attempt"
    );
    let hit_diagnostics = hit.upstream_diagnostics.as_ref().unwrap();
    assert_eq!(
        hit_diagnostics.selected.as_ref().unwrap().entry,
        "actual_supplier"
    );
    assert!(
        hit_diagnostics.attempts.is_empty(),
        "cached provenance is not a current attempt"
    );
}

fn query_group_supplier(address: std::net::SocketAddr, id: u16) -> hickory_proto::op::Message {
    use hickory_proto::op::{Message, Query};
    use hickory_proto::rr::{Name, RecordType};
    use std::net::UdpSocket;
    use std::time::Duration;
    let mut query = Message::new();
    query.set_id(id).add_query(Query::query(
        Name::from_ascii("example.test.").unwrap(),
        RecordType::A,
    ));
    let client = UdpSocket::bind("127.0.0.1:0").unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    client.send_to(&query.to_vec().unwrap(), address).unwrap();
    let mut buffer = vec![0; 65535];
    let length = client.recv(&mut buffer).unwrap();
    Message::from_vec(&buffer[..length]).unwrap()
}

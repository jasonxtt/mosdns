use mosdns_dns_core::{QueryHeader, QuestionInfo};
use mosdns_native_host::{AssemblyError, HostAssembly, ListenerKind, compile_yaml};
use mosdns_sequence_core::{
    ExecutionControl, ExecutionState, ExecutorOutcome, MachineStep, ValidatedExecutable,
};

const UDP: &str = include_str!("../../../tests/phase5a-baseline/configs/forward-udp.yaml");
const CACHE: &str = include_str!("../../../tests/phase5a-baseline/configs/cache.yaml");
const ROUTING: &str = include_str!("../../../tests/phase5a-baseline/configs/routing.yaml");

#[test]
fn invalid_configuration_is_rejected_before_pre_io_assembly() {
    let invalid = UDP.replace(
        "addr: \"udp://127.0.0.1:15453\"",
        "addr: \"udp://dns.example:15453\"",
    );
    assert!(compile_yaml(&invalid).is_err());
    assert!(matches!(
        HostAssembly::from_yaml(&invalid),
        Err(AssemblyError::Config(_))
    ));
}

#[test]
fn assembly_owns_one_upstream_and_no_listener_socket() {
    let assembly = HostAssembly::from_yaml(UDP).expect("frozen graph must assemble");
    assert_eq!(assembly.config().listener.listen.port(), 15353);
    assert_eq!(assembly.forward().endpoint().address().port(), 15453);
}

#[test]
fn frozen_w2_cache_configuration_compiles_to_udp_graph() {
    let config = compile_yaml(CACHE).expect("frozen W2 graph must compile");
    assert!(config.cache.is_some());
    assert_eq!(config.listener.kind, ListenerKind::Udp);
    assert_eq!(config.listener.entry, "phase5a_entry");
    assert_eq!(config.forward.endpoint.address().port(), 15455);
    assert_eq!(config.listener.listen.port(), 15355);
    let cache = config.cache.as_ref().expect("compiled cache identity");
    let mut machine = config
        .new_machine(
            ExecutionState::new(
                QueryHeader {
                    id: 1,
                    qr: false,
                    opcode: 0,
                    qdcount: 1,
                    ancount: 0,
                    nscount: 0,
                    arcount: 0,
                },
                QuestionInfo {
                    qname_wire: vec![0],
                    qtype: 1,
                    qclass: 1,
                },
            ),
            ExecutionControl::with_fuel(8),
        )
        .expect("compiled W2 machine");
    let first = match machine.step().expect("cache dispatch") {
        MachineStep::Dispatch(dispatch) => dispatch,
        MachineStep::Complete(_) => panic!("W2 cache must dispatch first"),
    };
    assert_eq!(first.executable(), cache.executable);
    let second = match machine
        .resume(first.executable(), Ok(ExecutorOutcome::Continue))
        .expect("forward dispatch")
    {
        MachineStep::Dispatch(dispatch) => dispatch,
        MachineStep::Complete(_) => panic!("W2 forward must dispatch second"),
    };
    assert_eq!(second.executable(), config.forward.executable);
}

#[test]
fn w2_cache_allowlist_and_graph_shape_are_strict() {
    let rejects = [
        CACHE.replace("size: 64", "size: 63"),
        CACHE.replace("size: 64", "size: 65"),
        CACHE.replace("size: 64", "size: \"64\""),
        CACHE.replace("      size: 64\n", ""),
        CACHE.replace("      lazy_cache_ttl: 0\n", "      lazy_cache_ttl: 1\n"),
        CACHE.replace("lazy_cache_ttl: 0", "lazy_cache_ttl: \"0\""),
        CACHE.replace("      lazy_cache_ttl: 0\n", ""),
        CACHE.replace("      lazy_cache_ttl: 0\n", "      lazy_cache_ttl: 0\n      extra: true\n"),
        CACHE.replace("- tag: phase5a_forward", "- tag: phase5a_cache_2\n    type: cache\n    args:\n      size: 64\n      lazy_cache_ttl: 0\n\n  - tag: phase5a_forward"),
        CACHE.replace(
            "  - tag: phase5a_cache\n    type: cache\n    args:\n      size: 64\n      lazy_cache_ttl: 0\n\n",
            "",
        ),
        CACHE.replace("  - tag: phase5a_udp", "  - tag: phase5a_tcp").replace("type: udp_server", "type: tcp_server"),
        CACHE.replace("udp://127.0.0.1:15455", "tcp://127.0.0.1:15455"),
        CACHE.replace("enable_audit: false", "enable_audit: true"),
        CACHE.replace(
            "- exec: $phase5a_cache\n      - exec: $phase5a_forward",
            "- exec: $phase5a_forward\n      - exec: $phase5a_cache",
        ),
        CACHE.replace(
            "      - exec: $phase5a_forward\n",
            "      - exec: $phase5a_forward\n      - exec: $phase5a_forward\n",
        ),
        CACHE.replace("$phase5a_cache", "$missing_cache"),
        CACHE.replace("entry: phase5a_entry", "entry: missing_entry"),
        CACHE.replace("args:\n      size: 64", "args: []\n    # size removed\n      size: 64"),
    ];

    for (index, yaml) in rejects.iter().enumerate() {
        assert!(
            compile_yaml(yaml).is_err(),
            "reject case {index} unexpectedly compiled"
        );
    }
}

#[test]
fn w1_graph_remains_accepted_and_cache_is_not_implicit() {
    let config = compile_yaml(UDP).expect("frozen W1 graph must compile");
    assert!(config.cache.is_none());
}

#[test]
fn frozen_w3_graph_compiles_to_three_owned_udp_routes_and_four_rules() {
    let config = compile_yaml(ROUTING).expect("frozen W3 graph must compile");
    assert!(config.cache.is_none());
    assert_eq!(config.listener.kind, ListenerKind::Udp);
    assert_eq!(config.listener.entry, "phase5a_entry");
    assert_eq!(config.forwards.len(), 3);
    assert_eq!(config.forward.tag, "phase5a_route_a");
    assert_eq!(config.forward.upstream_tag.as_deref(), Some("route_a"));
    assert_eq!(
        config
            .forwards
            .iter()
            .map(|forward| forward.upstream_tag.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("route_a"), Some("route_b"), Some("route_c")]
    );
    let sequence = config
        .program
        .sequence(config.sequence.sequence)
        .expect("W3 sequence");
    assert_eq!(sequence.rules.len(), 4);
    assert!(matches!(
        sequence.rules[0].executable,
        Some(ValidatedExecutable::Inline { .. })
    ));
    assert!(matches!(
        sequence.rules[1].executable,
        Some(ValidatedExecutable::External { .. })
    ));
    assert!(matches!(
        sequence.rules[2].executable,
        Some(ValidatedExecutable::Inline { .. })
    ));
    assert!(matches!(
        sequence.rules[3].executable,
        Some(ValidatedExecutable::Inline { .. })
    ));
}

#[test]
fn w3_names_order_and_data_are_not_fixture_constants() {
    let renamed = ROUTING
        .replace("phase5a_route_domains", "exact_rules")
        .replace("phase5a_route_a", "alpha_forward")
        .replace("phase5a_route_b", "beta_forward")
        .replace("phase5a_route_c", "gamma_forward")
        .replace("route_a", "alpha_upstream")
        .replace("route_b", "beta_upstream")
        .replace("route_c", "gamma_upstream")
        .replace("domain-hit.test", "Example.TEST.")
        .replace("192.0.2.10", "203.0.113.7")
        .replace("15456", "16456")
        .replace("15457", "16457")
        .replace("15458", "16458");
    let config = compile_yaml(&renamed).expect("renamed W3 graph must compile");
    assert_eq!(config.forward.tag, "alpha_forward");
    assert_eq!(config.forward.endpoint.address().port(), 16456);
    assert_eq!(
        config.forward.upstream_tag.as_deref(),
        Some("alpha_upstream")
    );
    assert_eq!(config.forwards.len(), 3);

    let reordered = r#"
log: { level: error }
plugins:
  - tag: listener
    type: udp_server
    args: { entry: route_entry, listen: "127.0.0.1:16556", enable_audit: false }
  - tag: route_entry
    type: sequence
    args:
      - matches: ["qname $domain_rules"]
        exec: ["$alpha", "exit"]
      - exec: "$beta"
      - matches: ["resp_ip 203.0.113.7"]
        exec: ["$alpha", "exit"]
      - matches: ["_true"]
        exec: ["$gamma", "exit"]
  - tag: gamma
    type: forward
    args: { upstreams: [ { tag: gamma-upstream, addr: "udp://127.0.0.1:16558" } ] }
  - tag: domain_rules
    type: domain_set
    args: { exps: ["full:Example.TEST."] }
  - tag: alpha
    type: forward
    args: { upstreams: [ { tag: alpha-upstream, addr: "udp://127.0.0.1:16556" } ] }
  - tag: beta
    type: forward
    args: { upstreams: [ { tag: beta-upstream, addr: "udp://127.0.0.1:16557" } ] }
"#;
    let reordered_config = compile_yaml(reordered).expect("declaration order must not matter");
    assert_eq!(reordered_config.forward.tag, "alpha");
    assert_eq!(reordered_config.forwards.len(), 3);
}

#[test]
fn w3_negative_grammar_and_graph_matrix_fails_closed() {
    let cases = [
        (
            "domain suffix expression",
            ROUTING.replace("full:domain-hit.test", "suffix:domain-hit.test"),
        ),
        (
            "domain expression list",
            ROUTING.replace(
                "        - \"full:domain-hit.test\"",
                "        - \"full:domain-hit.test\"\n        - \"full:other.test\"",
            ),
        ),
        (
            "unknown domain field",
            ROUTING.replace("      exps:", "      extra: true\n      exps:"),
        ),
        (
            "unknown qname reference",
            ROUTING.replace("qname $phase5a_route_domains", "qname $missing_domains"),
        ),
        (
            "IPv6 response matcher",
            ROUTING.replace("resp_ip 192.0.2.10", "resp_ip ::1"),
        ),
        (
            "wrong exec order",
            ROUTING.replace(
                "exec: [\"$phase5a_route_a\", \"exit\"]",
                "exec: [\"exit\", \"$phase5a_route_a\"]",
            ),
        ),
        (
            "missing exit",
            ROUTING.replace(
                "exec: [\"$phase5a_route_a\", \"exit\"]",
                "exec: \"$phase5a_route_a\"",
            ),
        ),
        (
            "two matchers",
            ROUTING.replace(
                "matches: [\"resp_ip 192.0.2.10\"]",
                "matches: [\"resp_ip 192.0.2.10\", \"_true\"]",
            ),
        ),
        (
            "duplicate upstream identity",
            ROUTING.replace(
                "tag: route_b\n          addr:",
                "tag: route_a\n          addr:",
            ),
        ),
        (
            "missing upstream identity",
            ROUTING.replace("tag: route_a", "tag: \"\""),
        ),
        (
            "TCP W3 listener",
            ROUTING
                .replace("type: udp_server", "type: tcp_server")
                .replace(
                    "enable_audit: false",
                    "enable_audit: false\n      idle_timeout: 1",
                ),
        ),
        (
            "TCP W3 upstream",
            ROUTING.replace("udp://127.0.0.1:15456", "tcp://127.0.0.1:15456"),
        ),
        (
            "same A and C route",
            ROUTING.replace("$phase5a_route_c", "$phase5a_route_a"),
        ),
    ];
    for (name, yaml) in cases {
        assert!(compile_yaml(&yaml).is_err(), "W3 case {name} must reject");
    }
}

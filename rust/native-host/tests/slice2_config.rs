use mosdns_dns_core::{QueryHeader, QuestionInfo};
use mosdns_native_host::{AssemblyError, HostAssembly, ListenerKind, compile_yaml};
use mosdns_sequence_core::{ExecutionControl, ExecutionState, ExecutorOutcome, MachineStep};

const UDP: &str = include_str!("../../../tests/phase5a-baseline/configs/forward-udp.yaml");
const CACHE: &str = include_str!("../../../tests/phase5a-baseline/configs/cache.yaml");

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

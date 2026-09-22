use mosdns_native_host::{AssemblyError, HostAssembly, compile_yaml};

const UDP: &str = include_str!("../../../tests/phase5a-baseline/configs/forward-udp.yaml");

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

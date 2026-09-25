use std::num::NonZeroUsize;

use mosdns_native_host::{HostAssembly, HostOptions};

const UDP_CONFIG: &str = include_str!("../../../tests/phase5a-baseline/configs/forward-udp.yaml");

#[test]
fn host_exposes_stable_empty_metrics_and_disabled_audit_snapshots() {
    let host = HostAssembly::from_yaml(UDP_CONFIG).expect("frozen W1 graph assembles");

    let metrics = host.metrics_snapshot();
    assert_eq!(metrics.admitted_total, 0);
    assert_eq!(metrics.completed_total, 0);
    assert_eq!(metrics.in_flight, 0);
    assert_eq!(metrics.malformed_total, 0);
    assert_eq!(metrics.send_succeeded_total, 0);
    assert_eq!(metrics.send_failed_total, 0);
    assert_eq!(metrics.canceled_total, 0);
    assert_eq!(metrics.no_response_total, 0);
    assert_eq!(metrics.duration.count, 0);
    assert_eq!(metrics.forward_attempts_by_upstream.len(), 1);
    assert!(
        metrics
            .forward_attempts_by_upstream
            .contains_key("phase5a_forward")
    );

    let audit = host.audit_snapshot();
    assert!(audit.records.is_empty());
    assert_eq!(audit.evicted_total, 0);

    assert_eq!(
        metrics,
        host.metrics_snapshot(),
        "snapshot reads are read-only"
    );
    assert_eq!(audit, host.audit_snapshot(), "snapshot reads are stable");
}

#[test]
fn audit_enabled_yaml_assembles_and_test_capacity_is_explicit() {
    let yaml = UDP_CONFIG.replace("enable_audit: false", "enable_audit: true");
    let config = mosdns_native_host::compile_yaml(&yaml)
        .expect("the existing listener audit flag is accepted");
    let options = HostOptions::default()
        .with_audit_capacity(NonZeroUsize::new(2).expect("test ring capacity is nonzero"));
    let host = HostAssembly::with_options(config, options)
        .expect("audit-enabled host assembles before listener I/O");

    assert!(host.audit_snapshot().records.is_empty());
    assert_eq!(host.audit_snapshot().evicted_total, 0);
}

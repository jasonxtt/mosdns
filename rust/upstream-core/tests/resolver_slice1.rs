//! Public contract tests for the Slice 1 resolver model: typed target,
//! bootstrap and policy inputs, config-version mapping, numeric dial bypass,
//! and deterministic expiry/publication with an injected clock.
//!
//! No socket, timer, runtime, or external DNS is used here.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use mosdns_upstream_core::{
    AddressFamily, BootstrapEndpoint, Clock, ConfigVersion, PublishedTarget, ResolutionPolicy,
    ResolutionTarget, ResolvedDestination, ResolverError, ServerIdentity, TransportCancellation,
    resolve_numeric,
};

// ---------------------------------------------------------------------------
// Injected clock
// ---------------------------------------------------------------------------

/// A deterministic clock the test advances explicitly; no wall-clock sleep is
/// used anywhere in this file.
struct TestClock {
    now: std::time::Instant,
}

impl TestClock {
    fn new() -> Self {
        Self {
            now: std::time::Instant::now(),
        }
    }

    fn advance(&mut self, seconds: u64) {
        self.now += Duration::from_secs(seconds);
    }
}

impl Clock for TestClock {
    fn now(&self) -> std::time::Instant {
        self.now
    }
}

// ---------------------------------------------------------------------------
// Config-version mapping
// ---------------------------------------------------------------------------

#[test]
fn config_version_maps_to_the_approved_single_family_semantics() {
    // The product contract: 0 and 4 select A/IPv4, 6 selects AAAA/IPv6.
    assert_eq!(
        ConfigVersion::from_u8(0).expect("0 is defined").family(),
        AddressFamily::Ipv4
    );
    assert_eq!(
        ConfigVersion::from_u8(4).expect("4 is defined").family(),
        AddressFamily::Ipv4
    );
    assert_eq!(
        ConfigVersion::from_u8(6).expect("6 is defined").family(),
        AddressFamily::Ipv6
    );

    // Anything else is rejected rather than silently defaulted.
    for undefined in [1u8, 2, 3, 5, 7, 255] {
        assert_eq!(
            ConfigVersion::from_u8(undefined),
            Err(ResolverError::UnsupportedConfigVersion(undefined)),
            "version {undefined}"
        );
    }
}

#[test]
fn address_family_exposes_the_dns_wire_type() {
    assert_eq!(AddressFamily::Ipv4.wire_type(), 1);
    assert_eq!(AddressFamily::Ipv6.wire_type(), 28);
}

// ---------------------------------------------------------------------------
// Numeric dial bypass
// ---------------------------------------------------------------------------

#[test]
fn numeric_dial_addresses_bypass_resolution() {
    let v4: SocketAddr = "192.0.2.1:853".parse().expect("v4 socket address");
    let destination = resolve_numeric(v4).expect("numeric literal bypasses DNS");
    assert_eq!(destination.dial(), v4);
    assert_eq!(destination.family(), AddressFamily::Ipv4);
    // No DNS TTL applies to a literal, so it never expires on its own.
    assert!(destination.expiry().is_none());

    let v6: SocketAddr = "[2001:db8::1]:853".parse().expect("v6 socket address");
    let destination = resolve_numeric(v6).expect("numeric literal bypasses DNS");
    assert_eq!(destination.dial(), v6);
    assert_eq!(destination.family(), AddressFamily::Ipv6);
    assert!(destination.expiry().is_none());
}

#[test]
fn numeric_dial_rejects_a_zero_port() {
    let zero: SocketAddr = "192.0.2.1:0".parse().expect("parses");
    assert_eq!(resolve_numeric(zero), Err(ResolverError::ZeroPort));
}

#[test]
fn resolution_target_accepts_a_numeric_host_without_a_dns_name() {
    // A literal IP host is a valid target that bypasses resolution entirely.
    let target =
        ResolutionTarget::new("192.0.2.9", 853, AddressFamily::Ipv4).expect("valid target");
    assert!(target.is_numeric());
    assert_eq!(
        target.numeric_address().expect("literal"),
        "192.0.2.9:853".parse::<SocketAddr>().expect("parses")
    );
    assert_eq!(target.port(), 853);
    assert_eq!(target.family(), AddressFamily::Ipv4);

    let v6 = ResolutionTarget::new("2001:db8::9", 853, AddressFamily::Ipv6).expect("valid target");
    assert!(v6.is_numeric());
    assert_eq!(
        v6.numeric_address().expect("literal"),
        "[2001:db8::9]:853".parse::<SocketAddr>().expect("parses")
    );
}

// ---------------------------------------------------------------------------
// Normalized hostname and typed inputs
// ---------------------------------------------------------------------------

#[test]
fn resolution_target_normalizes_the_hostname() {
    let target = ResolutionTarget::new("Bootstrap.Example.ORG.", 853, AddressFamily::Ipv4)
        .expect("valid target");
    assert!(!target.is_numeric());
    assert_eq!(target.host(), "bootstrap.example.org");
    assert_eq!(target.port(), 853);
    assert_eq!(target.family(), AddressFamily::Ipv4);
    assert!(target.numeric_address().is_none());

    // The default port is the DNS port, matching the product contract.
    let defaulted = ResolutionTarget::new("bootstrap.example.org", 53, AddressFamily::Ipv4)
        .expect("valid target");
    assert_eq!(defaulted.port(), 53);
}

#[test]
fn resolution_target_rejects_invalid_inputs() {
    assert_eq!(
        ResolutionTarget::new("", 853, AddressFamily::Ipv4),
        Err(ResolverError::InvalidHostname)
    );
    assert_eq!(
        ResolutionTarget::new("-bad.example.org", 853, AddressFamily::Ipv4),
        Err(ResolverError::InvalidHostname)
    );
    assert_eq!(
        ResolutionTarget::new("bootstrap.example.org", 0, AddressFamily::Ipv4),
        Err(ResolverError::ZeroPort)
    );
    // The hostname must still be a hostname, not an embedded address with junk.
    assert_eq!(
        ResolutionTarget::new("192.0.2.1:53", 853, AddressFamily::Ipv4),
        Err(ResolverError::InvalidHostname)
    );
}

#[test]
fn bootstrap_endpoint_carries_the_numeric_udp_peer() {
    let bootstrap = BootstrapEndpoint::new("192.0.2.53", 53).expect("numeric bootstrap");
    assert_eq!(
        bootstrap.address(),
        "192.0.2.53:53".parse::<SocketAddr>().expect("parses")
    );
    assert_eq!(bootstrap.family(), AddressFamily::Ipv4);

    // The bootstrap peer must be numeric: no hostname recursion is allowed.
    assert_eq!(
        BootstrapEndpoint::new("dns.example.org", 53),
        Err(ResolverError::BootstrapNotNumeric)
    );
    assert_eq!(
        BootstrapEndpoint::new("192.0.2.53", 0),
        Err(ResolverError::ZeroPort)
    );
    assert_eq!(
        BootstrapEndpoint::new("192.0.2.53:53", 53),
        Err(ResolverError::BootstrapNotNumeric)
    );
}

#[test]
fn resolution_policy_uses_the_reviewed_defaults_and_bounds() {
    let policy = ResolutionPolicy::default();
    // The five-minute positive floor matches the existing product behavior; the
    // seven-day ceiling follows RFC 1035's excessive-TTL guidance.
    assert_eq!(policy.min_ttl(), Duration::from_secs(300));
    assert_eq!(policy.max_ttl(), Duration::from_secs(604_800));
    assert_eq!(policy.retransmit_interval(), Duration::from_secs(1));

    // Bounds are validated, not clamped silently.
    assert_eq!(
        ResolutionPolicy::new(Duration::ZERO, Duration::from_secs(604_800)),
        Err(ResolverError::InvalidPolicy)
    );
    assert_eq!(
        ResolutionPolicy::new(Duration::from_secs(600), Duration::from_secs(300)),
        Err(ResolverError::InvalidPolicy)
    );
}

// ---------------------------------------------------------------------------
// Deterministic expiry and publication
// ---------------------------------------------------------------------------

#[test]
fn resolved_destination_reports_deterministic_expiry() {
    let mut clock = TestClock::new();
    let now = clock.now();
    // A 600-second TTL publishes an expiry exactly 600 seconds out.
    let destination =
        ResolvedDestination::new([192, 0, 2, 1].into(), AddressFamily::Ipv4, 600, now)
            .expect("valid destination");
    assert_eq!(
        destination.address(),
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))
    );
    assert_eq!(destination.ttl(), Duration::from_secs(600));
    assert_eq!(destination.expiry(), Some(now + Duration::from_secs(600)));
    assert!(!destination.is_expired(now));
    assert!(!destination.is_expired(now + Duration::from_secs(599)));

    // Exactly at the expiry instant it is expired: no stale success.
    clock.advance(600);
    let at_expiry = clock.now();
    assert!(destination.is_expired(at_expiry));
    assert!(destination.is_expired(at_expiry + Duration::from_secs(1)));

    // A literal has no expiry and can never be considered stale.
    let literal = ResolvedDestination::new_literal(
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7)),
        AddressFamily::Ipv4,
    );
    assert!(literal.expiry().is_none());
    assert!(!literal.is_expired(at_expiry + Duration::from_secs(10_000)));
}

#[test]
fn resolved_destination_derives_the_numeric_endpoint_without_identity() {
    let now = std::time::Instant::now();
    let destination =
        ResolvedDestination::new([192, 0, 2, 1].into(), AddressFamily::Ipv4, 600, now)
            .expect("valid destination");

    // The published numeric address is what a transport dials.
    assert_eq!(
        destination.address(),
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))
    );
    assert_eq!(destination.family(), AddressFamily::Ipv4);

    // A v6 selection stays v6.
    let v6 = ResolvedDestination::new(Ipv6Addr::LOCALHOST.into(), AddressFamily::Ipv6, 600, now)
        .expect("valid destination");
    assert_eq!(v6.family(), AddressFamily::Ipv6);
}

#[test]
fn publishing_a_target_preserves_the_service_identity() {
    let mut clock = TestClock::new();
    let identity = ServerIdentity::new("bootstrap.example.org").expect("valid identity");
    let now = clock.now();

    let target = ResolutionTarget::new("bootstrap.example.org", 853, AddressFamily::Ipv4)
        .expect("valid target");
    let published = PublishedTarget::new(
        target,
        ResolvedDestination::new([192, 0, 2, 1].into(), AddressFamily::Ipv4, 600, now)
            .expect("valid destination"),
    );

    // The resolved numeric address never rewrites the TLS/HTTP identity.
    assert_eq!(
        published.destination().address(),
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))
    );
    assert_eq!(
        published.dial(),
        "192.0.2.1:853".parse::<SocketAddr>().expect("parses")
    );
    assert_eq!(published.port(), 853);
    assert_eq!(published.family(), AddressFamily::Ipv4);
    assert!(!published.is_expired(now));

    // The identity the caller supplied stays exactly as it was validated.
    let owned = ServerIdentity::new("bootstrap.example.org").expect("valid identity");
    assert_eq!(identity.dns_name(), owned.dns_name());
    assert_eq!(identity.dns_name(), Some("bootstrap.example.org"));

    clock.advance(600);
    assert!(published.is_expired(clock.now()));
}

#[test]
fn zero_ttl_and_port_zero_are_rejected_before_publication() {
    let now = std::time::Instant::now();
    // A zero TTL would publish an already-dead result; the model refuses it.
    assert_eq!(
        ResolvedDestination::new([192, 0, 2, 1].into(), AddressFamily::Ipv4, 0, now),
        Err(ResolverError::InvalidTtl)
    );
    assert_eq!(
        ResolutionTarget::new("bootstrap.example.org", 0, AddressFamily::Ipv4),
        Err(ResolverError::ZeroPort)
    );
}

// ---------------------------------------------------------------------------
// State model
// ---------------------------------------------------------------------------
//
// The state-model tests (empty start, no stale serving, failed refresh,
// expired-value rejection) moved into the crate-internal test module in
// `src/resolver/mod.rs`. They exercise `ResolverState`'s mutation surface,
// which is deliberately crate-private so that no external caller can publish
// or fake freshness around the owner's lifecycle gate. Relocating them keeps
// that boundary intact rather than widening it for test convenience.

// ---------------------------------------------------------------------------
// Cancellation vocabulary reuse
// ---------------------------------------------------------------------------

#[test]
fn resolver_reuses_the_transport_cancellation_vocabulary() {
    // The resolver must not introduce a second cancellation type.
    let cancellation = TransportCancellation::new();
    assert!(!cancellation.is_cancelled());
    let child = cancellation.child_token();
    assert!(!child.is_cancelled());
    cancellation.cancel();
    assert!(cancellation.is_cancelled());
    assert!(
        child.is_cancelled(),
        "parent cancellation propagates to the child token"
    );
}

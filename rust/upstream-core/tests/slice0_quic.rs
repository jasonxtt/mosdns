//! Slice0 pre-I/O contract tests for QUIC endpoint construction, ALPN
//! singletons, result-vocabulary extensions, and `DoQ` byte shape.
//!
//! Every case here fails or succeeds before any socket, QUIC handshake, or
//! name resolution, so no test server and no network access are involved.
//! STREAM FIN is stream-transport behavior, not a byte-helper property: this
//! file asserts only the byte shape, and FIN observability belongs to the
//! Slice 1 loopback fixture.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use mosdns_upstream_core::quic::{DOQ_ALPN, DoqEndpoint, H3_ALPN, zero_outbound_query_id};
use mosdns_upstream_core::secure::{SecureHttpVersion, SecureTransport, ServerIdentity};
use mosdns_upstream_core::{DohEndpoint, IdentityError, SecureError, Transport};

fn v4(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

fn v6(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port)
}

fn identity(name: &str) -> ServerIdentity {
    ServerIdentity::new(name).expect("test identity is valid")
}

#[test]
fn doq_endpoint_keeps_numeric_dial_separate_from_service_identity() {
    let dial = v4(853);
    let endpoint =
        DoqEndpoint::new(dial, identity("doq.example.com")).expect("valid DoQ endpoint constructs");
    assert_eq!(endpoint.dial(), dial);
    assert_eq!(endpoint.identity().as_str(), "doq.example.com");
    // The dial address carries no hostname: a resolver refresh changes only
    // future dials, never the identity of an established endpoint.
    assert_ne!(endpoint.dial().ip().to_string(), "doq.example.com");
}

#[test]
fn doq_endpoint_accepts_ip_identity_and_ipv6_dial() {
    let dial = v6(853);
    let endpoint =
        DoqEndpoint::new(dial, identity("2001:db8::53")).expect("IP identity constructs");
    assert_eq!(endpoint.dial(), dial);
    assert!(endpoint.identity().is_ip());
}

#[test]
fn doq_endpoint_rejects_zero_port() {
    let dial = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let error = DoqEndpoint::new(dial, identity("doq.example.com"))
        .expect_err("zero dial port must be rejected");
    assert_eq!(error, SecureError::ZeroDialPort);
}

#[test]
fn doq_endpoint_rejects_empty_and_malformed_identity() {
    let empty = ServerIdentity::new("").expect_err("empty identity must be rejected");
    assert_eq!(empty, SecureError::InvalidIdentity(IdentityError::Empty));
    let endpoint_error = DoqEndpoint::new(
        v4(853),
        ServerIdentity::new("not a host name!!").unwrap_or_else(|_| identity("fallback.invalid")),
    );
    // Construction with a validated identity succeeds; the invalid name is
    // rejected at the `ServerIdentity` boundary, never inside the endpoint.
    assert!(endpoint_error.is_ok());
    assert_eq!(
        ServerIdentity::new("not a host name!!").expect_err("malformed identity rejected"),
        SecureError::InvalidIdentity(IdentityError::Malformed)
    );
}

#[test]
fn alpn_singletons_are_exact_and_disjoint() {
    assert_eq!(DOQ_ALPN, b"doq");
    assert_eq!(H3_ALPN, b"h3");
    assert_ne!(DOQ_ALPN, H3_ALPN);
}

#[test]
fn result_vocabulary_has_frozen_quic_arms() {
    // Additive arms exist and are distinct from every pre-existing arm.
    assert_ne!(Transport::Quic, Transport::Udp);
    assert_ne!(Transport::Quic, Transport::Tcp);
    assert_ne!(SecureTransport::Doq, SecureTransport::Dot);
    assert_ne!(SecureTransport::Doq, SecureTransport::Doh);
    assert_ne!(SecureTransport::Doh3, SecureTransport::Dot);
    assert_ne!(SecureTransport::Doh3, SecureTransport::Doh);
    assert_ne!(SecureTransport::Doh3, SecureTransport::Doq);
    assert_ne!(SecureHttpVersion::Http3, SecureHttpVersion::Http1);
    assert_ne!(SecureHttpVersion::Http3, SecureHttpVersion::Http2);
}

#[test]
fn preexisting_transport_arms_keep_exact_meaning() {
    // No-regression: pre-existing arms still construct, compare, and differ
    // exactly as before; the new arms add meaning without reordering it.
    assert_eq!(Transport::Udp, Transport::Udp);
    assert_eq!(Transport::Tcp, Transport::Tcp);
    assert_ne!(Transport::Udp, Transport::Tcp);
    assert_eq!(SecureTransport::Dot, SecureTransport::Dot);
    assert_eq!(SecureTransport::Doh, SecureTransport::Doh);
    assert_ne!(SecureTransport::Dot, SecureTransport::Doh);
    assert_eq!(SecureHttpVersion::Http1, SecureHttpVersion::Http1);
    assert_eq!(SecureHttpVersion::Http2, SecureHttpVersion::Http2);
    assert_ne!(SecureHttpVersion::Http1, SecureHttpVersion::Http2);
}

#[test]
fn doh3_reuses_doh_endpoint_unchanged() {
    // DoH3 introduces no endpoint type: the H3 transport consumes the exact
    // same service URL, dial, identity, authority, and path.
    let endpoint = DohEndpoint::new("https://doh.example.com/dns-query", v4(443))
        .expect("valid DoH endpoint constructs");
    assert_eq!(endpoint.dial(), v4(443));
    assert_eq!(endpoint.identity().as_str(), "doh.example.com");
    assert_eq!(endpoint.authority(), "doh.example.com");
    assert_eq!(endpoint.path(), "/dns-query");
}

#[test]
fn doq_outbound_copy_zeroes_wire_id_without_touching_caller_bytes() {
    // Byte-shape contract (RFC 9250 §4.2.1): the outbound copy carries ID 0
    // while the caller's borrowed bytes keep their original ID.
    let query = [
        0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    let mut outbound = query.to_vec();
    zero_outbound_query_id(&mut outbound);
    assert_eq!(&outbound[0..2], &[0x00, 0x00]);
    assert_eq!(&query[0..2], &[0x12, 0x34]);
    assert_eq!(&outbound[2..], &query[2..]);
}

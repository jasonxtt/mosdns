//! Slice0 pre-I/O contract tests for secure upstream endpoint construction.
//!
//! Every case here fails or succeeds before any socket or name resolution, so
//! no TLS certificate fixture, server, or network access is involved.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use mosdns_upstream_core::{
    DohEndpoint, DotEndpoint, IdentityError, SecureError, ServerIdentity, ServiceUrlError,
    SideEffectState,
};

fn v4(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

fn v6(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port)
}

#[test]
fn dot_endpoint_keeps_numeric_dial_separate_from_service_identity() {
    for dial in [v4(853), v6(853)] {
        let identity = ServerIdentity::new("dns.example").expect("valid DNS identity");
        let endpoint = DotEndpoint::new(dial, identity).expect("valid DoT endpoint");

        assert_eq!(endpoint.dial(), dial);
        assert_eq!(endpoint.dial().ip(), dial.ip());
        assert_eq!(endpoint.identity().as_str(), "dns.example");
        assert!(endpoint.identity().is_dns_name());
        assert!(!endpoint.identity().is_ip());
    }
}

#[test]
fn dot_endpoint_accepts_ip_identity_for_both_families() {
    let v4_endpoint = DotEndpoint::new(
        v4(853),
        ServerIdentity::new("192.0.2.1").expect("IPv4 identity"),
    )
    .expect("valid DoT endpoint");
    assert_eq!(
        v4_endpoint.identity().ip(),
        Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)))
    );

    // A bare and a bracketed IPv6 literal normalize to the same identity, and
    // neither matches the loopback dial destination.
    let bare = DotEndpoint::new(
        v6(853),
        ServerIdentity::new("2001:db8::1").expect("bare IPv6 identity"),
    )
    .expect("valid DoT endpoint");
    let bracketed = DotEndpoint::new(
        v6(853),
        ServerIdentity::new("[2001:db8::1]").expect("bracketed IPv6 identity"),
    )
    .expect("valid DoT endpoint");
    assert_eq!(bare.identity().as_str(), "2001:db8::1");
    assert_eq!(bare.identity(), bracketed.identity());
    assert_eq!(
        bare.identity().ip(),
        Some(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)))
    );
    assert_ne!(bare.dial().ip(), bare.identity().ip().expect("IP identity"));
}

#[test]
fn dot_endpoint_rejects_zero_dial_port_for_both_families() {
    for dial in [v4(0), v6(0)] {
        assert_eq!(
            DotEndpoint::new(dial, ServerIdentity::new("dns.example").expect("identity")),
            Err(SecureError::ZeroDialPort)
        );
    }
}

#[test]
fn server_identity_normalizes_valid_names_without_resolving() {
    for (input, expected) in [
        ("dns.example", "dns.example"),
        ("DNS.Example", "dns.example"),
        ("dns.example.", "dns.example"),
        ("xn--mnchen-3ya.example", "xn--mnchen-3ya.example"),
    ] {
        let identity = ServerIdentity::new(input).expect("valid DNS identity");
        assert!(identity.is_dns_name(), "{input}");
        assert_eq!(identity.dns_name(), Some(expected));
        assert_eq!(identity.as_str(), expected);
        assert_eq!(identity.ip(), None);
    }

    // A name that cannot resolve still constructs: no lookup is performed.
    assert!(ServerIdentity::new("do-not-resolve.invalid").is_ok());
}

#[test]
fn server_identity_rejects_empty_and_malformed_names() {
    assert_eq!(
        ServerIdentity::new(""),
        Err(SecureError::InvalidIdentity(IdentityError::Empty))
    );
    for malformed in [
        " ",
        "a..b",
        ".leading",
        "exa_mple.example",
        "-leading.example",
        "trailing-.example",
        "has space.example",
        "12345",
        "127.1",
        "192.168.1",
        "0x7f000001",
        "a.example:53",
    ] {
        assert_eq!(
            ServerIdentity::new(malformed),
            Err(SecureError::InvalidIdentity(IdentityError::Malformed)),
            "identity {malformed:?} should be rejected"
        );
    }

    let long_label = format!("{}.example", "a".repeat(64));
    assert_eq!(
        ServerIdentity::new(&long_label),
        Err(SecureError::InvalidIdentity(IdentityError::Malformed))
    );
    let long_name = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(63)
    );
    assert_eq!(
        ServerIdentity::new(&long_name),
        Err(SecureError::InvalidIdentity(IdentityError::Malformed))
    );
}

#[test]
fn doh_endpoint_preserves_service_authority_path_and_query_for_ipv4_dial() {
    let dial = v4(8443);
    let endpoint = DohEndpoint::new("https://dns.example:8443/resolve?foo=bar&dns=bogus", dial)
        .expect("valid DoH endpoint");

    // Service URL identity is independent of the numeric dial override.
    assert_eq!(endpoint.service_url().scheme(), "https");
    assert_eq!(endpoint.service_url().host_str(), Some("dns.example"));
    assert_eq!(endpoint.service_url().port(), Some(8443));
    assert_eq!(endpoint.path(), "/resolve");
    assert_eq!(endpoint.query(), Some("foo=bar&dns=bogus"));
    assert_eq!(endpoint.authority(), "dns.example:8443");
    assert_eq!(endpoint.host(), "dns.example");
    assert_eq!(endpoint.identity().as_str(), "dns.example");
    assert_eq!(endpoint.dial(), dial);
    assert!(endpoint.dial().ip().is_loopback());
}

#[test]
fn doh_endpoint_supports_ipv6_service_and_dial() {
    let endpoint = DohEndpoint::new("https://[2001:db8::53]/dns-query", v6(443))
        .expect("valid IPv6 DoH endpoint");
    assert_eq!(endpoint.host(), "[2001:db8::53]");
    assert_eq!(endpoint.authority(), "[2001:db8::53]");
    assert_eq!(endpoint.path(), "/dns-query");
    assert_eq!(
        endpoint.identity().ip(),
        Some(IpAddr::V6(Ipv6Addr::new(
            0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x53
        )))
    );
    assert!(endpoint.dial().ip().is_loopback());
}

#[test]
fn doh_endpoint_normalizes_an_empty_path_to_root() {
    let endpoint = DohEndpoint::new("https://dns.example", v4(443)).expect("valid endpoint");
    assert_eq!(endpoint.path(), "/");
    assert_eq!(endpoint.query(), None);
    assert_eq!(endpoint.authority(), "dns.example");

    let with_query = DohEndpoint::new("https://dns.example?a=1", v4(443)).expect("valid endpoint");
    assert_eq!(with_query.path(), "/");
    assert_eq!(with_query.query(), Some("a=1"));
}

#[test]
fn doh_endpoint_rejects_non_https_and_malformed_urls() {
    assert_eq!(
        DohEndpoint::new("http://dns.example/dns-query", v4(443)),
        Err(SecureError::InvalidServiceUrl(
            ServiceUrlError::UnsupportedScheme
        ))
    );
    assert_eq!(
        DohEndpoint::new("ftp://dns.example/dns-query", v4(443)),
        Err(SecureError::InvalidServiceUrl(
            ServiceUrlError::UnsupportedScheme
        ))
    );
    for malformed in [
        "",
        "not a url",
        "dns.example/dns-query",
        "https://dns.example:bad/",
    ] {
        assert!(
            matches!(
                DohEndpoint::new(malformed, v4(443)),
                Err(SecureError::InvalidServiceUrl(_))
            ),
            "URL {malformed:?} should be rejected"
        );
    }
    // A URL that parses without a host is still rejected before any dial.
    assert_eq!(
        DohEndpoint::new("https://", v4(443)),
        Err(SecureError::InvalidServiceUrl(ServiceUrlError::EmptyHost))
    );
}

#[test]
fn doh_endpoint_rejects_userinfo_and_fragments_without_leaking_them() {
    let credentials = DohEndpoint::new(
        "https://alice:hunter2@dns.example/dns-query?token=topsecret",
        v4(443),
    )
    .expect_err("userinfo is not allowed");
    assert_eq!(
        credentials,
        SecureError::InvalidServiceUrl(ServiceUrlError::UserInfo)
    );
    let text = credentials.to_string();
    for secret in ["alice", "hunter2", "topsecret", "dns.example"] {
        assert!(!text.contains(secret), "error Display leaked {secret:?}");
    }

    let fragment = DohEndpoint::new(
        "https://dns.example/dns-query?token=topsecret#section",
        v4(443),
    )
    .expect_err("fragment is not allowed");
    assert_eq!(
        fragment,
        SecureError::InvalidServiceUrl(ServiceUrlError::Fragment)
    );
    assert!(!fragment.to_string().contains("topsecret"));
}

#[test]
fn doh_endpoint_rejects_zero_dial_port() {
    for dial in [v4(0), v6(0)] {
        assert_eq!(
            DohEndpoint::new("https://dns.example/dns-query", dial),
            Err(SecureError::ZeroDialPort)
        );
    }
}

#[test]
fn doh_endpoint_does_not_resolve_the_service_host() {
    let endpoint = DohEndpoint::new("https://do-not-resolve.invalid/dns-query", v4(443))
        .expect("construction performs no DNS lookup");
    assert_eq!(endpoint.identity().as_str(), "do-not-resolve.invalid");
    assert_eq!(endpoint.dial(), v4(443));
}

#[test]
fn secure_construction_errors_are_typed_pre_io_failures() {
    assert_eq!(
        SecureError::ZeroDialPort.side_effect(),
        SideEffectState::NotSent
    );
    assert_eq!(
        SecureError::InvalidIdentity(IdentityError::Empty).side_effect(),
        SideEffectState::NotSent
    );
    assert_eq!(
        SecureError::InvalidServiceUrl(ServiceUrlError::Malformed).side_effect(),
        SideEffectState::NotSent
    );

    // The three construction paths are distinguishable and carry no URL text.
    assert_ne!(
        SecureError::ZeroDialPort,
        SecureError::InvalidServiceUrl(ServiceUrlError::Malformed)
    );
    assert_ne!(
        SecureError::InvalidIdentity(IdentityError::Empty),
        SecureError::InvalidIdentity(IdentityError::Malformed)
    );
}

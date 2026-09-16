//! Slice0 pre-I/O contract tests for secure upstream endpoint construction.
//!
//! Every case here fails or succeeds before any socket, name resolution, or TLS
//! handshake, so no test server and no network access are involved. The TLS
//! policy cases use one minimal synthetic root certificate only to make a
//! `RootCertStore` non-empty; see [`SLICE0_SYNTHETIC_ROOT_DER`].

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use mosdns_upstream_core::secure::{DohRequestError, TlsConfigError, TlsPolicy};
use mosdns_upstream_core::{
    DohEndpoint, DotEndpoint, ExchangeRequest, IdentityError, SecureError, ServerIdentity,
    ServiceUrlError, SideEffectState,
};
use rustls::RootCertStore;
use rustls::pki_types::CertificateDer;

/// A minimal, synthetic, non-secret trust anchor used only to make a
/// `RootCertStore` non-empty. The matching throwaway private key was generated
/// to a temporary path, never committed, and deleted; no real service
/// certificate or key is checked in. Provenance:
///
/// ```text
/// openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
///   -keyout slice0-key.pem -out slice0-cert.pem -days 3650 -nodes \
///   -subj "/CN=slice0-synthetic-root" \
///   -addext "basicConstraints=critical,CA:TRUE"
/// openssl x509 -in slice0-cert.pem -outform DER -out slice0-cert.der
/// ```
///
/// Subject `CN=slice0-synthetic-root`, valid 2026-09-16 to 2036-09-13.
const SLICE0_SYNTHETIC_ROOT_DER: &[u8] = &[
    0x30, 0x82, 0x01, 0x94, 0x30, 0x82, 0x01, 0x3b, 0xa0, 0x03, 0x02, 0x01, 0x02, 0x02, 0x14, 0x52,
    0xe2, 0x00, 0xbc, 0x66, 0xdf, 0x59, 0x22, 0x5c, 0x2f, 0xea, 0x2e, 0x16, 0x75, 0xde, 0xcc, 0xb1,
    0x13, 0x46, 0x91, 0x30, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02, 0x30,
    0x20, 0x31, 0x1e, 0x30, 0x1c, 0x06, 0x03, 0x55, 0x04, 0x03, 0x0c, 0x15, 0x73, 0x6c, 0x69, 0x63,
    0x65, 0x30, 0x2d, 0x73, 0x79, 0x6e, 0x74, 0x68, 0x65, 0x74, 0x69, 0x63, 0x2d, 0x72, 0x6f, 0x6f,
    0x74, 0x30, 0x1e, 0x17, 0x0d, 0x32, 0x36, 0x30, 0x39, 0x31, 0x36, 0x31, 0x35, 0x33, 0x36, 0x34,
    0x30, 0x5a, 0x17, 0x0d, 0x33, 0x36, 0x30, 0x39, 0x31, 0x33, 0x31, 0x35, 0x33, 0x36, 0x34, 0x30,
    0x5a, 0x30, 0x20, 0x31, 0x1e, 0x30, 0x1c, 0x06, 0x03, 0x55, 0x04, 0x03, 0x0c, 0x15, 0x73, 0x6c,
    0x69, 0x63, 0x65, 0x30, 0x2d, 0x73, 0x79, 0x6e, 0x74, 0x68, 0x65, 0x74, 0x69, 0x63, 0x2d, 0x72,
    0x6f, 0x6f, 0x74, 0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01,
    0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00, 0x04, 0xa0, 0xce,
    0xc9, 0xbd, 0xd0, 0x55, 0xa2, 0xad, 0x68, 0xd9, 0xda, 0x5a, 0xb8, 0x20, 0x81, 0x5f, 0x04, 0xfc,
    0xfb, 0x2b, 0xa8, 0x41, 0xd6, 0x40, 0x76, 0x29, 0x21, 0x52, 0x85, 0xb6, 0x64, 0xdd, 0x4e, 0x63,
    0xb1, 0xad, 0xed, 0xf5, 0x78, 0xcf, 0x5b, 0xd5, 0xda, 0xc6, 0xae, 0xed, 0xf7, 0xfd, 0xc7, 0xc0,
    0x87, 0x5f, 0x4a, 0x62, 0x00, 0x41, 0xa2, 0xa8, 0x1a, 0xd2, 0x46, 0xf3, 0xe8, 0xdf, 0xa3, 0x53,
    0x30, 0x51, 0x30, 0x1d, 0x06, 0x03, 0x55, 0x1d, 0x0e, 0x04, 0x16, 0x04, 0x14, 0xf0, 0x7f, 0x42,
    0x8e, 0xc6, 0x55, 0xbf, 0xdf, 0xae, 0x38, 0x82, 0xf0, 0xcf, 0xdb, 0xca, 0x3f, 0xfd, 0xca, 0x2a,
    0x87, 0x30, 0x1f, 0x06, 0x03, 0x55, 0x1d, 0x23, 0x04, 0x18, 0x30, 0x16, 0x80, 0x14, 0xf0, 0x7f,
    0x42, 0x8e, 0xc6, 0x55, 0xbf, 0xdf, 0xae, 0x38, 0x82, 0xf0, 0xcf, 0xdb, 0xca, 0x3f, 0xfd, 0xca,
    0x2a, 0x87, 0x30, 0x0f, 0x06, 0x03, 0x55, 0x1d, 0x13, 0x01, 0x01, 0xff, 0x04, 0x05, 0x30, 0x03,
    0x01, 0x01, 0xff, 0x30, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02, 0x03,
    0x47, 0x00, 0x30, 0x44, 0x02, 0x20, 0x0a, 0xe4, 0x83, 0x99, 0x73, 0xb6, 0x62, 0x8e, 0xc7, 0x5b,
    0x87, 0x81, 0xb2, 0xfb, 0x02, 0x2d, 0xdd, 0x66, 0x7d, 0xcf, 0x37, 0x24, 0x4a, 0x4e, 0xff, 0x8a,
    0xd9, 0x8e, 0x41, 0xd8, 0x69, 0xb4, 0x02, 0x20, 0x52, 0xe3, 0xe2, 0x97, 0xc0, 0xa6, 0xb9, 0x6c,
    0x62, 0xfe, 0x0a, 0x3e, 0x52, 0xd4, 0x64, 0x26, 0x2e, 0x59, 0x2b, 0x99, 0x34, 0xb7, 0xb9, 0x3b,
    0xcf, 0x4a, 0x32, 0x82, 0x8d, 0xc4, 0x08, 0xd8,
];

fn synthetic_root_store() -> RootCertStore {
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(SLICE0_SYNTHETIC_ROOT_DER))
        .expect("synthetic root certificate parses as a trust anchor");
    roots
}

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
fn doh_endpoint_reuses_shared_identity_validation_for_url_hosts() {
    // A URL-derived DNS host must obey the exact same identity contract as
    // `ServerIdentity::new`; no second, looser hostname rule is introduced.
    for host in ["exa_mple.example", "-leading.example", "trailing-.example"] {
        let url = format!("https://{host}/dns-query?token=topsecret");
        let from_url = DohEndpoint::new(&url, v4(443))
            .expect_err("a URL host must fail the shared identity validation");
        let direct =
            ServerIdentity::new(host).expect_err("the shared validator rejects this DNS name");
        assert_eq!(from_url, direct, "host {host:?}");
        assert_eq!(
            from_url,
            SecureError::InvalidIdentity(IdentityError::Malformed)
        );
        assert_eq!(from_url.side_effect(), SideEffectState::NotSent);

        let display = from_url.to_string();
        let debug = format!("{from_url:?}");
        for leaked in ["https://", host, "dns-query", "token", "topsecret"] {
            assert!(
                !display.contains(leaked),
                "Display leaked {leaked:?}: {display}"
            );
            assert!(!debug.contains(leaked), "Debug leaked {leaked:?}: {debug}");
        }
    }

    // A label longer than 63 bytes is accepted by the URL parser but rejected
    // by the shared validator.
    let overlong = format!("{}.example", "a".repeat(64));
    let url = format!("https://{overlong}/dns-query");
    let from_url =
        DohEndpoint::new(&url, v4(443)).expect_err("an overlong URL host label must be rejected");
    assert_eq!(
        from_url,
        ServerIdentity::new(&overlong).expect_err("the shared validator rejects this label")
    );
    assert_eq!(
        from_url,
        SecureError::InvalidIdentity(IdentityError::Malformed)
    );
}

#[test]
fn doh_endpoint_keeps_ip_url_hosts_successful_after_identity_reuse() {
    let v4_endpoint = DohEndpoint::new("https://192.0.2.1/dns-query", v4(443))
        .expect("an IPv4 URL host still constructs");
    assert!(v4_endpoint.identity().is_ip());
    assert_eq!(v4_endpoint.identity().as_str(), "192.0.2.1");

    let v6_endpoint = DohEndpoint::new("https://[2001:db8::53]/dns-query", v6(443))
        .expect("an IPv6 URL host still constructs");
    assert!(v6_endpoint.identity().is_ip());
    assert_eq!(v6_endpoint.identity().as_str(), "2001:db8::53");
}

#[test]
fn doh_endpoint_rejects_raw_cr_and_lf_before_url_parsing() {
    // The WHATWG URL parser strips raw ASCII CR/LF before parsing, so without
    // an explicit scan these inputs would be silently accepted; a CR/LF in a
    // path or query is a request-smuggling vector. The host spellings here are
    // also accepted by `url`, so the rejection must precede `Url::parse`.
    for input in [
        "https://dns.example/dns\r-query?token=topsecret",
        "https://dns.example/dns\n-query?token=topsecret",
        "https://dns.example/dns-query?token=top\rsecret",
        "https://dns.example/dns-query?token=top\nsecret",
        "https://dns.example/dns-query?token=topsecret\r",
        "https://exa\r_mple.example/dns-query?token=topsecret",
        "https://exa\n_mple.example/dns-query?token=topsecret",
    ] {
        let error = DohEndpoint::new(input, v4(443))
            .expect_err("a raw CR or LF must be rejected before URL parsing");
        assert_eq!(
            error,
            SecureError::InvalidServiceUrl(ServiceUrlError::ControlCharacter),
            "input {input:?}"
        );
        assert_eq!(error.side_effect(), SideEffectState::NotSent);

        // The data-free variant must not echo URL/query material or CR/LF.
        let display = error.to_string();
        let debug = format!("{error:?}");
        for leaked in [
            "https://",
            "dns.example",
            "dns-query",
            "token",
            "topsecret",
            "\r",
            "\n",
        ] {
            assert!(
                !display.contains(leaked),
                "Display leaked {leaked:?}: {display}"
            );
            assert!(!debug.contains(leaked), "Debug leaked {leaked:?}: {debug}");
        }
    }
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

#[test]
fn tls_policy_verified_rejects_empty_root_store_before_io() {
    // An empty trust store is a construction defect: it must never fall back
    // to insecure verification, the platform trust store, or a deferred error.
    let rejected = TlsPolicy::verified(RootCertStore::empty());
    assert!(
        matches!(
            rejected,
            Err(SecureError::TlsConfig(TlsConfigError::EmptyRootStore))
        ),
        "empty roots must fail with the typed TlsConfigError"
    );

    let error = rejected.expect_err("empty roots are rejected");
    assert_eq!(error.side_effect(), SideEffectState::NotSent);
    assert_ne!(error, SecureError::ZeroDialPort);
}

#[test]
fn tls_policy_verified_preserves_caller_supplied_roots() {
    let roots = synthetic_root_store();
    assert_eq!(roots.len(), 1);
    let expected_subject = roots.roots[0].subject.as_ref().to_vec();

    let policy = TlsPolicy::verified(roots).expect("non-empty roots are accepted");
    assert!(!policy.is_insecure_skip_verify());
    assert_eq!(policy.root_count(), Some(1));

    let stored = policy.roots().expect("verified policy keeps its roots");
    assert_eq!(stored.len(), 1);
    assert_eq!(
        stored.roots[0].subject.as_ref(),
        expected_subject.as_slice()
    );
}

#[test]
fn tls_policy_insecure_skip_verify_is_explicit_opt_in_only() {
    let insecure = TlsPolicy::insecure_skip_verify();
    assert!(insecure.is_insecure_skip_verify());
    assert!(insecure.roots().is_none());
    assert_eq!(insecure.root_count(), None);

    // Verified and insecure modes are structurally distinct. The only
    // constructors are `verified`, which requires non-empty roots and never
    // downgrades on failure, and this explicit opt-in. The mode field is
    // private and no method takes `&mut self`, so there is intentionally no
    // setter or post-failure fallback that could flip a verified policy.
    let verified = TlsPolicy::verified(synthetic_root_store()).expect("valid roots");
    assert!(!verified.is_insecure_skip_verify());
    assert!(verified.roots().is_some());
}

#[test]
fn tls_policy_errors_and_debug_expose_no_sensitive_material() {
    let error = SecureError::TlsConfig(TlsConfigError::EmptyRootStore);
    let display = error.to_string();
    let debug = format!("{error:?}");
    assert!(display.contains("root"), "unexpected Display: {display}");

    for leaked in [
        "https://",
        "token",
        "password",
        "userinfo",
        "-----BEGIN",
        "slice0-synthetic-root",
        "0x30",
    ] {
        assert!(
            !display.contains(leaked),
            "Display leaked {leaked:?}: {display}"
        );
        assert!(!debug.contains(leaked), "Debug leaked {leaked:?}: {debug}");
    }

    // `RootCertStore` prints only a root count, so a policy that owns the
    // synthetic certificate cannot expose certificate bytes through Debug.
    let policy = TlsPolicy::verified(synthetic_root_store()).expect("valid roots");
    let policy_debug = format!("{policy:?}");
    assert!(!policy_debug.contains("slice0-synthetic-root"));
    assert!(!policy_debug.contains("3082"));
}

/// A minimal, `dns-core`-valid query wire for the pure request-target contract.
fn doh_query_wire(id: u16) -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&[0x01, 0x00]); // RD=1, QR=0, opcode QUERY
    wire.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    wire.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT
    wire.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    wire.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT
    wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
    wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0x00]);
    wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]); // A IN
    wire
}

/// A wire-legal query whose label bytes make the standard base64 alphabet emit
/// `+` and `/`, so the contract test can prove the URL-safe engine is used.
fn alphabet_probe_query(id: u16) -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&[0x01, 0x00]);
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&[0x03, 0xFF, 0xC0, 0xFB]); // one 3-byte label
    wire.push(0x00); // root label
    wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
    wire
}

/// The decoded query pairs of an origin-form request target.
fn target_query_pairs(target: &str) -> Vec<(String, String)> {
    let query = target
        .split_once('?')
        .expect("an origin-form target always carries a query")
        .1;
    url::form_urlencoded::parse(query.as_bytes())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

/// The one generated `dns` value of an origin-form target.
fn generated_dns_value(target: &str) -> String {
    let mut values: Vec<String> = target_query_pairs(target)
        .into_iter()
        .filter(|(key, _)| key == "dns")
        .map(|(_, value)| value)
        .collect();
    assert_eq!(values.len(), 1, "exactly one generated dns pair: {target}");
    values.pop().expect("checked to hold exactly one value")
}

#[test]
fn doh_get_request_target_borrows_the_query_and_replaces_only_dns() {
    let id = 0xbeef;
    let query = doh_query_wire(id);
    let original = query.clone();

    let service_url =
        "https://dns.example:8443/resolve?foo=bar&dns=bogus&x=a%2Fb&dn%73=second&empty=";
    let endpoint = DohEndpoint::new(service_url, v4(8443)).expect("valid DoH endpoint");

    let target = endpoint
        .get_request_target(ExchangeRequest::new(&query).expect("valid query"))
        .expect("target builds");

    // The caller's borrowed bytes and original transaction ID are never touched.
    assert_eq!(query, original, "caller query bytes must not be mutated");
    assert_eq!(&query[..2], &id.to_be_bytes(), "original ID is preserved");
    assert_eq!(
        endpoint.query(),
        Some("foo=bar&dns=bogus&x=a%2Fb&dn%73=second&empty=")
    );

    // Origin-form: the endpoint path plus one query, with no scheme, authority,
    // dial address, userinfo, or fragment.
    assert!(target.starts_with("/resolve?"), "target: {target}");
    for absent in ["https://", "dns.example", "8443", "#", "@", "//"] {
        assert!(
            !target.contains(absent),
            "target leaked {absent:?}: {target}"
        );
    }

    // Unrelated decoded pairs survive in order and every decoded `dns` key is
    // replaced by exactly one generated pair appended at the end.
    let pairs = target_query_pairs(&target);
    assert_eq!(pairs.len(), 4, "pairs: {pairs:?}");
    assert_eq!(pairs[0], ("foo".to_owned(), "bar".to_owned()));
    assert_eq!(pairs[1], ("x".to_owned(), "a/b".to_owned()));
    assert_eq!(pairs[2], ("empty".to_owned(), String::new()));
    assert_eq!(pairs[3].0, "dns");
    assert_eq!(pairs[3].1, generated_dns_value(&target));
    for removed in ["bogus", "second"] {
        assert!(
            !target.contains(removed),
            "removed dns value leaked: {target}"
        );
    }

    // The generated value is the unpadded URL-safe base64 of the ID-zeroed copy.
    let encoded = generated_dns_value(&target);
    assert!(!encoded.contains('='), "padding is not allowed: {encoded}");
    assert!(
        !encoded.contains('+') && !encoded.contains('/'),
        "not URL-safe: {encoded}"
    );
    let mut expected = original;
    expected[..2].copy_from_slice(&[0, 0]);
    assert_eq!(
        URL_SAFE_NO_PAD
            .decode(&encoded)
            .expect("the dns value is valid URL-safe base64"),
        expected,
        "only the outbound copy's ID bytes are zeroed"
    );
}

#[test]
fn doh_get_request_target_uses_url_safe_unpadded_base64() {
    let query = alphabet_probe_query(0x0102);
    let mut expected = query.clone();
    expected[..2].copy_from_slice(&[0, 0]);

    let standard = STANDARD.encode(&expected);
    assert!(
        standard.contains('+') && standard.contains('/'),
        "the probe must exercise the non-URL-safe alphabet: {standard}"
    );

    let endpoint = DohEndpoint::new("https://dns.example/dns-query", v4(443)).expect("endpoint");
    let target = endpoint
        .get_request_target(ExchangeRequest::new(&query).expect("valid query"))
        .expect("target builds");
    let encoded = generated_dns_value(&target);

    assert_eq!(encoded, URL_SAFE_NO_PAD.encode(&expected));
    assert!(
        encoded.contains('-') && encoded.contains('_'),
        "URL-safe alphabet expected: {encoded}"
    );
    assert!(!encoded.contains('+') && !encoded.contains('/') && !encoded.contains('='));

    // A message whose length would carry standard padding also stays unpadded.
    let padded = doh_query_wire(0x0103);
    assert!(
        STANDARD.encode(&padded).contains('='),
        "the probe length must require standard padding"
    );
    let padded_target = endpoint
        .get_request_target(ExchangeRequest::new(&padded).expect("valid query"))
        .expect("target builds");
    assert!(!generated_dns_value(&padded_target).contains('='));
}

#[test]
fn doh_get_request_target_keeps_ipv6_path_and_dial_semantics() {
    let query = doh_query_wire(0x2001);
    let endpoint =
        DohEndpoint::new("https://[2001:db8::53]/dns-query", v6(443)).expect("IPv6 endpoint");
    let target = endpoint
        .get_request_target(ExchangeRequest::new(&query).expect("valid query"))
        .expect("target builds");

    assert!(target.starts_with("/dns-query?dns="), "target: {target}");
    assert!(
        !target.contains("2001:db8"),
        "service host leaked: {target}"
    );
    assert!(!target.contains('['), "authority leaked: {target}");
    assert_eq!(endpoint.authority(), "[2001:db8::53]");
    assert_eq!(endpoint.host(), "[2001:db8::53]");
    assert_eq!(endpoint.identity().as_str(), "2001:db8::53");
    assert_eq!(endpoint.dial(), v6(443));

    // A service URL without a path still emits the normalized root path.
    let root = DohEndpoint::new("https://dns.example", v4(443)).expect("endpoint");
    let root_target = root
        .get_request_target(ExchangeRequest::new(&query).expect("valid query"))
        .expect("target builds");
    assert!(root_target.starts_with("/?dns="), "target: {root_target}");
}

#[test]
fn doh_get_request_target_preserves_escaped_path_and_ignores_dial() {
    let endpoint =
        DohEndpoint::new("https://dns.example/a%20b/c%2Fd?z=1", v4(8443)).expect("endpoint");
    let query = doh_query_wire(0x0abc);
    let target = endpoint
        .get_request_target(ExchangeRequest::new(&query).expect("valid query"))
        .expect("target builds");

    assert!(target.starts_with("/a%20b/c%2Fd?"), "target: {target}");
    assert_eq!(endpoint.path(), "/a%20b/c%2Fd");
    assert_eq!(endpoint.dial(), v4(8443));
    assert!(!target.contains("8443"), "dial port leaked: {target}");
    assert_eq!(
        target_query_pairs(&target),
        vec![
            ("z".to_owned(), "1".to_owned()),
            ("dns".to_owned(), generated_dns_value(&target)),
        ]
    );
}

#[test]
fn doh_get_request_target_rejects_an_oversized_dns_query_before_encoding() {
    let mut query = doh_query_wire(0x4001);
    query[10..12].copy_from_slice(&1u16.to_be_bytes()); // ARCOUNT = 1
    query.resize(usize::from(u16::MAX) + 1, 0); // 65536 bytes
    let original = query.clone();

    // A path long enough that encoding this query would also break the 96 KiB
    // target bound, so observing the query variant proves it ran first.
    let long_path = format!("https://dns.example/{}", "a".repeat(20 * 1024));
    let endpoint = DohEndpoint::new(&long_path, v4(443)).expect("valid endpoint");
    let request = ExchangeRequest::new(&query).expect("the padded wire is a legal query");
    let error = endpoint
        .get_request_target(request)
        .expect_err("a DNS message over 65535 bytes is rejected");
    assert_eq!(
        error,
        SecureError::DohRequest(DohRequestError::QueryTooLarge)
    );
    assert_eq!(error.side_effect(), SideEffectState::NotSent);
    assert_eq!(query, original, "caller query bytes must not be mutated");

    // Exactly 65535 bytes remains accepted; a root-path endpoint keeps its
    // encoded target under the separate 96 KiB target bound.
    let mut at_limit = doh_query_wire(0x4002);
    at_limit[10..12].copy_from_slice(&1u16.to_be_bytes());
    at_limit.resize(usize::from(u16::MAX), 0);
    let root = DohEndpoint::new("https://dns.example", v4(443)).expect("valid endpoint");
    let limit_request = ExchangeRequest::new(&at_limit).expect("the padded wire is a legal query");
    assert_eq!(at_limit.len(), usize::from(u16::MAX));
    assert!(root.get_request_target(limit_request).is_ok());
}

#[test]
fn doh_get_request_target_rejects_a_target_over_96_kib() {
    let query = doh_query_wire(0x5001);
    let padding = "p".repeat(100 * 1024);
    let service_url = format!("https://dns.example/dns-query?pad={padding}");
    let endpoint = DohEndpoint::new(&service_url, v4(443)).expect("valid endpoint");

    let error = endpoint
        .get_request_target(ExchangeRequest::new(&query).expect("valid query"))
        .expect_err("a request target over 96 KiB is rejected");
    assert_eq!(
        error,
        SecureError::DohRequest(DohRequestError::TargetTooLarge)
    );
    assert_eq!(error.side_effect(), SideEffectState::NotSent);

    // The endpoint's URL material is still exposed unchanged after the failure.
    assert_eq!(endpoint.path(), "/dns-query");
    assert_eq!(endpoint.dial(), v4(443));
}

#[test]
fn doh_get_request_target_errors_expose_no_url_or_query_material() {
    for error in [
        SecureError::DohRequest(DohRequestError::QueryTooLarge),
        SecureError::DohRequest(DohRequestError::TargetTooLarge),
    ] {
        let display = error.to_string();
        let debug = format!("{error:?}");
        for leaked in [
            "https://",
            "dns.example",
            "dns-query",
            "dns=",
            "token",
            "topsecret",
            "2001:db8",
            "bogus",
        ] {
            assert!(
                !display.contains(leaked),
                "Display leaked {leaked:?}: {display}"
            );
            assert!(!debug.contains(leaked), "Debug leaked {leaked:?}: {debug}");
        }
        assert_ne!(error, SecureError::ZeroDialPort);
    }
}

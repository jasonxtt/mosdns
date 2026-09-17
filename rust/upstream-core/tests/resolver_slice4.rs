//! Slice 4 contract: composition of a resolved numeric destination into the
//! existing numeric UDP/TCP and secure DoT/DoH boundaries, with the service
//! identity and URL authority preserved and one original absolute deadline.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mosdns_upstream_core::{
    AddressFamily, BootstrapEndpoint, BootstrapResolver, Clock, ExchangeContext, ResolutionPolicy,
    ResolutionTarget, ResolvedUpstream, ResolverComposition, ResolverError, ServerIdentity,
    Transport, TransportCancellation, resolve_numeric,
};

#[derive(Debug)]
struct FixedClock(Instant);

impl Clock for FixedClock {
    fn now(&self) -> Instant {
        self.0
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

fn a_response(query: &[u8], ip: [u8; 4]) -> Vec<u8> {
    let id = u16::from_be_bytes([query[0], query[1]]);
    let mut position = 12;
    while query[position] != 0 {
        position += 1 + usize::from(query[position]);
    }
    position += 1;
    let question = &query[12..position + 4];
    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&0x8180u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(question);
    wire.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
    wire.extend_from_slice(&900u32.to_be_bytes());
    wire.extend_from_slice(&[0x00, 0x04]);
    wire.extend_from_slice(&ip);
    wire
}

/// A loopback bootstrap fixture answering one query.
fn fixture(ip: [u8; 4]) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
    let address = socket.local_addr().expect("addr");
    let handle = tokio::task::spawn_blocking(move || {
        let mut buffer = vec![0u8; 65535];
        let (length, peer) = socket.recv_from(&mut buffer).expect("recv");
        socket
            .send_to(&a_response(&buffer[..length], ip), peer)
            .expect("send");
    });
    (address, handle)
}

fn resolver_for(peer: SocketAddr) -> BootstrapResolver {
    BootstrapResolver::new(
        ResolutionTarget::new("bootstrap.example.org", 853, AddressFamily::Ipv4).expect("target"),
        BootstrapEndpoint::new(&peer.ip().to_string(), peer.port()).expect("bootstrap"),
        ResolutionPolicy::default(),
        Arc::new(FixedClock(Instant::now())),
    )
    .expect("resolver")
}

#[test]
fn a_resolved_address_composes_into_the_udp_boundary() {
    block_on(async {
        let (address, handle) = fixture([192, 0, 2, 71]);
        let resolver = resolver_for(address);
        let context = ExchangeContext::new(
            Instant::now() + Duration::from_secs(10),
            TransportCancellation::new(),
        );
        let published = resolver.resolve(context).await.expect("resolution");

        // The published numeric address feeds the existing numeric endpoint.
        let udp = ResolverComposition::endpoint(&published, Transport::Udp).expect("udp endpoint");
        assert_eq!(
            udp.address(),
            "192.0.2.71:853".parse::<SocketAddr>().expect("parses")
        );
        assert_eq!(udp.transport(), Transport::Udp);

        // And it can be wrapped in the reviewed plain transport unchanged.
        let plain = ResolvedUpstream::new(published.clone(), Transport::Tcp).expect("upstream");
        assert_eq!(
            plain.dial(),
            "192.0.2.71:853".parse::<SocketAddr>().expect("parses")
        );
        assert_eq!(
            plain.upstream().endpoint().address(),
            "192.0.2.71:853".parse::<SocketAddr>().expect("parses")
        );

        handle.await.expect("fixture");
    });
}

#[test]
fn dot_composition_keeps_the_original_sni_identity() {
    block_on(async {
        let (address, handle) = fixture([192, 0, 2, 72]);
        let resolver = resolver_for(address);
        let context = ExchangeContext::new(
            Instant::now() + Duration::from_secs(10),
            TransportCancellation::new(),
        );
        let published = resolver.resolve(context).await.expect("resolution");

        // The identity is the caller's; resolution never rewrites it.
        let identity = ServerIdentity::new("upstream.example.org").expect("identity");
        let dot = ResolverComposition::dot_endpoint(&published, &identity).expect("dot");
        assert_eq!(
            dot.dial(),
            "192.0.2.72:853".parse::<SocketAddr>().expect("parses")
        );
        assert_eq!(dot.identity().dns_name(), Some("upstream.example.org"));
        // The resolved numeric address did not leak into the identity.
        assert_ne!(dot.identity().dns_name(), Some("192.0.2.72"));

        handle.await.expect("fixture");
    });
}

#[test]
fn doh_composition_keeps_the_original_url_authority_and_path() {
    block_on(async {
        let (address, handle) = fixture([192, 0, 2, 73]);
        let resolver = BootstrapResolver::new(
            ResolutionTarget::new("upstream.example.org", 443, AddressFamily::Ipv4)
                .expect("target"),
            BootstrapEndpoint::new(&address.ip().to_string(), address.port()).expect("bootstrap"),
            ResolutionPolicy::default(),
            Arc::new(FixedClock(Instant::now())),
        )
        .expect("resolver");
        let context = ExchangeContext::new(
            Instant::now() + Duration::from_secs(10),
            TransportCancellation::new(),
        );
        let published = resolver.resolve(context).await.expect("resolution");

        let doh =
            ResolverComposition::doh_endpoint(&published, "https://doh.example.org/dns-query?x=1")
                .expect("doh");
        // The service authority and path stay those of the service.
        assert_eq!(
            doh.dial(),
            "192.0.2.73:443".parse::<SocketAddr>().expect("parses")
        );
        assert_eq!(doh.host(), "doh.example.org");
        assert_eq!(doh.path(), "/dns-query");
        assert_eq!(doh.query(), Some("x=1"));
        assert_ne!(doh.host(), "192.0.2.73");

        handle.await.expect("fixture");
    });
}

#[test]
fn a_numeric_dial_address_bypasses_the_resolver_entirely() {
    // Port 1 on loopback would fail immediately if any socket were opened.
    let published = resolve_numeric("192.0.2.88:853".parse().expect("addr")).expect("literal");
    assert_eq!(
        published.dial(),
        "192.0.2.88:853".parse::<SocketAddr>().expect("parses")
    );
    assert!(published.ttl().is_zero(), "a literal carries no TTL");

    let endpoint = ResolverComposition::endpoint(&published, Transport::Udp).expect("endpoint");
    assert_eq!(
        endpoint.address(),
        "192.0.2.88:853".parse::<SocketAddr>().expect("parses")
    );
}

#[test]
fn a_zero_port_publication_cannot_compose() {
    // `resolve_numeric` already rejects port zero, which is the first gate.
    assert_eq!(
        resolve_numeric("192.0.2.1:0".parse().expect("addr")),
        Err(ResolverError::ZeroPort)
    );
}

#[test]
fn resolution_and_handoff_share_one_original_deadline() {
    block_on(async {
        let (address, handle) = fixture([192, 0, 2, 74]);
        let resolver = resolver_for(address);

        // One context, created once, is used for resolution; the resolved
        // destination is then handed to the transport with the same deadline.
        let deadline = Instant::now() + Duration::from_secs(10);
        let context = ExchangeContext::new(deadline, TransportCancellation::new());
        let published = resolver.resolve(context.clone()).await.expect("resolution");

        let handoff = ResolvedUpstream::new(published, Transport::Udp).expect("upstream");
        let query = {
            let mut wire = Vec::new();
            wire.extend_from_slice(&0x1234u16.to_be_bytes());
            wire.extend_from_slice(&0x0100u16.to_be_bytes());
            wire.extend_from_slice(&1u16.to_be_bytes());
            wire.extend_from_slice(&0u16.to_be_bytes());
            wire.extend_from_slice(&0u16.to_be_bytes());
            wire.extend_from_slice(&0u16.to_be_bytes());
            wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
            wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0x00]);
            wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
            wire
        };
        let request = mosdns_upstream_core::ExchangeRequest::new(&query).expect("request");

        // The handoff reuses the caller's own absolute deadline verbatim; the
        // resolver granted no fresh budget.
        assert_eq!(context.deadline(), deadline);
        let prepared = handoff
            .upstream()
            .prepare_exchange(request, context.clone())
            .expect("prepared");
        assert_eq!(prepared.context().deadline(), deadline);

        handle.await.expect("fixture");
    });
}

#[test]
fn a_numeric_target_resolver_never_opens_a_bootstrap_socket() {
    block_on(async {
        let resolver = BootstrapResolver::new(
            ResolutionTarget::new("192.0.2.99", 853, AddressFamily::Ipv4).expect("literal"),
            BootstrapEndpoint::new("127.0.0.1", 1).expect("bootstrap"),
            ResolutionPolicy::default(),
            Arc::new(FixedClock(Instant::now())),
        )
        .expect("resolver");
        let context = ExchangeContext::new(
            Instant::now() + Duration::from_secs(10),
            TransportCancellation::new(),
        );
        let published = resolver.resolve(context).await.expect("literal bypass");
        assert_eq!(
            published.dial(),
            "192.0.2.99:853".parse::<SocketAddr>().expect("parses")
        );
    });
}

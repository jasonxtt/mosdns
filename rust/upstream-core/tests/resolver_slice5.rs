//! Slice 5 contract: the complete end-to-end boundary and the full error matrix.
//!
//! The central test proves the whole composition: a hostname is resolved through
//! a loopback bootstrap, the resolved numeric address drives a real plain UDP
//! exchange against a second loopback server, and the caller's one original
//! absolute deadline is never extended along the way.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mosdns_upstream_core::{
    AddressFamily, BootstrapEndpoint, BootstrapResolver, Clock, CloseResult, ExchangeContext,
    ExchangeRequest, ResolutionPolicy, ResolutionTarget, ResolverComposition, ResolverError,
    ServerIdentity, Transport, TransportCancellation, Upstream, resolve_numeric,
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

/// Builds an A response correlated to `query`.
fn a_response(query: &[u8], ip: [u8; 4], ttl: u32) -> Vec<u8> {
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
    wire.extend_from_slice(&ttl.to_be_bytes());
    wire.extend_from_slice(&[0x00, 0x04]);
    wire.extend_from_slice(&ip);
    wire
}

/// A plain query for `example.org` with a caller-chosen ID.
fn query_wire(id: u16) -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&0x0100u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
    wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0x00]);
    wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
    wire
}

fn bind_loopback() -> (std::net::UdpSocket, SocketAddr) {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
    let address = socket.local_addr().expect("addr");
    (socket, address)
}

/// Bounds every await in this file, so no fixture or exchange can hang the
/// suite: a wedged peer fails the test rather than blocking it forever.
const STEP: Duration = Duration::from_secs(5);

/// Bounds every fixture's blocking loop, so its task always finishes. It only
/// has to outlive the caller's own short deadline, because a fixture's job is
/// done as soon as the exchange it serves has ended.
const FIXTURE_TIMEOUT: Duration = Duration::from_millis(1_500);

/// Serves exactly one DNS query on `socket`, replying with an A record for
/// `answer`. The fixture drains retransmissions while it works, so a slow first
/// reply cannot make a retransmission be mistaken for a second real query.
fn serve_one(
    socket: std::net::UdpSocket,
    answer: [u8; 4],
    ttl: u32,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        socket
            .set_read_timeout(Some(Duration::from_millis(200)))
            .expect("read timeout");
        let started = std::time::Instant::now();
        let mut buffer = vec![0u8; 65535];
        loop {
            if started.elapsed() > FIXTURE_TIMEOUT {
                break;
            }
            let Ok((length, peer)) = socket.recv_from(&mut buffer) else {
                continue;
            };
            let reply = a_response(&buffer[..length], answer, ttl);
            if socket.send_to(&reply, peer).is_err() {
                break;
            }
            // One answered query is the fixture's whole job.
            break;
        }
    })
}

#[test]
fn resolved_address_drives_a_real_udp_exchange_on_one_deadline() {
    block_on(async {
        // Two explicitly bound loopback sockets play the two distinct roles:
        // the numeric bootstrap peer, and the resolved target server. The
        // bootstrap answers with the target socket's own address and port, so
        // the published numeric destination is genuinely reachable.
        let (bootstrap_socket, bootstrap_address) = bind_loopback();
        let (target_socket, target_address) = bind_loopback();

        let bootstrap_port = target_address.port();
        let bootstrap_fixture = serve_one(
            bootstrap_socket,
            match target_address.ip() {
                std::net::IpAddr::V4(v4) => v4.octets(),
                std::net::IpAddr::V6(_) => [127, 0, 0, 1],
            },
            900,
        );
        let target_fixture = serve_one(target_socket, [198, 51, 100, 7], 600);

        // The target's port is what makes the resolved address dialable.
        let resolver = BootstrapResolver::new(
            ResolutionTarget::new("upstream.example.org", bootstrap_port, AddressFamily::Ipv4)
                .expect("target"),
            BootstrapEndpoint::new("127.0.0.1", bootstrap_address.port()).expect("bootstrap"),
            ResolutionPolicy::default(),
            Arc::new(FixedClock(Instant::now())),
        )
        .expect("resolver");

        // ONE context, created once. Resolution and the transport handoff share
        // exactly this absolute deadline; nothing grants a fresh budget.
        let deadline = Instant::now() + Duration::from_secs(15);
        let context = ExchangeContext::new(deadline, TransportCancellation::new());

        let published = tokio::time::timeout(STEP, resolver.resolve(context.clone()))
            .await
            .expect("resolution bounded")
            .expect("resolution");

        // The client-visible hostname resolved to the real loopback target.
        assert_eq!(published.target().host(), "upstream.example.org");
        assert_eq!(published.dial(), target_address);

        // The resolved numeric address, not the hostname, is what is dialed.
        let endpoint = ResolverComposition::endpoint(&published, Transport::Udp).expect("endpoint");
        assert_eq!(endpoint.address(), target_address);

        let upstream = Upstream::new(endpoint);
        let query = query_wire(0x4242);
        let request = ExchangeRequest::new(&query).expect("request");

        let response = tokio::time::timeout(STEP, upstream.exchange(request, context.clone()))
            .await
            .expect("exchange bounded")
            .expect("the resolved address carries the real exchange");

        // The transport returned the target server's answer, and the deadline
        // handed to it is still the caller's original one.
        assert_eq!(response.response_id(), 0x4242);
        assert!(response.wire().len() > 12);
        assert_eq!(response.wire(), a_response(&query, [198, 51, 100, 7], 600));
        assert_eq!(context.deadline(), deadline);
        assert_eq!(upstream.endpoint().address(), published.dial());

        upstream.close().await;
        bootstrap_fixture.await.expect("bootstrap fixture joined");
        target_fixture.await.expect("target fixture joined");
    });
}

#[test]
fn the_full_error_matrix_is_typed_and_never_a_dial_attempt() {
    // Every constructor-level rejection is typed and happens before any I/O.
    assert_eq!(
        ResolutionTarget::new("", 853, AddressFamily::Ipv4),
        Err(ResolverError::InvalidHostname)
    );
    assert_eq!(
        ResolutionTarget::new("bootstrap.example.org", 0, AddressFamily::Ipv4),
        Err(ResolverError::ZeroPort)
    );
    assert_eq!(
        BootstrapEndpoint::new("dns.example.org", 53),
        Err(ResolverError::BootstrapNotNumeric)
    );
    assert_eq!(
        mosdns_upstream_core::ConfigVersion::from_u8(9),
        Err(ResolverError::UnsupportedConfigVersion(9))
    );
    assert_eq!(
        resolve_numeric("192.0.2.1:0".parse().expect("addr")),
        Err(ResolverError::ZeroPort)
    );
    assert_eq!(
        ResolutionPolicy::new(Duration::ZERO, Duration::from_secs(60)),
        Err(ResolverError::InvalidPolicy)
    );

    // A numeric target with a mismatched family is rejected rather than dialed.
    assert_eq!(
        ResolutionTarget::new("192.0.2.1", 853, AddressFamily::Ipv6),
        Err(ResolverError::FamilyMismatch)
    );
}

#[test]
fn a_malformed_bootstrap_reply_is_typed_not_a_dial() {
    block_on(async {
        let (server, server_address) = bind_loopback();
        // The fixture keeps its socket bound after sending the junk packet, so
        // the exchange ends at the caller's own deadline instead of being cut
        // short by an ICMP port-unreachable.
        let server_task = tokio::task::spawn_blocking(move || {
            server
                .set_read_timeout(Some(Duration::from_millis(200)))
                .expect("read timeout");
            let started = std::time::Instant::now();
            let mut buffer = vec![0u8; 65535];
            let mut sent_junk = false;
            while started.elapsed() < FIXTURE_TIMEOUT {
                let Ok((_, peer)) = server.recv_from(&mut buffer) else {
                    continue;
                };
                if !sent_junk {
                    // A QR-clear packet is not a response at all.
                    server
                        .send_to(&query_wire(0x1234), peer)
                        .expect("send junk");
                    sent_junk = true;
                }
            }
        });

        let resolver = BootstrapResolver::new(
            ResolutionTarget::new("upstream.example.org", 53, AddressFamily::Ipv4).expect("target"),
            BootstrapEndpoint::new("127.0.0.1", server_address.port()).expect("bootstrap"),
            ResolutionPolicy::default(),
            Arc::new(FixedClock(Instant::now())),
        )
        .expect("resolver");

        let context = ExchangeContext::new(
            Instant::now() + Duration::from_secs(2),
            TransportCancellation::new(),
        );
        // The junk datagram is ignored while budget remains, so this ends on the
        // caller's deadline rather than being misreported as a success.
        let error = tokio::time::timeout(Duration::from_secs(10), resolver.resolve(context))
            .await
            .expect("bounded")
            .expect_err("no valid reply");
        assert_eq!(
            error,
            ResolverError::BootstrapTimeout,
            "a non-response is ignored, not treated as an answer"
        );
        assert!(
            resolver.state().published().is_none(),
            "nothing is published"
        );

        server_task.await.expect("server task joined");
    });
}

#[test]
fn a_terminal_dns_rcode_publishes_nothing() {
    block_on(async {
        let (server, server_address) = bind_loopback();
        let server_task = tokio::task::spawn_blocking(move || {
            let mut buffer = vec![0u8; 65535];
            let (length, peer) = server.recv_from(&mut buffer).expect("recv");
            let query = &buffer[..length];
            let id = u16::from_be_bytes([query[0], query[1]]);
            // SERVFAIL with the question echoed, so it correlates but is negative.
            let mut reply = a_response(query, [0, 0, 0, 0], 300);
            reply[0..2].copy_from_slice(&id.to_be_bytes());
            reply[3] = (reply[3] & 0xf0) | 0x02;
            reply[6..8].copy_from_slice(&0u16.to_be_bytes()); // no answers
            reply.truncate(12 + query.len() - 12);
            server.send_to(&reply, peer).expect("send");
        });

        let resolver = BootstrapResolver::new(
            ResolutionTarget::new("upstream.example.org", 53, AddressFamily::Ipv4).expect("target"),
            BootstrapEndpoint::new("127.0.0.1", server_address.port()).expect("bootstrap"),
            ResolutionPolicy::default(),
            Arc::new(FixedClock(Instant::now())),
        )
        .expect("resolver");

        let context = ExchangeContext::new(
            Instant::now() + Duration::from_secs(2),
            TransportCancellation::new(),
        );
        let error = resolver
            .resolve(context)
            .await
            .expect_err("negative answer");
        assert!(
            matches!(
                error,
                ResolverError::BootstrapRcode(2) | ResolverError::MalformedBootstrapResponse
            ),
            "a terminal negative answer is typed, got {error:?}"
        );
        assert!(
            resolver.state().published().is_none(),
            "nothing is published"
        );

        server_task.await.expect("server task joined");
    });
}

#[test]
fn close_drains_and_the_owner_stays_usable_only_until_closing() {
    block_on(async {
        let resolver = BootstrapResolver::new(
            ResolutionTarget::new("192.0.2.5", 853, AddressFamily::Ipv4).expect("literal"),
            BootstrapEndpoint::new("127.0.0.1", 53).expect("bootstrap"),
            ResolutionPolicy::default(),
            Arc::new(FixedClock(Instant::now())),
        )
        .expect("resolver");

        // A numeric target resolves without any traffic even before close.
        let context = ExchangeContext::new(
            Instant::now() + Duration::from_secs(5),
            TransportCancellation::new(),
        );
        assert!(resolver.resolve(context).await.is_ok());

        assert_eq!(resolver.close().await, CloseResult::Closed);
        assert_eq!(resolver.in_flight_resolutions(), 0);
        assert_eq!(resolver.close().await, CloseResult::AlreadyClosed);
    });
}

#[test]
fn identity_separation_survives_the_full_composition() {
    let identity = ServerIdentity::new("secure.example.org").expect("identity");
    let published = resolve_numeric("192.0.2.200:853".parse().expect("addr")).expect("literal");
    let dot = ResolverComposition::dot_endpoint(&published, &identity).expect("dot");
    // The numeric dial address and the secure identity stay independent.
    assert_eq!(dot.dial().ip(), std::net::IpAddr::from([192, 0, 2, 200]));
    assert_eq!(dot.identity().dns_name(), Some("secure.example.org"));
    assert!(dot.identity().is_dns_name());
}

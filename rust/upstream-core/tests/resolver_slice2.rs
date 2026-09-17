//! Slice 2 contract: the bounded connected UDP bootstrap exchange.
//!
//! Every test uses a loopback fixture with a random high port and explicit
//! synchronization; no external DNS, port 53, or wall-clock sleep is used.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mosdns_upstream_core::{
    AddressFamily, BootstrapEndpoint, BootstrapResolver, Clock, ResolutionPolicy, ResolutionTarget,
    ResolverError, TransportCancellation,
};

/// A clock the test advances by hand.
#[derive(Debug)]
struct TestClock {
    now: std::time::Instant,
}

impl TestClock {
    fn new() -> Self {
        Self {
            now: std::time::Instant::now(),
        }
    }
}

impl Clock for TestClock {
    fn now(&self) -> std::time::Instant {
        self.now
    }
}

fn target() -> ResolutionTarget {
    ResolutionTarget::new("bootstrap.example.org", 853, AddressFamily::Ipv4).expect("target")
}

fn bootstrap(address: SocketAddr) -> BootstrapEndpoint {
    BootstrapEndpoint::new(&address.ip().to_string(), address.port()).expect("bootstrap")
}

fn resolver(peer: SocketAddr) -> Arc<BootstrapResolver> {
    Arc::new(
        BootstrapResolver::new(
            target(),
            bootstrap(peer),
            ResolutionPolicy::default(),
            Arc::new(TestClock::new()),
        )
        .expect("resolver"),
    )
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

fn context(deadline: Duration) -> mosdns_upstream_core::ExchangeContext {
    mosdns_upstream_core::ExchangeContext::new(
        Instant::now() + deadline,
        TransportCancellation::new(),
    )
}

/// Builds a correlated A response for the query in `query`.
fn a_response(query: &[u8], ip: [u8; 4], ttl: u32) -> Vec<u8> {
    let id = u16::from_be_bytes([query[0], query[1]]);
    // Find the question end to echo it verbatim.
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

/// Binds a loopback UDP fixture that answers exactly one query.
///
/// It returns the address to point the resolver at and a handle whose awaited
/// result is the reply it actually sent. The blocking fixture is spawned inside
/// the same runtime as the resolution, because the resolution itself must run
/// on a Tokio reactor.
fn one_shot_fixture(ip: [u8; 4], ttl: u32) -> (SocketAddr, tokio::task::JoinHandle<Vec<u8>>) {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind fixture");
    let address = socket.local_addr().expect("fixture address");
    let handle = tokio::task::spawn_blocking(move || {
        let mut buffer = vec![0u8; 65535];
        let (length, peer) = socket
            .recv_from(&mut buffer)
            .expect("receive bootstrap query");
        let reply = a_response(&buffer[..length], ip, ttl);
        socket.send_to(&reply, peer).expect("send bootstrap reply");
        reply
    });
    (address, handle)
}

#[test]
fn resolves_one_address_from_a_loopback_bootstrap() {
    block_on(async {
        let (address, fixture) = one_shot_fixture([192, 0, 2, 45], 900);
        let resolver = resolver(address);
        let published = resolver
            .resolve(context(Duration::from_secs(10)))
            .await
            .expect("resolved");

        assert_eq!(
            published.dial(),
            "192.0.2.45:853".parse::<SocketAddr>().expect("parses")
        );
        assert_eq!(published.family(), AddressFamily::Ipv4);
        assert_eq!(published.ttl(), Duration::from_secs(900));

        let reply = fixture.await.expect("fixture joined");
        assert!(!reply.is_empty(), "the fixture answered the sent query");
    });
}

#[test]
fn clamps_an_out_of_range_ttl_to_the_policy() {
    block_on(async {
        let (address, fixture) = one_shot_fixture([192, 0, 2, 46], 1);
        let resolver = resolver(address);
        let published = resolver
            .resolve(context(Duration::from_secs(10)))
            .await
            .expect("resolved");
        assert_eq!(
            published.ttl(),
            Duration::from_secs(300),
            "a one-second TTL is raised to the five-minute floor"
        );
        fixture.await.expect("fixture joined");
    });
}

#[test]
fn an_expired_deadline_never_reaches_the_network() {
    let resolver = resolver("127.0.0.1:1".parse().expect("addr"));
    let error = block_on(resolver.resolve(context(Duration::ZERO))).expect_err("deadline");
    assert_eq!(error, ResolverError::BootstrapTimeout);
}

#[test]
fn caller_cancellation_is_reported_as_cancelled() {
    let resolver = resolver("127.0.0.1:1".parse().expect("addr"));
    let cancellation = TransportCancellation::new();
    cancellation.cancel();
    let expired = mosdns_upstream_core::ExchangeContext::new(
        Instant::now() + Duration::from_secs(10),
        cancellation,
    );
    let error = block_on(resolver.resolve(expired)).expect_err("cancelled");
    assert_eq!(error, ResolverError::Cancelled);
}

#[test]
fn a_non_numeric_bootstrap_cannot_be_constructed() {
    assert_eq!(
        BootstrapEndpoint::new("dns.example.org", 53),
        Err(ResolverError::BootstrapNotNumeric)
    );
}

#[test]
fn a_numeric_target_resolves_without_any_bootstrap_traffic() {
    let numeric = ResolutionTarget::new("192.0.2.77", 853, AddressFamily::Ipv4).expect("literal");
    // Port 1 on loopback would fail immediately if a socket were actually used.
    let resolver = Arc::new(
        BootstrapResolver::new(
            numeric,
            bootstrap("127.0.0.1:1".parse().expect("addr")),
            ResolutionPolicy::default(),
            Arc::new(TestClock::new()),
        )
        .expect("resolver"),
    );
    let published = block_on(resolver.resolve(context(Duration::from_secs(10)))).expect("literal");
    assert_eq!(
        published.dial(),
        "192.0.2.77:853".parse::<SocketAddr>().expect("parses")
    );
    assert_eq!(
        published.destination().address(),
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 77))
    );
}

#[test]
fn close_shutdown_is_observable_and_idempotent() {
    let resolver = resolver("127.0.0.1:1".parse().expect("addr"));
    assert_eq!(
        resolver.lifecycle_state(),
        mosdns_upstream_core::LifecycleState::Open
    );
    assert_eq!(
        resolver.begin_close(),
        mosdns_upstream_core::CloseTransition::BeganClosing
    );
    assert_eq!(
        resolver.begin_close(),
        mosdns_upstream_core::CloseTransition::AlreadyClosing
    );
    let error = block_on(resolver.resolve(context(Duration::from_secs(10)))).expect_err("closed");
    assert_eq!(error, ResolverError::Closed);
    assert_eq!(
        block_on(resolver.close()),
        mosdns_upstream_core::CloseResult::Closed
    );
    assert_eq!(
        resolver.lifecycle_state(),
        mosdns_upstream_core::LifecycleState::Closed
    );
    assert_eq!(resolver.in_flight_resolutions(), 0);
}

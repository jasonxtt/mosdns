//! Contract tests for explicit dual-stack endpoint selection.
//!
//! Scope: `bootstrap_version` omitted / `4` / `6` / explicit `0`, independent
//! A+AAAA bootstrap collection under one caller-owned budget, A-preferred
//! selection, per-family candidate/TTL/expiry/diagnostic state, generation
//! replacement, and the closed error matrix.
//!
//! Every test is deterministic: no wall-clock sleep, no elapsed-time polling,
//! no external DNS. Fixtures are numeric loopback UDP sockets on random high
//! ports. Explicit barriers and a hand-advanced clock provide all ordering.
//!
//! What these tests deliberately do NOT cover, because the task forbids it:
//! Happy Eyeballs, target connection racing, connection-failure cross-family
//! fallback, protocol fallback, QUIC/HTTP3, pools, or a system resolver.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use mosdns_upstream_core::{
    AddressFamily, BootstrapEndpoint, BootstrapResolver, Clock, ConfigVersion, ExchangeContext,
    FamilyCandidate, ResolutionMode, ResolutionPolicy, ResolutionTarget, ResolverError,
    ServerIdentity, Transport, TransportCancellation,
};

// ---------------------------------------------------------------------------
// Deterministic clock
// ---------------------------------------------------------------------------

/// A clock the test advances by hand, shared with the resolver under test.
#[derive(Debug)]
struct ManualClock {
    now: std::sync::Mutex<Instant>,
}

impl ManualClock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            now: std::sync::Mutex::new(Instant::now()),
        })
    }

    fn advance(&self, seconds: u64) {
        let mut now = self.now.lock().expect("clock");
        *now += Duration::from_secs(seconds);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.now.lock().expect("clock")
    }
}

/// The upper bound on any single await in this file. A wedged peer, a missed
/// wake, or a deadlock fails the test instead of hanging it.
///
/// This is a **deadlock guard only**. Fixture lifetime is ended explicitly by
/// [`FixtureStop::stop`]; the deadline never terminates a fixture in normal
/// operation, and no assertion in this file depends on elapsed wall-clock time.
const DEADLINE: Duration = Duration::from_secs(20);

async fn bounded<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(DEADLINE, future)
        .await
        .expect("operation must complete within the test deadline")
}

/// Waits for an explicitly signalled fixture thread to finish.
///
/// A `JoinHandle` returned by `spawn_blocking` cannot be cancelled, so there is
/// no "abort" path: the owning test must release the fixture first. A stuck
/// handle means the fixture's protocol was violated, so this fails the test
/// instead of hanging the suite.
async fn join_fixture(handle: tokio::task::JoinHandle<()>) {
    // `bounded` already panics on the deadlock guard; the inner result is the
    // fixture thread's own join outcome.
    let joined = bounded(handle).await;
    joined.expect("fixture thread must not panic");
}

/// A one-byte marker that releases a fixture's blocking `recv_from`.
///
/// The marker datagram is deliberately shorter than a DNS header (12 bytes), so
/// a fixture can never mistake it for a query: every legitimate query is at
/// least a header plus a question. It is sent from a control socket that the
/// **test** owns, which is what makes classification unambiguous.
const STOP_MARKER: [u8; 1] = [0x00];

/// A deterministic stop handshake for a loopback fixture thread.
///
/// The test owns the fixture's lifetime entirely: [`Self::stop`] sends one
/// [`STOP_MARKER`] datagram from the test-owned control socket **to the fixture's
/// own UDP address**, which immediately wakes the fixture's **blocking**
/// `recv_from`. The fixture recognises the marker by the datagram's *source* (its
/// control peer) and by its being shorter than a DNS header, returns, and only
/// then does the caller await the join.
///
/// `peer` and `target` are different addresses and must not be confused: `peer`
/// is the control socket's own address, which is what the fixture compares the
/// *source* against; `target` is the fixture socket's address, which is where the
/// marker is *sent*.
///
/// There is no poll interval, no read timeout, and no elapsed-time deadline on
/// the fixture path: the fixture's `recv_from` blocks indefinitely until either a
/// real query or the marker arrives, and the marker is what ends it. A fixture
/// never stops because a wall clock elapsed or because it received some number
/// of datagrams.
struct FixtureStop {
    control: Arc<std::net::UdpSocket>,
    peer: SocketAddr,
    target: Option<SocketAddr>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl FixtureStop {
    /// Creates the control socket the fixture will recognise as its test peer.
    fn new() -> Self {
        let control = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("control bind");
        let peer = control.local_addr().expect("control addr");
        Self {
            control: Arc::new(control),
            peer,
            target: None,
            handle: None,
        }
    }

    /// The control socket's address, which the fixture compares *sources* against.
    fn peer(&self) -> SocketAddr {
        self.peer
    }

    /// Attaches the fixture socket's address and its thread.
    ///
    /// The address is where the marker is sent; it is the fixture's own bound
    /// loopback address from [`bind_loopback`].
    fn attach(&mut self, target: SocketAddr, handle: tokio::task::JoinHandle<()>) {
        self.target = Some(target);
        self.handle = Some(handle);
    }

    /// Sends the stop marker to the fixture, then awaits its join.
    ///
    /// The marker is sent before the join, so a fixture blocked in `recv_from`
    /// is woken by the datagram itself rather than by any timeout.
    async fn stop(&mut self) {
        let handle = self.handle.take().expect("fixture handle attached");
        let target = self.target.expect("fixture target attached");
        let sent = self
            .control
            .send_to(&STOP_MARKER, target)
            .expect("send stop marker");
        assert_eq!(sent, STOP_MARKER.len(), "the marker is one datagram");
        join_fixture(handle).await;
    }
}

/// Whether a datagram is the fixture's stop marker.
///
/// Both conditions are required. The source address proves the datagram came
/// from the test-owned control socket and not from the resolver, and the length
/// proves it is not a DNS message — so a resolver query can never be mistaken
/// for a release, whatever it contains.
fn is_stop_marker(from: SocketAddr, length: usize, control: SocketAddr) -> bool {
    from == control && length < 12
}

/// Reads one datagram from a fixture socket, blocking until one arrives.
///
/// There is no read timeout: the fixture's only exit is the stop marker, so a
/// missing marker surfaces as the outer join guard rather than as a silent
/// timeout-driven exit.
fn recv_query(socket: &std::net::UdpSocket, buffer: &mut [u8]) -> Option<(usize, SocketAddr)> {
    socket.recv_from(buffer).ok()
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

fn context(seconds: u64) -> ExchangeContext {
    ExchangeContext::new(
        Instant::now() + Duration::from_secs(seconds),
        TransportCancellation::new(),
    )
}

// ---------------------------------------------------------------------------
// Loopback bootstrap fixtures
// ---------------------------------------------------------------------------

/// The DNS wire question type for a family, read from the query the fixture got.
fn query_qtype(query: &[u8]) -> u16 {
    let mut position = 12;
    while query[position] != 0 {
        position += 1 + usize::from(query[position]);
    }
    // Skip the root label and read the 2-byte QTYPE.
    u16::from_be_bytes([query[position + 1], query[position + 2]])
}

/// Builds a response echoing `query`'s question with one answer of `family`.
fn a_response_for(query: &[u8], family: AddressFamily, address: IpAddr, ttl: u32) -> Vec<u8> {
    let id = u16::from_be_bytes([query[0], query[1]]);
    let mut position = 12;
    while query[position] != 0 {
        position += 1 + usize::from(query[position]);
    }
    position += 1;
    let question = &query[12..position + 4];

    let (rr_type, rdata) = match address {
        IpAddr::V4(v4) => (1u16, v4.octets().to_vec()),
        IpAddr::V6(v6) => (28u16, v6.octets().to_vec()),
    };
    assert_eq!(
        family,
        match address {
            IpAddr::V4(_) => AddressFamily::Ipv4,
            IpAddr::V6(_) => AddressFamily::Ipv6,
        }
    );

    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&0x8180u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(question);
    wire.extend_from_slice(&[0xc0, 0x0c]);
    wire.extend_from_slice(&rr_type.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&ttl.to_be_bytes());
    wire.extend_from_slice(&u16::try_from(rdata.len()).expect("rdata len").to_be_bytes());
    wire.extend_from_slice(&rdata);
    wire
}

/// A correlated response with a non-NOERROR rcode and no answers.
fn rcode_response(query: &[u8], rcode: u8) -> Vec<u8> {
    let id = u16::from_be_bytes([query[0], query[1]]);
    let mut position = 12;
    while query[position] != 0 {
        position += 1 + usize::from(query[position]);
    }
    position += 1;
    let question = &query[12..position + 4];

    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&0x8000u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(question);
    let header = u16::from_be_bytes([wire[2], wire[3]]) | u16::from(rcode);
    wire[2..4].copy_from_slice(&header.to_be_bytes());
    wire
}

/// A correlated TC=1 response: the question is intact, the header declares one
/// answer, and the answer body is absent.
///
/// TC is terminal immediately after correlation (the reviewed Slice 0
/// behavior), so this must be reported as truncated rather than as a malformed
/// body even though the declared record is missing.
fn truncated_response(query: &[u8]) -> Vec<u8> {
    let id = u16::from_be_bytes([query[0], query[1]]);
    let mut position = 12;
    while query[position] != 0 {
        position += 1 + usize::from(query[position]);
    }
    position += 1;
    let question = &query[12..position + 4];

    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    // QR set, TC set, RD/RA set, RCODE NOERROR.
    wire.extend_from_slice(&0x8380u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    wire.extend_from_slice(&1u16.to_be_bytes()); // ANCOUNT: one answer declared
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(question);
    wire
}

/// A fixture whose per-family answers change between generations.
///
/// The test drives the phase explicitly, so a real refresh against the *same*
/// resolver state can be observed without timing assumptions. Phase transitions
/// happen only when the test flips the shared cell, which it does while no leg
/// is in flight.
struct SwitchingFixture {
    address: SocketAddr,
    phase: Arc<AtomicUsize>,
    stop: FixtureStop,
}

impl SwitchingFixture {
    /// Advances to the next phase. Called between generations.
    fn set_phase(&self, phase: usize) {
        self.phase.store(phase, Ordering::SeqCst);
    }

    /// Sends the stop marker, then awaits the fixture's join.
    async fn stop(mut self) {
        self.stop.stop().await;
    }
}

/// Serves `plan[phase]` for each family, where each entry is `(A, AAAA)`.
///
/// The fixture exits when the test releases it; there is no elapsed-time
/// deadline and no datagram-count expectation, because the test decides how many
/// generations it drives.
fn switching_fixture(plan: Vec<(Answer, Answer)>) -> SwitchingFixture {
    let (socket, address) = bind_loopback();
    let phase = Arc::new(AtomicUsize::new(0));
    let phase_in = Arc::clone(&phase);
    let mut stop = FixtureStop::new();
    let control = stop.peer();

    let handle = tokio::task::spawn_blocking(move || {
        let mut buffer = vec![0u8; 65535];
        loop {
            let Some((length, peer)) = recv_query(&socket, &mut buffer) else {
                continue;
            };
            if is_stop_marker(peer, length, control) {
                return;
            }
            let query = &buffer[..length];
            let current = phase_in.load(Ordering::SeqCst);
            let Some((a, aaaa)) = plan.get(current) else {
                continue;
            };
            let (chosen, family) = if query_qtype(query) == 28 {
                (*aaaa, AddressFamily::Ipv6)
            } else {
                (*a, AddressFamily::Ipv4)
            };
            let reply = match chosen {
                Answer::Silent => continue,
                Answer::Rcode(code) => rcode_response(query, code),
                Answer::Address(address, ttl) => a_response_for(query, family, address, ttl),
            };
            socket.send_to(&reply, peer).expect("send");
        }
    });
    stop.attach(address, handle);
    SwitchingFixture {
        address,
        phase,
        stop,
    }
}

/// Binds a loopback UDP socket on an ephemeral high port.
fn bind_loopback() -> (std::net::UdpSocket, SocketAddr) {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
    let address = socket.local_addr().expect("addr");
    (socket, address)
}

/// What a fixture should answer for one family.
#[derive(Clone, Copy)]
enum Answer {
    /// Answer with this address and TTL.
    Address(IpAddr, u32),
    /// Answer with this terminal rcode and no records.
    Rcode(u8),
    /// Do not answer at all, so the leg ends on the caller's deadline.
    Silent,
}

/// A fixture that answers each family with an independently controlled result.
///
/// It records every query it receives, so tests can assert how many lookups
/// happened and which QTYPE each carried. Lifetime is explicit: the fixture runs
/// until the owning test sends the stop marker, and there is no elapsed-time
/// deadline and no datagram-count target.
struct DualFixture {
    address: SocketAddr,
    ipv4_seen: Arc<AtomicUsize>,
    ipv6_seen: Arc<AtomicUsize>,
    stop: FixtureStop,
}

/// Builds a dual-family fixture.
///
/// The fixture runs until the owning test releases it with
/// [`DualFixture::stop`]; it never stops on elapsed time or on a datagram count.
fn dual_fixture(ipv4: Answer, ipv6: Answer) -> DualFixture {
    let (socket, address) = bind_loopback();
    let ipv4_seen = Arc::new(AtomicUsize::new(0));
    let ipv6_seen = Arc::new(AtomicUsize::new(0));
    let ipv4_in = Arc::clone(&ipv4_seen);
    let ipv6_in = Arc::clone(&ipv6_seen);
    let mut stop = FixtureStop::new();
    let control = stop.peer();

    let handle = tokio::task::spawn_blocking(move || {
        let mut buffer = vec![0u8; 65535];
        loop {
            let Some((length, peer)) = recv_query(&socket, &mut buffer) else {
                continue;
            };
            if is_stop_marker(peer, length, control) {
                return;
            }
            let query = &buffer[..length];
            let reply = if query_qtype(query) == 28 {
                ipv6_in.fetch_add(1, Ordering::Relaxed);
                match ipv6 {
                    Answer::Silent => continue,
                    Answer::Rcode(code) => rcode_response(query, code),
                    Answer::Address(address, ttl) => {
                        a_response_for(query, AddressFamily::Ipv6, address, ttl)
                    }
                }
            } else {
                ipv4_in.fetch_add(1, Ordering::Relaxed);
                match ipv4 {
                    Answer::Silent => continue,
                    Answer::Rcode(code) => rcode_response(query, code),
                    Answer::Address(address, ttl) => {
                        a_response_for(query, AddressFamily::Ipv4, address, ttl)
                    }
                }
            };
            socket.send_to(&reply, peer).expect("send");
        }
    });
    stop.attach(address, handle);

    DualFixture {
        address,
        ipv4_seen,
        ipv6_seen,
        stop,
    }
}

impl DualFixture {
    fn queries(&self) -> (usize, usize) {
        (
            self.ipv4_seen.load(Ordering::Relaxed),
            self.ipv6_seen.load(Ordering::Relaxed),
        )
    }

    /// Sends the stop marker, then awaits the fixture's join.
    async fn stop(mut self) {
        self.stop.stop().await;
    }
}

const V4: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 10);
const V6: Ipv6Addr = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x10);

fn dual_resolver(
    host: &str,
    peer: SocketAddr,
    clock: Arc<ManualClock>,
    mode: ResolutionMode,
) -> BootstrapResolver {
    // The target's own declared family must agree with a single-family mode, so
    // dual mode builds against its preferred family like any other plan.
    let family = mode.preferred_family();
    BootstrapResolver::with_deterministic_ids_and_mode_for_tests(
        ResolutionTarget::new(host, 853, family).expect("target"),
        BootstrapEndpoint::new(&peer.ip().to_string(), peer.port()).expect("bootstrap"),
        ResolutionPolicy::default(),
        clock,
        mode,
    )
    .expect("resolver")
}

// ---------------------------------------------------------------------------
// Slice 0 — version mapping and mode model
// ---------------------------------------------------------------------------

#[test]
fn omitted_default_and_four_are_a_only_and_six_is_aaaa_only() {
    // The product default for an omitted version is `4`: A-only.
    assert_eq!(ConfigVersion::from_optional(None), Ok(ConfigVersion::Ipv4));
    assert_eq!(ConfigVersion::default(), ConfigVersion::Ipv4);
    assert_eq!(
        ConfigVersion::from_optional(Some(4)),
        Ok(ConfigVersion::Ipv4)
    );
    assert_eq!(
        ConfigVersion::from_optional(Some(6)),
        Ok(ConfigVersion::Ipv6)
    );
    // Explicit zero is the only dual entry point.
    assert_eq!(
        ConfigVersion::from_optional(Some(0)),
        Ok(ConfigVersion::Zero)
    );

    assert_eq!(ResolutionMode::Ipv4.families(), &[AddressFamily::Ipv4]);
    assert_eq!(ResolutionMode::Ipv6.families(), &[AddressFamily::Ipv6]);
    assert!(!ResolutionMode::Ipv4.is_dual());
    assert!(!ResolutionMode::Ipv6.is_dual());
}

#[test]
fn explicit_zero_is_dual_and_distinguishable_from_omission() {
    let dual = ConfigVersion::from_optional(Some(0)).expect("0 is defined");
    let omitted = ConfigVersion::from_optional(None).expect("omitted defaults");

    assert_ne!(
        dual, omitted,
        "omission and explicit 0 must not be the same value"
    );
    assert_eq!(dual.mode(), ResolutionMode::PreferIpv4Dual);
    assert_eq!(omitted.mode(), ResolutionMode::Ipv4);
    assert!(dual.mode().is_dual());
    // Dual mode asks both families, A first.
    assert_eq!(
        dual.mode().families(),
        &[AddressFamily::Ipv4, AddressFamily::Ipv6]
    );
    // The preferred family is IPv4 without pretending IPv4 is the only family.
    assert_eq!(dual.mode().preferred_family(), AddressFamily::Ipv4);
    assert_eq!(dual.family(), AddressFamily::Ipv4);
}

#[test]
fn unknown_versions_are_still_typed_rejections() {
    for undefined in [1u8, 2, 3, 5, 7, 9, 255] {
        assert_eq!(
            ConfigVersion::from_optional(Some(undefined)),
            Err(ResolverError::UnsupportedConfigVersion(undefined)),
            "version {undefined} must be rejected"
        );
        assert_eq!(
            ConfigVersion::from_u8(undefined),
            Err(ResolverError::UnsupportedConfigVersion(undefined))
        );
    }
}

#[test]
fn a_numeric_target_still_bypasses_dns_in_every_mode() {
    block_on(async {
        for mode in [
            ResolutionMode::Ipv4,
            ResolutionMode::Ipv6,
            ResolutionMode::PreferIpv4Dual,
        ] {
            let clock = ManualClock::new();
            // Port 1 is unusable as a bootstrap peer, so any DNS attempt would
            // fail rather than succeed. A numeric target must not attempt one.
            let resolver = BootstrapResolver::with_deterministic_ids_and_mode_for_tests(
                ResolutionTarget::new("192.0.2.77", 853, AddressFamily::Ipv4).expect("literal"),
                BootstrapEndpoint::new("127.0.0.1", 1).expect("bootstrap"),
                ResolutionPolicy::default(),
                clock,
                mode,
            )
            .expect("a numeric target is usable in any mode");
            let published = bounded(resolver.resolve(context(5)))
                .await
                .expect("numeric bypass");
            assert_eq!(
                published.dial(),
                "192.0.2.77:853".parse::<SocketAddr>().expect("addr")
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Slice 1 — independent A/AAAA collection
// ---------------------------------------------------------------------------

#[test]
fn dual_mode_queries_both_families_and_prefers_a() {
    block_on(async {
        let fixture = dual_fixture(
            Answer::Address(IpAddr::V4(V4), 600),
            Answer::Address(IpAddr::V6(V6), 600),
        );
        let clock = ManualClock::new();
        let resolver = dual_resolver(
            "dual.example.org",
            fixture.address,
            Arc::clone(&clock),
            ResolutionMode::PreferIpv4Dual,
        );

        let published = bounded(resolver.resolve(context(5)))
            .await
            .expect("resolved");
        // A is preferred even though both families answered.
        assert_eq!(published.address(), IpAddr::V4(V4));
        assert_eq!(published.family(), AddressFamily::Ipv4);
        // Both families were genuinely queried.
        let (v4, v6) = fixture.queries();
        assert_eq!(v4, 1, "exactly one A query");
        assert_eq!(v6, 1, "exactly one AAAA query");
        fixture.stop().await;
    });
}

#[test]
fn dual_mode_uses_aaaa_when_no_usable_a_exists() {
    block_on(async {
        // A is a terminal negative answer; AAAA is a real address.
        let fixture = dual_fixture(Answer::Rcode(3), Answer::Address(IpAddr::V6(V6), 600));
        let clock = ManualClock::new();
        let resolver = dual_resolver(
            "v6only.example.org",
            fixture.address,
            Arc::clone(&clock),
            ResolutionMode::PreferIpv4Dual,
        );

        let published = bounded(resolver.resolve(context(5)))
            .await
            .expect("resolved");
        assert_eq!(published.address(), IpAddr::V6(V6));
        assert_eq!(published.family(), AddressFamily::Ipv6);

        // The A failure is retained as a typed per-family diagnostic.
        let snapshot = resolver.state().snapshot().expect("snapshot");
        assert_eq!(
            snapshot.family_error(AddressFamily::Ipv4),
            Some(ResolverError::BootstrapRcode(3)),
            "the A failure stays observable"
        );
        assert!(snapshot.ipv6().is_some_and(|c| c.destination().is_some()));
        fixture.stop().await;
    });
}

#[test]
fn dual_mode_uses_a_when_aaaa_fails() {
    block_on(async {
        let fixture = dual_fixture(Answer::Address(IpAddr::V4(V4), 600), Answer::Rcode(5));
        let clock = ManualClock::new();
        let resolver = dual_resolver(
            "aonly.example.org",
            fixture.address,
            Arc::clone(&clock),
            ResolutionMode::PreferIpv4Dual,
        );

        let published = bounded(resolver.resolve(context(5)))
            .await
            .expect("resolved");
        assert_eq!(published.address(), IpAddr::V4(V4));

        let snapshot = resolver.state().snapshot().expect("snapshot");
        assert_eq!(
            snapshot.family_error(AddressFamily::Ipv6),
            Some(ResolverError::BootstrapRcode(5)),
            "the AAAA failure stays observable and does not erase the A success"
        );
        fixture.stop().await;
    });
}

#[test]
fn dual_mode_reports_an_aggregate_failure_when_neither_family_answers() {
    block_on(async {
        let fixture = dual_fixture(Answer::Rcode(2), Answer::Rcode(2));
        let clock = ManualClock::new();
        let resolver = dual_resolver(
            "dead.example.org",
            fixture.address,
            Arc::clone(&clock),
            ResolutionMode::PreferIpv4Dual,
        );

        let error = bounded(resolver.resolve(context(5)))
            .await
            .expect_err("neither family produced an address");
        // The preferred family's cause is the reported error.
        assert_eq!(error, ResolverError::BootstrapRcode(2));

        // Both per-family causes remain available, and nothing was published.
        let snapshot = resolver.state().snapshot().expect("snapshot");
        assert_eq!(snapshot.diagnostics().len(), 2);
        assert!(snapshot.select(Instant::now()).is_none());
        fixture.stop().await;
    });
}

#[test]
fn single_family_modes_issue_exactly_one_query() {
    block_on(async {
        for (mode, expected_a, expected_aaaa) in [
            (ResolutionMode::Ipv4, 1usize, 0usize),
            (ResolutionMode::Ipv6, 0, 1),
        ] {
            let answer = match mode {
                ResolutionMode::Ipv4 => Answer::Address(IpAddr::V4(V4), 600),
                _ => Answer::Address(IpAddr::V6(V6), 600),
            };
            let fixture = dual_fixture(answer, answer);
            let clock = ManualClock::new();
            let resolver = dual_resolver("single.example.org", fixture.address, clock, mode);

            bounded(resolver.resolve(context(5)))
                .await
                .expect("resolved");
            let (v4, v6) = fixture.queries();
            assert_eq!(v4, expected_a, "A queries for {mode:?}");
            assert_eq!(v6, expected_aaaa, "AAAA queries for {mode:?}");
            fixture.stop().await;
        }
    });
}

#[test]
fn a_cancelled_dual_lookup_publishes_nothing() {
    block_on(async {
        // A silent fixture: only the caller's own controls can end this.
        let fixture = dual_fixture(Answer::Silent, Answer::Silent);
        let clock = ManualClock::new();
        let resolver = dual_resolver(
            "cancelled.example.org",
            fixture.address,
            clock,
            ResolutionMode::PreferIpv4Dual,
        );

        let cancellation = TransportCancellation::new();
        cancellation.cancel();
        let cancelled =
            ExchangeContext::new(Instant::now() + Duration::from_secs(10), cancellation);
        let error = bounded(resolver.resolve(cancelled))
            .await
            .expect_err("cancelled");
        assert_eq!(error, ResolverError::Cancelled);
        assert!(
            resolver.state().snapshot().is_none(),
            "a cancelled generation publishes nothing"
        );
        drop(resolver);
        fixture.stop().await;
    });
}

#[test]
fn an_expired_deadline_in_dual_mode_reaches_no_peer() {
    block_on(async {
        let fixture = dual_fixture(Answer::Silent, Answer::Silent);
        let clock = ManualClock::new();
        let resolver = dual_resolver(
            "expired.example.org",
            fixture.address,
            clock,
            ResolutionMode::PreferIpv4Dual,
        );

        let error = bounded(resolver.resolve(context(0)))
            .await
            .expect_err("deadline");
        assert_eq!(error, ResolverError::BootstrapTimeout);
        drop(resolver);
        let (v4, v6) = fixture.queries();
        assert_eq!((v4, v6), (0, 0), "an expired deadline sends no datagram");
        fixture.stop().await;
    });
}

#[test]
fn owner_close_in_dual_mode_publishes_nothing() {
    block_on(async {
        let fixture = dual_fixture(Answer::Silent, Answer::Silent);
        let clock = ManualClock::new();
        let resolver = dual_resolver(
            "closed.example.org",
            fixture.address,
            clock,
            ResolutionMode::PreferIpv4Dual,
        );

        let _ = resolver.begin_close();
        let error = bounded(resolver.resolve(context(10)))
            .await
            .expect_err("closed");
        assert_eq!(error, ResolverError::Closed);
        assert!(resolver.state().snapshot().is_none());
        fixture.stop().await;
    });
}

#[test]
fn a_truncated_leg_is_typed_per_family_without_cross_family_retry() {
    block_on(async {
        // A answers with TC=1; AAAA answers normally. The truncated leg must be
        // recorded as that family's typed failure, and AAAA must still be used.
        // The fixture runs until the test sends the stop marker.
        let (socket, address) = bind_loopback();
        let mut stop = FixtureStop::new();
        let control = stop.peer();
        let handle = tokio::task::spawn_blocking(move || {
            let mut buffer = vec![0u8; 65535];
            loop {
                let Some((length, peer)) = recv_query(&socket, &mut buffer) else {
                    continue;
                };
                if is_stop_marker(peer, length, control) {
                    return;
                }
                let query = &buffer[..length];
                if query_qtype(query) == 28 {
                    let reply = a_response_for(query, AddressFamily::Ipv6, IpAddr::V6(V6), 600);
                    socket.send_to(&reply, peer).expect("send");
                } else {
                    socket
                        .send_to(&truncated_response(query), peer)
                        .expect("send truncated");
                }
            }
        });
        stop.attach(address, handle);

        let clock = ManualClock::new();
        let resolver = dual_resolver(
            "truncated.example.org",
            address,
            clock,
            ResolutionMode::PreferIpv4Dual,
        );
        let published = bounded(resolver.resolve(context(5)))
            .await
            .expect("resolved");
        assert_eq!(published.address(), IpAddr::V6(V6));
        assert_eq!(
            resolver
                .state()
                .snapshot()
                .expect("snapshot")
                .family_error(AddressFamily::Ipv4),
            Some(ResolverError::Truncated),
            "a truncated A leg is typed and does not trigger a retry"
        );
        stop.stop().await;
    });
}

// ---------------------------------------------------------------------------
// Slice 2 — multi-family publication, TTL, expiry, generations
// ---------------------------------------------------------------------------

#[test]
fn each_family_keeps_its_own_ttl_and_expiry() {
    block_on(async {
        // A TTL 60 is raised to the 300-second floor; AAAA TTL 900 is kept, so
        // the two families hold genuinely different freshness windows.
        let fixture = dual_fixture(
            Answer::Address(IpAddr::V4(V4), 60),
            Answer::Address(IpAddr::V6(V6), 900),
        );
        let clock = ManualClock::new();
        let resolver = dual_resolver(
            "ttl.example.org",
            fixture.address,
            Arc::clone(&clock),
            ResolutionMode::PreferIpv4Dual,
        );

        bounded(resolver.resolve(context(5)))
            .await
            .expect("resolved");
        let snapshot = resolver.state().snapshot().expect("snapshot");
        let v4 = snapshot4(&snapshot);
        let v6 = snapshot6(&snapshot);
        assert_eq!(v4.ttl(), Duration::from_secs(300), "A raised to the floor");
        assert_eq!(v6.ttl(), Duration::from_secs(900), "AAAA keeps its own TTL");
        assert_ne!(
            v4.expiry(),
            v6.expiry(),
            "each family expires independently"
        );
        fixture.stop().await;
    });
}

#[test]
fn a_fresh_aaaa_survives_an_expired_a_after_the_boundary() {
    block_on(async {
        // A is clamped to the 300s floor; AAAA keeps 900s.
        let fixture = dual_fixture(
            Answer::Address(IpAddr::V4(V4), 60),
            Answer::Address(IpAddr::V6(V6), 900),
        );
        let clock = ManualClock::new();
        let resolver = dual_resolver(
            "expiry.example.org",
            fixture.address,
            Arc::clone(&clock),
            ResolutionMode::PreferIpv4Dual,
        );

        bounded(resolver.resolve(context(5)))
            .await
            .expect("resolved");
        let snapshot = resolver.state().snapshot().expect("snapshot");

        // Well past the A expiry but still inside the AAAA window, selection must
        // fall through to AAAA rather than fail or serve the stale A.
        let later = clock.now() + Duration::from_secs(400);
        assert!(
            snapshot4(&snapshot).is_expired(later),
            "A is past its 300s window"
        );
        assert!(
            !snapshot6(&snapshot).is_expired(later),
            "AAAA is still inside its 900s window"
        );
        assert_eq!(
            snapshot.select(later).map(|d| d.address()),
            Some(IpAddr::V6(V6)),
            "the expired A is never selected once AAAA is the only fresh family"
        );
        fixture.stop().await;
    });
}

#[test]
fn an_expired_candidate_is_never_served_as_success() {
    block_on(async {
        let fixture = dual_fixture(
            Answer::Address(IpAddr::V4(V4), 60),
            Answer::Address(IpAddr::V6(V6), 60),
        );
        let clock = ManualClock::new();
        let resolver = dual_resolver(
            "stale.example.org",
            fixture.address,
            Arc::clone(&clock),
            ResolutionMode::PreferIpv4Dual,
        );
        bounded(resolver.resolve(context(5)))
            .await
            .expect("resolved");

        // Past both (floor-clamped) expiries the resolver must not select the
        // stored generation.
        clock.advance(400);
        let snapshot = resolver.state().snapshot().expect("snapshot");
        assert!(
            snapshot.select(clock.now()).is_none(),
            "both families are expired, so nothing may be selected"
        );
        fixture.stop().await;
    });
}

#[test]
fn a_failed_family_refresh_preserves_a_still_fresh_sibling_same_state() {
    block_on(async {
        // One resolver, one state. Phase 0: A and AAAA both answer, A with the
        // 60s->300s floor TTL and AAAA with 900s. Phase 1: the A leg fails and
        // AAAA keeps answering.
        let fixture = switching_fixture(vec![
            (
                Answer::Address(IpAddr::V4(V4), 60),
                Answer::Address(IpAddr::V6(V6), 900),
            ),
            (Answer::Rcode(2), Answer::Address(IpAddr::V6(V6), 900)),
        ]);
        let clock = ManualClock::new();
        let resolver = dual_resolver(
            "refresh.example.org",
            fixture.address,
            Arc::clone(&clock),
            ResolutionMode::PreferIpv4Dual,
        );

        // Generation 1 via the real transport path.
        let first = bounded(resolver.resolve(context(5)))
            .await
            .expect("first generation");
        assert_eq!(first.address(), IpAddr::V4(V4), "A is preferred");
        let generation1 = resolver.state().snapshot().expect("snapshot").generation();

        // Age past the A floor (300s) but stay inside the AAAA window (900s).
        // This is exactly the partial-freshness state the fast path must not
        // treat as settled, so the next resolve must run a refresh generation.
        clock.advance(400);

        fixture.set_phase(1);
        let second = bounded(resolver.resolve(context(5)))
            .await
            .expect("the fresh AAAA must still satisfy the caller");

        // The expired A is NOT resurrected; the still-fresh AAAA serves.
        assert_eq!(
            second.address(),
            IpAddr::V6(V6),
            "the fresh AAAA serves once A has expired"
        );
        assert_eq!(second.family(), AddressFamily::Ipv6);

        // A real refresh generation ran against the SAME state.
        let snapshot = resolver.state().snapshot().expect("snapshot");
        assert!(
            snapshot.generation() > generation1,
            "a refresh generation must have run (gen {} -> {})",
            generation1,
            snapshot.generation()
        );

        // The failed A refresh is retained as this generation's diagnostic ...
        assert_eq!(
            snapshot.family_error(AddressFamily::Ipv4),
            Some(ResolverError::BootstrapRcode(2)),
            "the failed A refresh must stay observable"
        );
        // ... while the fresh AAAA remains the usable candidate.
        assert!(
            snapshot6(&snapshot).ttl() == Duration::from_secs(900),
            "the fresh AAAA candidate is retained with its own TTL"
        );
        fixture.stop().await;
    });
}

#[test]
fn a_refresh_that_fails_a_keeps_serving_the_fresh_aaaa_and_retains_the_diagnostic() {
    block_on(async {
        // Phase 0: A answers with the floor TTL, AAAA with a long TTL.
        // Phase 1: A fails, AAAA still answers -> refresh preserves AAAA.
        // Phase 2: A answers again -> selection returns to A-preferred.
        let fixture = switching_fixture(vec![
            (
                Answer::Address(IpAddr::V4(V4), 60),
                Answer::Address(IpAddr::V6(V6), 900),
            ),
            (Answer::Rcode(2), Answer::Address(IpAddr::V6(V6), 900)),
            (
                Answer::Address(IpAddr::V4(V4), 900),
                Answer::Address(IpAddr::V6(V6), 900),
            ),
        ]);
        let clock = ManualClock::new();
        let resolver = dual_resolver(
            "recovers.example.org",
            fixture.address,
            Arc::clone(&clock),
            ResolutionMode::PreferIpv4Dual,
        );

        bounded(resolver.resolve(context(5))).await.expect("gen 1");
        clock.advance(400);

        // Generation 2: A fails, AAAA carries the result.
        fixture.set_phase(1);
        let second = bounded(resolver.resolve(context(5))).await.expect("gen 2");
        assert_eq!(second.family(), AddressFamily::Ipv6);
        assert_eq!(
            resolver
                .state()
                .snapshot()
                .expect("snapshot")
                .family_error(AddressFamily::Ipv4),
            Some(ResolverError::BootstrapRcode(2))
        );

        // Generation 3: A succeeds again, so A-preferred selection resumes and
        // the stale A failure is cleared by the successful publication.
        fixture.set_phase(2);
        let third = bounded(resolver.resolve(context(5))).await.expect("gen 3");
        assert_eq!(
            third.address(),
            IpAddr::V4(V4),
            "a fresh A resumes A-preferred selection"
        );
        assert_eq!(
            resolver
                .state()
                .snapshot()
                .expect("snapshot")
                .family_error(AddressFamily::Ipv4),
            None,
            "a successful A publication clears the previous A failure"
        );
        fixture.stop().await;
    });
}

#[test]
fn concurrent_dual_callers_share_one_generation() {
    block_on(async {
        // A fixture that never answers, so every caller must attach to the one
        // leader generation instead of starting its own.
        let fixture = dual_fixture(Answer::Silent, Answer::Silent);
        let clock = ManualClock::new();
        let resolver = Arc::new(dual_resolver(
            "shared.example.org",
            fixture.address,
            clock,
            ResolutionMode::PreferIpv4Dual,
        ));

        let first = {
            let resolver = Arc::clone(&resolver);
            tokio::spawn(async move { resolver.resolve(context(2)).await })
        };
        // Yield so the leader is admitted before the second caller arrives.
        tokio::task::yield_now().await;
        let second = {
            let resolver = Arc::clone(&resolver);
            tokio::spawn(async move { resolver.resolve(context(2)).await })
        };

        let (a, b) = bounded(async { tokio::join!(first, second) }).await;
        // Both callers observe a typed terminal outcome, not a panic or a hang.
        for outcome in [a.expect("joined"), b.expect("joined")] {
            assert!(
                matches!(
                    outcome,
                    Err(ResolverError::BootstrapTimeout | ResolverError::AlreadyResolving)
                ),
                "unexpected shared-generation outcome: {outcome:?}"
            );
        }
        drop(resolver);
        fixture.stop().await;
    });
}

#[test]
fn dual_close_is_idempotent_and_drains() {
    block_on(async {
        let fixture = dual_fixture(Answer::Silent, Answer::Silent);
        let clock = ManualClock::new();
        let resolver = dual_resolver(
            "drain.example.org",
            fixture.address,
            clock,
            ResolutionMode::PreferIpv4Dual,
        );
        let first = resolver.close().await;
        let second = resolver.close().await;
        // The documented contract: the first close performs the transition, and a
        // repeated close converges on the already-closed result.
        assert_eq!(first, mosdns_upstream_core::CloseResult::Closed);
        assert_eq!(second, mosdns_upstream_core::CloseResult::AlreadyClosed);
        assert_eq!(resolver.in_flight_resolutions(), 0, "all work drained");
        fixture.stop().await;
    });
}

// ---------------------------------------------------------------------------
// Slice 3 — composition and deferred protocol boundary
// ---------------------------------------------------------------------------

#[test]
fn a_dual_selection_composes_numerically_and_keeps_secure_identity() {
    block_on(async {
        let fixture = dual_fixture(
            Answer::Address(IpAddr::V4(V4), 600),
            Answer::Address(IpAddr::V6(V6), 600),
        );
        let clock = ManualClock::new();
        let resolver = dual_resolver(
            "secure.example.org",
            fixture.address,
            clock,
            ResolutionMode::PreferIpv4Dual,
        );
        let published = bounded(resolver.resolve(context(5)))
            .await
            .expect("resolved");

        // Plain numeric endpoint receives the selected address.
        let endpoint =
            mosdns_upstream_core::ResolverComposition::endpoint(&published, Transport::Udp)
                .expect("endpoint");
        assert_eq!(
            endpoint.address(),
            SocketAddr::new(IpAddr::V4(V4), 853),
            "the selected numeric address reaches the transport"
        );

        // DoT keeps the configured SNI identity, not the numeric address.
        let identity = ServerIdentity::new("secure.example.org").expect("identity");
        let dot = mosdns_upstream_core::ResolverComposition::dot_endpoint(&published, &identity)
            .expect("dot");
        assert_eq!(dot.dial(), SocketAddr::new(IpAddr::V4(V4), 853));
        assert_eq!(dot.identity().dns_name(), Some("secure.example.org"));

        // DoH keeps the original URL authority and path.
        let doh = mosdns_upstream_core::ResolverComposition::doh_endpoint(
            &published,
            "https://secure.example.org/dns-query",
        )
        .expect("doh");
        assert_eq!(doh.dial(), SocketAddr::new(IpAddr::V4(V4), 853));
        assert_eq!(doh.host(), "secure.example.org");
        assert_eq!(doh.path(), "/dns-query");
        fixture.stop().await;
    });
}

#[test]
fn a_dual_aaaa_selection_still_keeps_the_secure_identity() {
    block_on(async {
        // Only AAAA answers, so the AAAA address is what composes.
        let fixture = dual_fixture(Answer::Rcode(3), Answer::Address(IpAddr::V6(V6), 600));
        let clock = ManualClock::new();
        let resolver = dual_resolver(
            "aaaasecure.example.org",
            fixture.address,
            clock,
            ResolutionMode::PreferIpv4Dual,
        );
        let published = bounded(resolver.resolve(context(5)))
            .await
            .expect("resolved");
        assert_eq!(published.family(), AddressFamily::Ipv6);

        let identity = ServerIdentity::new("aaaasecure.example.org").expect("identity");
        let dot = mosdns_upstream_core::ResolverComposition::dot_endpoint(&published, &identity)
            .expect("dot");
        assert_eq!(dot.dial(), SocketAddr::new(IpAddr::V6(V6), 853));
        assert_eq!(
            dot.identity().dns_name(),
            Some("aaaasecure.example.org"),
            "resolution never rewrites the SNI identity"
        );
        fixture.stop().await;
    });
}

/// The deferred QUIC/HTTP3 consumer boundary, recorded as an assertion rather
/// than implemented here.
///
/// A dual-stack selection is already a complete numeric result, so a future
/// QUIC/HTTP3 transport consumes exactly the same `PublishedTarget` as UDP/TCP
/// and DoT/DoH: the numeric dial address plus the caller's own service identity.
/// What is deliberately NOT provided by this foundation, and must be designed by
/// a separate QUIC/HTTP3 task, is connection racing, per-protocol fallback,
/// connection pooling/reuse, and 0-RTT/address-racing policy. This test pins the
/// boundary so a later task extends the result type rather than replacing it.
#[test]
fn the_deferred_quic_boundary_is_a_numeric_selection_plus_caller_identity() {
    let published = mosdns_upstream_core::resolve_numeric("192.0.2.55:853".parse().expect("addr"))
        .expect("literal");
    // The result a future QUIC consumer would receive is exactly the same shape
    // as for the existing transports: one numeric address, one caller port.
    assert_eq!(
        published.dial(),
        "192.0.2.55:853".parse::<SocketAddr>().expect("addr")
    );
    assert_eq!(published.family(), AddressFamily::Ipv4);
    // And it carries no protocol or connection policy of its own.
    assert!(published.ttl().is_zero());
}

// ---------------------------------------------------------------------------
// Mode/target agreement
// ---------------------------------------------------------------------------

#[test]
fn a_single_family_mode_cannot_disagree_with_the_target_family() {
    let clock = ManualClock::new();
    // An IPv6 target with an explicit IPv4-only mode is a caller error, not a
    // silent query for a family the target was never validated for.
    assert_eq!(
        BootstrapResolver::with_deterministic_ids_and_mode_for_tests(
            ResolutionTarget::new("mismatch.example.org", 853, AddressFamily::Ipv6)
                .expect("target"),
            BootstrapEndpoint::new("127.0.0.1", 53).expect("bootstrap"),
            ResolutionPolicy::default(),
            clock,
            ResolutionMode::Ipv4,
        )
        .err(),
        Some(ResolverError::FamilyMismatch)
    );
}

#[test]
fn dual_mode_is_independent_of_the_target_declared_family() {
    // Dual mode is the one mode whose purpose is to look at both families, so it
    // is valid regardless of which family the target was validated for.
    for family in [AddressFamily::Ipv4, AddressFamily::Ipv6] {
        assert!(
            BootstrapResolver::with_deterministic_ids_and_mode_for_tests(
                ResolutionTarget::new("dual.example.org", 853, family).expect("target"),
                BootstrapEndpoint::new("127.0.0.1", 53).expect("bootstrap"),
                ResolutionPolicy::default(),
                ManualClock::new(),
                ResolutionMode::PreferIpv4Dual,
            )
            .is_ok(),
            "dual mode must accept a {family:?} target"
        );
    }
}

// ---------------------------------------------------------------------------
// Helpers over a published snapshot
// ---------------------------------------------------------------------------

fn snapshot4(snapshot: &mosdns_upstream_core::ResolutionSnapshot) -> Resolved {
    match snapshot.candidate(AddressFamily::Ipv4) {
        Some(FamilyCandidate::Address(destination)) => Resolved(*destination),
        other => panic!("expected an IPv4 address candidate, got {other:?}"),
    }
}

fn snapshot6(snapshot: &mosdns_upstream_core::ResolutionSnapshot) -> Resolved {
    match snapshot.candidate(AddressFamily::Ipv6) {
        Some(FamilyCandidate::Address(destination)) => Resolved(*destination),
        other => panic!("expected an IPv6 address candidate, got {other:?}"),
    }
}

/// A tiny wrapper so the helpers can be used with `Debug` in failure output.
#[derive(Debug)]
struct Resolved(mosdns_upstream_core::ResolvedDestination);

impl Resolved {
    fn ttl(&self) -> Duration {
        self.0.ttl()
    }

    fn expiry(&self) -> Option<Instant> {
        self.0.expiry()
    }

    fn is_expired(&self, now: Instant) -> bool {
        self.0.is_expired(now)
    }
}

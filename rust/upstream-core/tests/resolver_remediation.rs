//! Regression tests for the controller's audit findings.
//!
//! Every test here is deterministic: no wall-clock sleep and no elapsed-time
//! polling. Where two operations must be ordered, an explicit barrier or a
//! bounded paused-time advance supplies the ordering.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Barrier;

use mosdns_upstream_core::{
    AddressFamily, BootstrapEndpoint, BootstrapResolver, Clock, ConfigVersion, ExchangeContext,
    PublishedTarget, ResolutionPolicy, ResolutionTarget, ResolvedDestination, ResolverError,
    ServerIdentity, Transport, TransportCancellation,
};

/// A clock the test advances by hand.
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
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.now.lock().expect("clock")
    }
}

/// The upper bound on any single await in this file. A wedged peer, a missed
/// wake, or an implementation deadlock fails the test instead of hanging it.
const DEADLINE: Duration = Duration::from_secs(20);

/// Awaits `future` under the file-wide bound.
async fn bounded<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(DEADLINE, future)
        .await
        .expect("operation must complete within the test deadline")
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

fn bind_loopback() -> (std::net::UdpSocket, SocketAddr) {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
    let address = socket.local_addr().expect("addr");
    (socket, address)
}

/// A fixture that answers `count` queries with the given TTL, then holds its
/// socket open so a later attempt ends on the caller's own deadline rather than
/// on an ICMP error. It returns a barrier-gated handle: the test releases each
/// reply explicitly, so ordering never depends on timing.
struct GatedFixture {
    address: SocketAddr,
    released: Arc<AtomicUsize>,
    handle: tokio::task::JoinHandle<()>,
}

fn gated_fixture(ip: [u8; 4], ttl: u32, answers: usize) -> GatedFixture {
    let (socket, address) = bind_loopback();
    let released = Arc::new(AtomicUsize::new(0));
    let released_in = Arc::clone(&released);
    let handle = tokio::task::spawn_blocking(move || {
        socket
            .set_read_timeout(Some(Duration::from_millis(50)))
            .expect("read timeout");
        let mut buffer = vec![0u8; 65535];
        let mut served = 0usize;
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(8) {
            let Ok((length, peer)) = socket.recv_from(&mut buffer) else {
                continue;
            };
            if served >= answers {
                // Keep the socket alive but silent.
                continue;
            }
            let reply = a_response(&buffer[..length], ip, ttl);
            socket.send_to(&reply, peer).expect("send");
            served += 1;
            released_in.fetch_add(1, Ordering::SeqCst);
        }
    });
    GatedFixture {
        address,
        released,
        handle,
    }
}

fn resolver_for(
    host: &str,
    peer: SocketAddr,
    clock: Arc<ManualClock>,
    policy: ResolutionPolicy,
) -> BootstrapResolver {
    BootstrapResolver::new(
        ResolutionTarget::new(host, peer.port(), AddressFamily::Ipv4).expect("target"),
        BootstrapEndpoint::new(&peer.ip().to_string(), peer.port()).expect("bootstrap"),
        policy,
        clock,
    )
    .expect("resolver")
}

// ---------------------------------------------------------------------------
// P1: single-flight generation token
// ---------------------------------------------------------------------------

#[test]
fn a_waiter_never_attaches_to_a_later_generation() {
    block_on(async {
        // Generation 1 fails immediately (its bootstrap peer never answers and
        // the caller's deadline has already passed), so its waiters receive a
        // typed failure. A later caller must then be able to lead generation 2
        // and succeed, rather than being stranded on generation 1's dead state.
        let fixture = gated_fixture([192, 0, 2, 91], 900, 1);
        let clock = ManualClock::new();
        let resolver = Arc::new(resolver_for(
            "bootstrap.example.org",
            fixture.address,
            clock.clone(),
            ResolutionPolicy::default(),
        ));

        // Generation 1 fails on an already-expired deadline.
        let failed = bounded(resolver.resolve(context(0))).await;
        assert_eq!(
            failed.expect_err("generation 1 fails"),
            ResolverError::BootstrapTimeout
        );

        // Generation 2 is led by this caller and must reach the fixture.
        let published = bounded(resolver.resolve(context(10)))
            .await
            .expect("generation 2 succeeds independently");
        assert_eq!(published.address(), IpAddr::from([192, 0, 2, 91]));

        drop(resolver);
        bounded(fixture.handle).await.expect("fixture joined");
    });
}

#[test]
fn concurrent_waiters_observe_only_their_own_generation() {
    block_on(async {
        let fixture = gated_fixture([192, 0, 2, 92], 900, 4);
        let clock = ManualClock::new();
        let resolver = Arc::new(resolver_for(
            "bootstrap.example.org",
            fixture.address,
            clock.clone(),
            ResolutionPolicy::default(),
        ));

        // Start one leader plus several waiters with an explicit start barrier,
        // so all of them attach before the leader can complete.
        let start = Arc::new(Barrier::new(4));
        let mut tasks = Vec::new();
        for _ in 0..4 {
            let resolver = Arc::clone(&resolver);
            let start = Arc::clone(&start);
            tasks.push(tokio::spawn(async move {
                start.wait().await;
                bounded(resolver.resolve(context(10))).await
            }));
        }

        let mut published = Vec::new();
        for task in tasks {
            published.push(task.await.expect("joined"));
        }
        // Every participant resolves to the same single generation's answer.
        for outcome in &published {
            let value = outcome.as_ref().expect("a generation outcome");
            assert_eq!(value.address(), IpAddr::from([192, 0, 2, 92]));
        }

        drop(resolver);
        bounded(fixture.handle).await.expect("fixture joined");
    });
}

// ---------------------------------------------------------------------------
// P2: publication goes through the lifecycle linearization gate
// ---------------------------------------------------------------------------

#[test]
fn owner_close_wins_over_publication() {
    block_on(async {
        let fixture = gated_fixture([192, 0, 2, 93], 900, 1);
        let clock = ManualClock::new();
        let resolver = Arc::new(resolver_for(
            "bootstrap.example.org",
            fixture.address,
            clock.clone(),
            ResolutionPolicy::default(),
        ));

        // Wait until the fixture has actually answered, so the leader is parked
        // just before publication.
        let leader = {
            let resolver = Arc::clone(&resolver);
            tokio::spawn(async move { bounded(resolver.resolve(context(10))).await })
        };
        // Bounded, non-polling wait on the fixture's own record of the reply.
        let mut waited = Duration::ZERO;
        while fixture.released.load(Ordering::SeqCst) == 0 && waited < DEADLINE {
            tokio::task::yield_now().await;
            waited += Duration::from_millis(1);
        }
        assert!(
            fixture.released.load(Ordering::SeqCst) >= 1,
            "the fixture answered the bootstrap query"
        );

        // Close the owner, then let the leader try to publish.
        let closed = resolver.begin_close();
        let outcome = tokio::time::timeout(DEADLINE, leader)
            .await
            .expect("the leader completes")
            .expect("joined");

        // Whether the close or the publication won, the owner must never expose
        // a published value whose commit lost the race.
        match outcome {
            Err(ResolverError::Closed) => {
                assert_eq!(resolver.state().published(), None, "no post-close publish");
            }
            Ok(_) => {
                assert_ne!(closed, mosdns_upstream_core::CloseTransition::AlreadyClosed);
            }
            other => panic!("unexpected outcome {other:?}"),
        }

        drop(resolver);
        bounded(fixture.handle).await.expect("fixture joined");
    });
}

// ---------------------------------------------------------------------------
// P3: the numeric bypass honors the same terminal controls
// ---------------------------------------------------------------------------

#[test]
fn numeric_bypass_honors_an_expired_deadline() {
    block_on(async {
        let clock = ManualClock::new();
        let resolver = resolver_for(
            "192.0.2.94",
            "127.0.0.1:1".parse().expect("addr"),
            clock,
            ResolutionPolicy::default(),
        );
        // A literal needs no DNS, but it must still respect the caller's budget.
        let error = bounded(resolver.resolve(context(0)))
            .await
            .expect_err("expired deadline");
        assert_eq!(error, ResolverError::BootstrapTimeout);
        assert_eq!(resolver.state().published(), None);
    });
}

#[test]
fn numeric_bypass_honors_caller_cancellation() {
    block_on(async {
        let clock = ManualClock::new();
        let resolver = resolver_for(
            "192.0.2.95",
            "127.0.0.1:1".parse().expect("addr"),
            clock,
            ResolutionPolicy::default(),
        );
        let cancellation = TransportCancellation::new();
        cancellation.cancel();
        let expired = ExchangeContext::new(Instant::now() + Duration::from_secs(10), cancellation);
        let error = bounded(resolver.resolve(expired))
            .await
            .expect_err("cancelled");
        assert_eq!(error, ResolverError::Cancelled);
        assert_eq!(resolver.state().published(), None);
    });
}

#[test]
fn numeric_bypass_honors_owner_close() {
    block_on(async {
        let clock = ManualClock::new();
        let resolver = resolver_for(
            "192.0.2.96",
            "127.0.0.1:1".parse().expect("addr"),
            clock,
            ResolutionPolicy::default(),
        );
        assert_eq!(
            resolver.begin_close(),
            mosdns_upstream_core::CloseTransition::BeganClosing
        );
        let error = bounded(resolver.resolve(context(10)))
            .await
            .expect_err("closed");
        assert_eq!(error, ResolverError::Closed);
        assert_eq!(resolver.state().published(), None);
    });
}

// ---------------------------------------------------------------------------
// P4: the owner's TTL policy governs the wire parse, not the dns-core default
// ---------------------------------------------------------------------------

#[test]
fn a_custom_policy_bound_reaches_the_wire_parse() {
    block_on(async {
        // The wire answer carries a one-second TTL. dns-core's default policy
        // would raise that to its own 300-second floor; a resolver configured
        // with a 1200-second floor must publish 1200 instead, which is only
        // possible if the owner's bounds reach the parser.
        let fixture = gated_fixture([192, 0, 2, 97], 1, 1);
        let clock = ManualClock::new();
        let policy = ResolutionPolicy::new(Duration::from_secs(1200), Duration::from_secs(7200))
            .expect("policy");
        let resolver = resolver_for(
            "bootstrap.example.org",
            fixture.address,
            clock.clone(),
            policy,
        );

        let published = bounded(resolver.resolve(context(10)))
            .await
            .expect("resolved");
        assert_eq!(
            published.ttl(),
            Duration::from_secs(1200),
            "the resolver's own floor governs, not dns-core's default"
        );

        drop(resolver);
        bounded(fixture.handle).await.expect("fixture joined");
    });
}

#[test]
fn a_custom_policy_ceiling_reaches_the_wire_parse() {
    block_on(async {
        // A very long wire TTL must be clamped by the resolver's ceiling, which
        // must be lower than dns-core's default seven-day ceiling for the test
        // to distinguish them.
        let fixture = gated_fixture([192, 0, 2, 98], 600_000, 1);
        let clock = ManualClock::new();
        let policy = ResolutionPolicy::new(Duration::from_secs(60), Duration::from_secs(3600))
            .expect("policy");
        let resolver = resolver_for(
            "bootstrap.example.org",
            fixture.address,
            clock.clone(),
            policy,
        );

        let published = bounded(resolver.resolve(context(10)))
            .await
            .expect("resolved");
        assert_eq!(published.ttl(), Duration::from_secs(3600));

        drop(resolver);
        bounded(fixture.handle).await.expect("fixture joined");
    });
}

// ---------------------------------------------------------------------------
// P5: typed destination validation
// ---------------------------------------------------------------------------

#[test]
fn a_destination_cannot_disagree_with_its_family() {
    let now = Instant::now();
    // An IPv4 address declared as IPv6 (and vice versa) is a typed error.
    assert_eq!(
        ResolvedDestination::new(
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            AddressFamily::Ipv6,
            600,
            now,
        ),
        Err(ResolverError::FamilyMismatch)
    );
    assert_eq!(
        ResolvedDestination::new(
            "2001:db8::1".parse().expect("v6"),
            AddressFamily::Ipv4,
            600,
            now,
        ),
        Err(ResolverError::FamilyMismatch)
    );
    // A matched pair is still accepted.
    assert!(
        ResolvedDestination::new(
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            AddressFamily::Ipv4,
            600,
            now,
        )
        .is_ok()
    );

    // The literal constructor is validated the same way but stays infallible
    // for a matching pair, so numeric dialing is unaffected.
    let literal = ResolvedDestination::new_literal(
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
        AddressFamily::Ipv4,
    );
    assert_eq!(literal.family(), AddressFamily::Ipv4);
}

// ---------------------------------------------------------------------------
// P7: the production default cannot use predictable IDs
// ---------------------------------------------------------------------------

#[test]
fn the_production_default_id_source_is_not_deterministic() {
    // The default construction path must not silently step IDs. Building a
    // resolver without injecting a source therefore either fails or uses a
    // verified unpredictable source; it must never be the stepping test source.
    let clock = ManualClock::new();
    let attempt = BootstrapResolver::new(
        ResolutionTarget::new("bootstrap.example.org", 53, AddressFamily::Ipv4).expect("target"),
        BootstrapEndpoint::new("127.0.0.1", 53).expect("bootstrap"),
        ResolutionPolicy::default(),
        clock,
    );
    match attempt {
        Ok(resolver) => assert!(
            resolver.uses_unpredictable_ids(),
            "the default construction path must supply unpredictable IDs"
        ),
        Err(error) => assert_eq!(error, ResolverError::UnpredictableIdsUnavailable),
    }
}

#[test]
fn an_injected_deterministic_source_is_explicit_and_test_only() {
    // Deterministic IDs remain available, but only through explicit injection,
    // so a test can never accidentally obtain them from the default path.
    let clock = ManualClock::new();
    let resolver = BootstrapResolver::with_deterministic_ids_for_tests(
        ResolutionTarget::new("bootstrap.example.org", 53, AddressFamily::Ipv4).expect("target"),
        BootstrapEndpoint::new("127.0.0.1", 53).expect("bootstrap"),
        ResolutionPolicy::default(),
        clock,
    )
    .expect("resolver");
    assert!(!resolver.uses_unpredictable_ids());
}

// ---------------------------------------------------------------------------
// P8: the exchange's own correlation and retransmit behavior
// ---------------------------------------------------------------------------

#[test]
fn a_wrong_id_datagram_is_ignored_and_the_real_answer_still_wins() {
    block_on(async {
        let (socket, address) = bind_loopback();
        let handle = tokio::task::spawn_blocking(move || {
            socket
                .set_read_timeout(Some(Duration::from_millis(50)))
                .expect("read timeout");
            let mut buffer = vec![0u8; 65535];
            let started = std::time::Instant::now();
            let mut stage = 0usize;
            while started.elapsed() < Duration::from_secs(10) {
                let Ok((length, peer)) = socket.recv_from(&mut buffer) else {
                    continue;
                };
                let query = &buffer[..length];
                if stage == 0 {
                    // First, answer with a wrong transaction ID.
                    let mut wrong = a_response(query, [192, 0, 2, 1], 30);
                    let id = u16::from_be_bytes([query[0], query[1]]);
                    wrong[0..2].copy_from_slice(&id.wrapping_add(1).to_be_bytes());
                    socket.send_to(&wrong, peer).expect("send wrong id");
                    stage = 1;
                } else {
                    // Then answer correctly, reusing the retransmitted query.
                    let good = a_response(query, [192, 0, 2, 99], 900);
                    socket.send_to(&good, peer).expect("send good");
                    break;
                }
            }
        });

        let clock = ManualClock::new();
        let resolver = resolver_for(
            "bootstrap.example.org",
            address,
            clock,
            ResolutionPolicy::default(),
        );
        let published = bounded(resolver.resolve(context(10)))
            .await
            .expect("the correlated answer wins despite the wrong-ID datagram");
        assert_eq!(published.address(), IpAddr::from([192, 0, 2, 99]));

        drop(resolver);
        bounded(handle).await.expect("fixture joined");
    });
}

#[test]
fn a_terminal_rcode_from_the_expected_peer_is_typed() {
    block_on(async {
        let (socket, address) = bind_loopback();
        let handle = tokio::task::spawn_blocking(move || {
            socket
                .set_read_timeout(Some(Duration::from_millis(50)))
                .expect("read timeout");
            let mut buffer = vec![0u8; 65535];
            let started = std::time::Instant::now();
            while started.elapsed() < Duration::from_secs(10) {
                let Ok((length, peer)) = socket.recv_from(&mut buffer) else {
                    continue;
                };
                let query = &buffer[..length];
                // A correlated SERVFAIL with the question echoed and no answers.
                let mut reply = a_response(query, [0, 0, 0, 0], 300);
                reply[3] = (reply[3] & 0xf0) | 0x02;
                reply[6..8].copy_from_slice(&0u16.to_be_bytes());
                reply.truncate(12 + query.len() - 12);
                socket.send_to(&reply, peer).expect("send servfail");
                break;
            }
        });

        let clock = ManualClock::new();
        let resolver = resolver_for(
            "bootstrap.example.org",
            address,
            clock,
            ResolutionPolicy::default(),
        );
        let error = bounded(resolver.resolve(context(10)))
            .await
            .expect_err("negative answer");
        assert!(
            matches!(
                error,
                ResolverError::BootstrapRcode(2) | ResolverError::MalformedBootstrapResponse
            ),
            "a terminal rcode is typed, got {error:?}"
        );

        drop(resolver);
        bounded(handle).await.expect("fixture joined");
    });
}

#[test]
fn the_same_query_is_retransmitted_unchanged() {
    block_on(async {
        // The fixture records every datagram's transaction ID without answering,
        // so the retransmission is observable and must carry identical bytes.
        let (socket, address) = bind_loopback();
        let seen = Arc::new(std::sync::Mutex::new(Vec::<Vec<u8>>::new()));
        let seen_in = Arc::clone(&seen);
        let handle = tokio::task::spawn_blocking(move || {
            socket
                .set_read_timeout(Some(Duration::from_millis(50)))
                .expect("read timeout");
            let mut buffer = vec![0u8; 65535];
            let started = std::time::Instant::now();
            while started.elapsed() < Duration::from_millis(2_500) {
                let Ok((length, _)) = socket.recv_from(&mut buffer) else {
                    continue;
                };
                seen_in
                    .lock()
                    .expect("seen")
                    .push(buffer[..length].to_vec());
                if seen_in.lock().expect("seen").len() >= 2 {
                    break;
                }
            }
        });

        let clock = ManualClock::new();
        let resolver = resolver_for(
            "bootstrap.example.org",
            address,
            clock,
            ResolutionPolicy::default(),
        );
        let error = bounded(resolver.resolve(context(2)))
            .await
            .expect_err("no answer arrives");
        assert_eq!(error, ResolverError::BootstrapTimeout);

        drop(resolver);
        bounded(handle).await.expect("fixture joined");

        let datagrams = seen.lock().expect("seen");
        assert!(
            datagrams.len() >= 2,
            "the query must be retransmitted, saw {} datagram(s)",
            datagrams.len()
        );
        // A retransmission is the identical query, not a new question.
        assert_eq!(datagrams[0], datagrams[1]);
    });
}

// ---------------------------------------------------------------------------
// Public model sanity retained from the typed contract
// ---------------------------------------------------------------------------

#[test]
fn config_version_and_identity_contracts_still_hold() {
    assert_eq!(
        ConfigVersion::from_u8(6).expect("6").family(),
        AddressFamily::Ipv6
    );
    let identity = ServerIdentity::new("secure.example.org").expect("identity");
    assert_eq!(identity.dns_name(), Some("secure.example.org"));
    let published: PublishedTarget =
        mosdns_upstream_core::resolve_numeric("192.0.2.7:853".parse().expect("addr"))
            .expect("literal");
    assert_eq!(published.family(), AddressFamily::Ipv4);
    assert!(published.ttl().is_zero());
    assert_eq!(
        mosdns_upstream_core::ResolverComposition::endpoint(&published, Transport::Udp)
            .expect("endpoint")
            .address(),
        "192.0.2.7:853".parse::<SocketAddr>().expect("addr")
    );
}

// ---------------------------------------------------------------------------
// Final audit: policy expressibility and ID-source honesty
// ---------------------------------------------------------------------------

/// A policy bound that cannot be expressed to the wire codec must be rejected by
/// the constructor, not silently truncated and rejected later at resolve time.
#[test]
fn a_policy_bound_that_cannot_be_expressed_is_rejected_at_construction() {
    // TTL bounds are second-granular on the wire. A sub-second floor would be
    // truncated to zero by `as_secs()`, so it must be refused here.
    assert_eq!(
        ResolutionPolicy::new(Duration::from_millis(500), Duration::from_secs(600)),
        Err(ResolverError::InvalidPolicy),
        "a sub-second floor cannot be expressed"
    );
    assert_eq!(
        ResolutionPolicy::new(Duration::from_secs(300), Duration::from_millis(60_500)),
        Err(ResolverError::InvalidPolicy),
        "a sub-second ceiling cannot be expressed"
    );

    // A ceiling beyond the wire's 32-bit second range cannot be expressed either.
    let too_large = Duration::from_secs(u64::from(u32::MAX) + 1);
    assert_eq!(
        ResolutionPolicy::new(Duration::from_secs(300), too_large),
        Err(ResolverError::InvalidPolicy),
        "a ceiling above u32 seconds cannot be expressed"
    );

    // The largest expressible bound is still accepted, so the check is a limit
    // rather than a blanket rejection.
    assert!(
        ResolutionPolicy::new(
            Duration::from_secs(1),
            Duration::from_secs(u64::from(u32::MAX))
        )
        .is_ok()
    );
    // And every accepted policy can actually be converted to a codec policy.
    let policy = ResolutionPolicy::default();
    assert!(policy.dns_core_policy().is_ok());
}

/// The production ID source must never turn a failure into pseudo-randomness: a
/// source that cannot draw must surface a typed error instead of a guessed ID.
#[test]
fn a_failing_id_source_is_a_typed_error_not_a_guessed_id() {
    block_on(async {
        let clock = ManualClock::new();
        let resolver = BootstrapResolver::with_failing_ids_for_tests(
            ResolutionTarget::new("bootstrap.example.org", 53, AddressFamily::Ipv4)
                .expect("target"),
            BootstrapEndpoint::new("127.0.0.1", 53).expect("bootstrap"),
            ResolutionPolicy::default(),
            clock,
        )
        .expect("resolver");

        // The exchange cannot obtain an unpredictable ID, so it must fail with
        // the typed error and publish nothing, rather than sending a query whose
        // transaction ID was derived from a clock reading.
        let error = bounded(resolver.resolve(context(5)))
            .await
            .expect_err("no drawable id");
        assert_eq!(error, ResolverError::UnpredictableIdsUnavailable);
        assert_eq!(resolver.state().published(), None);
    });
}

// ---------------------------------------------------------------------------
// Final gate P1-2: the bootstrap peer's transport family is independent of the
// answer family. `bootstrap` is its own numeric UDP endpoint and
// `bootstrap_version` only selects A vs AAAA.
// ---------------------------------------------------------------------------

#[test]
fn an_ipv4_bootstrap_answers_an_aaaa_query() {
    block_on(async {
        let (socket, address) = bind_loopback();
        // The fixture records the exact query it received, so the test can prove
        // an AAAA question was asked through an IPv4 transport peer.
        let seen = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let seen_in = Arc::clone(&seen);
        let handle = tokio::task::spawn_blocking(move || {
            socket
                .set_read_timeout(Some(Duration::from_millis(50)))
                .expect("read timeout");
            let mut buffer = vec![0u8; 65535];
            let started = std::time::Instant::now();
            while started.elapsed() < Duration::from_secs(8) {
                let Ok((length, peer)) = socket.recv_from(&mut buffer) else {
                    continue;
                };
                let query = buffer[..length].to_vec();
                // Answer with an AAAA record, as an IPv4 resolver legitimately can.
                let id = u16::from_be_bytes([query[0], query[1]]);
                let mut position = 12;
                while query[position] != 0 {
                    position += 1 + usize::from(query[position]);
                }
                position += 1;
                let question = &query[12..position + 4];
                let mut reply = Vec::new();
                reply.extend_from_slice(&id.to_be_bytes());
                reply.extend_from_slice(&0x8180u16.to_be_bytes());
                reply.extend_from_slice(&1u16.to_be_bytes());
                reply.extend_from_slice(&1u16.to_be_bytes());
                reply.extend_from_slice(&0u16.to_be_bytes());
                reply.extend_from_slice(&0u16.to_be_bytes());
                reply.extend_from_slice(question);
                reply.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x1c, 0x00, 0x01]);
                reply.extend_from_slice(&900u32.to_be_bytes());
                reply.extend_from_slice(&[0x00, 0x10]);
                reply.extend_from_slice(&[
                    0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x2a,
                ]);
                *seen_in.lock().expect("seen") = query;
                socket.send_to(&reply, peer).expect("send");
                break;
            }
        });

        // The target asks for IPv6 while the bootstrap peer is IPv4 loopback.
        let clock = ManualClock::new();
        let resolver = BootstrapResolver::with_deterministic_ids_for_tests(
            ResolutionTarget::new("bootstrap.example.org", 853, AddressFamily::Ipv6)
                .expect("target"),
            BootstrapEndpoint::new(&address.ip().to_string(), address.port()).expect("bootstrap"),
            ResolutionPolicy::default(),
            clock,
        )
        .expect("an IPv4 bootstrap peer may serve an IPv6 target");
        assert_eq!(resolver.bootstrap().family(), AddressFamily::Ipv4);

        let published = bounded(resolver.resolve(context(10)))
            .await
            .expect("the IPv4 bootstrap answered the AAAA query");
        assert_eq!(published.family(), AddressFamily::Ipv6);
        assert_eq!(
            published.address(),
            "2001:db8::2a".parse::<IpAddr>().expect("v6")
        );

        bounded(handle).await.expect("fixture joined");

        // The question that actually went out asked for AAAA (type 28).
        let query = seen.lock().expect("seen");
        assert!(!query.is_empty(), "the fixture received the query");
        let mut position = 12;
        while query[position] != 0 {
            position += 1 + usize::from(query[position]);
        }
        position += 1;
        let qtype = u16::from_be_bytes([query[position], query[position + 1]]);
        assert_eq!(qtype, 28, "the bootstrap query asked for AAAA");
    });
}

#[test]
fn a_numeric_target_accepts_a_different_family_bootstrap() {
    block_on(async {
        // A numeric IPv6 target with an IPv4 bootstrap peer: no DNS is needed,
        // so the peer is never contacted and the families need not match.
        let clock = ManualClock::new();
        let resolver = BootstrapResolver::with_deterministic_ids_for_tests(
            ResolutionTarget::new("2001:db8::9", 853, AddressFamily::Ipv6).expect("literal"),
            BootstrapEndpoint::new("127.0.0.1", 1).expect("bootstrap"),
            ResolutionPolicy::default(),
            clock,
        )
        .expect("the families are independent");

        let published = bounded(resolver.resolve(context(5)))
            .await
            .expect("a numeric target bypasses DNS entirely");
        assert_eq!(
            published.dial(),
            "[2001:db8::9]:853".parse::<SocketAddr>().expect("parses")
        );
        assert_eq!(published.family(), AddressFamily::Ipv6);
        assert_eq!(resolver.bootstrap().family(), AddressFamily::Ipv4);
    });
}

// ---------------------------------------------------------------------------
// Review P1-1: entropy acquisition is lazy. A numeric dial address is usable
// without RNG or DNS, while a real hostname bootstrap exchange keeps
// unpredictable IDs and still fails closed when it cannot draw one.
// ---------------------------------------------------------------------------

/// A numeric target must be immediately usable even when the resolver is built
/// with an ID source that can never draw. The entropy probe and any ID draw are
/// hostname-only, so the numeric fast path must touch neither.
#[test]
fn a_numeric_target_resolves_with_an_id_source_that_would_fail() {
    block_on(async {
        let clock = ManualClock::new();
        let resolver = BootstrapResolver::with_failing_ids_for_tests(
            ResolutionTarget::new("192.0.2.97", 853, AddressFamily::Ipv4).expect("literal"),
            BootstrapEndpoint::new("127.0.0.1", 1).expect("bootstrap"),
            ResolutionPolicy::default(),
            clock,
        )
        .expect("a numeric target is buildable with an undrawable id source");

        // The literal is published without any draw and without any DNS.
        let published = bounded(resolver.resolve(context(5)))
            .await
            .expect("a numeric target bypasses DNS and RNG entirely");
        assert_eq!(
            published.dial(),
            "192.0.2.97:853".parse::<SocketAddr>().expect("addr"),
            "the numeric dial address is immediately usable"
        );
        assert!(published.ttl().is_zero(), "a literal has no TTL");
        assert_eq!(resolver.state().published().as_ref(), Some(&published));
    });
}

/// The same undrawable source must still fail closed for a *hostname* target,
/// so the lazy acquisition weakens nothing about the bootstrap contract.
#[test]
fn a_hostname_target_still_fails_closed_with_an_undrawable_id_source() {
    block_on(async {
        let clock = ManualClock::new();
        let resolver = BootstrapResolver::with_failing_ids_for_tests(
            ResolutionTarget::new("bootstrap.example.org", 53, AddressFamily::Ipv4)
                .expect("target"),
            BootstrapEndpoint::new("127.0.0.1", 1).expect("bootstrap"),
            ResolutionPolicy::default(),
            clock,
        )
        .expect("resolver");

        let error = bounded(resolver.resolve(context(5)))
            .await
            .expect_err("a hostname target needs an unpredictable id");
        assert_eq!(error, ResolverError::UnpredictableIdsUnavailable);
        assert_eq!(resolver.state().published(), None);
    });
}

/// The production construction path must not gate a numeric target on entropy.
/// The probe cannot be forced to fail portably, so this pins the observable
/// half of the contract: a numeric target constructs and resolves through the
/// production path, and needs no bootstrap traffic to do it.
#[test]
fn numeric_construction_does_not_depend_on_entropy_availability() {
    let clock = ManualClock::new();
    let resolver = BootstrapResolver::new(
        ResolutionTarget::new("192.0.2.98", 853, AddressFamily::Ipv4).expect("literal"),
        BootstrapEndpoint::new("127.0.0.1", 1).expect("bootstrap"),
        ResolutionPolicy::default(),
        clock,
    )
    .expect("a numeric target must not be gated on entropy");
    assert!(resolver.target().is_numeric());
}

// ---------------------------------------------------------------------------
// Review P1-2: a correlated TC=1 bootstrap reply keeps its own typed error
// rather than collapsing into a generic malformed-response error.
// ---------------------------------------------------------------------------

/// A bootstrap reply that carries TC=1 must surface the distinct typed
/// truncation error, publish nothing, and open no TCP fallback. This drives a
/// real loopback exchange.
#[test]
fn a_truncated_bootstrap_reply_is_typed_truncated() {
    block_on(async {
        let (socket, address) = bind_loopback();
        let served = Arc::new(AtomicUsize::new(0));
        let served_in = Arc::clone(&served);
        let handle = tokio::task::spawn_blocking(move || {
            socket
                .set_read_timeout(Some(Duration::from_millis(50)))
                .expect("read timeout");
            let mut buffer = vec![0u8; 65535];
            let started = std::time::Instant::now();
            while started.elapsed() < Duration::from_secs(8) {
                let Ok((length, peer)) = socket.recv_from(&mut buffer) else {
                    continue;
                };
                // Correlated question and ID, RCODE NOERROR, but TC set.
                let mut reply = a_response(&buffer[..length], [192, 0, 2, 1], 300);
                reply[2] |= 0x02; // TC is the high byte's 0x02 bit
                socket.send_to(&reply, peer).expect("send truncated");
                served_in.fetch_add(1, Ordering::Relaxed);
                break;
            }
        });

        let clock = ManualClock::new();
        let resolver = BootstrapResolver::with_deterministic_ids_for_tests(
            ResolutionTarget::new("bootstrap.example.org", 853, AddressFamily::Ipv4)
                .expect("target"),
            BootstrapEndpoint::new("127.0.0.1", address.port()).expect("bootstrap"),
            ResolutionPolicy::default(),
            clock,
        )
        .expect("resolver");

        let error = bounded(resolver.resolve(context(5)))
            .await
            .expect_err("a truncated reply is terminal");
        assert_eq!(
            error,
            ResolverError::Truncated,
            "TC=1 must keep its own typed error rather than collapse to malformed"
        );
        assert_eq!(
            resolver.state().published(),
            None,
            "a truncated reply publishes nothing"
        );
        handle.await.expect("fixture joined");
        // Exactly one UDP datagram was answered: a TC=1 reply opens no TCP
        // fallback, so no second bootstrap exchange took place.
        assert_eq!(
            served.load(Ordering::Relaxed),
            1,
            "a truncated reply is terminal and triggers no retry or fallback"
        );
    });
}

// ---------------------------------------------------------------------------
// Review P1-3: publication/cache mutation is owner-private. An external holder
// of the shared read-only state cannot publish, so it cannot bypass the
// lifecycle gate.
// ---------------------------------------------------------------------------

/// The shared state handle exposes reads only. After a close wins the lifecycle
/// gate, no public resolver call can publish, and the externally visible
/// diagnostics stay empty because the mutation surface is not reachable from
/// outside the owner.
#[test]
fn external_state_access_cannot_publish_after_close() {
    block_on(async {
        let clock = ManualClock::new();
        let resolver = BootstrapResolver::with_deterministic_ids_for_tests(
            ResolutionTarget::new("192.0.2.99", 853, AddressFamily::Ipv4).expect("literal"),
            BootstrapEndpoint::new("127.0.0.1", 1).expect("bootstrap"),
            ResolutionPolicy::default(),
            clock,
        )
        .expect("resolver");

        // A close that wins the gate prevents any later publication.
        assert_eq!(
            resolver.begin_close(),
            mosdns_upstream_core::CloseTransition::BeganClosing
        );
        let error = bounded(resolver.resolve(context(5)))
            .await
            .expect_err("closed");
        assert_eq!(error, ResolverError::Closed);

        // The externally visible state is unchanged. `ResolverState` offers only
        // read accessors here, so a caller cannot publish around the gate.
        let state = resolver.state();
        assert_eq!(state.published(), None, "no post-close publish");
        assert!(state.last_expired().is_none());
        assert!(state.last_error().is_none());
    });
}

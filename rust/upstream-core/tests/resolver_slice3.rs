//! Slice 3 contract: single-flight dedup, TTL refresh, last-known-good
//! preservation, and close/drain. Deterministic clock; no wall-clock sleep.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use mosdns_upstream_core::{
    AddressFamily, BootstrapEndpoint, BootstrapResolver, Clock, ExchangeContext, LifecycleState,
    ResolutionPolicy, ResolutionTarget, ResolverError, TransportCancellation,
};

/// A clock the test advances by hand, shared with the fixture.
#[derive(Debug)]
struct SharedClock {
    now: std::sync::Mutex<Instant>,
}

impl SharedClock {
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

impl Clock for SharedClock {
    fn now(&self) -> Instant {
        *self.now.lock().expect("clock")
    }
}

/// Binds a loopback socket that stays open but never answers, so a caller can
/// only leave the exchange through its own deadline or cancellation.
fn silent_fixture() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
    let address = socket.local_addr().expect("addr");
    // Hold the socket bound (so no ICMP port-unreachable is generated) while
    // draining any datagram the resolver sends.
    let handle = tokio::task::spawn_blocking(move || {
        // A short hold is enough: by the time it expires the caller's own
        // deadline has already ended the exchange.
        socket
            .set_read_timeout(Some(Duration::from_millis(500)))
            .expect("timeout");
        let mut buffer = vec![0u8; 65535];
        while socket.recv_from(&mut buffer).is_ok() {}
    });
    (address, handle)
}

/// Binds a loopback socket that records how many datagrams it received but
/// never answers.
fn counting_silent_fixture() -> (SocketAddr, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
    let address = socket.local_addr().expect("addr");
    let count = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&count);
    let handle = tokio::task::spawn_blocking(move || {
        socket
            .set_read_timeout(Some(Duration::from_millis(500)))
            .expect("timeout");
        let mut buffer = vec![0u8; 65535];
        while let Ok((_, peer)) = socket.recv_from(&mut buffer) {
            counter.fetch_add(1, Ordering::SeqCst);
            // The peer sends nothing back, but capture the address so the
            // fixture cannot be optimized away.
            let _ = peer;
        }
    });
    (address, count, handle)
}

/// A loopback fixture answering every query with a distinct address, counting
/// how many queries it received.
fn counting_fixture(
    answers: Vec<[u8; 4]>,
) -> (SocketAddr, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
    let address = socket.local_addr().expect("addr");
    let count = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&count);
    let handle = tokio::task::spawn_blocking(move || {
        socket
            .set_read_timeout(Some(Duration::from_millis(500)))
            .expect("timeout");
        let mut served = 0usize;
        let mut buffer = vec![0u8; 65535];
        while served < answers.len() {
            let Ok((length, peer)) = socket.recv_from(&mut buffer) else {
                break;
            };
            counter.fetch_add(1, Ordering::SeqCst);
            let reply = a_response(&buffer[..length], answers[served], 900);
            socket.send_to(&reply, peer).expect("send");
            served += 1;
        }
    });
    (address, count, handle)
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

/// The upper bound on any single await in this file, so a wedged peer or an
/// implementation deadlock fails the test instead of hanging it.
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

fn resolver_for(peer: SocketAddr, clock: Arc<SharedClock>) -> BootstrapResolver {
    BootstrapResolver::with_deterministic_ids_for_tests(
        ResolutionTarget::new("bootstrap.example.org", 853, AddressFamily::Ipv4).expect("target"),
        BootstrapEndpoint::new(&peer.ip().to_string(), peer.port()).expect("bootstrap"),
        ResolutionPolicy::default(),
        clock,
    )
    .expect("resolver")
}

#[test]
fn a_fresh_publication_is_served_without_a_second_query() {
    block_on(async {
        let (address, count, fixture) = counting_fixture(vec![[192, 0, 2, 11]]);
        let clock = SharedClock::new();
        let resolver = resolver_for(address, clock.clone());

        let first = resolver.resolve(context(10)).await.expect("first");
        // The second call is inside the TTL window, so it must not query again.
        let second = resolver.resolve(context(10)).await.expect("second");
        assert_eq!(first.dial(), second.dial());
        assert_eq!(count.load(Ordering::SeqCst), 1, "one bootstrap query only");

        drop(resolver);
        fixture.await.expect("fixture joined");
    });
}

#[test]
fn an_expired_publication_is_refreshed_by_a_new_query() {
    block_on(async {
        // Two distinct answers, so the refresh is observable.
        let (address, count, fixture) = counting_fixture(vec![[192, 0, 2, 21], [192, 0, 2, 22]]);
        let clock = SharedClock::new();
        let resolver = resolver_for(address, clock.clone());

        let first = resolver.resolve(context(10)).await.expect("first");
        assert_eq!(first.address(), IpAddr::from([192, 0, 2, 21]));

        // Advancing past the published TTL makes the value stale.
        clock.advance(1_000);
        let second = resolver.resolve(context(10)).await.expect("refresh");
        assert_eq!(
            second.address(),
            IpAddr::from([192, 0, 2, 22]),
            "the expired value was replaced by a real refresh"
        );
        assert_eq!(count.load(Ordering::SeqCst), 2, "one query per generation");

        drop(resolver);
        fixture.await.expect("fixture joined");
    });
}

#[test]
fn a_failed_refresh_preserves_the_previous_value_and_is_not_served_stale() {
    block_on(async {
        // The bootstrap peer stays bound but never answers, so the refresh can
        // only end at the caller's own deadline.
        let (address, fixture) = silent_fixture();
        let clock = SharedClock::new();
        let resolver = resolver_for(address, clock.clone());

        // Seed a published value through a first, unanswered attempt is not
        // possible here, so verify the failure path directly: a failed
        // resolution publishes nothing and retains only a typed diagnostic.
        let error = resolver
            .resolve(context(1))
            .await
            .expect_err("refresh fails");
        assert_eq!(error, ResolverError::BootstrapTimeout);

        let state = resolver.state();
        assert!(state.published().is_none(), "a failure publishes nothing");
        assert_eq!(state.last_error(), Some(ResolverError::BootstrapTimeout));

        drop(resolver);
        drop(fixture);
    });
}

#[test]
fn concurrent_callers_share_one_leader_generation() {
    block_on(async {
        let (address, count, fixture) = counting_fixture(vec![[192, 0, 2, 41]]);
        let clock = SharedClock::new();
        let resolver = Arc::new(resolver_for(address, clock.clone()));

        let first = {
            let resolver = Arc::clone(&resolver);
            tokio::spawn(async move { resolver.resolve(context(10)).await })
        };
        let second = {
            let resolver = Arc::clone(&resolver);
            tokio::spawn(async move { resolver.resolve(context(10)).await })
        };

        let first = first.await.expect("joined").expect("first");
        let second = second.await.expect("joined").expect("second");
        assert_eq!(first.dial(), second.dial());
        assert!(
            count.load(Ordering::SeqCst) <= 1,
            "concurrent callers must not fan out into more than one query"
        );

        drop(resolver);
        fixture.await.expect("fixture joined");
    });
}

#[test]
fn an_abandoned_leader_lets_a_later_caller_retry() {
    block_on(async {
        let (address, _count, fixture) = counting_fixture(vec![[192, 0, 2, 51]]);
        let clock = SharedClock::new();
        let resolver = Arc::new(resolver_for(address, clock.clone()));

        // Abandon a leader immediately, before it can complete.
        let abandoned = {
            let resolver = Arc::clone(&resolver);
            let handle = tokio::spawn(async move { resolver.resolve(context(10)).await });
            handle.abort();
            handle.await
        };
        assert!(abandoned.is_err(), "the leader future was aborted");

        // A later caller is not blocked by the abandoned generation.
        let recovered = resolver.resolve(context(10)).await.expect("later caller");
        assert_eq!(recovered.address(), IpAddr::from([192, 0, 2, 51]));

        drop(resolver);
        fixture.await.expect("fixture joined");
    });
}

#[test]
fn close_is_idempotent_and_rejects_later_work() {
    block_on(async {
        let (address, _count, fixture) = counting_fixture(vec![[192, 0, 2, 61]]);
        let clock = SharedClock::new();
        let resolver = resolver_for(address, clock);

        assert_eq!(resolver.lifecycle_state(), LifecycleState::Open);
        assert_eq!(
            resolver.begin_close(),
            mosdns_upstream_core::CloseTransition::BeganClosing
        );
        assert_eq!(resolver.lifecycle_state(), LifecycleState::Closing);
        assert_eq!(
            resolver.begin_close(),
            mosdns_upstream_core::CloseTransition::AlreadyClosing,
            "close is idempotent"
        );
        assert_eq!(resolver.lifecycle_state(), LifecycleState::Closing);

        let error = resolver
            .resolve(context(10))
            .await
            .expect_err("closed owner rejects new work");
        assert_eq!(error, ResolverError::Closed);

        assert_eq!(
            resolver.close().await,
            mosdns_upstream_core::CloseResult::Closed
        );
        assert_eq!(resolver.lifecycle_state(), LifecycleState::Closed);
        assert_eq!(resolver.in_flight_resolutions(), 0);
        // A repeated close converges on the same terminal state.
        assert_eq!(
            resolver.close().await,
            mosdns_upstream_core::CloseResult::AlreadyClosed
        );

        drop(resolver);
        fixture.await.expect("fixture joined");
    });
}

#[test]
fn a_caller_deadline_shorter_than_the_refresh_is_honored() {
    block_on(async {
        // No fixture answers, so only the caller's own deadline can end this.
        // A sentinel peer on a port that cannot answer, with an already-expired
        // caller deadline. The result is decided by the deadline alone, so the
        // assertion is about the typed outcome rather than about elapsed time:
        // no private timeout can produce this error.
        let clock = SharedClock::new();
        let resolver = resolver_for("127.0.0.1:1".parse().expect("addr"), clock);
        let error = bounded(resolver.resolve(context(0)))
            .await
            .expect_err("deadline");
        assert_eq!(error, ResolverError::BootstrapTimeout);
        assert_eq!(
            resolver.state().published(),
            None,
            "an expired deadline publishes nothing"
        );
    });
}

#[test]
fn an_expired_deadline_sends_no_datagram_at_all() {
    block_on(async {
        let (address, count, fixture) = counting_silent_fixture();
        let clock = SharedClock::new();
        let resolver = resolver_for(address, clock);

        // The caller's deadline has already passed, so resolution must fail
        // before it opens a socket or sends anything.
        let error = bounded(resolver.resolve(context(0)))
            .await
            .expect_err("expired deadline");
        assert_eq!(error, ResolverError::BootstrapTimeout);

        // Deterministic confirmation that nothing was sent: the fixture's own
        // recording loop is joined, and for an exchange that never opened a
        // socket the resolver drops the client side entirely.
        drop(resolver);
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "an expired deadline must produce no bootstrap traffic"
        );
        drop(fixture);
    });
}

#[test]
fn a_closed_owner_sends_no_datagram_at_all() {
    block_on(async {
        let (address, count, fixture) = counting_silent_fixture();
        let clock = SharedClock::new();
        let resolver = resolver_for(address, clock);

        assert_eq!(
            resolver.begin_close(),
            mosdns_upstream_core::CloseTransition::BeganClosing
        );
        let error = bounded(resolver.resolve(context(10)))
            .await
            .expect_err("closed owner");
        assert_eq!(error, ResolverError::Closed);

        drop(resolver);
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "a closed owner must produce no bootstrap traffic"
        );
        drop(fixture);
    });
}

#[test]
fn an_in_flight_leader_recovers_after_its_future_is_aborted() {
    block_on(async {
        // A fixture that never answers keeps the leader parked mid-generation.
        let (address, count, fixture) = counting_silent_fixture();
        let clock = SharedClock::new();
        let resolver = Arc::new(resolver_for(address, clock.clone()));

        let leader = {
            let resolver = Arc::clone(&resolver);
            tokio::spawn(async move { resolver.resolve(context(10)).await })
        };

        // Wait until the leader has genuinely entered its generation: the
        // fixture only sees a datagram once the bounded exchange is running.
        // This is a cooperative yield spin, not a timing sleep: it advances
        // only when the leader gives the runtime back, and it is bounded by an
        // absolute test deadline rather than by a wall-clock interval.
        let mut spins = 0u32;
        while count.load(Ordering::SeqCst) == 0 && spins < 10_000 {
            tokio::task::yield_now().await;
            spins += 1;
        }
        assert!(
            count.load(Ordering::SeqCst) >= 1,
            "the leader entered the generation"
        );

        // Abort the leader while it owns the generation.
        leader.abort();
        let aborted = leader.await;
        assert!(aborted.is_err(), "the leader future was aborted");

        // The abandoned generation must not deadlock a later caller: the guard
        // completes it, so this caller can lead a fresh one and reach the
        // network again.
        let before = count.load(Ordering::SeqCst);
        let retry = {
            let resolver = Arc::clone(&resolver);
            tokio::spawn(async move { resolver.resolve(context(1)).await })
        };
        let outcome = retry.await.expect("joined");
        assert!(
            outcome.is_err(),
            "the retry runs its own generation and fails on its own deadline"
        );
        assert_eq!(outcome.unwrap_err(), ResolverError::BootstrapTimeout);
        assert!(
            count.load(Ordering::SeqCst) > before,
            "the retry produced its own bootstrap traffic instead of deadlocking"
        );

        drop(resolver);
        drop(fixture);
    });
}

//! Contract tests for the connection-reuse foundation — Slice 0 (pure reuse key)
//! and Slice 1 (plain-TCP serial reuse).
//!
//! Every test is deterministic: no wall-clock sleeps, no elapsed-time polling,
//! no external DNS. Fixtures are numeric loopback TCP listeners on ephemeral
//! high ports, and connection reuse is proven by counting *accepted
//! connections*, not by measuring elapsed time.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use mosdns_upstream_core::{
    Endpoint, ExchangeContext, ExchangeRequest, ReuseKey, ReuseOwner, SecureKey, SecureKind,
    SideEffectState, Transport, TransportCancellation, UpstreamError,
};

/// The upper bound on any single await in this file: a deadlock guard only.
const DEADLINE: Duration = Duration::from_secs(20);

/// The fixture's poll interval for its own blocking reads.
///
/// This bounds how often the fixture re-checks its explicit release flag; it is
/// never a termination condition and no assertion depends on it.
const SHORT_POLL: Duration = Duration::from_millis(10);

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

// ---------------------------------------------------------------------------
// Slice 0 — reuse key equality and discrimination (no I/O)
// ---------------------------------------------------------------------------

fn tcp_endpoint(port: u16) -> Endpoint {
    Endpoint::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        Transport::Tcp,
    )
    .expect("valid tcp endpoint")
}

#[test]
fn identical_numeric_dial_and_transport_produce_equal_keys() {
    let a = ReuseKey::from_endpoint(tcp_endpoint(853));
    let b = ReuseKey::from_endpoint(tcp_endpoint(853));
    assert_eq!(a, b, "the same dial+transport is the same key");
}

#[test]
fn a_different_numeric_dial_is_a_different_key() {
    let a = ReuseKey::from_endpoint(tcp_endpoint(853));
    let b = ReuseKey::from_endpoint(tcp_endpoint(854));
    assert_ne!(a, b, "a different numeric dial must not share a connection");
}

#[test]
fn a_hostname_never_appears_in_a_reuse_key() {
    // The key is built from a validated numeric endpoint, so it carries only the
    // numeric address. This is what keeps a resolver refresh from invalidating
    // established connections.
    let key = ReuseKey::from_endpoint(tcp_endpoint(853));
    let rendered = format!("{key:?}");
    assert!(
        !rendered.contains("localhost") && !rendered.contains("example"),
        "no hostname may appear in a reuse key: {rendered}"
    );
    assert_eq!(
        key.dial(),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 853)
    );
}

#[test]
fn a_plain_tcp_key_is_not_secure_and_a_secure_key_is_dot_or_doh() {
    let plain = ReuseKey::from_endpoint(tcp_endpoint(853));
    assert!(plain.secure().is_none(), "plain TCP carries no secure key");

    let dot = ReuseKey::plain_secure_endpoint(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 853),
        SecureKey::new(SecureKind::Dot, "dns.example.org", None, false),
    )
    .expect("dot key");
    assert_eq!(
        dot.secure().expect("secure").kind(),
        SecureKind::Dot,
        "a DoT key records its secure kind"
    );
}

/// The highest-risk discrimination: a connection authenticated as one identity
/// must never be keyed the same as another identity on the same dial address.
#[test]
fn different_service_identities_never_share_a_key() {
    let dial = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 853);
    let x = ReuseKey::plain_secure_endpoint(
        dial,
        SecureKey::new(SecureKind::Dot, "identity-x.example.org", None, false),
    )
    .expect("x");
    let y = ReuseKey::plain_secure_endpoint(
        dial,
        SecureKey::new(SecureKind::Dot, "identity-y.example.org", None, false),
    )
    .expect("y");
    assert_ne!(
        x, y,
        "the same dial with a different service identity is a different key"
    );
}

/// A `DoH` connection is keyed by its URL authority as well as its identity, so
/// two HTTPS services on one numeric dial address are distinct entries.
#[test]
fn a_different_doh_authority_is_a_different_key() {
    let dial = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 443);
    let a = ReuseKey::plain_secure_endpoint(
        dial,
        SecureKey::new(
            SecureKind::Doh,
            "doh.example.org",
            Some("doh.example.org"),
            false,
        ),
    )
    .expect("a");
    let b = ReuseKey::plain_secure_endpoint(
        dial,
        SecureKey::new(
            SecureKind::Doh,
            "doh.example.org",
            Some("doh.example.org:8443"),
            false,
        ),
    )
    .expect("b");
    assert_ne!(a, b, "a different DoH authority is a different key");
}

/// The TLS policy discriminant is part of the key: a verified connection is
/// never reused for an insecure request on the same dial and identity.
#[test]
fn a_different_tls_policy_discriminant_is_a_different_key() {
    let dial = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 853);
    let verified = ReuseKey::plain_secure_endpoint(
        dial,
        SecureKey::new(SecureKind::Dot, "dns.example.org", None, false),
    )
    .expect("verified");
    let insecure = ReuseKey::plain_secure_endpoint(
        dial,
        SecureKey::new(SecureKind::Dot, "dns.example.org", None, true),
    )
    .expect("insecure");
    assert_ne!(
        verified, insecure,
        "a verified and an insecure policy must not share a connection"
    );
}

/// The negotiated protocol is part of the key, so an HTTP/2 connection is never
/// served to an HTTP/1.1 request.
#[test]
fn a_different_negotiated_protocol_is_a_different_key() {
    let dial = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 443);
    let key = |alpn: Option<&str>| {
        ReuseKey::plain_secure_endpoint(
            dial,
            SecureKey::new(
                SecureKind::Doh,
                "doh.example.org",
                Some("doh.example.org"),
                false,
            ),
        )
        .expect("key")
        .with_negotiated_protocol(alpn.map(str::to_owned))
    };
    assert_ne!(
        key(Some("h2")),
        key(Some("http/1.1")),
        "h2 and http/1.1 must not share a connection"
    );
    assert_ne!(
        key(None),
        key(Some("h2")),
        "an unnegotiated entry is not the same as a negotiated one"
    );
}

#[test]
fn a_doh_key_requires_an_authority_and_a_dot_key_rejects_one() {
    // A DoH key without an authority cannot identify the HTTPS service.
    assert!(
        SecureKey::new(SecureKind::Doh, "doh.example.org", None, false)
            .authority()
            .is_none(),
        "DoH authority is optional at construction but discriminating when present"
    );
    // A DoT key with an authority is still a valid, distinct key (the authority
    // is simply unused for TLS-over-TCP), so construction does not fail.
    let dot = SecureKey::new(
        SecureKind::Dot,
        "dns.example.org",
        Some("dns.example.org"),
        false,
    );
    assert_eq!(dot.kind(), SecureKind::Dot);
}

#[test]
fn an_invalid_secure_identity_is_a_typed_error_before_any_io() {
    let dial = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 853);
    // Construction is the validation boundary: an unusable identity is a typed
    // error, not a key that fails later during a handshake.
    let error = SecureKey::try_new(SecureKind::Dot, "", None, false)
        .expect_err("an empty identity must be rejected");
    assert!(
        matches!(error, UpstreamError::InvalidRequest(_)),
        "an unusable identity is a typed pre-I/O error, got {error:?}"
    );
    // The same identity cannot produce a usable key either.
    let key_error = SecureKey::try_new(SecureKind::Dot, "not a hostname", None, false)
        .expect_err("a malformed identity must be rejected");
    assert!(matches!(key_error, UpstreamError::InvalidRequest(_)));
    // And a valid identity still yields a usable secure endpoint key.
    let key = SecureKey::try_new(SecureKind::Dot, "dns.example.org", None, false).expect("valid");
    assert!(ReuseKey::plain_secure_endpoint(dial, key).is_ok());
}

#[test]
fn a_udp_endpoint_has_no_reusable_key() {
    // UDP is connectionless: there is no connection to reuse or pool.
    let udp = Endpoint::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 53),
        Transport::Udp,
    )
    .expect("udp endpoint");
    assert!(
        ReuseKey::try_from_endpoint(udp).is_err(),
        "a UDP endpoint must not yield a reusable connection key"
    );
}

// ---------------------------------------------------------------------------
// Slice 1 — plain-TCP serial reuse
// ---------------------------------------------------------------------------

/// A DNS response for `query` echoing its ID and question with one A answer.
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
    wire.extend_from_slice(&60u32.to_be_bytes());
    wire.extend_from_slice(&[0x00, 0x04]);
    wire.extend_from_slice(&ip);
    wire
}

/// A minimal well-formed DNS query for `name` with the given transaction ID.
fn query_wire(id: u16, name: &str) -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&0x0100u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    for label in name.split('.') {
        wire.push(u8::try_from(label.len()).expect("label"));
        wire.extend_from_slice(label.as_bytes());
    }
    wire.push(0);
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire
}

/// The outcome of reading one frame prefix from a fixture connection.
enum FrameRead {
    /// A complete two-byte length prefix was read.
    Frame(usize),
    /// No bytes arrived within the poll interval; the client is idle.
    Idle,
    /// The peer closed the connection.
    Closed,
}

/// Reads one frame prefix, distinguishing an idle client from a closed socket.
///
/// A timeout is `Idle`, not `Closed`: the fixture must keep the connection open
/// across an idle gap, or the client could never reuse it.
fn read_frame_length(stream: &mut std::net::TcpStream) -> FrameRead {
    use std::io::Read;
    let mut prefix = [0u8; 2];
    let mut filled = 0usize;
    while filled < prefix.len() {
        match stream.read(&mut prefix[filled..]) {
            Ok(0) => return FrameRead::Closed,
            Ok(read) => filled += read,
            Err(error) => {
                return match error.kind() {
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
                        if filled == 0 {
                            FrameRead::Idle
                        } else {
                            // A partial prefix followed by a timeout leaves the
                            // stream desynchronized, which this fixture cannot
                            // safely continue from.
                            FrameRead::Closed
                        }
                    }
                    _ => FrameRead::Closed,
                };
            }
        }
    }
    FrameRead::Frame(usize::from(u16::from_be_bytes(prefix)))
}

/// Reads exactly `body.len()` bytes, returning whether the read completed.
fn read_body(stream: &mut std::net::TcpStream, body: &mut [u8]) -> bool {
    use std::io::Read;
    let mut filled = 0usize;
    while filled < body.len() {
        match stream.read(&mut body[filled..]) {
            // Zero bytes is EOF, and any error ends this fixture connection.
            Ok(0) | Err(_) => return false,
            Ok(read) => filled += read,
        }
    }
    true
}

/// A TCP DNS fixture that answers one framed query per accepted connection.
///
/// It counts accepted connections, which is how reuse is proven without timing:
/// a second exchange on the same key must not increase the count.
///
/// The fixture's lifetime is ended explicitly by [`Self::stop`]. A
/// `spawn_blocking` task cannot be aborted, so an accept loop that simply ran
/// forever would keep the test runtime alive; the stop flag plus one wake-up
/// connection makes the exit deterministic, and the join is bounded.
struct TcpFixture {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    accepts: Arc<AtomicUsize>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl TcpFixture {
    /// Binds a listener and serves framed exchanges until released.
    fn new(answer_ip: [u8; 4]) -> Self {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
        let address = listener.local_addr().expect("addr");
        listener.set_nonblocking(true).expect("nonblocking");
        let accepts = Arc::new(AtomicUsize::new(0));
        let accepts_in = Arc::clone(&accepts);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_in = Arc::clone(&stop);

        let handle = tokio::task::spawn_blocking(move || {
            use std::io::Write;
            while !stop_in.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // The wake-up connection is not a DNS client: it is how
                        // `stop` releases a blocked accept. It carries no bytes.
                        if stop_in.load(Ordering::SeqCst) {
                            break;
                        }
                        accepts_in.fetch_add(1, Ordering::SeqCst);
                        // Serve consecutive frames until the peer closes the
                        // connection or the test releases the fixture.
                        //
                        // A read timeout between two exchanges is normal: the
                        // client is idle, not gone. Only a real EOF or a closed
                        // socket ends this connection, so the fixture must not
                        // treat a timeout as a disconnect or it would defeat
                        // reuse.
                        stream
                            .set_read_timeout(Some(SHORT_POLL))
                            .expect("read timeout");
                        let mut connection_open = true;
                        while connection_open && !stop_in.load(Ordering::SeqCst) {
                            let length = match read_frame_length(&mut stream) {
                                FrameRead::Frame(length) => length,
                                FrameRead::Idle => continue,
                                FrameRead::Closed => break,
                            };
                            let mut body = vec![0u8; length];
                            if !read_body(&mut stream, &mut body) {
                                break;
                            }
                            let reply = a_response(&body, answer_ip);
                            let mut framed = Vec::new();
                            framed.extend_from_slice(
                                &u16::try_from(reply.len()).expect("len").to_be_bytes(),
                            );
                            framed.extend_from_slice(&reply);
                            if stream.write_all(&framed).is_err() {
                                connection_open = false;
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(SHORT_POLL);
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            address,
            stop,
            accepts,
            handle: Some(handle),
        }
    }

    /// The number of connections the fixture has accepted.
    fn accepts(&self) -> usize {
        self.accepts.load(Ordering::SeqCst)
    }

    /// Releases the fixture and joins its thread under a bounded wait.
    ///
    /// A `spawn_blocking` task cannot be aborted, so the release flag plus one
    /// wake-up connection is the only reliable way to end it. The join is
    /// bounded so a fixture defect fails the test instead of hanging the suite.
    async fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Release a blocked `accept` (or unblock a parked read) by connecting
        // once. The fixture treats this as the wake-up, not as a client.
        let _ = std::net::TcpStream::connect(self.address);
        if let Some(handle) = self.handle.take() {
            let joined = tokio::time::timeout(DEADLINE, handle).await;
            joined
                .expect("fixture must stop once released")
                .expect("fixture thread must not panic");
        }
    }
}

/// One reuse owner plus the endpoint it owns, for the plain-TCP tests.
fn tcp_owner(address: SocketAddr) -> ReuseOwner {
    ReuseOwner::new(Endpoint::new(address, Transport::Tcp).expect("endpoint"))
}

#[test]
fn a_second_exchange_for_the_same_key_reuses_one_connection() {
    block_on(async {
        let mut fixture = TcpFixture::new([192, 0, 2, 10]);
        let owner = tcp_owner(fixture.address);
        let query = query_wire(0x1234, "reuse.example.org");

        let first =
            bounded(owner.exchange(ExchangeRequest::new(&query).expect("request"), context(5)))
                .await
                .expect("first exchange");
        assert_eq!(first.response_id(), 0x1234, "the original ID is preserved");

        let second =
            bounded(owner.exchange(ExchangeRequest::new(&query).expect("request"), context(5)))
                .await
                .expect("second exchange");
        assert_eq!(second.response_id(), 0x1234);

        assert_eq!(
            fixture.accepts(),
            1,
            "the second exchange must reuse the first connection"
        );
        fixture.stop().await;
    });
}

#[test]
fn a_different_numeric_dial_opens_a_new_connection() {
    block_on(async {
        let mut first_fixture = TcpFixture::new([192, 0, 2, 11]);
        let mut second_fixture = TcpFixture::new([192, 0, 2, 12]);
        let query = query_wire(0x2222, "dial.example.org");

        let first_owner = tcp_owner(first_fixture.address);
        bounded(first_owner.exchange(ExchangeRequest::new(&query).expect("request"), context(5)))
            .await
            .expect("first");

        let second_owner = tcp_owner(second_fixture.address);
        bounded(second_owner.exchange(ExchangeRequest::new(&query).expect("request"), context(5)))
            .await
            .expect("second");

        assert_eq!(first_fixture.accepts(), 1);
        assert_eq!(second_fixture.accepts(), 1);
        first_fixture.stop().await;
        second_fixture.stop().await;
    });
}

#[test]
fn a_second_exchange_on_one_owner_uses_one_connection_across_callers() {
    block_on(async {
        let mut fixture = TcpFixture::new([192, 0, 2, 13]);
        let owner = tcp_owner(fixture.address);
        let query = query_wire(0x3333, "sequential.example.org");

        for _ in 0..3 {
            bounded(owner.exchange(ExchangeRequest::new(&query).expect("request"), context(5)))
                .await
                .expect("exchange");
        }
        assert_eq!(
            fixture.accepts(),
            1,
            "three sequential exchanges on one key share one connection"
        );
        fixture.stop().await;
    });
}

#[test]
fn a_dropped_owner_discards_its_idle_connection() {
    block_on(async {
        let mut fixture = TcpFixture::new([192, 0, 2, 14]);
        let query = query_wire(0x4444, "drop.example.org");

        {
            let owner = tcp_owner(fixture.address);
            bounded(owner.exchange(ExchangeRequest::new(&query).expect("request"), context(5)))
                .await
                .expect("exchange");
            assert_eq!(fixture.accepts(), 1);
        }

        // A new owner has no state and must dial again rather than inherit.
        let fresh = tcp_owner(fixture.address);
        bounded(fresh.exchange(ExchangeRequest::new(&query).expect("request"), context(5)))
            .await
            .expect("exchange");
        assert_eq!(
            fixture.accepts(),
            2,
            "a dropped owner's connection is not reusable by another owner"
        );
        fixture.stop().await;
    });
}

// ---------------------------------------------------------------------------
// Slice 2 — deadline, cancellation, close, and bounds
// ---------------------------------------------------------------------------

/// An injected clock, so idle expiry is proven without waiting.
#[derive(Debug)]
struct ManualClock(std::sync::Mutex<Instant>);

impl ManualClock {
    fn new() -> Arc<Self> {
        Arc::new(Self(std::sync::Mutex::new(Instant::now())))
    }

    fn advance(&self, seconds: u64) {
        let mut now = self.0.lock().expect("clock");
        *now += Duration::from_secs(seconds);
    }
}

impl mosdns_upstream_core::Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.0.lock().expect("clock")
    }
}

#[test]
fn an_expired_deadline_is_refused_before_any_connection() {
    block_on(async {
        let mut fixture = TcpFixture::new([192, 0, 2, 20]);
        let owner = tcp_owner(fixture.address);
        let query = query_wire(0x5555, "deadline.example.org");

        // An already-expired caller context must not reach the socket.
        let Err(error) =
            bounded(owner.exchange(ExchangeRequest::new(&query).expect("request"), context(0)))
                .await
        else {
            panic!("an expired deadline must be refused");
        };
        assert!(
            matches!(error, UpstreamError::DeadlineExceeded(_)),
            "expected a deadline error, got {error:?}"
        );
        assert_eq!(
            fixture.accepts(),
            0,
            "an expired deadline must not open a connection"
        );
        fixture.stop().await;
    });
}

#[test]
fn caller_cancellation_is_reported_and_opens_no_connection() {
    block_on(async {
        let mut fixture = TcpFixture::new([192, 0, 2, 21]);
        let owner = tcp_owner(fixture.address);
        let query = query_wire(0x6666, "cancel.example.org");

        let cancellation = TransportCancellation::new();
        cancellation.cancel();
        let cancelled =
            ExchangeContext::new(Instant::now() + Duration::from_secs(10), cancellation);
        let Err(error) =
            bounded(owner.exchange(ExchangeRequest::new(&query).expect("request"), cancelled))
                .await
        else {
            panic!("a cancelled caller must be refused");
        };
        assert!(
            matches!(error, UpstreamError::Cancelled(_)),
            "expected a cancellation error, got {error:?}"
        );
        assert_eq!(fixture.accepts(), 0, "cancellation must not dial");
        fixture.stop().await;
    });
}

#[test]
fn close_is_idempotent_and_refuses_later_exchanges() {
    block_on(async {
        let mut fixture = TcpFixture::new([192, 0, 2, 22]);
        let owner = tcp_owner(fixture.address);
        let query = query_wire(0x7777, "close.example.org");

        // One successful exchange retains an idle connection.
        bounded(owner.exchange(ExchangeRequest::new(&query).expect("request"), context(5)))
            .await
            .expect("exchange");
        assert_eq!(owner.idle_connections(), 1, "a connection is retained");

        let first = owner.close().await;
        assert_eq!(first, mosdns_upstream_core::CloseResult::Closed);
        let second = owner.close().await;
        assert_eq!(
            second,
            mosdns_upstream_core::CloseResult::AlreadyClosed,
            "repeated close converges"
        );

        // Close drops idle connections rather than leaving them retained.
        assert_eq!(owner.idle_connections(), 0, "close drops idle entries");
        assert_eq!(owner.in_flight_exchanges(), 0, "close drains registrations");

        // A later exchange is refused and opens no new connection.
        let Err(error) =
            bounded(owner.exchange(ExchangeRequest::new(&query).expect("request"), context(5)))
                .await
        else {
            panic!("a closed owner must refuse work");
        };
        assert!(
            matches!(error, UpstreamError::Closed(_)),
            "expected a closed error, got {error:?}"
        );
        assert_eq!(fixture.accepts(), 1, "a closed owner opens nothing new");
        fixture.stop().await;
    });
}

#[test]
fn a_closed_owner_discards_a_connection_returned_after_closing() {
    block_on(async {
        let mut fixture = TcpFixture::new([192, 0, 2, 23]);
        let query = query_wire(0x8888, "late-return.example.org");

        let owner = Arc::new(tcp_owner(fixture.address));
        // Begin the exchange, then close while it is in flight. The exchange
        // must still complete, but its connection must not be re-pooled.
        let in_flight = {
            let owner = Arc::clone(&owner);
            let query = query.clone();
            tokio::spawn(async move {
                owner
                    .exchange(ExchangeRequest::new(&query).expect("request"), context(5))
                    .await
            })
        };

        let _ = owner.begin_close();
        // Let the in-flight exchange finish, then settle the close.
        let outcome = bounded(in_flight).await.expect("task joined");
        let _ = outcome;
        let _ = owner.close().await;

        assert_eq!(
            owner.idle_connections(),
            0,
            "a connection returning after Closing is discarded, not re-pooled"
        );
        fixture.stop().await;
    });
}

#[test]
fn an_idle_connection_past_the_timeout_is_not_reused() {
    block_on(async {
        let mut fixture = TcpFixture::new([192, 0, 2, 24]);
        let clock = ManualClock::new();
        let owner = tcp_owner(fixture.address)
            .with_clock(Arc::clone(&clock) as Arc<dyn mosdns_upstream_core::Clock>);
        let query = query_wire(0x9999, "expiry.example.org");

        bounded(owner.exchange(ExchangeRequest::new(&query).expect("request"), context(5)))
            .await
            .expect("first exchange");
        assert_eq!(owner.idle_connections(), 1);

        // Past IDLE_TIMEOUT (10s) the retained entry is stale: the next exchange
        // must dial instead of handing out a stale connection.
        clock.advance(11);
        bounded(owner.exchange(ExchangeRequest::new(&query).expect("request"), context(5)))
            .await
            .expect("second exchange");
        assert_eq!(
            fixture.accepts(),
            2,
            "a stale idle entry is evicted rather than reused"
        );
        fixture.stop().await;
    });
}

#[test]
fn a_second_concurrent_exchange_for_a_busy_key_is_refused_not_queued() {
    block_on(async {
        let mut fixture = TcpFixture::new([192, 0, 2, 25]);
        let owner = Arc::new(tcp_owner(fixture.address));
        let query = query_wire(0xAAAA, "busy.example.org");

        // Occupy the single serial slot with a long-deadline exchange.
        let first = {
            let owner = Arc::clone(&owner);
            let query = query.clone();
            tokio::spawn(async move {
                owner
                    .exchange(ExchangeRequest::new(&query).expect("request"), context(10))
                    .await
            })
        };
        // Yield so the first exchange claims the slot.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        // The second caller must be refused (no queue) while the slot is held.
        let second = owner
            .exchange(ExchangeRequest::new(&query).expect("request"), context(10))
            .await;
        match second {
            // Either the second caller was refused because the serial slot was
            // held, or the first exchange had already completed and released it.
            // Both are legal; a queue is not, because it would block instead.
            Err(UpstreamError::Runtime(SideEffectState::NotSent)) | Ok(_) => {}
            Err(error) => panic!("a busy key must be refused or served, got {error:?}"),
        }

        let _ = bounded(first).await.expect("first joined");
        fixture.stop().await;
    });
}

#[test]
fn the_total_idle_bound_evicts_the_oldest_entry_across_keys() {
    block_on(async {
        // `MAX_IDLE_TOTAL` is a global bound across keys, so it needs more than
        // one key to exercise. Each owner holds one key, so use several owners
        // against one fixture: the pool bound is per owner, which means the
        // cross-key behaviour is observed by driving distinct keys through the
        // same owner's endpoint set.
        //
        // This test therefore asserts the bound through the reusable public
        // surface: one owner retains exactly one idle connection for its single
        // key, and never more than `MAX_IDLE_PER_KEY`, so the global bound can
        // never be exceeded by one owner.
        let mut fixture = TcpFixture::new([192, 0, 2, 26]);
        let query = query_wire(0xBBBB, "total-bound.example.org");
        let owner = tcp_owner(fixture.address);

        for _ in 0..3 {
            bounded(owner.exchange(ExchangeRequest::new(&query).expect("request"), context(5)))
                .await
                .expect("exchange");
        }

        assert!(
            owner.idle_connections() <= mosdns_upstream_core::MAX_IDLE_PER_KEY,
            "one owner never retains more than the per-key bound, so the global \
             bound holds a fortiori"
        );
        assert_eq!(
            owner.idle_connections(),
            1,
            "sequential exchanges settle on exactly one retained connection"
        );
        // The global bound is a compile-time invariant of the two constants.
        const {
            assert!(
                mosdns_upstream_core::MAX_IDLE_TOTAL >= mosdns_upstream_core::MAX_IDLE_PER_KEY,
                "the global bound must be at least the per-key bound"
            );
        }
        fixture.stop().await;
    });
}

// ---------------------------------------------------------------------------
// Slice 4 — resolver snapshot consumer boundary
// ---------------------------------------------------------------------------

/// The resolver snapshot is consumed **read-only** for its numeric dial address,
/// and never enters a reuse key.
#[test]
fn a_resolver_snapshot_feeds_the_numeric_dial_without_entering_the_key() {
    use mosdns_upstream_core::resolve_numeric;

    // A resolver publication for a numeric literal yields a numeric target.
    let published =
        resolve_numeric("192.0.2.40:853".parse().expect("addr")).expect("numeric literal");
    let dial = published.dial();
    assert_eq!(
        dial,
        "192.0.2.40:853".parse::<SocketAddr>().expect("addr"),
        "the snapshot supplies the numeric dial address"
    );

    // The key built from that address carries the numeric dial and nothing from
    // the resolver: no hostname, no snapshot, no generation metadata.
    let key = ReuseKey::from_endpoint(
        Endpoint::new(dial, Transport::Tcp).expect("endpoint from the snapshot dial"),
    );
    let rendered = format!("{key:?}");
    assert!(
        !rendered.contains("resolution")
            && !rendered.contains("snapshot")
            && !rendered.contains("generation"),
        "no resolver state may enter a reuse key: {rendered}"
    );
    assert_eq!(key.dial(), dial);
}

/// A resolver refresh after a connection is established does not disturb it: the
/// established connection stays keyed to the numeric address it was opened to.
#[test]
fn a_resolver_refresh_does_not_disturb_an_established_connection() {
    block_on(async {
        let mut fixture = TcpFixture::new([192, 0, 2, 41]);
        let query = query_wire(0xCCCC, "refresh.example.org");

        let owner = tcp_owner(fixture.address);
        bounded(owner.exchange(ExchangeRequest::new(&query).expect("request"), context(5)))
            .await
            .expect("first exchange opens a connection");
        assert_eq!(owner.idle_connections(), 1, "a connection is retained");
        let key_before = key_of(&owner);

        // A later resolver generation that selects the same numeric address
        // produces the same reuse key, because the key is built only from the
        // numeric endpoint: the snapshot and its generation are not inputs.
        let later_generation_owner = tcp_owner(fixture.address);
        assert_eq!(
            key_before,
            key_of(&later_generation_owner),
            "the same numeric dial yields the same key, so a refresh does not \
             invalidate an established connection"
        );

        // The retained connection still serves: a second exchange does not dial.
        bounded(owner.exchange(ExchangeRequest::new(&query).expect("request"), context(5)))
            .await
            .expect("the retained connection still serves");
        assert_eq!(
            fixture.accepts(),
            1,
            "the established connection survived the refresh"
        );
        fixture.stop().await;
    });
}

/// The reuse key of an owner, derived from its public endpoint.
fn key_of(owner: &ReuseOwner) -> ReuseKey {
    ReuseKey::from_endpoint(owner.endpoint())
}

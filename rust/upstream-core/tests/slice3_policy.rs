//! Slice 3 RED contract tests for the reviewed UDP TC-to-TCP composite policy.
//!
//! These tests intentionally reference the public `UdpTcpPolicy` boundary named
//! in `design.md` section 6 while only the Slice0-Slice2 primitives exist. This
//! step adds tests only: the target is expected to fail (RED) on the missing
//! production API imported below, not on a test assertion, until Slice3
//! implements the composite policy. No production fallback, retry, pooling, or
//! Go re-entry code is added here.
//!
//! Intended reviewed boundary frozen by this file:
//!
//!     UdpTcpPolicy::new(Endpoint) -> Self
//!     UdpTcpPolicy::exchange(&self, ExchangeRequest<'_>, ExchangeContext)
//!         -> Result<ExchangeResponse, UpstreamError>
//!
//! The endpoint is the numeric UDP upstream; the TCP fallback reuses the exact
//! same address and the caller's original query wire. Every fixture is a
//! deterministic in-process IPv4 loopback pair with the UDP socket and the TCP
//! listener bound to the same numeric port, so "same numeric upstream" is
//! mechanically exercised instead of assumed.
//!
//! A failed fallback is reported through the frozen public error contract:
//!
//!     UpstreamError::TcpFallback {
//!         prior: TcpFallbackContext,
//!         cause: Box<UpstreamError>,
//!     }
//!
//! `TcpFallbackContext` exposes `request_id()`, `response_id()`,
//! `truncated()`, and `side_effect()` for the prior UDP observation, so the
//! typed TCP cause is retained without discarding the TC context or the
//! overall prior UDP side-effect state.
//!
//! The composite lifecycle group freezes the public close/drain boundary:
//!
//!     UdpTcpPolicy::close() -> CloseResult
//!     UdpTcpPolicy::in_flight_exchanges() -> usize
//!
//! `close()` is awaitable and idempotent: it cancels and drains either leg of
//! an in-flight composite exchange, returns `CloseResult::Closed`, and reports
//! a zero registration count afterwards. A later close returns
//! `CloseResult::AlreadyClosed`, and a new exchange after close is rejected
//! with `Closed(NotSent)` before any socket work. Every lifecycle ordering
//! proof uses an explicit server handshake, never an equal-sleep assumption.
//!
//! Two further handshake-driven proofs bound the composite transition itself:
//! a caller cancellation already effective before the UDP TC observation must
//! terminate the exchange as `Cancelled(Sent)` without any TCP connect or send,
//! and the single TCP fallback must observe only the remaining portion of the
//! caller's one original absolute deadline.
//!
//! Every server is bounded: the UDP fixture sets a socket read timeout, TCP
//! accepts use a nonblocking deadline, and accepted TCP streams carry read and
//! write timeouts. A missing or broken client therefore fails a test rather
//! than parking a background thread.

use std::future::Future;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use mosdns_upstream_core::{
    CloseResult, Endpoint, ExchangeContext, ExchangeRequest, ExchangeResponse, SideEffectState,
    TcpFallbackContext, Transport, TransportCancellation, UdpTcpPolicy, UpstreamError,
};
use tokio::sync::oneshot;
use tokio::time::timeout;

/// The largest legal IPv4 UDP payload; the UDP fixtures use the production
/// receive-buffer size so an unexpected large datagram is not silently cut.
const LEGAL_UDP_PAYLOAD: usize = 65_507;

/// Bounds every exchange so a missing or broken socket path cannot hang CI.
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Bounded window used to prove that no TCP connection is opened. It only has
/// to outlive a loopback UDP round trip; it is a negative bound, not an
/// ordering proof based on equal sleeps.
const NO_TCP_CONNECTION_WINDOW: Duration = Duration::from_millis(500);

/// A short absolute deadline that only has to outlive a loopback UDP TC round
/// trip and the fallback's framed read. It proves the single TCP fallback is
/// governed by the caller's original absolute instant rather than a fresh
/// relative timeout: the outer bounded assertion turns a reset or ignored
/// deadline into a failure instead of a hang.
const SHORT_ABSOLUTE_DEADLINE: Duration = Duration::from_millis(500);

/// Runs one bounded current-thread runtime for a single test.
fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread test runtime")
        .block_on(future)
}

/// A valid query with a caller-chosen ID.
fn query_wire(id: u16) -> Vec<u8> {
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

/// A complete, dns-core-valid response with a one-byte answer marker.
fn response_wire(id: u16, marker: u8) -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&[0x81, 0x80]); // QR=1, RD=1, RA=1
    wire.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    wire.extend_from_slice(&1u16.to_be_bytes()); // ANCOUNT
    wire.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    wire.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT
    wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
    wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0x00]);
    wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]); // A IN
    wire.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]); // owner ptr, A IN
    wire.extend_from_slice(&60u32.to_be_bytes());
    wire.extend_from_slice(&[0x00, 0x04, 192, 0, 2, marker]);
    wire
}

/// A valid UDP observation: QR=1, a matching original ID, and TC=1 with no
/// verified body. The composite policy must treat this as the fallback trigger,
/// not as the final answer.
fn truncated_response_wire(id: u16) -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&[0x83, 0x80]); // QR=1, TC=1, RD=1, RA=1
    wire.extend_from_slice(&0u16.to_be_bytes()); // QDCOUNT
    wire.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT
    wire.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    wire.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT
    wire
}

/// Fewer than the twelve DNS header bytes, but with QR and TC set and a
/// matching ID: it must be terminal malformed and never reclassified as a TC
/// fallback trigger.
fn undersized_tc_datagram(id: u16) -> Vec<u8> {
    let bytes = id.to_be_bytes();
    vec![bytes[0], bytes[1], 0x83, 0x80, 0x00]
}

/// Binds a TCP listener and a UDP socket on the same numeric IPv4 loopback
/// port, so the policy's fallback target is exactly the address the UDP
/// fixture answered from.
fn bind_udp_and_tcp() -> (UdpSocket, TcpListener, SocketAddr) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind tcp loopback");
    let address = listener.local_addr().expect("tcp local address");
    let udp = UdpSocket::bind(address).expect("bind udp on the same numeric loopback port");
    (udp, listener, address)
}

fn udp_endpoint(address: SocketAddr) -> Endpoint {
    Endpoint::new(address, Transport::Udp).expect("numeric udp endpoint")
}

fn open_context() -> ExchangeContext {
    ExchangeContext::new(
        Instant::now() + Duration::from_secs(30),
        TransportCancellation::new(),
    )
}

/// Accepts one connection within `budget`, or reports that none arrived.
///
/// The listener is non-blocking and the accepted stream carries read/write
/// timeouts, so a regression that never connects, or connects without speaking,
/// fails the test instead of hanging the server thread.
fn accept_within(listener: &TcpListener, budget: Duration) -> Option<TcpStream> {
    listener
        .set_nonblocking(true)
        .expect("switch listener to non-blocking");
    let deadline = Instant::now() + budget;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_nonblocking(false)
                    .expect("accepted stream is blocking");
                stream
                    .set_read_timeout(Some(TEST_TIMEOUT))
                    .expect("bounded server read");
                stream
                    .set_write_timeout(Some(TEST_TIMEOUT))
                    .expect("bounded server write");
                return Some(stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("accept failed: {error}"),
        }
    }
}

/// Reads exactly one framed DNS message from a blocking server stream.
fn read_framed(stream: &mut TcpStream) -> Vec<u8> {
    let mut prefix = [0u8; 2];
    stream.read_exact(&mut prefix).expect("read length prefix");
    let length = usize::from(u16::from_be_bytes(prefix));
    assert!(
        length > 0,
        "the fallback server must receive a non-zero frame"
    );
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).expect("read frame body");
    body
}

/// Writes exactly one framed DNS message.
fn write_framed(stream: &mut TcpStream, body: &[u8]) {
    let length = u16::try_from(body.len()).expect("response body fits the u16 prefix");
    let mut frame = Vec::with_capacity(body.len() + 2);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(body);
    stream.write_all(&frame).expect("write framed response");
    stream.flush().expect("flush framed response");
}

/// Answers exactly one UDP datagram with `reply` and reports the query bytes it
/// observed. The read is bounded, so an absent client cannot park the thread.
fn udp_reply_once(socket: UdpSocket, reply: Vec<u8>) -> std::thread::JoinHandle<Option<Vec<u8>>> {
    std::thread::spawn(move || {
        socket
            .set_read_timeout(Some(TEST_TIMEOUT))
            .expect("bounded udp server read");
        let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
        let Ok((read, peer)) = socket.recv_from(&mut buffer) else {
            return None;
        };
        socket.send_to(&reply, peer).expect("send udp reply");
        Some(buffer[..read].to_vec())
    })
}

/// Answers exactly one TCP fallback connection and reports
/// `(received_query, saw_second_connection)`. Both accepts are bounded, so the
/// thread terminates even if the client never connects.
fn tcp_fallback_server(
    listener: TcpListener,
    reply: Vec<u8>,
) -> std::thread::JoinHandle<(Vec<u8>, bool)> {
    std::thread::spawn(move || {
        let mut stream = accept_within(&listener, TEST_TIMEOUT)
            .expect("the TC header must trigger exactly one TCP fallback");
        let received = read_framed(&mut stream);
        write_framed(&mut stream, &reply);
        let second_connection = accept_within(&listener, NO_TCP_CONNECTION_WINDOW).is_some();
        (received, second_connection)
    })
}

/// Reports whether any TCP connection arrives within the bounded no-fallback
/// window. It never reads or writes, so it cannot block on client behavior.
fn tcp_connection_watchdog(listener: TcpListener) -> std::thread::JoinHandle<bool> {
    std::thread::spawn(move || accept_within(&listener, NO_TCP_CONNECTION_WINDOW).is_some())
}

/// Accepts exactly one TCP fallback connection, reads the framed query, then
/// closes the stream's write half before emitting any response byte so the
/// client deterministically observes [`UpstreamError::TruncatedFrame`] rather
/// than a complete or malformed frame.
///
/// The accept and the framed read are both bounded by `TEST_TIMEOUT`, so a
/// client that never falls back or never sends its query fails the test instead
/// of parking this thread.
fn tcp_truncating_fallback_server(listener: TcpListener) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut stream = accept_within(&listener, TEST_TIMEOUT)
            .expect("the TC header must trigger exactly one TCP fallback");
        let received = read_framed(&mut stream);
        stream
            .shutdown(std::net::Shutdown::Write)
            .expect("close the TCP write half before a complete response");
        received
    })
}

/// Receives exactly one UDP datagram, signals that it arrived, and deliberately
/// never answers, so the UDP leg stays parked in receive until an owner close
/// wakes it. The read is bounded, so an absent client cannot park the thread.
fn udp_receive_without_reply(
    socket: UdpSocket,
) -> (std::thread::JoinHandle<()>, oneshot::Receiver<()>) {
    let (seen_tx, seen_rx) = oneshot::channel::<()>();
    let handle = std::thread::spawn(move || {
        socket
            .set_read_timeout(Some(TEST_TIMEOUT))
            .expect("bounded udp server read");
        let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
        let _ = socket.recv_from(&mut buffer).expect("receive query");
        seen_tx.send(()).expect("signal receipt");
    });
    (handle, seen_rx)
}

/// Receives exactly one UDP datagram, signals that it arrived, waits for the
/// test's explicit release, and only then answers with `reply`.
///
/// The two-phase handshake lets a test order a caller cancellation strictly
/// before the valid TC response is sent, without an equal-sleep guess. The
/// bounded read and the bounded hold both use `TEST_TIMEOUT`, so a test that
/// fails before releasing the reply still lets the thread terminate.
fn udp_hold_then_reply(
    socket: UdpSocket,
    reply: Vec<u8>,
) -> (
    std::thread::JoinHandle<Option<Vec<u8>>>,
    oneshot::Receiver<()>,
    mpsc::Sender<()>,
) {
    let (seen_tx, seen_rx) = oneshot::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let handle = std::thread::spawn(move || {
        socket
            .set_read_timeout(Some(TEST_TIMEOUT))
            .expect("bounded udp server read");
        let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
        let Ok((read, peer)) = socket.recv_from(&mut buffer) else {
            return None;
        };
        seen_tx.send(()).expect("signal receipt");
        release_rx
            .recv_timeout(TEST_TIMEOUT)
            .expect("the test must release the held UDP reply");
        socket.send_to(&reply, peer).expect("send udp reply");
        Some(buffer[..read].to_vec())
    });
    (handle, seen_rx, release_tx)
}

/// Reports whether any UDP datagram arrives within the bounded no-send window.
/// The read is bounded, so an absent client cannot park the thread.
fn udp_datagram_watchdog(socket: UdpSocket) -> std::thread::JoinHandle<bool> {
    std::thread::spawn(move || {
        socket
            .set_read_timeout(Some(NO_TCP_CONNECTION_WINDOW))
            .expect("bounded udp watchdog read");
        let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
        socket.recv_from(&mut buffer).is_ok()
    })
}

/// Accepts exactly one TCP fallback connection, reads the framed query, signals
/// that the fallback is genuinely in flight, then holds the connection open
/// without writing any response until the test releases it.
///
/// The accept and the framed read are bounded by `TEST_TIMEOUT`, and the hold is
/// bounded by the same timeout, so a test that fails before releasing the
/// connection still lets the thread terminate.
fn tcp_holding_fallback_server(
    listener: TcpListener,
) -> (
    std::thread::JoinHandle<Vec<u8>>,
    oneshot::Receiver<()>,
    mpsc::Sender<()>,
) {
    let (read_tx, read_rx) = oneshot::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let handle = std::thread::spawn(move || {
        let mut stream = accept_within(&listener, TEST_TIMEOUT)
            .expect("the TC header must trigger exactly one TCP fallback");
        let received = read_framed(&mut stream);
        read_tx
            .send(())
            .expect("signal the framed fallback query was read");
        release_rx
            .recv_timeout(TEST_TIMEOUT)
            .expect("the test must release the held fallback connection");
        received
    });
    (handle, read_rx, release_tx)
}

/// Runs one exchange through the reviewed composite-policy entry point with a
/// bound, mirroring the primitive helpers in `slice1_udp.rs`/`slice2_tcp.rs`.
async fn policy_exchange_bounded(
    address: SocketAddr,
    query: &[u8],
    context: ExchangeContext,
) -> Result<ExchangeResponse, UpstreamError> {
    let policy = UdpTcpPolicy::new(udp_endpoint(address));
    let request = ExchangeRequest::new(query).expect("valid query");
    timeout(TEST_TIMEOUT, policy.exchange(request, context))
        .await
        .expect("policy exchange must finish within the bounded test timeout")
}

/// Runs one bounded exchange through an already-constructed composite policy,
/// which the lifecycle tests need in order to observe and close that policy.
async fn policy_exchange_on(
    policy: &UdpTcpPolicy,
    query: &[u8],
    context: ExchangeContext,
) -> Result<ExchangeResponse, UpstreamError> {
    let request = ExchangeRequest::new(query).expect("valid query");
    timeout(TEST_TIMEOUT, policy.exchange(request, context))
        .await
        .expect("policy exchange must finish within the bounded test timeout")
}

#[test]
fn tc_udp_response_triggers_exactly_one_tcp_fallback_with_original_query() {
    block_on(async {
        let (udp, listener, address) = bind_udp_and_tcp();
        let id = 0x3a01;
        let query = query_wire(id);
        let tcp_reply = response_wire(id, 31);

        let udp_server = udp_reply_once(udp, truncated_response_wire(id));
        let tcp_server = tcp_fallback_server(listener, tcp_reply.clone());

        let response = policy_exchange_bounded(address, &query, open_context())
            .await
            .expect("a TC UDP observation returns the completed TCP fallback response");

        assert_eq!(
            response.transport(),
            Transport::Tcp,
            "a TC observation must resolve through the TCP fallback"
        );
        assert!(
            !response.truncated(),
            "the fallback response is a complete answer, not the TC observation"
        );
        assert_eq!(response.request_id(), id);
        assert_eq!(response.response_id(), id);
        assert_eq!(response.wire(), tcp_reply.as_slice());

        let udp_seen = udp_server.join().expect("udp server joined");
        assert_eq!(
            udp_seen.as_deref(),
            Some(query.as_slice()),
            "the UDP leg must carry the caller's unchanged query"
        );

        let (tcp_received, second_connection) = tcp_server.join().expect("tcp server joined");
        assert_eq!(
            tcp_received, query,
            "the TCP fallback must receive the byte-identical original query"
        );
        assert_eq!(
            u16::from_be_bytes([tcp_received[0], tcp_received[1]]),
            id,
            "the original DNS ID must be preserved on the TCP wire"
        );
        assert!(
            !second_connection,
            "a single TC observation must cause exactly one TCP fallback, not a retry"
        );
    });
}

#[test]
fn complete_udp_response_does_not_open_a_tcp_connection() {
    block_on(async {
        let (udp, listener, address) = bind_udp_and_tcp();
        let id = 0x3a02;
        let query = query_wire(id);
        let expected = response_wire(id, 32);

        let udp_server = udp_reply_once(udp, expected.clone());
        let watchdog = tcp_connection_watchdog(listener);

        let response = policy_exchange_bounded(address, &query, open_context())
            .await
            .expect("a complete UDP response succeeds without any fallback");

        assert_eq!(response.transport(), Transport::Udp);
        assert!(!response.truncated());
        assert_eq!(response.request_id(), id);
        assert_eq!(response.response_id(), id);
        assert_eq!(response.wire(), expected.as_slice());

        assert!(
            !watchdog.join().expect("watchdog joined"),
            "a complete UDP response must not cause a TCP connection"
        );
        assert!(
            udp_server.join().expect("udp server joined").is_some(),
            "the UDP fixture must have served the query"
        );
    });
}

#[test]
fn undersized_tc_bit_datagram_is_terminal_malformed_without_tcp_fallback() {
    block_on(async {
        let (udp, listener, address) = bind_udp_and_tcp();
        let id = 0x3a03;
        let query = query_wire(id);

        let udp_server = udp_reply_once(udp, undersized_tc_datagram(id));
        let watchdog = tcp_connection_watchdog(listener);

        let error = policy_exchange_bounded(address, &query, open_context())
            .await
            .err()
            .expect("an undersized datagram with TC set is terminal malformed");

        assert_eq!(
            error,
            UpstreamError::MalformedResponse,
            "an undersized header must not be reclassified as a TC observation"
        );
        assert!(
            !watchdog.join().expect("watchdog joined"),
            "a malformed UDP datagram must not be followed by a TCP fallback"
        );
        assert!(
            udp_server.join().expect("udp server joined").is_some(),
            "the UDP fixture must have served the query"
        );
    });
}

#[test]
fn tcp_fallback_failure_preserves_prior_truncated_udp_context() {
    block_on(async {
        let (udp, listener, address) = bind_udp_and_tcp();
        let id = 0x3a04;
        let query = query_wire(id);

        let udp_server = udp_reply_once(udp, truncated_response_wire(id));
        let tcp_server = tcp_truncating_fallback_server(listener);

        let error = policy_exchange_bounded(address, &query, open_context())
            .await
            .err()
            .expect("a TCP fallback closed before a complete frame must fail");

        match &error {
            UpstreamError::TcpFallback { prior, cause } => {
                let prior: &TcpFallbackContext = prior;
                assert_eq!(
                    prior.request_id(),
                    id,
                    "the prior context must record the original query ID"
                );
                assert_eq!(
                    prior.response_id(),
                    id,
                    "the prior context must record the matching UDP response ID"
                );
                assert!(
                    prior.truncated(),
                    "the prior context must record the UDP TC=1 observation"
                );
                assert_eq!(
                    prior.side_effect(),
                    SideEffectState::Sent,
                    "the UDP query had already crossed the network"
                );
                assert_eq!(
                    **cause,
                    UpstreamError::TruncatedFrame,
                    "the nested cause must remain the typed TCP framing failure"
                );
            }
            other => panic!("expected UpstreamError::TcpFallback, got {other:?}"),
        }

        assert_eq!(
            error.side_effect(),
            SideEffectState::Sent,
            "the outer error must retain the prior UDP side-effect state"
        );

        let udp_seen = udp_server.join().expect("udp server joined");
        assert_eq!(
            udp_seen.as_deref(),
            Some(query.as_slice()),
            "the UDP leg must have carried the caller's unchanged query"
        );
        assert_eq!(
            tcp_server.join().expect("tcp server joined"),
            query,
            "the TCP fallback must have carried the byte-identical original query"
        );
    });
}

#[test]
fn close_cancels_in_flight_udp_exchange_with_closed_sent() {
    block_on(async {
        let (udp, _listener, address) = bind_udp_and_tcp();
        let id = 0x3b01;
        let query = query_wire(id);

        // The server receives the query and deliberately never answers, so the
        // UDP leg stays parked in receive until the owner close wakes it.
        let (server, query_seen) = udp_receive_without_reply(udp);
        let policy = Arc::new(UdpTcpPolicy::new(udp_endpoint(address)));
        let exchange_task = {
            let policy = Arc::clone(&policy);
            let query = query.clone();
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                policy.exchange(request, open_context()).await
            })
        };

        timeout(TEST_TIMEOUT, query_seen)
            .await
            .expect("query observed bounded")
            .expect("query observed");
        assert_eq!(
            policy.in_flight_exchanges(),
            1,
            "the in-flight UDP leg must be reported by the composite policy"
        );

        // Awaitable close cancels the owner scope and drains the registration;
        // the parked exchange returns the typed owner-close error.
        let close_result = timeout(TEST_TIMEOUT, policy.close())
            .await
            .expect("composite close must be awaitable and bounded");
        assert_eq!(close_result, CloseResult::Closed);

        let error = timeout(TEST_TIMEOUT, exchange_task)
            .await
            .expect("exchange bounded after owner close")
            .expect("exchange joined")
            .err()
            .expect("owner close terminates the in-flight exchange");
        assert_eq!(error, UpstreamError::Closed(SideEffectState::Sent));
        assert_eq!(
            policy.in_flight_exchanges(),
            0,
            "close must drain every in-flight composite exchange"
        );

        // A second close observes the existing terminal state instead of
        // starting a new drain.
        let second_close = timeout(TEST_TIMEOUT, policy.close())
            .await
            .expect("second close bounded");
        assert_eq!(second_close, CloseResult::AlreadyClosed);
        assert_eq!(policy.in_flight_exchanges(), 0);

        server.join().expect("udp server joined");
    });
}

#[test]
fn exchange_after_composite_close_is_rejected_without_udp_send() {
    block_on(async {
        let (udp, listener, address) = bind_udp_and_tcp();
        let id = 0x3b02;
        let query = query_wire(id);

        let policy = UdpTcpPolicy::new(udp_endpoint(address));
        assert_eq!(policy.close().await, CloseResult::Closed);
        assert_eq!(policy.in_flight_exchanges(), 0);

        // Bounded negative observations started before the rejected exchange:
        // no datagram may be sent and the TCP fallback must never be reached.
        let udp_watchdog = udp_datagram_watchdog(udp);
        let tcp_watchdog = tcp_connection_watchdog(listener);

        let error = policy_exchange_on(&policy, &query, open_context())
            .await
            .err()
            .expect("a closed composite policy rejects new work");
        assert_eq!(error, UpstreamError::Closed(SideEffectState::NotSent));
        assert_eq!(
            policy.in_flight_exchanges(),
            0,
            "a rejected exchange must never register"
        );

        assert!(
            !udp_watchdog.join().expect("udp watchdog joined"),
            "a closed composite policy must not send another UDP query"
        );
        assert!(
            !tcp_watchdog.join().expect("tcp watchdog joined"),
            "a rejected exchange must not reach the TCP fallback"
        );
    });
}

#[test]
fn close_drains_in_flight_tcp_fallback_with_nested_closed_sent() {
    block_on(async {
        let (udp, listener, address) = bind_udp_and_tcp();
        let id = 0x3b03;
        let query = query_wire(id);

        // The UDP leg serves a valid TC observation, which routes the exchange
        // into exactly one TCP fallback whose server reads the framed query and
        // then holds the connection open without responding.
        let udp_server = udp_reply_once(udp, truncated_response_wire(id));
        let (tcp_server, fallback_read, release_fallback) = tcp_holding_fallback_server(listener);

        let policy = Arc::new(UdpTcpPolicy::new(udp_endpoint(address)));
        let exchange_task = {
            let policy = Arc::clone(&policy);
            let query = query.clone();
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                policy.exchange(request, open_context()).await
            })
        };

        timeout(TEST_TIMEOUT, fallback_read)
            .await
            .expect("fallback query read bounded")
            .expect("the TCP fallback must read the framed query");
        assert_eq!(
            policy.in_flight_exchanges(),
            1,
            "the in-flight TCP fallback must be reported by the composite policy"
        );

        let close_result = timeout(TEST_TIMEOUT, policy.close())
            .await
            .expect("close must return within the bounded test timeout");
        assert_eq!(close_result, CloseResult::Closed);

        let error = timeout(TEST_TIMEOUT, exchange_task)
            .await
            .expect("fallback exchange bounded after close")
            .expect("fallback exchange joined")
            .err()
            .expect("owner close terminates the in-flight fallback");
        match &error {
            UpstreamError::TcpFallback { prior, cause } => {
                let prior: &TcpFallbackContext = prior;
                assert_eq!(prior.request_id(), id);
                assert_eq!(prior.response_id(), id);
                assert!(prior.truncated());
                assert_eq!(
                    prior.side_effect(),
                    SideEffectState::Sent,
                    "the UDP query had already crossed the network"
                );
                assert_eq!(
                    **cause,
                    UpstreamError::Closed(SideEffectState::Sent),
                    "the nested TCP cause must remain the typed owner-close error"
                );
            }
            other => panic!("expected UpstreamError::TcpFallback, got {other:?}"),
        }
        assert_eq!(
            error.side_effect(),
            SideEffectState::Sent,
            "the composite error must retain the overall Sent state"
        );
        assert_eq!(
            policy.in_flight_exchanges(),
            0,
            "close must drain the fallback registration"
        );

        release_fallback
            .send(())
            .expect("release the held fallback connection");
        assert_eq!(
            tcp_server.join().expect("tcp server joined"),
            query,
            "the TCP fallback must have carried the byte-identical original query"
        );
        assert_eq!(
            udp_server.join().expect("udp server joined").as_deref(),
            Some(query.as_slice()),
            "the UDP leg must have carried the caller's unchanged query"
        );
    });
}

#[test]
fn composite_close_is_idempotent_on_an_unused_policy() {
    block_on(async {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, 9));
        let policy = UdpTcpPolicy::new(udp_endpoint(address));

        assert_eq!(policy.close().await, CloseResult::Closed);
        assert_eq!(policy.in_flight_exchanges(), 0);

        assert_eq!(policy.close().await, CloseResult::AlreadyClosed);
        assert_eq!(policy.close().await, CloseResult::AlreadyClosed);
        assert_eq!(policy.in_flight_exchanges(), 0);
    });
}

#[test]
fn cancellation_before_the_udp_tc_observation_prevents_any_tcp_fallback() {
    block_on(async {
        let (udp, listener, address) = bind_udp_and_tcp();
        let id = 0x3c01;
        let query = query_wire(id);

        // The UDP server receives the query, announces it, and holds the valid
        // TC response until the test explicitly releases it. That two-phase
        // handshake lets caller cancellation become effective strictly before
        // the TC observation, with no equal-sleep assumption.
        let (udp_server, query_seen, release_udp) =
            udp_hold_then_reply(udp, truncated_response_wire(id));
        // The watchdog is the negative bound: a cancelled exchange must never
        // reach the TCP fallback for the whole observation window.
        let tcp_watchdog = tcp_connection_watchdog(listener);

        let cancellation = TransportCancellation::new();
        // The deadline stays far in the future, so only caller cancellation can
        // terminate the exchange.
        let context = ExchangeContext::new(
            Instant::now() + Duration::from_secs(30),
            cancellation.clone(),
        );
        let policy = Arc::new(UdpTcpPolicy::new(udp_endpoint(address)));
        let exchange_task = {
            let policy = Arc::clone(&policy);
            let query = query.clone();
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                policy.exchange(request, context).await
            })
        };

        timeout(TEST_TIMEOUT, query_seen)
            .await
            .expect("query observed bounded")
            .expect("query observed");
        assert_eq!(
            policy.in_flight_exchanges(),
            1,
            "the query has been sent, so the UDP leg is registered in flight"
        );

        // Cancellation is requested while the UDP leg is parked in receive and
        // before the server is allowed to send the valid TC response. A later
        // implementation that reads the TC observation first and then starts a
        // fallback would fail here: cancellation must win before any TCP work.
        cancellation.cancel();
        release_udp.send(()).expect("release the held UDP reply");

        let error = timeout(TEST_TIMEOUT, exchange_task)
            .await
            .expect("cancelled exchange bounded")
            .expect("exchange joined")
            .err()
            .expect("cancellation before the TC observation terminates the exchange");
        assert_eq!(
            error,
            UpstreamError::Cancelled(SideEffectState::Sent),
            "the query was already sent, so cancellation reports Sent before any TCP work"
        );
        assert_eq!(
            policy.in_flight_exchanges(),
            0,
            "the cancelled exchange must drain its registration"
        );

        assert!(
            !tcp_watchdog.join().expect("tcp watchdog joined"),
            "cancellation before the TC observation must never open a TCP connection"
        );
        assert_eq!(
            udp_server.join().expect("udp server joined").as_deref(),
            Some(query.as_slice()),
            "the UDP leg must have carried the caller's unchanged query"
        );
    });
}

#[test]
fn tcp_fallback_honors_the_remaining_original_absolute_deadline() {
    block_on(async {
        let (udp, listener, address) = bind_udp_and_tcp();
        let id = 0x3c02;
        let query = query_wire(id);

        // The UDP leg serves a valid TC observation. The TCP fallback then
        // accepts, reads the framed query, signals `fallback_read`, and holds
        // the connection open without ever responding.
        let udp_server = udp_reply_once(udp, truncated_response_wire(id));
        let (tcp_server, fallback_read, release_fallback) = tcp_holding_fallback_server(listener);

        // One original absolute deadline for the whole composite exchange. It
        // is short enough that the unanswered fallback must observe it, and the
        // outer bounded assertion below fails instead of hanging if an
        // implementation resets or ignores it.
        let deadline = Instant::now() + SHORT_ABSOLUTE_DEADLINE;
        let context = ExchangeContext::new(deadline, TransportCancellation::new());
        let policy = Arc::new(UdpTcpPolicy::new(udp_endpoint(address)));
        let exchange_task = {
            let policy = Arc::clone(&policy);
            let query = query.clone();
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                policy.exchange(request, context).await
            })
        };

        // The fallback is genuinely in flight and has read the framed query, so
        // only the original absolute deadline can terminate the exchange.
        timeout(TEST_TIMEOUT, fallback_read)
            .await
            .expect("fallback query read bounded")
            .expect("the TCP fallback must read the framed query");

        let error = timeout(TEST_TIMEOUT, exchange_task)
            .await
            .expect("the fallback must observe the original bounded deadline instead of hanging")
            .expect("exchange joined")
            .err()
            .expect("an unanswered held fallback must fail");

        // Always release and join the held server immediately after the bounded
        // observation, before any assertion can panic, so no server thread is
        // left parked on a failing test.
        release_fallback
            .send(())
            .expect("release the held fallback connection");
        let tcp_received = tcp_server.join().expect("tcp server joined");
        let udp_received = udp_server.join().expect("udp server joined");

        assert!(
            Instant::now() >= deadline,
            "the exchange can only have ended by reaching the original absolute deadline"
        );
        assert_eq!(
            tcp_received, query,
            "the TCP fallback must have carried the byte-identical original query"
        );
        assert_eq!(
            udp_received.as_deref(),
            Some(query.as_slice()),
            "the UDP leg must have carried the caller's unchanged query"
        );

        match &error {
            UpstreamError::TcpFallback { prior, cause } => {
                let prior: &TcpFallbackContext = prior;
                assert_eq!(prior.request_id(), id);
                assert_eq!(prior.response_id(), id);
                assert!(
                    prior.truncated(),
                    "the prior context must record the UDP TC=1 observation"
                );
                assert_eq!(
                    prior.side_effect(),
                    SideEffectState::Sent,
                    "the UDP query had already crossed the network"
                );
                assert_eq!(
                    **cause,
                    UpstreamError::DeadlineExceeded(SideEffectState::Sent),
                    "the nested TCP cause must be the original absolute deadline, not a reset timeout"
                );
            }
            other => panic!("expected UpstreamError::TcpFallback, got {other:?}"),
        }
        assert_eq!(
            error.side_effect(),
            SideEffectState::Sent,
            "the composite error must retain the overall Sent state"
        );
        assert_eq!(
            policy.in_flight_exchanges(),
            0,
            "the deadline-terminated fallback must drain its registration"
        );
    });
}

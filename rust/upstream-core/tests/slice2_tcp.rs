//! Slice 2 contract tests for the fresh plain-TCP exchange primitive.
//!
//! Every test runs a deterministic in-process `TcpListener` on an ephemeral
//! IPv4 loopback port and drives real `Upstream::exchange` calls. The server
//! reads exactly one framed query and writes exactly one framed response,
//! deliberately in small chunks, so the client must reassemble stream
//! fragments into a single message. No test depends on wall time beyond the
//! bounded exchange timeouts and the bounded accept deadline.

use std::future::Future;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use mosdns_upstream_core::{
    CloseResult, CloseTransition, Endpoint, ExchangeContext, ExchangeRequest, SideEffectState,
    Transport, TransportCancellation, Upstream, UpstreamError,
};
use tokio::sync::oneshot;
use tokio::task::{JoinHandle, spawn_blocking};
use tokio::time::timeout;

/// Bounds every exchange so a missing or broken socket path cannot hang CI.
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Bounded window used to prove a terminal control error never reconnects.
const NO_RETRY_WINDOW: Duration = Duration::from_millis(250);

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

/// Binds a fresh blocking TCP listener on the IPv4 loopback.
fn bind_listener() -> (TcpListener, SocketAddr) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind ipv4 loopback");
    let address = listener.local_addr().expect("local address");
    (listener, address)
}

fn tcp_endpoint(address: SocketAddr) -> Endpoint {
    Endpoint::new(address, Transport::Tcp).expect("numeric tcp endpoint")
}

fn open_context() -> ExchangeContext {
    ExchangeContext::new(
        Instant::now() + Duration::from_secs(30),
        TransportCancellation::new(),
    )
}

/// Accepts one connection within `budget`, or reports that none arrived.
///
/// The listener is switched to non-blocking mode so a regression that never
/// opens a fresh connection fails the test instead of hanging it.
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
    assert!(length > 0, "the server must receive a non-zero frame");
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).expect("read frame body");
    body
}

/// Writes one framed DNS message in deterministic `chunk`-sized pieces.
fn write_framed_in_chunks(stream: &mut TcpStream, body: &[u8], chunk: usize) {
    let length = u16::try_from(body.len()).expect("response body fits the u16 prefix");
    let mut frame = Vec::with_capacity(body.len() + 2);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(body);
    for piece in frame.chunks(chunk) {
        stream.write_all(piece).expect("write frame piece");
        stream.flush().expect("flush frame piece");
    }
}

#[test]
fn fresh_numeric_tcp_exchange_returns_matching_framed_response() {
    block_on(async {
        let (listener, address) = bind_listener();
        let id = 0x1234;
        let query = query_wire(id);
        let expected_query = query.clone();
        let expected = response_wire(id, 1);
        let reply = expected.clone();
        let server_query = expected_query.clone();
        let server = spawn_blocking(move || {
            let mut stream =
                accept_within(&listener, TEST_TIMEOUT).expect("exactly one fresh connection");
            let received = read_framed(&mut stream);
            assert_eq!(
                received, server_query,
                "the server receives the exact, unchanged query frame"
            );
            assert_eq!(
                u16::from_be_bytes([received[0], received[1]]),
                id,
                "the original DNS ID is preserved on the wire"
            );
            // Fragment the two-byte prefix and the body into one-byte pieces so
            // the client must reassemble a single message from stream chunks.
            write_framed_in_chunks(&mut stream, &reply, 1);
        });

        let upstream = Upstream::new(tcp_endpoint(address));
        let request = ExchangeRequest::new(&query).expect("valid query");
        let response = timeout(TEST_TIMEOUT, upstream.exchange(request, open_context()))
            .await
            .expect("tcp exchange bounded")
            .expect("tcp exchange succeeds");

        assert_eq!(
            query, expected_query,
            "the caller query bytes must not change"
        );
        assert_eq!(response.transport(), Transport::Tcp);
        assert_eq!(response.request_id(), id);
        assert_eq!(response.response_id(), id);
        assert!(!response.truncated());
        assert_eq!(response.wire(), expected.as_slice());

        server.await.expect("server task joined");
    });
}

#[test]
fn sequential_exchanges_open_a_fresh_connection_each_time() {
    block_on(async {
        let (listener, address) = bind_listener();
        let first_id = 0x0a01;
        let second_id = 0x0b02;
        let expected = [response_wire(first_id, 8), response_wire(second_id, 9)];
        let replies = expected.clone();
        let server: JoinHandle<()> = spawn_blocking(move || {
            // A fresh connection must arrive for each exchange; if the client
            // reused one stream the second accept would time out and fail here.
            for reply in replies {
                let mut stream = accept_within(&listener, TEST_TIMEOUT)
                    .expect("a fresh connection per exchange");
                let received = read_framed(&mut stream);
                assert_eq!(
                    received,
                    query_wire(u16::from_be_bytes([reply[0], reply[1]])),
                    "each connection carries exactly its own unchanged query"
                );
                write_framed_in_chunks(&mut stream, &reply, 3);
            }
        });

        let upstream = Upstream::new(tcp_endpoint(address));
        for (id, want) in [(first_id, &expected[0]), (second_id, &expected[1])] {
            let query = query_wire(id);
            let request = ExchangeRequest::new(&query).expect("valid query");
            let response = timeout(TEST_TIMEOUT, upstream.exchange(request, open_context()))
                .await
                .expect("tcp exchange bounded")
                .expect("tcp exchange succeeds");
            assert_eq!(response.transport(), Transport::Tcp);
            assert_eq!(response.request_id(), id);
            assert_eq!(response.response_id(), id);
            assert_eq!(response.wire(), want.as_slice());
        }

        server.await.expect("server task joined");
    });
}

#[test]
fn concurrent_exchanges_use_separate_streams_and_owned_responses() {
    block_on(async {
        let (listener, address) = bind_listener();
        let first_id = 0x1a01;
        let second_id = 0x1a02;
        let first_expected = response_wire(first_id, 20);
        let second_expected = response_wire(second_id, 21);
        let first_reply = first_expected.clone();
        let second_reply = second_expected.clone();
        // Both handlers must have received their query before either replies, so
        // the two exchanges are genuinely in flight at the same time.
        let barrier = Arc::new(Barrier::new(2));
        let server = spawn_blocking(move || {
            let mut handlers = Vec::new();
            for _ in 0..2 {
                let mut stream =
                    accept_within(&listener, TEST_TIMEOUT).expect("two separate connections");
                let barrier = Arc::clone(&barrier);
                let first = first_reply.clone();
                let second = second_reply.clone();
                handlers.push(std::thread::spawn(move || {
                    let received = read_framed(&mut stream);
                    let received_id = u16::from_be_bytes([received[0], received[1]]);
                    barrier.wait();
                    let reply = if received_id == first_id {
                        first
                    } else {
                        assert_eq!(received_id, second_id, "unexpected query id");
                        second
                    };
                    write_framed_in_chunks(&mut stream, &reply, 2);
                }));
            }
            for handler in handlers {
                handler.join().expect("handler joined");
            }
        });

        let upstream = Arc::new(Upstream::new(tcp_endpoint(address)));
        let first = {
            let upstream = Arc::clone(&upstream);
            let query = query_wire(first_id);
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                upstream.exchange(request, open_context()).await
            })
        };
        let second = {
            let upstream = Arc::clone(&upstream);
            let query = query_wire(second_id);
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                upstream.exchange(request, open_context()).await
            })
        };

        let first_response = timeout(TEST_TIMEOUT, first)
            .await
            .expect("first exchange bounded")
            .expect("first exchange joined")
            .expect("first exchange succeeds");
        let second_response = timeout(TEST_TIMEOUT, second)
            .await
            .expect("second exchange bounded")
            .expect("second exchange joined")
            .expect("second exchange succeeds");

        assert_eq!(first_response.transport(), Transport::Tcp);
        assert_eq!(first_response.response_id(), first_id);
        assert_eq!(first_response.wire(), first_expected.as_slice());
        assert_eq!(second_response.transport(), Transport::Tcp);
        assert_eq!(second_response.response_id(), second_id);
        assert_eq!(second_response.wire(), second_expected.as_slice());

        server.await.expect("server task joined");
    });
}

/// Accepts exactly one fresh connection, reads exactly one framed query,
/// deterministically signals that the query was consumed, then withholds any
/// response until the client closes the connection.
///
/// The returned flag is `true` only when the peer closed the stream (EOF)
/// within the bounded wait, which is the observable proof that a terminated
/// exchange dropped its fresh `TcpStream`. The server then also confirms, with
/// a short bounded accept probe, that the terminal error was never retried or
/// reconnected.
fn holding_server(
    listener: TcpListener,
    expected_query: Vec<u8>,
    consumed: oneshot::Sender<()>,
) -> JoinHandle<bool> {
    spawn_blocking(move || {
        let mut stream =
            accept_within(&listener, TEST_TIMEOUT).expect("exactly one fresh connection");
        let received = read_framed(&mut stream);
        assert_eq!(
            received, expected_query,
            "the server receives the exact, unchanged query frame"
        );
        consumed.send(()).expect("the test waits for consumption");

        stream
            .set_read_timeout(Some(TEST_TIMEOUT))
            .expect("set read timeout");
        let mut probe = [0u8; 1];
        let closed_by_peer = loop {
            match stream.read(&mut probe) {
                Ok(0) => break true,
                Ok(read) => panic!("unexpected {read} extra bytes after the query frame"),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    break false;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => panic!("probe read failed: {error}"),
            }
        };

        // A control-terminated exchange must never retry or reconnect.
        assert!(
            accept_within(&listener, NO_RETRY_WINDOW).is_none(),
            "a control-terminated tcp exchange must not retry or reconnect"
        );

        closed_by_peer
    })
}

#[test]
fn caller_cancellation_while_waiting_for_the_response_is_cancelled_with_sent() {
    block_on(async {
        let (listener, address) = bind_listener();
        let id = 0x2b01;
        let query = query_wire(id);
        let expected_query = query.clone();
        let (consumed_tx, consumed_rx) = oneshot::channel::<()>();
        let server = holding_server(listener, expected_query.clone(), consumed_tx);

        let cancellation = TransportCancellation::new();
        let context = ExchangeContext::new(
            Instant::now() + Duration::from_secs(30),
            cancellation.clone(),
        );
        let upstream = Arc::new(Upstream::new(tcp_endpoint(address)));
        let exchange = {
            let upstream = Arc::clone(&upstream);
            let query = query.clone();
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                upstream.exchange(request, context).await
            })
        };

        // The server has consumed the complete query frame, so the full write
        // already finished and the exchange is waiting for the framed response.
        // Cancelling only after that deterministic ordering makes the expected
        // side-effect state unambiguous (`Sent`, never `MaybeSent`).
        timeout(TEST_TIMEOUT, consumed_rx)
            .await
            .expect("query consumption observed bounded")
            .expect("query consumed");
        cancellation.cancel();

        let outcome = timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("cancelled tcp exchange bounded")
            .expect("exchange task joined");
        let error = outcome
            .err()
            .expect("caller cancellation terminates the response wait");
        assert_eq!(error, UpstreamError::Cancelled(SideEffectState::Sent));
        assert_eq!(error.side_effect(), SideEffectState::Sent);
        assert_eq!(
            query, expected_query,
            "the caller query bytes must not change"
        );

        assert!(
            server.await.expect("server task joined"),
            "the cancelled exchange drops its fresh stream"
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);
    });
}

#[test]
fn absolute_deadline_while_waiting_for_the_response_is_deadline_exceeded_with_sent() {
    block_on(async {
        let (listener, address) = bind_listener();
        let id = 0x2b02;
        let query = query_wire(id);
        let expected_query = query.clone();
        let (consumed_tx, consumed_rx) = oneshot::channel::<()>();
        let server = holding_server(listener, expected_query.clone(), consumed_tx);

        // One absolute deadline for the whole exchange. Local loopback connect
        // and write finish far below it, so the deadline can only win while the
        // response read is still waiting; that state is `Sent`.
        let context = ExchangeContext::new(
            Instant::now() + Duration::from_millis(500),
            TransportCancellation::new(),
        );
        let upstream = Arc::new(Upstream::new(tcp_endpoint(address)));
        let exchange = {
            let upstream = Arc::clone(&upstream);
            let query = query.clone();
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                upstream.exchange(request, context).await
            })
        };

        timeout(TEST_TIMEOUT, consumed_rx)
            .await
            .expect("query consumption observed bounded")
            .expect("query consumed");

        let outcome = timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("deadline-terminated tcp exchange bounded")
            .expect("exchange task joined");
        let error = outcome
            .err()
            .expect("the absolute deadline terminates the response wait");
        assert_eq!(
            error,
            UpstreamError::DeadlineExceeded(SideEffectState::Sent)
        );
        assert_eq!(error.side_effect(), SideEffectState::Sent);
        assert_eq!(
            query, expected_query,
            "the caller query bytes must not change"
        );

        assert!(
            server.await.expect("server task joined"),
            "the deadline-terminated exchange drops its fresh stream"
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);
    });
}

#[test]
fn owner_close_while_waiting_for_the_response_is_closed_with_sent() {
    block_on(async {
        let (listener, address) = bind_listener();
        let id = 0x2b03;
        let query = query_wire(id);
        let expected_query = query.clone();
        let (consumed_tx, consumed_rx) = oneshot::channel::<()>();
        let server = holding_server(listener, expected_query.clone(), consumed_tx);

        let upstream = Arc::new(Upstream::new(tcp_endpoint(address)));
        let exchange = {
            let upstream = Arc::clone(&upstream);
            let query = query.clone();
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                upstream.exchange(request, open_context()).await
            })
        };

        timeout(TEST_TIMEOUT, consumed_rx)
            .await
            .expect("query consumption observed bounded")
            .expect("query consumed");
        assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);

        let outcome = timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("closed tcp exchange bounded")
            .expect("exchange task joined");
        let error = outcome
            .err()
            .expect("owner close terminates the response wait");
        assert_eq!(error, UpstreamError::Closed(SideEffectState::Sent));
        assert_eq!(error.side_effect(), SideEffectState::Sent);
        assert_eq!(
            query, expected_query,
            "the caller query bytes must not change"
        );

        assert_eq!(upstream.close().await, CloseResult::Closed);
        assert_eq!(upstream.in_flight_exchanges(), 0);
        assert!(
            server.await.expect("server task joined"),
            "the closed exchange drops its fresh stream"
        );
    });
}

// ---------------------------------------------------------------------------
// Deterministic full-exchange terminal error/side-effect matrix
//
// Every test below drives the public `Upstream::exchange` entry point against a
// scripted one-connection loopback server. Each case asserts the typed terminal
// category, the `SideEffectState`, that the caller's borrowed query bytes are
// unchanged, that the in-flight registration was released, and (through the
// shared `scripted_server` probe) that the terminal error never reconnects or
// retries. No case depends on a sleep-only timing guess.
// ---------------------------------------------------------------------------

/// A query one byte larger than the two-byte DNS-over-TCP length prefix can
/// encode, but still a wire-legal request for the `dns-core` query boundary.
///
/// `dns-core`'s query oracle validates the header and exactly one question and
/// does not descend into the single permitted additional record, so declaring
/// `ARCOUNT = 1` and padding the wire lets the request boundary accept it while
/// the outbound frame gate must reject it.
fn oversized_query(id: u16) -> Vec<u8> {
    let mut wire = query_wire(id);
    wire[10..12].copy_from_slice(&1u16.to_be_bytes()); // ARCOUNT = 1
    wire.resize(usize::from(u16::MAX) + 1, 0);
    wire
}

/// A complete frame with QR set and the request ID, but an invalid `dns-core`
/// wire: the header declares one question and then ends.
fn invalid_dns_response_wire(id: u16) -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&[0x81, 0x80]); // QR=1, RD=1, RA=1
    wire.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT claims a question
    wire.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT
    wire.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    wire.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT
    wire
}

/// Accepts exactly one fresh connection, runs `handle`, drops the stream, then
/// proves with one bounded accept probe that the terminal error never retried
/// or reconnected.
fn scripted_server<F>(listener: TcpListener, handle: F) -> JoinHandle<()>
where
    F: FnOnce(&mut TcpStream) + Send + 'static,
{
    spawn_blocking(move || {
        let mut stream =
            accept_within(&listener, TEST_TIMEOUT).expect("exactly one fresh connection");
        handle(&mut stream);
        drop(stream);
        assert!(
            accept_within(&listener, NO_RETRY_WINDOW).is_none(),
            "a terminal tcp error must not retry or reconnect"
        );
    })
}

/// Drives exactly one bounded full exchange and returns its terminal error.
///
/// Every error-matrix case asserts the same two caller-visible invariants: the
/// borrowed query bytes are unchanged and the in-flight registration is
/// released regardless of the error category.
async fn exchange_error(upstream: &Upstream, query: &[u8], expected_query: &[u8]) -> UpstreamError {
    let request = ExchangeRequest::new(query).expect("valid query");
    let error = timeout(TEST_TIMEOUT, upstream.exchange(request, open_context()))
        .await
        .expect("tcp exchange bounded")
        .err()
        .expect("the exchange must terminate with an error");
    assert_eq!(
        query, expected_query,
        "the caller query bytes must not change"
    );
    assert_eq!(
        upstream.in_flight_exchanges(),
        0,
        "every terminal error path must release its in-flight registration"
    );
    error
}

/// One EOF-before-a-complete-prefix sub-case: the server reads the query, writes
/// `prefix` (fewer than two bytes), then drops the stream.
async fn eof_prefix_case(id: u16, prefix: &[u8]) {
    assert!(prefix.len() < 2, "the prefix must remain incomplete");
    let (listener, address) = bind_listener();
    let query = query_wire(id);
    let expected_query = query.clone();
    let server_query = expected_query.clone();
    let prefix = prefix.to_vec();
    let server = scripted_server(listener, move |stream| {
        let received = read_framed(stream);
        assert_eq!(received, server_query, "the exact query frame arrives");
        if !prefix.is_empty() {
            stream.write_all(&prefix).expect("write partial prefix");
            stream.flush().expect("flush partial prefix");
        }
        // Returning drops the stream, so the peer observes EOF.
    });

    let upstream = Upstream::new(tcp_endpoint(address));
    let error = exchange_error(&upstream, &query, &expected_query).await;
    assert_eq!(error, UpstreamError::TruncatedFrame);
    assert_eq!(error.side_effect(), SideEffectState::Sent);

    server.await.expect("server task joined");
}

#[test]
fn outbound_query_over_u16_max_is_frame_too_large_before_any_connection() {
    block_on(async {
        let (listener, address) = bind_listener();
        let query = oversized_query(0x3a01);
        assert_eq!(query.len(), usize::from(u16::MAX) + 1);
        let expected_query = query.clone();

        let upstream = Upstream::new(tcp_endpoint(address));
        // The request boundary accepts the padded wire, so the rejection under
        // test is the outbound frame-size gate, not query validation.
        let request = ExchangeRequest::new(&query).expect("padded query is wire-legal");
        let error = timeout(TEST_TIMEOUT, upstream.exchange(request, open_context()))
            .await
            .expect("oversize exchange bounded")
            .err()
            .expect("an unframeable query must fail");
        assert_eq!(error, UpstreamError::FrameTooLarge);
        assert_eq!(error.side_effect(), SideEffectState::NotSent);
        assert_eq!(
            query, expected_query,
            "the caller query bytes must not change"
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);

        // One bounded accept probe: the size gate must reject before connect.
        assert!(
            accept_within(&listener, NO_RETRY_WINDOW).is_none(),
            "an oversize query must never open a TCP connection"
        );
    });
}

#[test]
fn zero_length_inbound_prefix_is_malformed_response_with_sent() {
    block_on(async {
        let (listener, address) = bind_listener();
        let id = 0x3a02;
        let query = query_wire(id);
        let expected_query = query.clone();
        let server_query = expected_query.clone();
        let server = scripted_server(listener, move |stream| {
            let received = read_framed(stream);
            assert_eq!(received, server_query, "the exact query frame arrives");
            stream.write_all(&[0x00, 0x00]).expect("write zero prefix");
            stream.flush().expect("flush zero prefix");
        });

        let upstream = Upstream::new(tcp_endpoint(address));
        let error = exchange_error(&upstream, &query, &expected_query).await;
        assert_eq!(error, UpstreamError::MalformedResponse);
        assert_eq!(error.side_effect(), SideEffectState::Sent);

        server.await.expect("server task joined");
    });
}

#[test]
fn eof_during_the_prefix_is_truncated_frame_with_sent() {
    block_on(async {
        // Zero prefix bytes (immediate EOF) and exactly one prefix byte.
        eof_prefix_case(0x3a03, &[]).await;
        eof_prefix_case(0x3a04, &[0x00]).await;
    });
}

#[test]
fn eof_during_the_declared_body_is_truncated_frame_with_sent() {
    block_on(async {
        let (listener, address) = bind_listener();
        let id = 0x3a05;
        let query = query_wire(id);
        let expected_query = query.clone();
        let server_query = expected_query.clone();
        let server = scripted_server(listener, move |stream| {
            let received = read_framed(stream);
            assert_eq!(received, server_query, "the exact query frame arrives");
            // Declare eight body bytes but send only three, then close.
            stream
                .write_all(&[0x00, 0x08, 0x01, 0x02, 0x03])
                .expect("write partial body");
            stream.flush().expect("flush partial body");
        });

        let upstream = Upstream::new(tcp_endpoint(address));
        let error = exchange_error(&upstream, &query, &expected_query).await;
        assert_eq!(error, UpstreamError::TruncatedFrame);
        assert_eq!(error.side_effect(), SideEffectState::Sent);

        server.await.expect("server task joined");
    });
}

#[test]
fn complete_qr_clear_response_is_malformed_response_with_sent() {
    block_on(async {
        let (listener, address) = bind_listener();
        let id = 0x3a06;
        let query = query_wire(id);
        let expected_query = query.clone();
        // A complete, correctly framed message whose QR bit is clear: it is a
        // query, not a response, so header inspection must reject it.
        let reply = query_wire(id);
        let server_query = expected_query.clone();
        let server = scripted_server(listener, move |stream| {
            let received = read_framed(stream);
            assert_eq!(received, server_query, "the exact query frame arrives");
            write_framed_in_chunks(stream, &reply, 2);
        });

        let upstream = Upstream::new(tcp_endpoint(address));
        let error = exchange_error(&upstream, &query, &expected_query).await;
        assert_eq!(error, UpstreamError::MalformedResponse);
        assert_eq!(error.side_effect(), SideEffectState::Sent);

        server.await.expect("server task joined");
    });
}

#[test]
fn complete_invalid_dns_wire_is_malformed_response_with_sent() {
    block_on(async {
        let (listener, address) = bind_listener();
        let id = 0x3a07;
        let query = query_wire(id);
        let expected_query = query.clone();
        let reply = invalid_dns_response_wire(id);
        let server_query = expected_query.clone();
        let server = scripted_server(listener, move |stream| {
            let received = read_framed(stream);
            assert_eq!(received, server_query, "the exact query frame arrives");
            write_framed_in_chunks(stream, &reply, 3);
        });

        let upstream = Upstream::new(tcp_endpoint(address));
        let error = exchange_error(&upstream, &query, &expected_query).await;
        assert_eq!(error, UpstreamError::MalformedResponse);
        assert_eq!(error.side_effect(), SideEffectState::Sent);

        server.await.expect("server task joined");
    });
}

#[test]
fn complete_wrong_transaction_id_response_is_response_mismatch_with_sent() {
    block_on(async {
        let (listener, address) = bind_listener();
        let request_id = 0x3a08;
        let wrong_id = 0x9999;
        let query = query_wire(request_id);
        let expected_query = query.clone();
        // A complete, dns-core-valid response that answers a different query.
        let reply = response_wire(wrong_id, 7);
        let server_query = expected_query.clone();
        let server = scripted_server(listener, move |stream| {
            let received = read_framed(stream);
            assert_eq!(received, server_query, "the exact query frame arrives");
            write_framed_in_chunks(stream, &reply, 1);
        });

        let upstream = Upstream::new(tcp_endpoint(address));
        let error = exchange_error(&upstream, &query, &expected_query).await;
        assert_eq!(error, UpstreamError::ResponseMismatch);
        assert_eq!(error.side_effect(), SideEffectState::Sent);

        server.await.expect("server task joined");
    });
}

#[test]
fn refused_connect_is_connect_with_not_sent_and_releases_registration() {
    block_on(async {
        // Bind then immediately release an ephemeral loopback port so the TCP
        // connect is refused deterministically instead of depending on a
        // well-known port being closed.
        let probe = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind ephemeral probe");
        let address = probe.local_addr().expect("probe address");
        drop(probe);

        let upstream = Upstream::new(tcp_endpoint(address));
        let query = query_wire(0x3a09);
        let expected_query = query.clone();
        let error = exchange_error(&upstream, &query, &expected_query).await;
        assert_eq!(error, UpstreamError::Connect);
        assert_eq!(error.side_effect(), SideEffectState::NotSent);
    });
}

// The remaining matrix row — a non-EOF response read failure staying
// `Receive(Sent)` — is not exposed at this integration level. A stable
// `SO_LINGER` is unavailable (`TcpStream::set_linger` is still unstable), so a
// loopback peer can only end the stream with FIN, which the framing reader
// correctly reports as `TruncatedFrame`; forcing an RST would depend on an
// unread-data race rather than a deterministic fixture. The deterministic proof
// is the in-crate unit test
// `tcp::tests::read_frame_maps_non_eof_read_failure_to_receive_with_sent_state`,
// which drives the framing reader with a `ConnectionReset` error and asserts
// `Receive(Sent)`. At the full-exchange level the same `Sent` state is
// guaranteed because the response read runs only after the complete framed
// write, exactly as the matrix requires.

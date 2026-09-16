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
    Endpoint, ExchangeContext, ExchangeRequest, Transport, TransportCancellation, Upstream,
};
use tokio::task::{JoinHandle, spawn_blocking};
use tokio::time::timeout;

/// Bounds every exchange so a missing or broken socket path cannot hang CI.
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

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

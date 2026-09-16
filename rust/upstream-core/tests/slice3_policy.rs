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
//! Every server is bounded: the UDP fixture sets a socket read timeout, TCP
//! accepts use a nonblocking deadline, and accepted TCP streams carry read and
//! write timeouts. A missing or broken client therefore fails a test rather
//! than parking a background thread.

use std::future::Future;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

use mosdns_upstream_core::{
    Endpoint, ExchangeContext, ExchangeRequest, ExchangeResponse, Transport, TransportCancellation,
    UdpTcpPolicy, UpstreamError,
};
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

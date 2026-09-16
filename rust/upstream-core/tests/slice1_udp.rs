//! Slice 1 RED contract tests for the reviewed one-exchange/one-socket UDP
//! primitive.
//!
//! These tests intentionally reference the `Upstream::exchange` entry point
//! sketched in `design.md` section 3 before Slice 1 implements it. This step
//! adds tests only: the target is expected to fail to compile (RED) until the
//! UDP primitive exists. No production UDP/network code is added here.
//!
//! The fixtures are deterministic: every exchange runs on an ephemeral IPv4
//! loopback socket with a bounded current-thread runtime, mock servers use
//! blocking std sockets on Tokio's blocking pool, and no test depends on wall
//! time beyond small fixed ordering sleeps.

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, mpsc};

use std::time::{Duration, Instant};

use mosdns_upstream_core::{
    CloseResult, CloseTransition, Endpoint, ExchangeContext, ExchangeRequest, ExchangeResponse,
    SideEffectState, Transport, TransportCancellation, Upstream, UpstreamError,
};
use tokio::sync::oneshot;
use tokio::task::{JoinHandle, spawn_blocking};
use tokio::time::{sleep, timeout};

/// The largest legal IPv4 UDP payload: 65535 wire bytes minus the 20-byte IPv4
/// and 8-byte UDP headers.
const LEGAL_UDP_PAYLOAD: usize = 65_507;

/// The largest UDP datagram this host will actually deliver on loopback.
/// Linux carries the full legal IPv4 payload; macOS caps
/// `net.inet.udp.maxdgram` at 9216 by default, so its probe stays just below
/// that cap while still exceeding the old 4095-byte receive buffer.
#[cfg(target_os = "linux")]
const CAPACITY_PROBE: usize = LEGAL_UDP_PAYLOAD;
#[cfg(not(target_os = "linux"))]
const CAPACITY_PROBE: usize = 9_000;

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

/// A complete, dns-core-valid response padded to exactly `target_len` bytes.
fn large_response_wire(id: u16, target_len: usize) -> Vec<u8> {
    const QUESTION: &[u8] = &[
        0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0x03, b'o', b'r', b'g', 0x00, 0x00, 0x01,
        0x00, 0x01,
    ];
    const ANSWER: &[u8] = &[
        0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3c, 0x00, 0x04, 192, 0, 2, 1,
    ];
    let available = target_len.saturating_sub(12 + QUESTION.len());
    let answer_count = u16::try_from(available / ANSWER.len()).expect("answer count fits u16");
    let mut wire = Vec::with_capacity(target_len);
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&[0x81, 0x80]); // QR=1, RD=1, RA=1
    wire.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    wire.extend_from_slice(&answer_count.to_be_bytes()); // ANCOUNT
    wire.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    wire.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT
    wire.extend_from_slice(QUESTION);
    for _ in 0..answer_count {
        wire.extend_from_slice(ANSWER);
    }
    // Trailing bytes after the last declared record are legal and tolerated by
    // dns-core, so zero padding can reach the exact target length.
    wire.resize(target_len, 0);
    wire
}

/// Binds a fresh UDP socket on the IPv4 loopback with an ephemeral port.
fn bind_ipv4() -> (UdpSocket, SocketAddr) {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind ipv4 loopback");
    let address = socket.local_addr().expect("local address");
    (socket, address)
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

/// Receives one datagram and sends `reply` back to its source.
fn reply_once(socket: UdpSocket, reply: Vec<u8>) -> JoinHandle<()> {
    spawn_blocking(move || {
        let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
        let (_, peer) = socket.recv_from(&mut buffer).expect("receive query");
        socket.send_to(&reply, peer).expect("send reply");
    })
}

/// Runs one exchange through the reviewed public entry point with a bound.
async fn exchange_bounded(
    address: SocketAddr,
    query: &[u8],
    context: ExchangeContext,
) -> Result<ExchangeResponse, UpstreamError> {
    let upstream = Upstream::new(udp_endpoint(address));
    let request = ExchangeRequest::new(query).expect("valid query");
    timeout(TEST_TIMEOUT, upstream.exchange(request, context))
        .await
        .expect("exchange must finish within the bounded test timeout")
}

#[test]
fn numeric_ipv4_loopback_exchange_returns_matching_response() {
    block_on(async {
        let (server, address) = bind_ipv4();
        let id = 0x1234;
        let expected = response_wire(id, 1);
        let server_task = reply_once(server, expected.clone());

        let query = query_wire(id);
        let response = exchange_bounded(address, &query, open_context())
            .await
            .expect("udp exchange succeeds");

        assert_eq!(response.transport(), Transport::Udp);
        assert_eq!(response.request_id(), id);
        assert_eq!(response.response_id(), id);
        assert!(!response.truncated());
        assert_eq!(response.wire(), expected.as_slice());

        server_task.await.expect("server task joined");
    });
}

#[test]
fn numeric_ipv6_loopback_exchange_returns_matching_response() {
    block_on(async {
        let Ok(server) = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)) else {
            return; // host without an IPv6 loopback
        };
        let address = server.local_addr().expect("local address");
        let id = 0x5678;
        let expected = response_wire(id, 2);
        let server_task = reply_once(server, expected.clone());

        let query = query_wire(id);
        let response = exchange_bounded(address, &query, open_context())
            .await
            .expect("udp exchange succeeds");

        assert_eq!(response.request_id(), id);
        assert_eq!(response.response_id(), id);
        assert_eq!(response.wire(), expected.as_slice());

        server_task.await.expect("server task joined");
    });
}

#[test]
fn borrowed_query_bytes_and_original_id_are_unchanged() {
    block_on(async {
        let (server, address) = bind_ipv4();
        let id = 0xabcd;
        let query = query_wire(id);
        let snapshot = query.clone();
        let server_snapshot = snapshot.clone();
        let expected = response_wire(id, 3);
        let reply = expected.clone();
        let server_task = spawn_blocking(move || {
            let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
            let (read, peer) = server.recv_from(&mut buffer).expect("receive query");
            assert_eq!(&buffer[..read], server_snapshot.as_slice());
            server.send_to(&reply, peer).expect("send reply");
        });

        let response = exchange_bounded(address, &query, open_context())
            .await
            .expect("udp exchange succeeds");

        assert_eq!(query, snapshot, "caller query bytes must not change");
        assert_eq!(response.request_id(), id);
        assert_eq!(response.response_id(), id);
        assert_eq!(response.wire(), expected.as_slice());

        server_task.await.expect("server task joined");
    });
}

#[test]
fn qr_clear_expected_peer_datagram_is_malformed() {
    block_on(async {
        let (server, address) = bind_ipv4();
        let id = 0x0102;
        let not_a_response = query_wire(id); // QR clear
        let server_task = reply_once(server, not_a_response);

        let query = query_wire(id);
        let error = exchange_bounded(address, &query, open_context())
            .await
            .err()
            .expect("a QR-clear datagram is not an accepted response");
        assert_eq!(error, UpstreamError::MalformedResponse);

        server_task.await.expect("server task joined");
    });
}

#[test]
fn wrong_id_datagram_is_ignored_until_a_matching_id_arrives() {
    block_on(async {
        let (server, address) = bind_ipv4();
        let request_id = 0x2222;
        let wrong = response_wire(0x9999, 4);
        let expected = response_wire(request_id, 5);
        let reply = expected.clone();
        let server_task = spawn_blocking(move || {
            let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
            let (_, peer) = server.recv_from(&mut buffer).expect("receive query");
            server.send_to(&wrong, peer).expect("send wrong id");
            server.send_to(&reply, peer).expect("send matching id");
        });

        let query = query_wire(request_id);
        let response = exchange_bounded(address, &query, open_context())
            .await
            .expect("matching response accepted");

        assert_eq!(response.response_id(), request_id);
        assert_eq!(response.wire(), expected.as_slice());

        server_task.await.expect("server task joined");
    });
}

#[test]
fn wrong_peer_datagram_is_ignored() {
    block_on(async {
        let (server, address) = bind_ipv4();
        let (spoof, _spoof_address) = bind_ipv4();
        let (peer_tx, peer_rx) = mpsc::channel::<SocketAddr>();
        let id = 0x3333;
        let spoof_reply = response_wire(id, 6);
        let expected = response_wire(id, 7);
        let reply = expected.clone();

        let spoof_task = spawn_blocking(move || {
            let peer = peer_rx.recv().expect("client peer address");
            spoof.send_to(&spoof_reply, peer).expect("send spoof reply");
        });
        let server_task = spawn_blocking(move || {
            let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
            let (_, peer) = server.recv_from(&mut buffer).expect("receive query");
            peer_tx.send(peer).expect("report client peer");
            std::thread::sleep(Duration::from_millis(50));
            server.send_to(&reply, peer).expect("send reply");
        });

        let query = query_wire(id);
        let response = exchange_bounded(address, &query, open_context())
            .await
            .expect("expected-peer response accepted");

        assert_eq!(response.response_id(), id);
        assert_eq!(
            response.wire(),
            expected.as_slice(),
            "a datagram from an unexpected source must not win"
        );

        server_task.await.expect("server task joined");
        spoof_task.await.expect("spoof task joined");
    });
}

#[test]
fn undersized_expected_peer_datagram_is_malformed() {
    block_on(async {
        let (server, address) = bind_ipv4();
        let id: u16 = 0x4444;
        let id_bytes = id.to_be_bytes();
        let undersized = vec![id_bytes[0], id_bytes[1], 0x81, 0x80, 0x00];
        let server_task = reply_once(server, undersized);

        let query = query_wire(id);
        let error = exchange_bounded(address, &query, open_context())
            .await
            .err()
            .expect("an undersized expected-peer datagram is terminal");
        assert_eq!(error, UpstreamError::MalformedResponse);

        server_task.await.expect("server task joined");
    });
}

#[test]
fn malformed_expected_peer_datagram_is_malformed() {
    block_on(async {
        let (server, address) = bind_ipv4();
        let id: u16 = 0x5555;
        // QR set and matching ID, but QDCOUNT=1 with no question bytes.
        let mut malformed = id.to_be_bytes().to_vec();
        malformed.extend_from_slice(&[0x81, 0x80, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
        let server_task = reply_once(server, malformed);

        let query = query_wire(id);
        let error = exchange_bounded(address, &query, open_context())
            .await
            .err()
            .expect("a malformed expected-peer datagram is terminal");
        assert_eq!(error, UpstreamError::MalformedResponse);

        server_task.await.expect("server task joined");
    });
}

#[test]
fn concurrent_exchanges_are_isolated() {
    block_on(async {
        let (first_server, first_address) = bind_ipv4();
        let (second_server, second_address) = bind_ipv4();
        let first_id = 0x0a01;
        let second_id = 0x0b02;
        let first_expected = response_wire(first_id, 8);
        let second_expected = response_wire(second_id, 9);
        let first_reply = first_expected.clone();
        let second_reply = second_expected.clone();

        let first_task = spawn_blocking(move || {
            let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
            let (_, peer) = first_server
                .recv_from(&mut buffer)
                .expect("receive first query");
            // Delay the first reply so the two exchanges genuinely overlap.
            std::thread::sleep(Duration::from_millis(50));
            first_server
                .send_to(&first_reply, peer)
                .expect("send first reply");
        });
        let second_task = spawn_blocking(move || {
            let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
            let (_, peer) = second_server
                .recv_from(&mut buffer)
                .expect("receive second query");
            second_server
                .send_to(&second_reply, peer)
                .expect("send second reply");
        });

        let first_query = query_wire(first_id);
        let second_query = query_wire(second_id);
        let first_handle = tokio::spawn(async move {
            exchange_bounded(first_address, &first_query, open_context()).await
        });
        let second_handle = tokio::spawn(async move {
            exchange_bounded(second_address, &second_query, open_context()).await
        });
        let first_response = first_handle
            .await
            .expect("first exchange joined")
            .expect("first exchange succeeds");
        let second_response = second_handle
            .await
            .expect("second exchange joined")
            .expect("second exchange succeeds");

        assert_eq!(first_response.response_id(), first_id);
        assert_eq!(first_response.wire(), first_expected.as_slice());
        assert_eq!(second_response.response_id(), second_id);
        assert_eq!(second_response.wire(), second_expected.as_slice());

        first_task.await.expect("first server joined");
        second_task.await.expect("second server joined");
    });
}

#[test]
fn cancellation_before_send_is_cancelled_without_side_effects() {
    block_on(async {
        let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9);
        let cancellation = TransportCancellation::new();
        cancellation.cancel();
        let context = ExchangeContext::new(Instant::now() + Duration::from_secs(30), cancellation);
        let query = query_wire(0x6666);

        let error = exchange_bounded(address, &query, context)
            .await
            .err()
            .expect("cancellation stops the exchange before any send");
        assert_eq!(error, UpstreamError::Cancelled(SideEffectState::NotSent));
    });
}

#[test]
fn cancellation_after_send_is_cancelled_with_sent_side_effect() {
    block_on(async {
        let (server, address) = bind_ipv4();
        let (seen_tx, seen_rx) = oneshot::channel::<()>();
        let server_task = spawn_blocking(move || {
            let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
            let _ = server.recv_from(&mut buffer).expect("receive query");
            seen_tx.send(()).expect("signal receipt");
        });

        let query = query_wire(0x7777);
        let cancellation = TransportCancellation::new();
        let upstream = Arc::new(Upstream::new(udp_endpoint(address)));
        let client_task = {
            let upstream = Arc::clone(&upstream);
            let cancellation = cancellation.clone();
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                upstream
                    .exchange(
                        request,
                        ExchangeContext::new(
                            Instant::now() + Duration::from_secs(30),
                            cancellation,
                        ),
                    )
                    .await
            })
        };

        timeout(TEST_TIMEOUT, seen_rx)
            .await
            .expect("query observed bounded")
            .expect("query observed");
        // Let the client task record the completed send before cancelling.
        sleep(Duration::from_millis(50)).await;
        cancellation.cancel();
        let outcome = timeout(TEST_TIMEOUT, client_task)
            .await
            .expect("client bounded")
            .expect("client joined");
        assert_eq!(
            outcome.err().expect("cancellation terminates the exchange"),
            UpstreamError::Cancelled(SideEffectState::Sent)
        );

        server_task.await.expect("server task joined");
    });
}

#[test]
fn expired_deadline_before_send_is_deadline_exceeded_without_side_effects() {
    block_on(async {
        let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9);
        let context = ExchangeContext::new(Instant::now(), TransportCancellation::new());
        let query = query_wire(0x8888);

        let error = exchange_bounded(address, &query, context)
            .await
            .err()
            .expect("an expired deadline stops the exchange before any send");
        assert_eq!(
            error,
            UpstreamError::DeadlineExceeded(SideEffectState::NotSent)
        );
    });
}

#[test]
fn deadline_after_send_is_deadline_exceeded_with_sent_side_effect() {
    block_on(async {
        let (server, address) = bind_ipv4();
        let (seen_tx, seen_rx) = oneshot::channel::<()>();
        let server_task = spawn_blocking(move || {
            let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
            let _ = server.recv_from(&mut buffer).expect("receive query");
            seen_tx.send(()).expect("signal receipt");
        });

        let query = query_wire(0x9999);
        let context = ExchangeContext::new(
            Instant::now() + Duration::from_millis(500),
            TransportCancellation::new(),
        );
        let client_task = tokio::spawn(async move {
            let upstream = Upstream::new(udp_endpoint(address));
            let request = ExchangeRequest::new(&query).expect("valid query");
            upstream.exchange(request, context).await
        });

        timeout(TEST_TIMEOUT, seen_rx)
            .await
            .expect("query observed bounded")
            .expect("query observed");
        let outcome = timeout(TEST_TIMEOUT, client_task)
            .await
            .expect("client bounded")
            .expect("client joined");
        assert_eq!(
            outcome.err().expect("the deadline terminates the exchange"),
            UpstreamError::DeadlineExceeded(SideEffectState::Sent)
        );

        server_task.await.expect("server task joined");
    });
}

#[test]
fn owner_close_during_receive_is_closed_with_sent_side_effect() {
    block_on(async {
        let (server, address) = bind_ipv4();
        let (seen_tx, seen_rx) = oneshot::channel::<()>();
        let server_task = spawn_blocking(move || {
            let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
            let _ = server.recv_from(&mut buffer).expect("receive query");
            seen_tx.send(()).expect("signal receipt");
        });

        let query = query_wire(0xaaaa);
        let upstream = Arc::new(Upstream::new(udp_endpoint(address)));
        let client_task = {
            let upstream = Arc::clone(&upstream);
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                upstream.exchange(request, open_context()).await
            })
        };

        timeout(TEST_TIMEOUT, seen_rx)
            .await
            .expect("query observed bounded")
            .expect("query observed");
        assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);
        let outcome = timeout(TEST_TIMEOUT, client_task)
            .await
            .expect("client bounded")
            .expect("client joined");
        assert_eq!(
            outcome.err().expect("owner close terminates the exchange"),
            UpstreamError::Closed(SideEffectState::Sent)
        );

        server_task.await.expect("server task joined");
    });
}

#[test]
fn exchange_sends_exactly_one_datagram() {
    block_on(async {
        let (server, address) = bind_ipv4();
        let id = 0xbbbb;
        let expected = response_wire(id, 10);
        let reply = expected.clone();
        let server_task = spawn_blocking(move || {
            let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
            let (_, peer) = server.recv_from(&mut buffer).expect("receive query");
            server.send_to(&reply, peer).expect("send reply");
            // The first slice has no automatic UDP retransmission.
            server
                .set_read_timeout(Some(Duration::from_millis(250)))
                .expect("set read timeout");
            server.recv_from(&mut buffer).is_err()
        });

        let query = query_wire(id);
        let response = exchange_bounded(address, &query, open_context())
            .await
            .expect("udp exchange succeeds");
        assert_eq!(response.response_id(), id);
        assert_eq!(response.wire(), expected.as_slice());

        let no_duplicate = server_task.await.expect("server task joined");
        assert!(no_duplicate, "the exchange must not resend the query");
    });
}

#[test]
fn legal_udp_wire_above_the_old_4095_buffer_is_not_truncated() {
    block_on(async {
        let (server, address) = bind_ipv4();
        let id = 0xcccc;
        let expected = large_response_wire(id, 5_000);
        let server_task = reply_once(server, expected.clone());

        let query = query_wire(id);
        let response = exchange_bounded(address, &query, open_context())
            .await
            .expect("large udp exchange succeeds");
        assert_eq!(response.response_id(), id);
        assert_eq!(response.wire().len(), expected.len());
        assert_eq!(response.wire(), expected.as_slice());

        server_task.await.expect("server task joined");
    });
}

#[test]
fn udp_wire_up_to_legal_datagram_capacity_is_not_truncated() {
    block_on(async {
        let (server, address) = bind_ipv4();
        let id = 0xdddd;
        // On Linux this is the full legal IPv4 UDP payload; on a host with a
        // lower UDP datagram cap it is that host's maximum.
        let expected = large_response_wire(id, CAPACITY_PROBE);
        assert_eq!(expected.len(), CAPACITY_PROBE);
        let server_task = reply_once(server, expected.clone());

        let query = query_wire(id);
        let response = exchange_bounded(address, &query, open_context())
            .await
            .expect("maximum-size udp exchange succeeds");
        assert_eq!(response.response_id(), id);
        assert_eq!(response.wire(), expected.as_slice());

        server_task.await.expect("server task joined");
    });
}

#[test]
fn late_datagram_cannot_complete_a_later_exchange() {
    block_on(async {
        let (first_server, first_address) = bind_ipv4();
        let (seen_tx, seen_rx) = oneshot::channel::<()>();
        let first_task = spawn_blocking(move || {
            let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
            let (_, peer) = first_server
                .recv_from(&mut buffer)
                .expect("receive first query");
            seen_tx.send(()).expect("signal receipt");
            // Deliver an otherwise valid response only after the cancelled
            // exchange has returned and released its socket.
            std::thread::sleep(Duration::from_millis(50));
            let _ = first_server.send_to(&response_wire(0x1111, 11), peer);
        });

        let first_query = query_wire(0x1111);
        let first_cancellation = TransportCancellation::new();
        let first_upstream = Arc::new(Upstream::new(udp_endpoint(first_address)));
        let first_client = {
            let upstream = Arc::clone(&first_upstream);
            let cancellation = first_cancellation.clone();
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&first_query).expect("valid query");
                upstream
                    .exchange(
                        request,
                        ExchangeContext::new(
                            Instant::now() + Duration::from_secs(30),
                            cancellation,
                        ),
                    )
                    .await
            })
        };

        timeout(TEST_TIMEOUT, seen_rx)
            .await
            .expect("first query observed bounded")
            .expect("first query observed");
        sleep(Duration::from_millis(50)).await;
        first_cancellation.cancel();
        let first_outcome = timeout(TEST_TIMEOUT, first_client)
            .await
            .expect("first client bounded")
            .expect("first client joined");
        assert!(
            first_outcome.is_err(),
            "a cancelled exchange must not succeed"
        );

        // A later exchange on a fresh socket must only observe its own peer.
        let (second_server, second_address) = bind_ipv4();
        let second_id = 0x2222;
        let second_expected = response_wire(second_id, 12);
        let second_reply = second_expected.clone();
        let second_task = spawn_blocking(move || {
            let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
            let (_, peer) = second_server
                .recv_from(&mut buffer)
                .expect("receive second query");
            std::thread::sleep(Duration::from_millis(50));
            second_server
                .send_to(&second_reply, peer)
                .expect("send second reply");
        });

        let second_query = query_wire(second_id);
        let second_response = exchange_bounded(second_address, &second_query, open_context())
            .await
            .expect("second exchange succeeds");
        assert_eq!(second_response.response_id(), second_id);
        assert_eq!(second_response.wire(), second_expected.as_slice());

        first_task.await.expect("first server joined");
        second_task.await.expect("second server joined");
    });
}

#[test]
fn closed_upstream_rejects_exchange_without_socket_work() {
    block_on(async {
        let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9);
        let upstream = Upstream::new(udp_endpoint(address));
        assert_eq!(upstream.close(), CloseResult::Closed);

        let query = query_wire(0xeeee);
        let request = ExchangeRequest::new(&query).expect("valid query");
        let error = timeout(TEST_TIMEOUT, upstream.exchange(request, open_context()))
            .await
            .expect("closed exchange bounded")
            .err()
            .expect("a closed upstream rejects new work");
        assert_eq!(error, UpstreamError::Closed(SideEffectState::NotSent));
    });
}

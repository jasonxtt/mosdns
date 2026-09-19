//! Slice3 contract tests for the QUIC transports (`design.md` §6-§7).
//!
//! The slice covers three things the Slice1/Slice2 loopback suites left open:
//!
//! * the caller's single absolute deadline terminating an exchange that is
//!   stalled in each QUIC phase (connect/TLS handshake, stream open, response
//!   wait), with no private timer anywhere in production;
//! * the fixed owner-close → caller-cancel → deadline → commit precedence and
//!   the `Lifecycle` close contract (drain, refuse, idempotent, zero residue);
//! * the structured peer stream-error-code → typed-error mapping: an `H3`
//!   `RemoteTerminate` code becomes a typed `PeerStreamTerminated` with a code
//!   *category* drawn only from the RFC 9114 §8.1 HTTP/3 code space, with RFC
//!   9114 §8's unexpected/unknown handling applied to the rest; a `DoQ` reset
//!   with any RFC 9250 code is the terminal missing-response-FIN protocol error,
//!   and a nonzero peer `DoQ` wire ID is the typed nonzero-response-ID protocol
//!   error. None of these ever commits;
//! * the `design.md` §7 calibration of a post-request-FIN `DoH3` head-phase loss
//!   as a `Sent` receive failure, and not the weaker `MaybeSent` of the
//!   HTTP/1.1/HTTP/2 head-not-received variant.
//!
//! Every test drives a real in-process QUIC/h3 server on an ephemeral IPv4
//! loopback port with the fixture's trusted synthetic certificate, so the ALPN,
//! certificate, and identity checks are all genuine. A stalled phase is blocked
//! by transport configuration (an unanswered UDP sink, a zero bidirectional
//! stream budget, or a server that withholds its response) rather than by a
//! sleep, and ordering between the test and the server is carried by explicit
//! `oneshot` signals or by `connection.closed()`.

mod fixtures;

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use fixtures::FixtureSet;
use hyper::body::Bytes;
use mosdns_upstream_core::quic::{DOQ_ALPN, Doh3Upstream, DoqEndpoint, DoqUpstream, H3_ALPN};
use mosdns_upstream_core::secure::{
    DohEndpoint, DohProtocolError, PeerStreamError, SecureError, TlsPolicy,
};
use mosdns_upstream_core::{
    CloseResult, CloseTransition, ExchangeContext, ExchangeRequest, ServerIdentity,
    SideEffectState, TransportCancellation, UpstreamError,
};
use quinn::VarInt;
use quinn::crypto::rustls::{HandshakeData, QuicServerConfig};
use rustls::ServerConfig;
use tokio::sync::oneshot;
use tokio::time::timeout;

/// Bounds every fixture wait so a broken path fails instead of hanging.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);
/// The bounded deadline handed to an exchange that is expected to complete.
const EXCHANGE_DEADLINE: Duration = Duration::from_secs(10);
/// The single short absolute deadline handed to an exchange whose phase is
/// deliberately stalled; it must fire and end the exchange.
const STALLED_DEADLINE: Duration = Duration::from_millis(400);
/// A bounded probe after the first connection closes, proving no second
/// connection was opened.
const ACCEPT_PROBE: Duration = Duration::from_millis(200);
/// The DNS wire upper bound (`u16::MAX`) plus the two-byte stream prefix.
const MAX_DOQ_MESSAGE: usize = 65_537;
/// RFC 9250 §4.3 `DOQ_PROTOCOL_ERROR`.
const DOQ_PROTOCOL_ERROR: u32 = 0x2;

/// Runs one bounded current-thread runtime for a single test step.
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

/// A complete, `dns-core`-valid response with a one-byte answer marker.
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

/// Prefixes `body` with its two-byte big-endian stream length.
fn framed(body: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(body.len() + 2);
    frame.extend_from_slice(
        &u16::try_from(body.len())
            .expect("body fits the u16 prefix")
            .to_be_bytes(),
    );
    frame.extend_from_slice(body);
    frame
}

/// One framed response message with an explicit peer wire ID.
fn framed_body(wire_id: u16, marker: u8) -> Vec<u8> {
    framed(&response_wire(wire_id, marker))
}

/// Builds a QUIC server configuration presenting the fixture's trusted leaf and
/// offering exactly `alpn`. `max_bidi` overrides the advertised bidirectional
/// stream budget when supplied.
fn doq_server_config(
    set: &FixtureSet,
    alpn: Vec<Vec<u8>>,
    max_bidi: Option<u32>,
) -> quinn::ServerConfig {
    let mut tls =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("ring provider supports TLS 1.3")
            .with_no_client_auth()
            .with_single_cert(vec![set.good.cert.clone()], set.good.key.clone_key())
            .expect("synthetic certificate and key are consistent");
    tls.alpn_protocols = alpn;
    let quic = QuicServerConfig::try_from(tls).expect("a TLS1.3 config converts to QUIC");
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(quic));
    if let Some(max_bidi) = max_bidi {
        let mut transport = quinn::TransportConfig::default();
        transport.max_concurrent_bidi_streams(VarInt::from_u32(max_bidi));
        config.transport_config(Arc::new(transport));
    }
    config
}

/// A bound UDP socket that never answers, so no QUIC handshake can complete.
struct UdpSink {
    _socket: UdpSocket,
    address: SocketAddr,
}

impl UdpSink {
    fn start() -> Self {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind the silent UDP sink");
        let address = socket.local_addr().expect("the sink address");
        Self {
            _socket: socket,
            address,
        }
    }
}

// ---------------------------------------------------------------------------
// A DoQ server whose response script and stream budget are per-test
// ---------------------------------------------------------------------------

/// What the scripted `DoQ` server does with the one accepted stream.
#[derive(Clone, Copy, Debug)]
enum DoqMode {
    /// Answer with one framed message carrying `wire_id`, then STREAM FIN.
    Respond { wire_id: u16, marker: u8 },
    /// Answer with one zeroed-ID framed message, then abort the send stream
    /// with the RFC 9250 application error `code` instead of a normal FIN.
    ResetAfterResponse { marker: u8, code: u32 },
    /// Answer with one zeroed-ID framed message, then leave the stream open with
    /// no FIN until the client gives up.
    RespondThenHold { marker: u8 },
    /// Complete the QUIC/TLS handshake and hold the connection without ever
    /// reading a stream (used with a zero stream budget).
    HandshakeOnly,
    /// Read the request through its FIN, signal readiness, and withhold every
    /// response byte until the client gives up.
    Holding,
}

/// What the `DoQ` server observed from the single accepted connection.
struct DoqEvidence {
    alpn: Option<Vec<u8>>,
    request: Option<Vec<u8>>,
}

/// A scripted in-process QUIC server driven on its own thread and runtime.
struct DoqServer {
    address: SocketAddr,
    accepts: Arc<AtomicUsize>,
    handle: std::thread::JoinHandle<Option<DoqEvidence>>,
}

impl DoqServer {
    fn start(
        set: &FixtureSet,
        alpn: Vec<Vec<u8>>,
        max_bidi: Option<u32>,
        mode: DoqMode,
        ready: Option<oneshot::Sender<()>>,
    ) -> Self {
        let config = doq_server_config(set, alpn, max_bidi);
        let (address_tx, address_rx) = std::sync::mpsc::channel::<SocketAddr>();
        let accepts = Arc::new(AtomicUsize::new(0));
        let accepts_thread = Arc::clone(&accepts);

        let handle = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build server runtime");
            runtime.block_on(async move {
                let endpoint =
                    quinn::Endpoint::server(config, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                        .expect("bind the ephemeral loopback QUIC endpoint");
                let address = endpoint.local_addr().expect("the bound address");
                address_tx
                    .send(address)
                    .expect("the test learns the address");

                let incoming = endpoint.accept().await?;
                accepts_thread.fetch_add(1, Ordering::SeqCst);
                let Ok(connection) = incoming.await else {
                    return None;
                };

                let alpn = connection
                    .handshake_data()
                    .and_then(|data| data.downcast::<HandshakeData>().ok())
                    .and_then(|data| data.protocol.clone());

                let request = run_doq(&connection, mode, ready).await;

                // A second connection opened before the first closed would be
                // queued here; the bounded probe accounts for it.
                if timeout(ACCEPT_PROBE, endpoint.accept())
                    .await
                    .is_ok_and(|second| second.is_some())
                {
                    accepts_thread.fetch_add(1, Ordering::SeqCst);
                }

                Some(DoqEvidence { alpn, request })
            })
        });

        let address = address_rx
            .recv_timeout(TEST_TIMEOUT)
            .expect("the server binds within the bounded wait");
        Self {
            address,
            accepts,
            handle,
        }
    }

    fn join(self) -> (usize, Option<DoqEvidence>) {
        let evidence = self.handle.join().expect("server thread joined");
        let accepts = self.accepts.load(Ordering::SeqCst);
        (accepts, evidence)
    }
}

/// Runs the scripted `DoQ` behavior and keeps every stream half and the
/// connection alive until the client closes, so a held stream is genuinely
/// open rather than an artefact of a dropped handle.
async fn run_doq(
    connection: &quinn::Connection,
    mode: DoqMode,
    ready: Option<oneshot::Sender<()>>,
) -> Option<Vec<u8>> {
    if matches!(mode, DoqMode::HandshakeOnly) {
        let _ = timeout(TEST_TIMEOUT, connection.closed()).await;
        return None;
    }

    let Ok((mut send, mut recv)) = connection.accept_bi().await else {
        let _ = timeout(TEST_TIMEOUT, connection.closed()).await;
        return None;
    };
    // `read_to_end` only returns once the request-side STREAM FIN is observed.
    let Ok(request) = recv.read_to_end(MAX_DOQ_MESSAGE).await else {
        let _ = timeout(TEST_TIMEOUT, connection.closed()).await;
        return None;
    };

    match mode {
        DoqMode::Respond { wire_id, marker } => {
            let _ = send.write_all(&framed_body(wire_id, marker)).await;
            let _ = send.finish();
        }
        DoqMode::ResetAfterResponse { marker, code } => {
            let _ = send.write_all(&framed_body(0, marker)).await;
            let _ = send.reset(VarInt::from_u32(code));
        }
        DoqMode::RespondThenHold { marker } => {
            let _ = send.write_all(&framed_body(0, marker)).await;
        }
        DoqMode::Holding => {}
        DoqMode::HandshakeOnly => unreachable!("handled before the stream read"),
    }
    if let Some(ready) = ready {
        let _ = ready.send(());
    }

    let _ = timeout(TEST_TIMEOUT, connection.closed()).await;
    Some(request)
}

// ---------------------------------------------------------------------------
// An HTTP/3 server whose response script and stream budget are per-test
// ---------------------------------------------------------------------------

/// What the scripted HTTP/3 server does with the one accepted request.
#[derive(Clone)]
enum H3Mode {
    /// Reset the response stream with the HTTP/3 application error `code`
    /// before any response head, so the client fails in its head phase.
    ResetBeforeResponse { code: u64 },
    /// Send a response head and then reset the response stream with `code`, so
    /// the client fails in its body phase.
    ResetAfterHead { code: u64 },
    /// Send a complete valid response head and body, then leave the stream open
    /// with no FIN.
    HoldAfterBody(Vec<u8>),
    /// Send a complete valid response head and body, then a trailing HEADERS
    /// field section (the H3 shape of a trailing extra response) and finish.
    TrailersAfterBody(Vec<u8>),
    /// Complete the QUIC/TLS handshake and hold the connection without any h3
    /// server work (used with a zero stream budget).
    HandshakeOnly,
    /// Read the request through its send-side FIN and then close the QUIC
    /// connection without ever sending a response head, so the client ends in
    /// its response-head phase with an ordinary connection loss.
    CloseAfterRequest,
    /// Read the request and withhold every response byte.
    Hold,
}

/// What the HTTP/3 server observed from the single accepted connection.
struct H3Evidence {
    alpn: Option<Vec<u8>>,
    request_fin: bool,
}

/// A scripted in-process HTTP/3 server driven on its own thread and runtime.
struct Doh3Server {
    address: SocketAddr,
    accepts: Arc<AtomicUsize>,
    handle: std::thread::JoinHandle<H3Evidence>,
}

impl Doh3Server {
    fn start(
        set: &FixtureSet,
        max_bidi: Option<u32>,
        mode: H3Mode,
        ready: Option<oneshot::Sender<()>>,
    ) -> Self {
        let config = doq_server_config(set, vec![H3_ALPN.to_vec()], max_bidi);
        let (address_tx, address_rx) = std::sync::mpsc::channel::<SocketAddr>();
        let accepts = Arc::new(AtomicUsize::new(0));
        let accepts_thread = Arc::clone(&accepts);

        let handle = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build server runtime");
            runtime.block_on(async move {
                let endpoint =
                    quinn::Endpoint::server(config, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                        .expect("bind the ephemeral loopback QUIC endpoint");
                let address = endpoint.local_addr().expect("the bound address");
                address_tx
                    .send(address)
                    .expect("the test learns the address");

                let Some(incoming) = endpoint.accept().await else {
                    return H3Evidence {
                        alpn: None,
                        request_fin: false,
                    };
                };
                accepts_thread.fetch_add(1, Ordering::SeqCst);
                let Ok(connection) = incoming.await else {
                    return H3Evidence {
                        alpn: None,
                        request_fin: false,
                    };
                };

                let alpn = connection
                    .handshake_data()
                    .and_then(|data| data.downcast::<HandshakeData>().ok())
                    .and_then(|data| data.protocol.clone());

                let request_fin = run_h3(&connection, mode, ready).await;

                if timeout(ACCEPT_PROBE, endpoint.accept())
                    .await
                    .is_ok_and(|second| second.is_some())
                {
                    accepts_thread.fetch_add(1, Ordering::SeqCst);
                }

                H3Evidence { alpn, request_fin }
            })
        });

        let address = address_rx
            .recv_timeout(TEST_TIMEOUT)
            .expect("the server binds within the bounded wait");
        Self {
            address,
            accepts,
            handle,
        }
    }

    fn join(self) -> (usize, H3Evidence) {
        let evidence = self.handle.join().expect("server thread joined");
        let accepts = self.accepts.load(Ordering::SeqCst);
        (accepts, evidence)
    }
}

/// Sends a valid `200 application/dns-message` head and `body`, optionally
/// finishing the response stream.
async fn send_dns_body(
    stream: &mut h3::server::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
    body: &[u8],
    finish: bool,
) -> Result<(), h3::error::StreamError> {
    let head = hyper::Response::builder()
        .status(200)
        .header("content-type", "application/dns-message")
        .header(
            "content-length",
            u64::try_from(body.len()).expect("body length fits u64"),
        )
        .body(())
        .expect("valid scripted response head");
    stream.send_response(head).await?;
    if !body.is_empty() {
        stream.send_data(Bytes::copy_from_slice(body)).await?;
    }
    if finish {
        stream.finish().await?;
    }
    Ok(())
}

/// Runs the scripted HTTP/3 behavior, keeping the h3 server connection and its
/// request stream alive until the client closes the QUIC connection.
async fn run_h3(
    connection: &quinn::Connection,
    mode: H3Mode,
    ready: Option<oneshot::Sender<()>>,
) -> bool {
    if matches!(mode, H3Mode::HandshakeOnly) {
        let _ = timeout(TEST_TIMEOUT, connection.closed()).await;
        return false;
    }

    let Ok(mut server) = h3::server::builder()
        .build::<_, Bytes>(h3_quinn::Connection::new(connection.clone()))
        .await
    else {
        let _ = timeout(TEST_TIMEOUT, connection.closed()).await;
        return false;
    };
    let Ok(Some(resolver)) = server.accept().await else {
        let _ = timeout(TEST_TIMEOUT, connection.closed()).await;
        return false;
    };
    let Ok((_request, mut stream)) = resolver.resolve_request().await else {
        let _ = timeout(TEST_TIMEOUT, connection.closed()).await;
        return false;
    };

    // `recv_data` only returns `Ok(None)` once the request stream ended, so its
    // success is the request send-side FIN evidence.
    let mut request_fin = false;
    loop {
        match stream.recv_data().await {
            Ok(Some(_)) => {}
            Ok(None) => {
                request_fin = true;
                break;
            }
            Err(_) => break,
        }
    }

    match mode {
        H3Mode::ResetBeforeResponse { code } => {
            stream.stop_stream(h3::error::Code::from(code));
        }
        H3Mode::ResetAfterHead { code } => {
            let head = hyper::Response::builder()
                .status(200)
                .header("content-type", "application/dns-message")
                .header("content-length", 40_u64)
                .body(())
                .expect("valid scripted response head");
            let _ = stream.send_response(head).await;
            stream.stop_stream(h3::error::Code::from(code));
        }
        H3Mode::HoldAfterBody(body) => {
            let _ = send_dns_body(&mut stream, &body, false).await;
            if let Some(ready) = ready {
                let _ = ready.send(());
            }
        }
        H3Mode::TrailersAfterBody(body) => {
            let _ = send_dns_body(&mut stream, &body, false).await;
            let mut trailers = hyper::HeaderMap::new();
            trailers.append(
                hyper::header::HeaderName::from_static("x-mosdns-trailer"),
                hyper::header::HeaderValue::from_static("1"),
            );
            let _ = stream.send_trailers(trailers).await;
            // The trailing field section is only observable as "extra content
            // after a complete body" once the stream really ends, so the fixture
            // finishes after it.
            let _ = stream.finish().await;
        }
        H3Mode::CloseAfterRequest => {
            // The request send-side FIN has already been observed above, so the
            // request was fully written and finished before this close. An
            // ordinary connection loss now lands in the client's response-head
            // phase and must be reported as `Sent` (design.md §7).
            connection.close(0u32.into(), b"");
        }
        H3Mode::Hold => {
            if let Some(ready) = ready {
                let _ = ready.send(());
            }
        }
        H3Mode::HandshakeOnly => unreachable!("handled before the h3 build"),
    }

    let _ = timeout(TEST_TIMEOUT, connection.closed()).await;
    request_fin
}

// ---------------------------------------------------------------------------
// Owners, contexts, and the shared control-precedence assertions
// ---------------------------------------------------------------------------

fn doq_owner(set: &FixtureSet, address: SocketAddr) -> DoqUpstream {
    let endpoint = DoqEndpoint::new(
        address,
        ServerIdentity::new("dns.example").expect("valid service identity"),
    )
    .expect("valid DoQ endpoint");
    DoqUpstream::new(
        endpoint,
        TlsPolicy::verified(set.root_store_a()).expect("verified TLS policy"),
    )
    .expect("DoQ owner constructs")
}

fn doh3_owner(set: &FixtureSet, address: SocketAddr) -> Doh3Upstream {
    let endpoint =
        DohEndpoint::new("https://dns.example/dns-query", address).expect("valid DoH endpoint");
    Doh3Upstream::new(
        endpoint,
        TlsPolicy::verified(set.root_store_a()).expect("verified TLS policy"),
    )
    .expect("DoH3 owner constructs")
}

fn open_context() -> ExchangeContext {
    ExchangeContext::new(
        Instant::now() + EXCHANGE_DEADLINE,
        TransportCancellation::new(),
    )
}

fn stalled_context() -> ExchangeContext {
    ExchangeContext::new(
        Instant::now() + STALLED_DEADLINE,
        TransportCancellation::new(),
    )
}

/// A numeric dial address nothing is bound to. Every precedence case fails
/// before any socket work, so the port is never contacted.
fn unused_address() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, 1))
}

/// Asserts the fixed owner-close → caller-cancel → deadline order through one
/// exchange that fails before any socket action.
fn assert_doq_control_precedence(closing: bool, cancelled: bool, expected: &SecureError) {
    let set = FixtureSet::generate();
    let upstream = doq_owner(&set, unused_address());
    let cancellation = TransportCancellation::new();
    if cancelled {
        cancellation.cancel();
    }
    // The deadline is already in the past, so only the precedence order decides
    // which of the three terminal controls wins.
    let context = ExchangeContext::new(Instant::now(), cancellation);
    if closing {
        assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);
    }
    let query = query_wire(0xA0FF);
    let error = block_on(async {
        upstream
            .exchange(ExchangeRequest::new(&query).expect("valid query"), context)
            .await
            .expect_err("a terminal control ends the exchange")
    });
    assert_eq!(&error, expected);
    assert_eq!(upstream.in_flight_exchanges(), 0);
}

/// Asserts the fixed owner-close → caller-cancel → deadline order for `DoH3`.
fn assert_doh3_control_precedence(closing: bool, cancelled: bool, expected: &SecureError) {
    let set = FixtureSet::generate();
    let upstream = doh3_owner(&set, unused_address());
    let cancellation = TransportCancellation::new();
    if cancelled {
        cancellation.cancel();
    }
    let context = ExchangeContext::new(Instant::now(), cancellation);
    if closing {
        assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);
    }
    let query = query_wire(0xA1FF);
    let error = block_on(async {
        upstream
            .exchange(ExchangeRequest::new(&query).expect("valid query"), context)
            .await
            .expect_err("a terminal control ends the exchange")
    });
    assert_eq!(&error, expected);
    assert_eq!(upstream.in_flight_exchanges(), 0);
}

// ---------------------------------------------------------------------------
// DoQ: deadline across the stalled connect/handshake, stream-open, and
// response phases, plus the close and error-code contracts
// ---------------------------------------------------------------------------

#[test]
fn doq_connect_and_handshake_deadline_is_not_sent() {
    let set = FixtureSet::generate();
    // The sink is bound but never answers, so quinn's single `connecting.await`
    // - which covers the transport handshake and TLS together - cannot finish.
    let sink = UdpSink::start();
    let upstream = doq_owner(&set, sink.address);
    let query = query_wire(0xA001);

    let error = block_on(async {
        upstream
            .exchange(
                ExchangeRequest::new(&query).expect("valid query"),
                stalled_context(),
            )
            .await
            .expect_err("the absolute deadline ends a stalled handshake")
    });

    assert_eq!(
        error,
        SecureError::Transport(UpstreamError::DeadlineExceeded(SideEffectState::NotSent)),
        "a stalled QUIC/TLS handshake sends no DNS byte, so it is NotSent"
    );
    assert_eq!(upstream.in_flight_exchanges(), 0);
    drop(sink);
}

#[test]
fn doq_alpn_mismatch_during_the_handshake_is_not_sent() {
    let set = FixtureSet::generate();
    // The server offers only `h3`; the client offers only `doq`, so the
    // handshake must fail before any DNS byte can be written.
    let server = DoqServer::start(
        &set,
        vec![H3_ALPN.to_vec()],
        None,
        DoqMode::HandshakeOnly,
        None,
    );
    let upstream = doq_owner(&set, server.address);
    let query = query_wire(0xA002);

    let error = block_on(async {
        upstream
            .exchange(
                ExchangeRequest::new(&query).expect("valid query"),
                open_context(),
            )
            .await
            .expect_err("an ALPN mismatch fails the handshake")
    });

    assert!(
        matches!(error, SecureError::Tls(_)),
        "expected a typed TLS failure, got {error:?}"
    );
    assert_eq!(error.side_effect(), SideEffectState::NotSent);
    assert_eq!(upstream.in_flight_exchanges(), 0);

    let (accepts, evidence) = server.join();
    assert_eq!(accepts, 1, "the single QUIC attempt was accepted");
    assert!(
        evidence.and_then(|e| e.request).is_none(),
        "no DoQ request may be written after a failed handshake"
    );
}

#[test]
fn doq_stream_open_deadline_is_not_sent() {
    let set = FixtureSet::generate();
    // The server advertises a zero bidirectional stream budget, so the client's
    // `open_bi` waits and the shared deadline must win before any write.
    let server = DoqServer::start(
        &set,
        vec![DOQ_ALPN.to_vec()],
        Some(0),
        DoqMode::HandshakeOnly,
        None,
    );
    let upstream = doq_owner(&set, server.address);
    let query = query_wire(0xA003);

    let error = block_on(async {
        upstream
            .exchange(
                ExchangeRequest::new(&query).expect("valid query"),
                stalled_context(),
            )
            .await
            .expect_err("the absolute deadline ends a stalled stream open")
    });

    assert_eq!(
        error,
        SecureError::Transport(UpstreamError::DeadlineExceeded(SideEffectState::NotSent)),
        "a stream that never opened sent nothing"
    );
    assert_eq!(upstream.in_flight_exchanges(), 0);

    let (accepts, evidence) = server.join();
    assert_eq!(accepts, 1);
    assert_eq!(
        evidence.and_then(|e| e.alpn).as_deref(),
        Some(DOQ_ALPN),
        "the handshake itself completed before the stream open stalled"
    );
}

#[test]
fn doq_response_wait_deadline_is_sent_and_never_commits() {
    let set = FixtureSet::generate();
    let server = DoqServer::start(&set, vec![DOQ_ALPN.to_vec()], None, DoqMode::Holding, None);
    let upstream = doq_owner(&set, server.address);
    let query = query_wire(0xA004);

    let error = block_on(async {
        upstream
            .exchange(
                ExchangeRequest::new(&query).expect("valid query"),
                stalled_context(),
            )
            .await
            .expect_err("the absolute deadline ends a stalled response wait")
    });

    assert_eq!(
        error,
        SecureError::Transport(UpstreamError::DeadlineExceeded(SideEffectState::Sent)),
        "the framed request was already written, so the wait is Sent"
    );
    assert_eq!(upstream.in_flight_exchanges(), 0);

    let (accepts, evidence) = server.join();
    assert_eq!(accepts, 1);
    // The server observed the request-side FIN with a zeroed wire ID, so the
    // request really was transmitted before the deadline terminated the wait.
    let mut zeroed = query.clone();
    zeroed[0] = 0;
    zeroed[1] = 0;
    assert_eq!(evidence.and_then(|e| e.request), Some(framed(&zeroed)));
}

#[test]
fn doq_complete_frame_without_peer_fin_deadline_is_sent_and_never_commits() {
    let set = FixtureSet::generate();
    // The peer sends a complete, valid frame but never the response-side FIN;
    // the exchange must not treat the bytes as a committed success.
    let server = DoqServer::start(
        &set,
        vec![DOQ_ALPN.to_vec()],
        None,
        DoqMode::RespondThenHold { marker: 0x2a },
        None,
    );
    let upstream = doq_owner(&set, server.address);
    let query = query_wire(0xA005);

    let error = block_on(async {
        upstream
            .exchange(
                ExchangeRequest::new(&query).expect("valid query"),
                stalled_context(),
            )
            .await
            .expect_err("a response without FIN cannot complete")
    });

    assert_eq!(
        error,
        SecureError::Transport(UpstreamError::DeadlineExceeded(SideEffectState::Sent)),
        "a complete-looking frame without FIN is still a Sent read failure"
    );
    assert_eq!(upstream.in_flight_exchanges(), 0);
    let (accepts, _evidence) = server.join();
    assert_eq!(accepts, 1);
}

#[test]
fn doq_nonzero_peer_response_id_is_a_terminal_protocol_error() {
    let set = FixtureSet::generate();
    // RFC 9250 §4.2.1: the peer's wire ID must be zero. A nonzero ID is a
    // terminal DoQ protocol error and its response must never commit.
    let server = DoqServer::start(
        &set,
        vec![DOQ_ALPN.to_vec()],
        None,
        DoqMode::Respond {
            wire_id: 0x1234,
            marker: 0x2a,
        },
        None,
    );
    let upstream = doq_owner(&set, server.address);
    let query = query_wire(0xA006);

    let error = block_on(async {
        upstream
            .exchange(
                ExchangeRequest::new(&query).expect("valid query"),
                open_context(),
            )
            .await
            .expect_err("a nonzero peer wire ID is a terminal protocol error")
    });

    assert_eq!(error, SecureError::DoqProtocolNonzeroResponseId);
    assert_eq!(error.side_effect(), SideEffectState::Sent);
    assert_eq!(upstream.in_flight_exchanges(), 0);
    let (accepts, _evidence) = server.join();
    assert_eq!(accepts, 1);
}

#[test]
fn doq_peer_reset_codes_are_terminal_missing_fin_without_commit() {
    // RFC 9250 §4.3 codes 0x0 NO_ERROR, 0x1 INTERNAL_ERROR, 0x2
    // PROTOCOL_ERROR, and 0x3 REQUEST_CANCELLED all abort the response stream
    // without the required response-side FIN. None may commit, and none may be
    // mistaken for a caller-local cancellation.
    for code in [0x0_u32, 0x1, DOQ_PROTOCOL_ERROR, 0x3] {
        let set = FixtureSet::generate();
        let server = DoqServer::start(
            &set,
            vec![DOQ_ALPN.to_vec()],
            None,
            DoqMode::ResetAfterResponse { marker: 0x2a, code },
            None,
        );
        let upstream = doq_owner(&set, server.address);
        let query = query_wire(0xA007);

        let error = block_on(async {
            upstream
                .exchange(
                    ExchangeRequest::new(&query).expect("valid query"),
                    open_context(),
                )
                .await
                .expect_err("a peer reset is a terminal protocol error")
        });

        assert_eq!(
            error,
            SecureError::DoqProtocolMissingResponseFin,
            "reset code {code:#x}"
        );
        assert_eq!(error.side_effect(), SideEffectState::Sent);
        assert_eq!(upstream.in_flight_exchanges(), 0);
        let (accepts, _evidence) = server.join();
        assert_eq!(
            accepts, 1,
            "reset code {code:#x} opens no second connection"
        );
    }
}

#[test]
fn doq_owner_close_drains_refuses_new_exchanges_and_converges() {
    block_on(async {
        let set = FixtureSet::generate();
        let (ready_tx, ready_rx) = oneshot::channel::<()>();
        let server = DoqServer::start(
            &set,
            vec![DOQ_ALPN.to_vec()],
            None,
            DoqMode::Holding,
            Some(ready_tx),
        );
        let upstream = Arc::new(doq_owner(&set, server.address));
        let query = query_wire(0xA008);
        let exchange = {
            let upstream = Arc::clone(&upstream);
            let query = query.clone();
            tokio::spawn(async move {
                upstream
                    .exchange(
                        ExchangeRequest::new(&query).expect("valid query"),
                        open_context(),
                    )
                    .await
            })
        };

        timeout(TEST_TIMEOUT, ready_rx)
            .await
            .expect("the server reads the request within the bound")
            .expect("the readiness signal is delivered");

        // `close` begins the owner shutdown, cancels the owner token, and only
        // returns after the in-flight exchange has released its guard.
        assert_eq!(upstream.close().await, CloseResult::Closed);
        let error = timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("the drained exchange is bounded")
            .expect("the exchange task joins")
            .expect_err("owner close terminates the in-flight exchange");
        assert!(
            matches!(error, SecureError::Transport(UpstreamError::Closed(_))),
            "owner close must report the typed Closed cause, got {error:?}"
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);

        // Repeat close converges, and a new exchange is refused before any
        // socket work.
        assert_eq!(upstream.close().await, CloseResult::AlreadyClosed);
        let refused = upstream
            .exchange(
                ExchangeRequest::new(&query).expect("valid query"),
                open_context(),
            )
            .await
            .expect_err("a closed owner refuses new exchanges");
        assert_eq!(
            refused,
            SecureError::Transport(UpstreamError::Closed(SideEffectState::NotSent))
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);

        let (accepts, evidence) = server.join();
        assert_eq!(accepts, 1, "a drained exchange opens no second connection");
        assert_eq!(evidence.and_then(|e| e.alpn).as_deref(), Some(DOQ_ALPN));
    });
}

#[test]
fn doq_owner_close_beats_caller_cancellation_in_flight() {
    block_on(async {
        let set = FixtureSet::generate();
        let (ready_tx, ready_rx) = oneshot::channel::<()>();
        let server = DoqServer::start(
            &set,
            vec![DOQ_ALPN.to_vec()],
            None,
            DoqMode::Holding,
            Some(ready_tx),
        );
        let upstream = Arc::new(doq_owner(&set, server.address));
        let cancellation = TransportCancellation::new();
        let context =
            ExchangeContext::new(Instant::now() + EXCHANGE_DEADLINE, cancellation.clone());
        let query = query_wire(0xA009);
        let exchange = {
            let upstream = Arc::clone(&upstream);
            let query = query.clone();
            tokio::spawn(async move {
                upstream
                    .exchange(ExchangeRequest::new(&query).expect("valid query"), context)
                    .await
            })
        };

        timeout(TEST_TIMEOUT, ready_rx)
            .await
            .expect("the server reads the request within the bound")
            .expect("the readiness signal is delivered");

        // Both controls become ready while the exchange is outstanding; owner
        // close has the higher precedence and must win.
        assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);
        cancellation.cancel();

        let error = timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("the controlled exchange is bounded")
            .expect("the exchange task joins")
            .expect_err("a terminal control ends the exchange");
        assert_eq!(
            error,
            SecureError::Transport(UpstreamError::Closed(SideEffectState::Sent))
        );
        assert_eq!(upstream.close().await, CloseResult::Closed);
        assert_eq!(upstream.in_flight_exchanges(), 0);

        let (accepts, _evidence) = server.join();
        assert_eq!(accepts, 1);
    });
}

#[test]
fn doq_control_precedence_is_owner_then_caller_then_deadline() {
    // Owner close wins over an already-cancelled caller and a past deadline.
    assert_doq_control_precedence(
        true,
        true,
        &SecureError::Transport(UpstreamError::Closed(SideEffectState::NotSent)),
    );
    // Caller cancellation wins over the past deadline while the owner is open.
    assert_doq_control_precedence(
        false,
        true,
        &SecureError::Transport(UpstreamError::Cancelled(SideEffectState::NotSent)),
    );
    // The past deadline alone is DeadlineExceeded.
    assert_doq_control_precedence(
        false,
        false,
        &SecureError::Transport(UpstreamError::DeadlineExceeded(SideEffectState::NotSent)),
    );
}

// ---------------------------------------------------------------------------
// DoH3: the same deadline/close/precedence contract plus the structured
// HTTP/3 peer stream-error-code mapping
// ---------------------------------------------------------------------------

#[test]
fn doh3_connect_and_handshake_deadline_is_not_sent() {
    let set = FixtureSet::generate();
    let sink = UdpSink::start();
    let upstream = doh3_owner(&set, sink.address);
    let query = query_wire(0xB001);

    let error = block_on(async {
        upstream
            .exchange(
                ExchangeRequest::new(&query).expect("valid query"),
                stalled_context(),
            )
            .await
            .expect_err("the absolute deadline ends a stalled handshake")
    });

    assert_eq!(
        error,
        SecureError::Transport(UpstreamError::DeadlineExceeded(SideEffectState::NotSent))
    );
    assert_eq!(upstream.in_flight_exchanges(), 0);
    drop(sink);
}

#[test]
fn doh3_request_stream_blocked_deadline_is_terminal_before_any_response() {
    let set = FixtureSet::generate();
    // The server advertises a zero bidirectional stream budget, so h3's
    // `send_request` (stream open plus request write in one call) is blocked and
    // the deadline must win before any response can exist.
    let server = Doh3Server::start(&set, Some(0), H3Mode::HandshakeOnly, None);
    let upstream = doh3_owner(&set, server.address);
    let query = query_wire(0xB002);

    let error = block_on(async {
        upstream
            .exchange(
                ExchangeRequest::new(&query).expect("valid query"),
                stalled_context(),
            )
            .await
            .expect_err("the absolute deadline ends a stalled stream open")
    });

    // h3 0.0.8 fuses the stream open with the request-head write into one
    // `send_request` call, so a failure there is conservatively `MaybeSent`
    // rather than claiming the request definitely reached the peer.
    assert_eq!(
        error,
        SecureError::Transport(UpstreamError::DeadlineExceeded(SideEffectState::MaybeSent))
    );
    assert_eq!(upstream.in_flight_exchanges(), 0);

    let (accepts, evidence) = server.join();
    assert_eq!(accepts, 1);
    assert_eq!(evidence.alpn.as_deref(), Some(H3_ALPN));
    assert!(
        !evidence.request_fin,
        "no request byte may have been decoded"
    );
}

#[test]
fn doh3_response_wait_deadline_is_sent_and_never_commits() {
    let set = FixtureSet::generate();
    let server = Doh3Server::start(&set, None, H3Mode::Hold, None);
    let upstream = doh3_owner(&set, server.address);
    let query = query_wire(0xB003);

    let error = block_on(async {
        upstream
            .exchange(
                ExchangeRequest::new(&query).expect("valid query"),
                stalled_context(),
            )
            .await
            .expect_err("the absolute deadline ends a stalled response wait")
    });

    // The request head was written and its send side finished, so any failure
    // while waiting for the response is `Sent`.
    assert_eq!(
        error,
        SecureError::Transport(UpstreamError::DeadlineExceeded(SideEffectState::Sent))
    );
    assert_eq!(upstream.in_flight_exchanges(), 0);

    let (accepts, evidence) = server.join();
    assert_eq!(accepts, 1);
    assert!(evidence.request_fin, "the request reached the server first");
}

#[test]
fn doh3_missing_response_fin_after_a_complete_body_never_commits() {
    block_on(async {
        let set = FixtureSet::generate();
        // The server sends a complete, valid head and body but never the
        // response-side FIN, so the response never completes; a stalled exchange
        // must end at the caller's deadline with the already-written request
        // reported as `Sent`, and must never commit the complete-looking body.
        let server = Doh3Server::start(
            &set,
            None,
            H3Mode::HoldAfterBody(response_wire(0, 0x2a)),
            None,
        );
        let upstream = doh3_owner(&set, server.address);
        let query = query_wire(0xB004);

        let error = upstream
            .exchange(
                ExchangeRequest::new(&query).expect("valid query"),
                stalled_context(),
            )
            .await
            .expect_err("a body without a response FIN is not a success");

        assert_eq!(
            error,
            SecureError::Transport(UpstreamError::DeadlineExceeded(SideEffectState::Sent))
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);
        let (accepts, evidence) = server.join();
        assert_eq!(accepts, 1);
        assert!(evidence.request_fin, "the request reached the server first");
    });
}

#[test]
fn doh3_trailing_extra_field_section_is_not_a_success() {
    block_on(async {
        let set = FixtureSet::generate();
        // After the complete body the server starts another HEADERS field
        // section: the H3 shape of a trailing extra response. It must never be
        // silently accepted as a finished body.
        let server = Doh3Server::start(
            &set,
            None,
            H3Mode::TrailersAfterBody(response_wire(0, 0x2a)),
            None,
        );
        let upstream = doh3_owner(&set, server.address);
        let query = query_wire(0xB005);

        let error = upstream
            .exchange(
                ExchangeRequest::new(&query).expect("valid query"),
                open_context(),
            )
            .await
            .expect_err("a trailing field section is not a success");

        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::IncompleteBody)
        );
        assert_eq!(error.side_effect(), SideEffectState::Sent);
        assert_eq!(upstream.in_flight_exchanges(), 0);
        let (accepts, _evidence) = server.join();
        assert_eq!(accepts, 1);
    });
}

#[test]
fn doh3_peer_stream_termination_codes_are_typed_and_never_commit() {
    // RFC 9114 §8.1 codes: 0x100 H3_NO_ERROR, 0x101 general protocol error,
    // 0x102 internal error, and 0x10c request cancelled. The peer terminated the
    // response stream before any head, so each is a typed peer termination and
    // none may commit. NO_ERROR is *not* benign here: the response never
    // completed.
    let cases = [
        (0x100_u64, PeerStreamError::NoError),
        (0x101, PeerStreamError::ProtocolError),
        (0x102, PeerStreamError::InternalError),
        (0x10c, PeerStreamError::RequestCancelled),
    ];
    for (code, category) in cases {
        let set = FixtureSet::generate();
        let server = Doh3Server::start(&set, None, H3Mode::ResetBeforeResponse { code }, None);
        let upstream = doh3_owner(&set, server.address);
        let query = query_wire(0xB006);

        let error = block_on(async {
            upstream
                .exchange(
                    ExchangeRequest::new(&query).expect("valid query"),
                    open_context(),
                )
                .await
                .expect_err("a peer stream termination is terminal")
        });

        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::PeerStreamTerminated { code: category }),
            "peer stream code {code:#x}"
        );
        assert_eq!(error.side_effect(), SideEffectState::Sent);
        assert_eq!(upstream.in_flight_exchanges(), 0);
        let (accepts, _evidence) = server.join();
        assert_eq!(
            accepts, 1,
            "peer stream code {code:#x} opens no second connection"
        );
    }
}

#[test]
fn doh3_non_h3_and_unclassified_peer_stream_codes_are_not_miscategorized() {
    // These codes are delivered on the wire exactly like the HTTP/3 codes
    // above, but they are not in the reviewed four-category HTTP/3 mapping:
    //
    // * `0x0`-`0x3` are RFC 9000 §20.1 *transport* error codes. Using one on an
    //   HTTP/3 request stream is an error code in an unexpected context, so
    //   RFC 9114 §8 requires it to be treated as equivalent to `H3_NO_ERROR`
    //   (`0x100`) - never as an H3 protocol error or a request cancellation.
    //   This is exactly where the old low-code aliases were wrong.
    // * `0x119` is a reserved `0x1f * N + 0x21` grease code (N = 8); RFC 9114
    //   §8.1 reserves that space to exercise the unknown-code rule, so it is
    //   also `H3_NO_ERROR`-equivalent even though it is above `0x100`.
    // * `0x103` (H3_STREAM_CREATION_ERROR) and `0x200`
    //   (QPACK_DECOMPRESSION_FAILED) are defined HTTP/3-family codes this
    //   client does not classify into one of the four categories; they are
    //   reported as the unclassified `Other` category rather than being
    //   mislabelled.
    //
    // Every case is a terminal peer termination that is `Sent` and never
    // commits, and none is mistaken for a caller-local cancellation.
    let cases = [
        (0x0_u64, PeerStreamError::NoError),
        (0x1, PeerStreamError::NoError),
        (0x2, PeerStreamError::NoError),
        (0x3, PeerStreamError::NoError),
        (0x119, PeerStreamError::NoError),
        (0x103, PeerStreamError::Other),
        (0x200, PeerStreamError::Other),
    ];
    for (code, category) in cases {
        let set = FixtureSet::generate();
        let server = Doh3Server::start(&set, None, H3Mode::ResetBeforeResponse { code }, None);
        let upstream = doh3_owner(&set, server.address);
        let query = query_wire(0xB00C);

        let error = block_on(async {
            upstream
                .exchange(
                    ExchangeRequest::new(&query).expect("valid query"),
                    open_context(),
                )
                .await
                .expect_err("a peer stream termination is terminal")
        });

        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::PeerStreamTerminated { code: category }),
            "peer stream code {code:#x}"
        );
        assert_eq!(error.side_effect(), SideEffectState::Sent);
        assert_eq!(upstream.in_flight_exchanges(), 0);
        let (accepts, _evidence) = server.join();
        assert_eq!(
            accepts, 1,
            "peer stream code {code:#x} opens no second connection"
        );
    }
}

#[test]
fn doh3_response_head_connection_loss_after_request_fin_is_sent_and_never_commits() {
    block_on(async {
        let set = FixtureSet::generate();
        // The server reads the request through its send-side FIN and then closes
        // the QUIC connection before sending any response head, so the client
        // fails in its response-head phase with an ordinary connection loss.
        // `design.md` §7 requires `Sent` here: the request was fully written and
        // its send side finished, so delivery is proven and the failure is a
        // post-write read failure - not the weaker `MaybeSent` the H1/H2
        // `ResponseHeadNotReceived` variant carries.
        let server = Doh3Server::start(&set, None, H3Mode::CloseAfterRequest, None);
        let upstream = doh3_owner(&set, server.address);
        let query = query_wire(0xB00B);

        let error = upstream
            .exchange(
                ExchangeRequest::new(&query).expect("valid query"),
                open_context(),
            )
            .await
            .expect_err("a lost connection before the response head is a failure");

        assert_eq!(
            error,
            SecureError::Transport(UpstreamError::Receive(SideEffectState::Sent)),
            "a post-request-FIN head loss is an ordinary Sent receive failure"
        );
        assert_eq!(error.side_effect(), SideEffectState::Sent);
        assert_eq!(
            upstream.in_flight_exchanges(),
            0,
            "the failed head phase leaves no registration residue"
        );
        let (accepts, evidence) = server.join();
        assert_eq!(accepts, 1, "a failed head phase opens no second connection");
        assert!(
            evidence.request_fin,
            "the request reached the server before the connection was lost"
        );
    });
}

#[test]
fn doh3_peer_stream_termination_after_the_head_is_typed_and_never_commits() {
    block_on(async {
        let set = FixtureSet::generate();
        // The head arrives, then the peer resets the stream mid-body. The
        // failure is in the body phase and keeps the same typed peer
        // termination, never a local cancellation.
        let server = Doh3Server::start(&set, None, H3Mode::ResetAfterHead { code: 0x10c }, None);
        let upstream = doh3_owner(&set, server.address);
        let query = query_wire(0xB007);

        let error = upstream
            .exchange(
                ExchangeRequest::new(&query).expect("valid query"),
                open_context(),
            )
            .await
            .expect_err("a mid-body peer reset is terminal");

        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::PeerStreamTerminated {
                code: PeerStreamError::RequestCancelled
            })
        );
        assert_eq!(error.side_effect(), SideEffectState::Sent);
        assert_eq!(upstream.in_flight_exchanges(), 0);
        let (accepts, _evidence) = server.join();
        assert_eq!(accepts, 1);
    });
}

#[test]
fn doh3_owner_close_drains_refuses_new_exchanges_and_converges() {
    block_on(async {
        let set = FixtureSet::generate();
        let (ready_tx, ready_rx) = oneshot::channel::<()>();
        let server = Doh3Server::start(&set, None, H3Mode::Hold, Some(ready_tx));
        let upstream = Arc::new(doh3_owner(&set, server.address));
        let query = query_wire(0xB008);
        let exchange = {
            let upstream = Arc::clone(&upstream);
            let query = query.clone();
            tokio::spawn(async move {
                upstream
                    .exchange(
                        ExchangeRequest::new(&query).expect("valid query"),
                        open_context(),
                    )
                    .await
            })
        };

        timeout(TEST_TIMEOUT, ready_rx)
            .await
            .expect("the server reads the request within the bound")
            .expect("the readiness signal is delivered");

        assert_eq!(upstream.close().await, CloseResult::Closed);
        let error = timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("the drained exchange is bounded")
            .expect("the exchange task joins")
            .expect_err("owner close terminates the in-flight exchange");
        assert!(
            matches!(error, SecureError::Transport(UpstreamError::Closed(_))),
            "owner close must report the typed Closed cause, got {error:?}"
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);

        assert_eq!(upstream.close().await, CloseResult::AlreadyClosed);
        let refused = upstream
            .exchange(
                ExchangeRequest::new(&query).expect("valid query"),
                open_context(),
            )
            .await
            .expect_err("a closed owner refuses new exchanges");
        assert_eq!(
            refused,
            SecureError::Transport(UpstreamError::Closed(SideEffectState::NotSent))
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);

        let (accepts, _evidence) = server.join();
        assert_eq!(accepts, 1);
    });
}

#[test]
fn doh3_owner_close_beats_caller_cancellation_in_flight() {
    block_on(async {
        let set = FixtureSet::generate();
        let (ready_tx, ready_rx) = oneshot::channel::<()>();
        let server = Doh3Server::start(&set, None, H3Mode::Hold, Some(ready_tx));
        let upstream = Arc::new(doh3_owner(&set, server.address));
        let cancellation = TransportCancellation::new();
        let context =
            ExchangeContext::new(Instant::now() + EXCHANGE_DEADLINE, cancellation.clone());
        let query = query_wire(0xB009);
        let exchange = {
            let upstream = Arc::clone(&upstream);
            let query = query.clone();
            tokio::spawn(async move {
                upstream
                    .exchange(ExchangeRequest::new(&query).expect("valid query"), context)
                    .await
            })
        };

        timeout(TEST_TIMEOUT, ready_rx)
            .await
            .expect("the server reads the request within the bound")
            .expect("the readiness signal is delivered");

        assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);
        cancellation.cancel();

        let error = timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("the controlled exchange is bounded")
            .expect("the exchange task joins")
            .expect_err("a terminal control ends the exchange");
        assert_eq!(
            error,
            SecureError::Transport(UpstreamError::Closed(SideEffectState::Sent))
        );
        assert_eq!(upstream.close().await, CloseResult::Closed);
        assert_eq!(upstream.in_flight_exchanges(), 0);

        let (accepts, _evidence) = server.join();
        assert_eq!(accepts, 1);
    });
}

#[test]
fn doh3_control_precedence_is_owner_then_caller_then_deadline() {
    assert_doh3_control_precedence(
        true,
        true,
        &SecureError::Transport(UpstreamError::Closed(SideEffectState::NotSent)),
    );
    assert_doh3_control_precedence(
        false,
        true,
        &SecureError::Transport(UpstreamError::Cancelled(SideEffectState::NotSent)),
    );
    assert_doh3_control_precedence(
        false,
        false,
        &SecureError::Transport(UpstreamError::DeadlineExceeded(SideEffectState::NotSent)),
    );
}

#[test]
fn doh3_caller_cancellation_after_a_complete_body_is_not_a_late_success() {
    block_on(async {
        let set = FixtureSet::generate();
        let (ready_tx, ready_rx) = oneshot::channel::<()>();
        // The server delivers a complete, valid head and body but never the
        // response FIN, so the exchange is still outstanding when the caller
        // cancels. The control must win; the complete-looking body must not be
        // committed after the fact.
        let server = Doh3Server::start(
            &set,
            None,
            H3Mode::HoldAfterBody(response_wire(0, 0x2a)),
            Some(ready_tx),
        );
        let upstream = Arc::new(doh3_owner(&set, server.address));
        let cancellation = TransportCancellation::new();
        let context =
            ExchangeContext::new(Instant::now() + EXCHANGE_DEADLINE, cancellation.clone());
        let query = query_wire(0xB00A);
        let exchange = {
            let upstream = Arc::clone(&upstream);
            let query = query.clone();
            tokio::spawn(async move {
                upstream
                    .exchange(ExchangeRequest::new(&query).expect("valid query"), context)
                    .await
            })
        };

        timeout(TEST_TIMEOUT, ready_rx)
            .await
            .expect("the server sends the body within the bound")
            .expect("the readiness signal is delivered");

        cancellation.cancel();
        let error = timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("the cancelled exchange is bounded")
            .expect("the exchange task joins")
            .expect_err("caller cancellation wins over the incomplete body");

        assert_eq!(
            error,
            SecureError::Transport(UpstreamError::Cancelled(SideEffectState::Sent)),
            "a local cancellation must stay a typed control error, not a peer error"
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);

        let (accepts, _evidence) = server.join();
        assert_eq!(accepts, 1);
    });
}

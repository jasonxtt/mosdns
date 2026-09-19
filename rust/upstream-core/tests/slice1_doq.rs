//! Slice1 contract test for the one-exchange DNS-over-QUIC primitive.
//!
//! The test runs a deterministic in-process QUIC server over an ephemeral IPv4
//! loopback port and drives one real [`DoqUpstream::exchange`] call. The server
//! presents a synthetic leaf issued by the fixture's trusted root and offers
//! exactly the `doq` ALPN, so the client's `TlsPolicy`-derived configuration
//! performs a real certificate/identity verification against local material.
//!
//! The asserted public contract is the successful one-shot shape only:
//!
//! * the exchange reports `SecureTransport::Doq` with no HTTP version and the
//!   caller's original request/response ID restored;
//! * the two-byte big-endian DNS framing carries a zeroed wire ID on the wire;
//! * the server observes the request-side STREAM FIN (its `read_to_end` returns);
//! * the client observes the peer response-side STREAM FIN (`read_to_end`);
//! * exactly one connection is accepted.
//!
//! Caller cancellation is exercised by two fixtures that withhold the response
//! and wait for the client's receive-side `STOP_SENDING` frame: one cancels
//! while the client waits for the response, and one keeps the outbound write
//! blocked on a tiny advertised flow-control window so the cancellation lands
//! after `open_bi` but before the request FIN. Both prove the active
//! `DOQ_REQUEST_CANCELLED` (0x3) cancel on the wire.
//!
//! Production cancels the receive side with a single `RecvStream::stop` call
//! and immediately returns the typed local control error; it makes no claim
//! that the frame reached the peer, because Quinn 0.11.7 exposes no awaitable
//! `STOP_SENDING` flush or peer-acknowledgement future and this crate may not
//! fake one with a yield, sleep, short timeout, polling loop, or
//! connection/endpoint close plus `wait_idle`. To observe the frame
//! deterministically, the cancellation tests install the debug-only
//! [`DoqStopPause`] seam: the exchange parks right after the production `stop`
//! while the real Quinn driver keeps running on the caller's runtime, the
//! server reports its `SendStream::stopped()` observation, and only then does
//! the test release the exchange. The seam is a test observation device, never
//! a production flush mechanism; it is compiled only under `debug_assertions`.
//! Error-code mapping beyond that local cancellation and pooling remain
//! separate follow-up jobs.

mod fixtures;

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use fixtures::FixtureSet;
#[cfg(debug_assertions)]
use mosdns_upstream_core::quic::DoqStopPause;
use mosdns_upstream_core::quic::{DOQ_ALPN, DoqEndpoint, DoqUpstream};
use mosdns_upstream_core::secure::{SecureError, SecureResponse, SecureTransport, TlsPolicy};
use mosdns_upstream_core::{
    ExchangeContext, ExchangeRequest, ServerIdentity, SideEffectState, TransportCancellation,
};
// Only the debug-only cancellation tests inspect the exact typed cause.
#[cfg(debug_assertions)]
use mosdns_upstream_core::UpstreamError;

use quinn::crypto::rustls::{HandshakeData, QuicServerConfig};
use rustls::ServerConfig;
use tokio::sync::oneshot;
use tokio::time::timeout;

/// Bounds every socket phase so a broken path fails instead of hanging.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);
/// The bounded client deadline handed to the exchange.
const EXCHANGE_DEADLINE: Duration = Duration::from_secs(10);
/// The DNS wire upper bound (`u16::MAX`) plus the two-byte stream prefix.
const MAX_DOQ_MESSAGE: usize = 65_537;
/// A bounded probe after the first connection closes, used to prove the client
/// did not open a second connection.
const ACCEPT_PROBE: Duration = Duration::from_millis(200);
/// RFC 9250 §4.3 `DOQ_PROTOCOL_ERROR`, the code used to abort the response send
/// stream instead of completing it with a normal STREAM FIN.
const DOQ_PROTOCOL_ERROR: u32 = 0x2;
/// RFC 9250 §4.3 `DOQ_REQUEST_CANCELLED`, the receive-side code the client must
/// send via `STOP_SENDING` when a local control decision cancels the
/// outstanding response. Only the debug-only cancellation seam can observe it,
/// so it is compiled out of release test builds.
#[cfg(debug_assertions)]
const DOQ_REQUEST_CANCELLED: u32 = 0x3;
/// A deliberately tiny advertised per-stream receive window, smaller than the
/// framed query. The client's outbound write cannot fit, so it blocks on flow
/// control until the server reads; the write-phase fixture never reads, so the
/// blocked write is deterministic without any sleep. Only the debug-only
/// write-phase cancellation fixture uses it.
#[cfg(debug_assertions)]
const FLOW_CONTROL_WINDOW: u32 = 8;

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

/// Builds the concatenated response stream for one frame per marker, each with
/// a zeroed wire ID (RFC 9250 §4.2.1).
fn framed_markers(markers: &[u8]) -> Vec<u8> {
    let mut payload = Vec::new();
    for marker in markers {
        payload.extend_from_slice(&framed(&response_wire(0, *marker)));
    }
    payload
}

/// Builds the QUIC server configuration presenting the fixture's trusted leaf
/// and offering exactly `doq`.
fn server_config(set: &FixtureSet) -> quinn::ServerConfig {
    let mut tls =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("ring provider supports TLS 1.3")
            .with_no_client_auth()
            .with_single_cert(vec![set.good.cert.clone()], set.good.key.clone_key())
            .expect("synthetic certificate and key are consistent");
    tls.alpn_protocols = vec![DOQ_ALPN.to_vec()];
    let quic = QuicServerConfig::try_from(tls).expect("a TLS1.3 config converts to QUIC");
    quinn::ServerConfig::with_crypto(Arc::new(quic))
}

/// Like [`server_config`] but advertises a deliberately tiny per-stream receive
/// window, so a client write larger than the window blocks on flow control until
/// the server reads the stream. Only the debug-only write-phase cancellation
/// fixture uses it.
#[cfg(debug_assertions)]
fn server_config_with_window(set: &FixtureSet, stream_receive_window: u32) -> quinn::ServerConfig {
    let mut config = server_config(set);
    let mut transport = quinn::TransportConfig::default();
    transport.stream_receive_window(quinn::VarInt::from_u32(stream_receive_window));
    config.transport_config(Arc::new(transport));
    config
}

/// What the server observed from the single accepted exchange.
struct ServerEvidence {
    /// The ALPN protocol the handshake negotiated.
    alpn: Option<Vec<u8>>,
    /// The complete request frame read up to the request-side STREAM FIN.
    request: Vec<u8>,
}

/// A scripted in-process QUIC server driven on its own thread and runtime.
struct DoqServer {
    address: SocketAddr,
    accepts: Arc<AtomicUsize>,
    evidence: oneshot::Receiver<ServerEvidence>,
    handle: std::thread::JoinHandle<()>,
}

/// How a scripted server ends its response send stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResponseEnding {
    /// Complete the response with a normal response-side STREAM FIN.
    StreamFin,
    /// Abort the response stream with the RFC 9250 `DOQ_PROTOCOL_ERROR` (`0x2`)
    /// code and no STREAM FIN.
    StreamReset,
    /// Terminate the whole connection without any response-side STREAM FIN,
    /// modelling a peer whose connection is lost before the response completes.
    ConnectionClose,
}

impl DoqServer {
    /// Starts a server that accepts one connection, answers the single `DoQ`
    /// stream with a zeroed wire ID, and records what it observed.
    fn start(set: &FixtureSet) -> Self {
        Self::start_with_markers(set, &[0x2a])
    }

    /// Starts a server that answers the single accepted `DoQ` stream with one
    /// zeroed wire-ID frame per marker, then a normal STREAM FIN. More than one
    /// marker models the protocol-violating trailing-response peer.
    fn start_with_markers(set: &FixtureSet, markers: &[u8]) -> Self {
        Self::start_scripted(set, &framed_markers(markers), ResponseEnding::StreamFin)
    }

    /// Starts a server that writes the response frame(s) and then aborts the
    /// response send stream with the RFC 9250 `DOQ_PROTOCOL_ERROR` (`0x2`) code
    /// instead of a normal STREAM FIN, modelling a peer that never completes
    /// the response stream.
    fn start_without_normal_fin(set: &FixtureSet, markers: &[u8]) -> Self {
        Self::start_scripted(set, &framed_markers(markers), ResponseEnding::StreamReset)
    }

    /// Starts a server that writes `raw_response` byte-for-byte and then
    /// terminates the whole connection without ever finishing the response
    /// stream, modelling a peer whose connection is lost before the response
    /// completes with a normal STREAM FIN.
    fn start_closed_without_response_fin(set: &FixtureSet, raw_response: &[u8]) -> Self {
        Self::start_scripted(set, raw_response, ResponseEnding::ConnectionClose)
    }

    /// Starts a scripted server that writes `payload` byte-for-byte and then
    /// ends the response according to `ending`.
    fn start_scripted(set: &FixtureSet, payload: &[u8], ending: ResponseEnding) -> Self {
        let payload = payload.to_vec();
        let config = server_config(set);
        let (address_tx, address_rx) = std::sync::mpsc::channel::<SocketAddr>();
        let (evidence_tx, evidence_rx) = oneshot::channel::<ServerEvidence>();
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
                    return;
                };
                accepts_thread.fetch_add(1, Ordering::SeqCst);
                let Ok(connection) = incoming.await else {
                    return;
                };

                let alpn = connection
                    .handshake_data()
                    .and_then(|data| data.downcast::<HandshakeData>().ok())
                    .and_then(|data| data.protocol.clone());

                let Ok((mut send, mut recv)) = connection.accept_bi().await else {
                    return;
                };
                // `read_to_end` only returns once the peer's request-side STREAM
                // FIN is observed, so its success is the FIN evidence.
                let Ok(request) = recv.read_to_end(MAX_DOQ_MESSAGE).await else {
                    return;
                };
                // A DoQ server answers with wire ID zero (RFC 9250 §4.2.1) and
                // the scripted payload is written byte-for-byte.
                if send.write_all(&payload).await.is_err() {
                    return;
                }
                match ending {
                    ResponseEnding::StreamFin => {
                        if send.finish().is_err() {
                            return;
                        }
                    }
                    ResponseEnding::StreamReset => {
                        // RFC 9250 §4.2: the peer never completed the response
                        // stream with a normal STREAM FIN; the fixture aborts it
                        // deterministically with the DoQ protocol-error code.
                        if send
                            .reset(quinn::VarInt::from_u32(DOQ_PROTOCOL_ERROR))
                            .is_err()
                        {
                            return;
                        }
                    }
                    ResponseEnding::ConnectionClose => {
                        // The response stream is never finished: the whole
                        // connection is terminated instead, so the client's
                        // read-to-FIN observes connection loss without any
                        // response-side STREAM FIN.
                        connection.close(quinn::VarInt::from_u32(DOQ_PROTOCOL_ERROR), b"");
                    }
                }
                let _ = evidence_tx.send(ServerEvidence { alpn, request });

                // Keep the endpoint alive until the client closes so the driver
                // can transmit the response; a bounded wait keeps a broken path
                // from hanging the test.
                let _ = timeout(TEST_TIMEOUT, connection.closed()).await;
                endpoint.wait_idle().await;

                // A second connection opened before the first closed would be
                // queued here; the bounded probe accounts for it.
                if timeout(ACCEPT_PROBE, endpoint.accept())
                    .await
                    .is_ok_and(|second| second.is_some())
                {
                    accepts_thread.fetch_add(1, Ordering::SeqCst);
                }
            });
        });

        let address = address_rx
            .recv_timeout(TEST_TIMEOUT)
            .expect("the server binds within the bounded wait");
        Self {
            address,
            accepts,
            evidence: evidence_rx,
            handle,
        }
    }

    fn join(self) -> (usize, ServerEvidence) {
        self.handle.join().expect("server thread joined");
        let accepts = self.accepts.load(Ordering::SeqCst);
        let evidence = self
            .evidence
            .blocking_recv()
            .expect("the server sends its observations");
        (accepts, evidence)
    }
}

/// What the holding server observed from a cancelled exchange. Only the
/// debug-only cancellation seam produces this observation.
#[cfg(debug_assertions)]
struct CancelEvidence {
    /// The ALPN protocol the handshake negotiated.
    alpn: Option<Vec<u8>>,
    /// The complete request frame read up to the request-side STREAM FIN.
    request: Vec<u8>,
    /// The application error code carried by the peer's receive-side
    /// `STOP_SENDING` frame, or `None` when no such frame was observed.
    stopped_code: Option<u64>,
}

/// A server that reads one request, withholds its response, and reports the
/// peer's receive-side `STOP_SENDING` observation. Only the debug-only
/// cancellation seam can observe the stop, so it is compiled out of release
/// test builds.
#[cfg(debug_assertions)]
struct HoldingDoqServer {
    address: SocketAddr,
    /// Resolves once the request was read through its request-side FIN and the
    /// server is withholding the response.
    request_ready: oneshot::Receiver<()>,
    /// Resolves with the observations after the peer's stop arrives.
    evidence: oneshot::Receiver<CancelEvidence>,
    accepts: Arc<AtomicUsize>,
    handle: std::thread::JoinHandle<()>,
}

#[cfg(debug_assertions)]
impl HoldingDoqServer {
    /// Starts a server that accepts one connection, reads the single `DoQ`
    /// request through its FIN, and then never writes a response byte. It
    /// blocks on the stream's own `stopped()` signal, so a client that never
    /// cancels the receive side leaves the server waiting instead of hiding the
    /// omission behind a sleep.
    fn start(set: &FixtureSet) -> Self {
        Self::start_with_window(set, None)
    }

    /// Starts a server that signals readiness as soon as it accepts the bidi
    /// stream, before it reads or observes the request FIN, withholds the
    /// response, and then waits on the stream's `stopped()` signal. The tiny
    /// advertised receive window keeps the client's outbound write blocked on
    /// flow control, so the cancellation deterministically lands in the write
    /// phase rather than racing to the response read.
    fn start_blocked_on_write(set: &FixtureSet) -> Self {
        Self::start_with_window(set, Some(FLOW_CONTROL_WINDOW))
    }

    /// Starts a scripted holding server. `window` is `None` for the
    /// response-wait fixture (readiness is the observed request-side FIN) and
    /// `Some` for the write-phase fixture (readiness is the accepted bidi
    /// stream, and the request is deliberately never read).
    fn start_with_window(set: &FixtureSet, window: Option<u32>) -> Self {
        let config = match window {
            Some(window) => server_config_with_window(set, window),
            None => server_config(set),
        };
        let reads_request = window.is_none();
        let (address_tx, address_rx) = std::sync::mpsc::channel::<SocketAddr>();
        let (ready_tx, ready_rx) = oneshot::channel::<()>();
        let (evidence_tx, evidence_rx) = oneshot::channel::<CancelEvidence>();
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
                    return;
                };
                accepts_thread.fetch_add(1, Ordering::SeqCst);
                let Ok(connection) = incoming.await else {
                    return;
                };

                let alpn = connection
                    .handshake_data()
                    .and_then(|data| data.downcast::<HandshakeData>().ok())
                    .and_then(|data| data.protocol.clone());

                let Ok((mut send, mut recv)) = connection.accept_bi().await else {
                    return;
                };
                // Readiness is the phase boundary the test drives:
                //   * response-wait fixture: the client's request-side FIN,
                //     observed only when `read_to_end` returns.
                //   * write-phase fixture: the accepted bidi stream itself,
                //     before any request byte is read, so the tiny receive
                //     window has not yet been widened and the client's write is
                //     still blocked on flow control.
                let request = if reads_request {
                    let Ok(request) = recv.read_to_end(MAX_DOQ_MESSAGE).await else {
                        return;
                    };
                    if ready_tx.send(()).is_err() {
                        return;
                    }
                    request
                } else {
                    if ready_tx.send(()).is_err() {
                        return;
                    }
                    Vec::new()
                };

                // Deliberately withhold the response and wait on the stream's
                // own receive-side stop observation. There is no sleep here: the
                // server's progress is exactly the peer's STOP_SENDING frame.
                let stopped_code = match send.stopped().await {
                    Ok(code) => code.map(quinn::VarInt::into_inner),
                    Err(_) => None,
                };
                let _ = evidence_tx.send(CancelEvidence {
                    alpn,
                    request,
                    stopped_code,
                });

                // Keep the endpoint alive until the client closes so the stop
                // is observable; the bounded wait keeps a broken path from
                // hanging the test.
                let _ = timeout(TEST_TIMEOUT, connection.closed()).await;
                endpoint.wait_idle().await;

                if timeout(ACCEPT_PROBE, endpoint.accept())
                    .await
                    .is_ok_and(|second| second.is_some())
                {
                    accepts_thread.fetch_add(1, Ordering::SeqCst);
                }
            });
        });

        let address = address_rx
            .recv_timeout(TEST_TIMEOUT)
            .expect("the server binds within the bounded wait");
        Self {
            address,
            request_ready: ready_rx,
            evidence: evidence_rx,
            accepts,
            handle,
        }
    }
}

#[test]
fn doq_one_shot_exchange_succeeds_over_loopback() {
    let set = FixtureSet::generate();
    let server = DoqServer::start(&set);
    let address = server.address;

    let endpoint = DoqEndpoint::new(
        address,
        ServerIdentity::new("dns.example").expect("valid service identity"),
    )
    .expect("valid DoQ endpoint");
    let upstream = DoqUpstream::new(
        endpoint,
        TlsPolicy::verified(set.root_store_a()).expect("verified TLS policy"),
    )
    .expect("DoQ owner constructs");

    let query = query_wire(0xBEEF);
    let context = ExchangeContext::new(
        Instant::now() + EXCHANGE_DEADLINE,
        TransportCancellation::new(),
    );

    let response: SecureResponse = block_on(async {
        let request = ExchangeRequest::new(&query).expect("valid query");
        upstream
            .exchange(request, context)
            .await
            .expect("the one-shot DoQ exchange succeeds")
    });

    // The frozen public shape: DoQ transport, no HTTP version, the caller's
    // original IDs restored in both the metadata and the returned wire.
    assert_eq!(response.transport(), SecureTransport::Doq);
    assert_eq!(response.http_version(), None);
    assert_eq!(response.request_id(), 0xBEEF);
    assert_eq!(response.response_id(), 0xBEEF);
    assert_eq!(response.wire(), response_wire(0xBEEF, 0x2a).as_slice());
    assert!(!response.truncated());
    // The RAII registration is released when the exchange future returns.
    assert_eq!(upstream.in_flight_exchanges(), 0);

    let (accepts, evidence) = server.join();
    assert_eq!(accepts, 1, "exactly one connection is accepted");
    assert_eq!(evidence.alpn.as_deref(), Some(DOQ_ALPN));

    // The server observed the request-side FIN with the two-byte prefix and a
    // zeroed wire ID, while the caller's borrowed query bytes never changed.
    let mut zeroed = query.clone();
    zeroed[0] = 0;
    zeroed[1] = 0;
    assert_eq!(evidence.request, framed(&zeroed));
    assert_eq!(&query[0..2], &[0xBE, 0xEF]);
}

#[test]
fn doq_trailing_second_response_is_rejected_without_commit() {
    let set = FixtureSet::generate();
    let server = DoqServer::start_with_markers(&set, &[0x2a, 0x2b]);
    let address = server.address;

    let endpoint = DoqEndpoint::new(
        address,
        ServerIdentity::new("dns.example").expect("valid service identity"),
    )
    .expect("valid DoQ endpoint");
    let upstream = DoqUpstream::new(
        endpoint,
        TlsPolicy::verified(set.root_store_a()).expect("verified TLS policy"),
    )
    .expect("DoQ owner constructs");

    let query = query_wire(0xBEEF);
    let context = ExchangeContext::new(
        Instant::now() + EXCHANGE_DEADLINE,
        TransportCancellation::new(),
    );

    let error = block_on(async {
        let request = ExchangeRequest::new(&query).expect("valid query");
        upstream
            .exchange(request, context)
            .await
            .expect_err("a trailing second response is rejected, never committed")
    });

    // The request was fully written before the trailing response arrived, so
    // the terminal rejection is a `Sent` protocol error that names the
    // trailing-response violation.
    assert_eq!(error, SecureError::DoqProtocolTrailingResponse);
    assert_eq!(error.side_effect(), SideEffectState::Sent);
    // The RAII registration is released on the terminal error path too.
    assert_eq!(upstream.in_flight_exchanges(), 0);

    let (accepts, evidence) = server.join();
    assert_eq!(accepts, 1, "exactly one connection is accepted");
    assert_eq!(evidence.alpn.as_deref(), Some(DOQ_ALPN));

    // The server still observed the request-side FIN with a zeroed wire ID; the
    // caller's borrowed query bytes never changed.
    let mut zeroed = query.clone();
    zeroed[0] = 0;
    zeroed[1] = 0;
    assert_eq!(evidence.request, framed(&zeroed));
}

/// Drives one cancellation exchange to the debug-only post-`stop` pause,
/// observes the peer's receive-side stop, then releases the exchange.
///
/// Production calls `RecvStream::stop(DOQ_REQUEST_CANCELLED)` and returns the
/// typed local control error without waiting for the peer. This helper uses the
/// debug-only [`DoqStopPause`] seam only as a *test observation device*: the
/// exchange future stays pending after that `stop`, so the real Quinn driver
/// keeps running on the caller's runtime and the server's `stopped()`
/// observation proves the frame was actually transmitted. No sleep, yield,
/// short timeout, polling loop, or endpoint teardown is used to fake a flush.
#[cfg(debug_assertions)]
fn cancel_and_observe_stop(
    upstream: &DoqUpstream,
    server: &mut HoldingDoqServer,
    pause: &DoqStopPause,
    cancellation: &TransportCancellation,
    query: &[u8],
) -> (SecureError, CancelEvidence) {
    let context = ExchangeContext::new(Instant::now() + EXCHANGE_DEADLINE, cancellation.clone());
    block_on(async {
        let request = ExchangeRequest::new(query).expect("valid query");
        let exchange = upstream.exchange(request, context);
        tokio::pin!(exchange);

        // The fixture's readiness boundary is phase-specific: the response-wait
        // fixture signals after the request-side FIN, and the write-phase
        // fixture signals as soon as the bidi stream is accepted. An exchange
        // that finished here would mean the wrong phase was cancelled.
        tokio::select! {
            ready = &mut server.request_ready => {
                ready.expect("the server signals request readiness");
            }
            result = &mut exchange => {
                panic!("the exchange completed before cancellation: {result:?}");
            }
        }

        cancellation.cancel();

        // The exchange parks immediately after the production `RecvStream::stop`.
        // The bound only stops a broken path from hanging the suite; the
        // release below is what lets the exchange finish.
        timeout(TEST_TIMEOUT, async {
            tokio::select! {
                () = pause.arrived() => {}
                result = &mut exchange => {
                    panic!("the exchange returned before the post-stop observation: {result:?}");
                }
            }
        })
        .await
        .expect("the exchange parks on the post-stop observation within the bound");

        // The exchange future is still parked, so its Quinn connection is alive
        // and driven; the server's observation is the real STOP_SENDING frame
        // for the code production sent.
        let evidence = timeout(TEST_TIMEOUT, &mut server.evidence)
            .await
            .expect("the server observes the receive-side stop within the bound")
            .expect("the server sends its observations");

        pause.release();

        let error = exchange
            .await
            .expect_err("the local control decision terminates the exchange");

        (error, evidence)
    })
}

#[test]
#[cfg(debug_assertions)]
fn doq_caller_cancellation_stops_the_receive_side_with_request_cancelled() {
    let set = FixtureSet::generate();
    let mut server = HoldingDoqServer::start(&set);
    let address = server.address;

    let endpoint = DoqEndpoint::new(
        address,
        ServerIdentity::new("dns.example").expect("valid service identity"),
    )
    .expect("valid DoQ endpoint");
    let upstream = DoqUpstream::new(
        endpoint,
        TlsPolicy::verified(set.root_store_a()).expect("verified TLS policy"),
    )
    .expect("DoQ owner constructs");
    let pause = Arc::new(DoqStopPause::new());
    upstream.install_stop_pause(Arc::clone(&pause));

    let query = query_wire(0xBEEF);
    let cancellation = TransportCancellation::new();

    let (error, evidence) =
        cancel_and_observe_stop(&upstream, &mut server, &pause, &cancellation, &query);

    // The pre-existing typed local control error is returned unchanged: no
    // string conversion and no new generic receive error.
    assert_eq!(
        error,
        SecureError::Transport(UpstreamError::Cancelled(SideEffectState::Sent))
    );
    // The RAII registration is released on the cancellation path too.
    assert_eq!(upstream.in_flight_exchanges(), 0);

    let accepts = server.accepts.load(Ordering::SeqCst);
    server.handle.join().expect("server thread joined");
    assert_eq!(accepts, 1, "exactly one connection is accepted");
    assert_eq!(evidence.alpn.as_deref(), Some(DOQ_ALPN));
    // The active cancellation is observable on the wire as the RFC 9250
    // DOQ_REQUEST_CANCELLED (0x3) receive-side stop, not just a local mapping.
    assert_eq!(
        evidence.stopped_code,
        Some(u64::from(DOQ_REQUEST_CANCELLED)),
        "the server observes receive-side STOP_SENDING with DOQ_REQUEST_CANCELLED"
    );

    // The server observed the request-side FIN with a zeroed wire ID before it
    // was stopped; the caller's borrowed query bytes never changed.
    let mut zeroed = query.clone();
    zeroed[0] = 0;
    zeroed[1] = 0;
    assert_eq!(evidence.request, framed(&zeroed));
    assert_eq!(&query[0..2], &[0xBE, 0xEF]);
}

#[test]
#[cfg(debug_assertions)]
fn doq_caller_cancellation_during_outbound_write_stops_the_receive_side() {
    let set = FixtureSet::generate();
    let mut server = HoldingDoqServer::start_blocked_on_write(&set);
    let address = server.address;

    let endpoint = DoqEndpoint::new(
        address,
        ServerIdentity::new("dns.example").expect("valid service identity"),
    )
    .expect("valid DoQ endpoint");
    let upstream = DoqUpstream::new(
        endpoint,
        TlsPolicy::verified(set.root_store_a()).expect("verified TLS policy"),
    )
    .expect("DoQ owner constructs");
    let pause = Arc::new(DoqStopPause::new());
    upstream.install_stop_pause(Arc::clone(&pause));

    let query = query_wire(0xBEEF);
    let cancellation = TransportCancellation::new();

    // The write-phase fixture signals as soon as it accepts the bidi stream,
    // before reading any request byte. The framed query cannot fit the tiny
    // advertised receive window, so the outbound write is still blocked on flow
    // control when the cancellation lands, and the shared helper proves the
    // post-`open_bi` stop even though the request FIN was never reached.
    let (error, evidence) =
        cancel_and_observe_stop(&upstream, &mut server, &pause, &cancellation, &query);

    // The write may already have been partially accepted, so the phase's own
    // existing state is `MaybeSent`; the typed error is returned unchanged with
    // no string conversion and no new generic receive error.
    assert_eq!(
        error,
        SecureError::Transport(UpstreamError::Cancelled(SideEffectState::MaybeSent))
    );
    // The RAII registration is released on the cancellation path too.
    assert_eq!(upstream.in_flight_exchanges(), 0);

    let accepts = server.accepts.load(Ordering::SeqCst);
    server.handle.join().expect("server thread joined");
    assert_eq!(accepts, 1, "exactly one connection is accepted");
    assert_eq!(evidence.alpn.as_deref(), Some(DOQ_ALPN));
    // A control decision that wins after `open_bi` must actively cancel the
    // receive side even when the exchange never reached the response read. This
    // is the write-phase analogue of the response-wait cancellation test.
    assert_eq!(
        evidence.stopped_code,
        Some(u64::from(DOQ_REQUEST_CANCELLED)),
        "the server observes receive-side STOP_SENDING with DOQ_REQUEST_CANCELLED"
    );
}

#[test]
fn doq_response_without_normal_fin_is_rejected_without_commit() {
    let set = FixtureSet::generate();
    // The server writes the complete response frame and then aborts its send
    // stream with `DOQ_PROTOCOL_ERROR` (0x2) instead of a normal STREAM FIN.
    let server = DoqServer::start_without_normal_fin(&set, &[0x2a]);
    let address = server.address;

    let endpoint = DoqEndpoint::new(
        address,
        ServerIdentity::new("dns.example").expect("valid service identity"),
    )
    .expect("valid DoQ endpoint");
    let upstream = DoqUpstream::new(
        endpoint,
        TlsPolicy::verified(set.root_store_a()).expect("verified TLS policy"),
    )
    .expect("DoQ owner constructs");

    let query = query_wire(0xBEEF);
    let context = ExchangeContext::new(
        Instant::now() + EXCHANGE_DEADLINE,
        TransportCancellation::new(),
    );

    let error = block_on(async {
        let request = ExchangeRequest::new(&query).expect("valid query");
        upstream
            .exchange(request, context)
            .await
            .expect_err("a response that never completes with normal FIN is rejected")
    });

    // The peer aborted the response stream instead of sending a normal STREAM
    // FIN. The request was already fully written, so the terminal rejection is
    // a `Sent` missing-response-FIN protocol error that is never committed.
    assert_eq!(error, SecureError::DoqProtocolMissingResponseFin);
    assert_eq!(error.side_effect(), SideEffectState::Sent);
    // The RAII registration is released on the terminal error path too.
    assert_eq!(upstream.in_flight_exchanges(), 0);

    let (accepts, evidence) = server.join();
    assert_eq!(accepts, 1, "exactly one connection is accepted");
    assert_eq!(evidence.alpn.as_deref(), Some(DOQ_ALPN));

    // The server observed the request-side FIN with a zeroed wire ID before it
    // aborted the response; the caller's borrowed query bytes never changed.
    let mut zeroed = query.clone();
    zeroed[0] = 0;
    zeroed[1] = 0;
    assert_eq!(evidence.request, framed(&zeroed));
}

/// Drives one exchange against a server that writes `raw_response` byte-for-byte
/// and then terminates the whole connection without ever finishing the response
/// stream, and asserts the terminal missing-FIN outcome.
fn assert_connection_loss_without_response_fin(raw_response: &[u8]) {
    let set = FixtureSet::generate();
    // The server never sends a response-side STREAM FIN: it closes the
    // connection instead, so the client's read-to-FIN observes connection loss.
    let server = DoqServer::start_closed_without_response_fin(&set, raw_response);
    let address = server.address;

    let endpoint = DoqEndpoint::new(
        address,
        ServerIdentity::new("dns.example").expect("valid service identity"),
    )
    .expect("valid DoQ endpoint");
    let upstream = DoqUpstream::new(
        endpoint,
        TlsPolicy::verified(set.root_store_a()).expect("verified TLS policy"),
    )
    .expect("DoQ owner constructs");

    let query = query_wire(0xBEEF);
    let context = ExchangeContext::new(
        Instant::now() + EXCHANGE_DEADLINE,
        TransportCancellation::new(),
    );

    let error = block_on(async {
        let request = ExchangeRequest::new(&query).expect("valid query");
        upstream
            .exchange(request, context)
            .await
            .expect_err("a connection lost before the response FIN is rejected")
    });

    // The connection was lost before a normal response STREAM FIN, so the
    // terminal rejection is the same `Sent` missing-FIN protocol error as the
    // stream-reset case, and the response is never committed.
    assert_eq!(error, SecureError::DoqProtocolMissingResponseFin);
    assert_eq!(error.side_effect(), SideEffectState::Sent);
    // The RAII registration is released on the terminal error path too.
    assert_eq!(upstream.in_flight_exchanges(), 0);

    let (accepts, evidence) = server.join();
    assert_eq!(accepts, 1, "exactly one connection is accepted");
    assert_eq!(evidence.alpn.as_deref(), Some(DOQ_ALPN));

    // The server observed the request-side FIN with a zeroed wire ID before it
    // terminated the connection; the caller's borrowed query bytes never
    // changed.
    let mut zeroed = query.clone();
    zeroed[0] = 0;
    zeroed[1] = 0;
    assert_eq!(evidence.request, framed(&zeroed));
}

#[test]
fn doq_connection_lost_with_full_frame_payload_is_rejected_as_missing_fin() {
    // The fixture writes a complete, well-formed response frame to Quinn's send
    // path and then terminates the connection before response-side STREAM FIN;
    // the client must not commit bytes without observing that FIN.
    assert_connection_loss_without_response_fin(&framed(&response_wire(0, 0x2a)));
}

#[test]
fn doq_connection_lost_with_partial_frame_payload_is_rejected_as_missing_fin() {
    // The fixture writes a truncated response-frame payload to Quinn's send
    // path and then terminates the connection before response-side STREAM FIN;
    // the partial payload must not be treated as a shorter message.
    let complete = framed(&response_wire(0, 0x2a));
    assert_connection_loss_without_response_fin(&complete[..6]);
}

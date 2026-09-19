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
//! Negative/error-mapping behaviors, cancellation, and pooling are separate
//! follow-up jobs and are deliberately not exercised here.

mod fixtures;

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use fixtures::FixtureSet;
use mosdns_upstream_core::quic::{DOQ_ALPN, DoqEndpoint, DoqUpstream};
use mosdns_upstream_core::secure::{SecureError, SecureResponse, SecureTransport, TlsPolicy};
use mosdns_upstream_core::{
    ExchangeContext, ExchangeRequest, ServerIdentity, SideEffectState, TransportCancellation,
};

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
        Self::start_scripted(set, markers, true)
    }

    /// Starts a server that writes the response frame(s) and then aborts the
    /// response send stream with the RFC 9250 `DOQ_PROTOCOL_ERROR` (`0x2`) code
    /// instead of a normal STREAM FIN, modelling a peer that never completes
    /// the response stream.
    fn start_without_normal_fin(set: &FixtureSet, markers: &[u8]) -> Self {
        Self::start_scripted(set, markers, false)
    }

    /// Starts a scripted server that writes one zeroed wire-ID frame per
    /// marker and completes the send stream either with a normal STREAM FIN or
    /// with a deterministic abort.
    fn start_scripted(set: &FixtureSet, markers: &[u8], normal_fin: bool) -> Self {
        let markers = markers.to_vec();
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
                // A DoQ server answers with wire ID zero (RFC 9250 §4.2.1). A
                // conforming server sends exactly one response frame; a caller
                // that passes more than one marker models the trailing-response
                // violation this test rejects.
                for marker in &markers {
                    let body = response_wire(0, *marker);
                    if send.write_all(&framed(&body)).await.is_err() {
                        return;
                    }
                }
                if normal_fin {
                    if send.finish().is_err() {
                        return;
                    }
                } else if send
                    .reset(quinn::VarInt::from_u32(DOQ_PROTOCOL_ERROR))
                    .is_err()
                {
                    // RFC 9250 §4.2: the peer never completed the response
                    // stream with a normal STREAM FIN; the audit API aborts it
                    // deterministically with the DoQ protocol-error code.
                    return;
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

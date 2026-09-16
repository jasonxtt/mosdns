//! Slice1 contract tests for the one-exchange DNS-over-TLS primitive.
//!
//! Every test runs a deterministic in-process TLS server over an ephemeral IPv4
//! loopback port and drives real `DotUpstream::exchange` calls. The server
//! presents synthetic certificates from `fixtures`, so the valid, wrong-name,
//! expired, and unknown-issuer cases are exercised against a local listener
//! rather than any public DNS service.
//!
//! The server is built with the same rustls/tokio-rustls stack the client uses,
//! so the handshake, record framing, and close behaviour are the real ones. The
//! tests assert the observable public contract: the typed error, the
//! `SideEffectState`, that the caller's borrowed query bytes never change, and
//! that registration is released on every path.

mod fixtures;

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fixtures::{FixtureSet, ServerIdentityFixture};
use mosdns_upstream_core::secure::{
    CertificateRejection, DotUpstream, SecureError, SecureResponse, SecureTransport,
    TlsHandshakeFailure, TlsPolicy,
};
use mosdns_upstream_core::{
    CloseResult, CloseTransition, DotEndpoint, ExchangeContext, ExchangeRequest, ServerIdentity,
    SideEffectState, TransportCancellation, UpstreamError,
};
use rustls::ServerConfig;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener as AsyncTcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;

/// Bounds every exchange so a broken socket path fails instead of hanging.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

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

/// A valid response with the TC bit set in the header flags.
fn truncated_response_wire(id: u16, marker: u8) -> Vec<u8> {
    let mut wire = response_wire(id, marker);
    wire[2] |= 0x02; // TC
    wire
}

/// A complete frame with QR set and the request ID, but an invalid DNS wire:
/// the header declares one question and then ends.
fn invalid_dns_response_wire(id: u16) -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&[0x81, 0x80]);
    wire.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT claims a question
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire
}

/// Builds a server configuration presenting `fixture`'s certificate and key.
///
/// The fixture owns generated DER that is dropped with the test, so the config
/// is built from clones of it.
fn server_config(fixture: &ServerIdentityFixture) -> Arc<ServerConfig> {
    ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("ring provider supports the safe default protocol versions")
        .with_no_client_auth()
        .with_single_cert(vec![fixture.cert.clone()], fixture.key.clone_key())
        .map(Arc::new)
        .expect("synthetic certificate and key are consistent")
}

/// Builds a server configuration that presents a valid, trusted certificate but
/// signs the handshake with a foreign private key.
///
/// The presented chain is the genuine `dns.example` leaf under the trusted root
/// A, so the chain, service name, and validity window all pass. Only the
/// signature over the handshake transcript is wrong. This isolates the handshake
/// signature check from every certificate check, which makes the failure
/// unambiguous in both TLS modes.
///
/// It uses the crate's own key loader (the same one `with_single_cert` uses) but
/// skips that constructor's key/cert consistency check, which is what prevents
/// this otherwise-invalid pairing from being constructed the usual way.
fn mismatched_handshake_key_server_config(set: &FixtureSet) -> Arc<ServerConfig> {
    use rustls::sign::CertifiedKey;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    // The trusted leaf, paired with the private key of a different certificate.
    let cert_chain = vec![set.good.cert.clone()];
    let foreign_key = set.wrong_name.key.clone_key();
    let signing_key = provider
        .key_provider
        .load_private_key(foreign_key)
        .expect("the foreign key loads through the same key provider");
    let certified = CertifiedKey::new(cert_chain, signing_key);

    ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("ring provider supports the safe default protocol versions")
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(rustls::sign::SingleCertAndKey::from(certified)))
        .into()
}

/// Binds a fresh blocking TCP listener on the IPv4 loopback.
fn bind_listener() -> (TcpListener, SocketAddr) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind ipv4 loopback");
    let address = listener.local_addr().expect("local address");
    (listener, address)
}

/// A `DoT` endpoint for `identity` dialing `address`.
fn dot_endpoint(address: SocketAddr, identity: &str) -> DotEndpoint {
    DotEndpoint::new(
        address,
        ServerIdentity::new(identity).expect("valid service identity"),
    )
    .expect("valid dot endpoint")
}

/// A verified `DoT` owner dialing `address` as `identity` against root A.
fn verified_owner(set: &FixtureSet, address: SocketAddr, identity: &str) -> DotUpstream {
    DotUpstream::new(
        dot_endpoint(address, identity),
        TlsPolicy::verified(set.root_store_a()).expect("verified policy"),
    )
    .expect("owner")
}

fn open_context() -> ExchangeContext {
    ExchangeContext::new(
        Instant::now() + Duration::from_secs(30),
        TransportCancellation::new(),
    )
}

/// Reads exactly one framed DNS message.
async fn read_framed(stream: &mut TlsStream<TcpStream>) -> Vec<u8> {
    let mut prefix = [0u8; 2];
    stream.read_exact(&mut prefix).await.expect("read prefix");
    let length = usize::from(u16::from_be_bytes(prefix));
    assert!(length > 0, "the server must receive a non-zero frame");
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).await.expect("read body");
    body
}

/// Writes one framed DNS message in deterministic `chunk`-sized pieces.
async fn write_framed_in_chunks(stream: &mut TlsStream<TcpStream>, body: &[u8], chunk: usize) {
    let length = u16::try_from(body.len()).expect("response fits the u16 prefix");
    let mut frame = Vec::with_capacity(body.len() + 2);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(body);
    for piece in frame.chunks(chunk) {
        stream.write_all(piece).await.expect("write piece");
        stream.flush().await.expect("flush piece");
    }
}

/// A boxed per-connection server script.
type Script = std::pin::Pin<Box<dyn Future<Output = ()> + Send>>;

/// A scripted TLS server driven on its own thread and runtime.
///
/// The listener is bound on the calling thread and moved to a dedicated OS
/// thread owning a current-thread runtime, so the exchange under test runs on
/// its own runtime while the server performs a real TLS handshake. Scripts are
/// async, so they never nest `block_on` inside that runtime.
struct TlsServer {
    address: SocketAddr,
    handle: std::thread::JoinHandle<()>,
}

impl TlsServer {
    /// Starts a server presenting `fixture` that runs `script` for each of
    /// `expected` connections.
    fn start<F, Fut>(fixture: &ServerIdentityFixture, expected: usize, script: F) -> Self
    where
        F: FnMut(TlsStream<TcpStream>, usize) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Self::start_with_config(server_config(fixture), expected, script)
    }

    /// Starts a server from an explicit configuration.
    fn start_with_config<F, Fut>(config: Arc<ServerConfig>, expected: usize, script: F) -> Self
    where
        F: FnMut(TlsStream<TcpStream>, usize) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let (listener, address) = bind_listener();
        listener
            .set_nonblocking(true)
            .expect("listener non-blocking");
        let handle = std::thread::spawn(move || {
            let mut script = script;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build server runtime");
            runtime.block_on(async move {
                let listener =
                    AsyncTcpListener::from_std(listener).expect("adopt the standard listener");
                for index in 0..expected {
                    let accepted = timeout(TEST_TIMEOUT, listener.accept())
                        .await
                        .expect("a connection arrives within the bounded wait")
                        .expect("accept succeeds");
                    let (stream, _) = accepted;
                    let acceptor = TlsAcceptor::from(Arc::clone(&config));
                    match timeout(TEST_TIMEOUT, acceptor.accept(stream)).await {
                        Ok(Ok(tls)) => script(tls, index).await,
                        // A rejected handshake is a valid outcome for the
                        // certificate-failure cases; the script is not run.
                        Ok(Err(_)) => {}
                        Err(elapsed) => panic!("server handshake timed out: {elapsed}"),
                    }
                }
            });
        });
        Self { address, handle }
    }

    fn join(self) {
        self.handle.join().expect("server thread joined");
    }
}

/// Starts a raw (non-TLS) listener that accepts one connection and reports it.
///
/// The listener never speaks TLS, so the client stays parked inside its
/// handshake. The accepted connection is signalled through the returned
/// receiver, which is what lets a test order a control after the TCP connect
/// without any sleep: the signal *is* the ordering evidence.
fn silent_listener_signalled() -> (
    SocketAddr,
    oneshot::Receiver<()>,
    std::thread::JoinHandle<()>,
) {
    let (listener, address) = bind_listener();
    listener
        .set_nonblocking(true)
        .expect("listener non-blocking");
    let (connected_tx, connected_rx) = oneshot::channel::<()>();
    let handle = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build server runtime");
        runtime.block_on(async move {
            let listener = AsyncTcpListener::from_std(listener).expect("adopt listener");
            let (mut stream, _) = timeout(TEST_TIMEOUT, listener.accept())
                .await
                .expect("a connection arrives")
                .expect("accept succeeds");
            connected_tx
                .send(())
                .expect("the test observes the established connection");
            // Read until the client goes away; returning on EOF keeps the
            // server thread joinable without any timing dependency.
            let mut scratch = [0u8; 64];
            while let Ok(read) = stream.read(&mut scratch).await {
                if read == 0 {
                    return;
                }
            }
        });
    });
    (address, connected_rx, handle)
}

/// Runs one exchange for `query` and returns its typed outcome.
async fn exchange_owned(
    upstream: &DotUpstream,
    query: &[u8],
    context: ExchangeContext,
) -> Result<SecureResponse, SecureError> {
    let request = ExchangeRequest::new(query).expect("valid query");
    upstream.exchange(request, context).await
}

/// Spawns one exchange so a test can apply a control while it is parked.
fn spawn_exchange(
    upstream: &Arc<DotUpstream>,
    query: &[u8],
    context: ExchangeContext,
) -> tokio::task::JoinHandle<Result<SecureResponse, SecureError>> {
    let upstream = Arc::clone(upstream);
    let query = query.to_vec();
    tokio::spawn(async move {
        let request = ExchangeRequest::new(&query).expect("valid query");
        upstream.exchange(request, context).await
    })
}

/// A script that reads one query frame, signals consumption, then holds the
/// connection open without ever answering.
fn holding_script(
    consumed: oneshot::Sender<()>,
) -> impl FnMut(TlsStream<TcpStream>, usize) -> Script {
    let consumed = std::sync::Mutex::new(Some(consumed));
    move |mut tls: TlsStream<TcpStream>, _index| {
        let sender = consumed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        Box::pin(async move {
            let _ = read_framed(&mut tls).await;
            if let Some(sender) = sender {
                sender.send(()).expect("the test waits for consumption");
            }
            // Hold the stream open: the client must terminate the exchange
            // through a control, not by the server answering.
            std::future::pending::<()>().await;
        }) as Script
    }
}

/// A script that reads one query frame, then writes `reply` in `chunk` pieces.
///
/// The reply is cloned per connection so the closure stays reusable.
fn reply_script(reply: Vec<u8>, chunk: usize) -> impl FnMut(TlsStream<TcpStream>, usize) -> Script {
    move |mut tls: TlsStream<TcpStream>, _index| {
        let reply = reply.clone();
        Box::pin(async move {
            let _ = read_framed(&mut tls).await;
            write_framed_in_chunks(&mut tls, &reply, chunk).await;
        }) as Script
    }
}

// ---------------------------------------------------------------------------
// Certificate authentication matrix
// ---------------------------------------------------------------------------

#[test]
fn trusted_certificate_for_the_service_identity_completes_the_exchange() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let id = 0x5101;
        let expected = response_wire(id, 1);
        let server = TlsServer::start(&set.good, 1, reply_script(expected.clone(), 3));

        let upstream = verified_owner(&set, server.address, "dns.example");
        let query = query_wire(id);
        let expected_query = query.clone();
        let response = exchange_owned(&upstream, &query, open_context())
            .await
            .expect("a trusted certificate authenticates the service");

        assert_eq!(response.transport(), SecureTransport::Dot);
        assert_eq!(response.request_id(), id);
        assert_eq!(response.response_id(), id);
        assert_eq!(response.wire(), expected.as_slice());
        assert!(!response.truncated());
        assert_eq!(query, expected_query, "the caller query must not change");
        assert_eq!(upstream.in_flight_exchanges(), 0);

        server.join();
    });
}

#[test]
fn certificate_for_a_different_name_is_rejected_before_any_query() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let id = 0x5102;
        // The server presents a valid, trusted certificate covering only
        // `other.example`, so the service-name check must fail.
        let server = TlsServer::start(&set.wrong_name, 1, |_tls, _| async {});

        let upstream = verified_owner(&set, server.address, "dns.example");
        let query = query_wire(id);
        let expected_query = query.clone();
        let error = exchange_owned(&upstream, &query, open_context())
            .await
            .expect_err("a name mismatch must fail the handshake");

        assert_eq!(
            error,
            SecureError::Tls(TlsHandshakeFailure::Certificate(
                CertificateRejection::NotValidForName
            ))
        );
        // The handshake failed before any DNS byte, so the query was provably
        // never sent.
        assert_eq!(error.side_effect(), SideEffectState::NotSent);
        assert_eq!(query, expected_query, "the caller query must not change");
        assert_eq!(upstream.in_flight_exchanges(), 0);

        server.join();
    });
}

#[test]
fn expired_certificate_is_rejected_before_any_query() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let id = 0x5103;
        let server = TlsServer::start(&set.expired, 1, |_tls, _| async {});

        let upstream = verified_owner(&set, server.address, "dns.example");
        let error = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect_err("an expired certificate must fail the handshake");

        assert_eq!(
            error,
            SecureError::Tls(TlsHandshakeFailure::Certificate(
                CertificateRejection::Expired
            ))
        );
        assert_eq!(error.side_effect(), SideEffectState::NotSent);
        assert_eq!(upstream.in_flight_exchanges(), 0);

        server.join();
    });
}

#[test]
fn certificate_from_an_untrusted_issuer_is_rejected_before_any_query() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let id = 0x5104;
        // Valid for `dns.example` and unexpired, but issued by root B, which is
        // deliberately absent from the client's trust store.
        let server = TlsServer::start(&set.unknown_issuer, 1, |_tls, _| async {});

        let upstream = verified_owner(&set, server.address, "dns.example");
        let error = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect_err("an untrusted issuer must fail the handshake");

        assert_eq!(
            error,
            SecureError::Tls(TlsHandshakeFailure::Certificate(
                CertificateRejection::UnknownIssuer
            ))
        );
        assert_eq!(error.side_effect(), SideEffectState::NotSent);
        assert_eq!(upstream.in_flight_exchanges(), 0);

        server.join();
    });
}

#[test]
fn the_same_untrusted_certificate_verifies_under_its_own_root() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        // Proves the unknown-issuer case above is about the anchor set and not
        // an unparseable certificate: the same leaf authenticates when root B
        // is supplied as the trust anchor.
        let id = 0x5105;
        let expected = response_wire(id, 5);
        let server = TlsServer::start(&set.unknown_issuer, 1, reply_script(expected.clone(), 4));

        let upstream = DotUpstream::new(
            dot_endpoint(server.address, "dns.example"),
            TlsPolicy::verified(set.root_store_b()).expect("verified policy"),
        )
        .expect("owner");

        let response = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect("the leaf authenticates against its real issuer");
        assert_eq!(response.wire(), expected.as_slice());

        server.join();
    });
}

#[test]
fn insecure_policy_accepts_an_untrusted_certificate_only_when_explicitly_chosen() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let id = 0x5106;
        let expected = response_wire(id, 6);
        let server = TlsServer::start(&set.unknown_issuer, 1, reply_script(expected.clone(), 8));

        // The explicit opt-in policy is the only way to reach this outcome; the
        // verified policy for the same endpoint fails, as asserted above.
        let upstream = DotUpstream::new(
            dot_endpoint(server.address, "dns.example"),
            TlsPolicy::insecure_skip_verify(),
        )
        .expect("owner");

        let response = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect("the explicit insecure policy skips chain/name checks");
        assert_eq!(response.wire(), expected.as_slice());

        server.join();
    });
}

#[test]
fn a_verified_policy_never_retries_after_a_rejection() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let id = 0x5107;
        // The server accepts exactly one connection. A hidden insecure retry or
        // a second connection attempt would leave the client blocked and the
        // server waiting, so this test would time out instead.
        let server = TlsServer::start(&set.unknown_issuer, 1, |_tls, _| async {});

        let upstream = verified_owner(&set, server.address, "dns.example");
        let error = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect_err("verification failure is terminal");
        assert!(matches!(error, SecureError::Tls(_)));
        assert_eq!(error.side_effect(), SideEffectState::NotSent);
        assert_eq!(upstream.in_flight_exchanges(), 0);

        server.join();
    });
}

#[test]
fn a_bad_handshake_signature_fails_even_under_the_insecure_policy() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        // The chain is valid and the leaf is trusted, but the server signs the
        // handshake with a foreign key. An insecure policy skips chain, name and
        // time checks only; it must still reject the handshake signature, which
        // proves the provider's TLS1.2/1.3 signature verification stays enforced.
        let config = mismatched_handshake_key_server_config(&set);
        let server = TlsServer::start_with_config(config, 1, |_tls, _| async {});

        let upstream = DotUpstream::new(
            dot_endpoint(server.address, "dns.example"),
            TlsPolicy::insecure_skip_verify(),
        )
        .expect("owner");

        let error = exchange_owned(&upstream, &query_wire(0x5108), open_context())
            .await
            .expect_err("a bad handshake signature must fail in every mode");
        // The provider's signature check rejects the handshake exactly as it
        // would under verified TLS, so skipping chain/name/time checks did not
        // disable the cryptographic handshake proof.
        assert_eq!(
            error,
            SecureError::Tls(TlsHandshakeFailure::Certificate(
                CertificateRejection::BadSignature
            ))
        );
        // The handshake failed before any DNS byte.
        assert_eq!(error.side_effect(), SideEffectState::NotSent);
        assert_eq!(upstream.in_flight_exchanges(), 0);

        server.join();
    });
}

#[test]
fn a_bad_handshake_signature_also_fails_under_the_verified_policy() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let config = mismatched_handshake_key_server_config(&set);
        let server = TlsServer::start_with_config(config, 1, |_tls, _| async {});

        let upstream = verified_owner(&set, server.address, "dns.example");
        let error = exchange_owned(&upstream, &query_wire(0x5109), open_context())
            .await
            .expect_err("a bad handshake signature must fail under verified TLS");
        assert_eq!(
            error,
            SecureError::Tls(TlsHandshakeFailure::Certificate(
                CertificateRejection::BadSignature
            ))
        );
        assert_eq!(error.side_effect(), SideEffectState::NotSent);
        assert_eq!(upstream.in_flight_exchanges(), 0);

        server.join();
    });
}

// ---------------------------------------------------------------------------
// TLS flush, framing, and DNS response validation
// ---------------------------------------------------------------------------
#[test]
fn fragmented_response_frame_is_reassembled_across_tls_records() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let id = 0x5201;
        let expected = response_wire(id, 7);
        // One byte per record forces both the prefix and the body across many
        // TLS records and reads.
        let server = TlsServer::start(&set.good, 1, reply_script(expected.clone(), 1));

        let upstream = verified_owner(&set, server.address, "dns.example");
        let response = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect("the exact frame is reassembled from fragments");
        assert_eq!(response.wire(), expected.as_slice());

        server.join();
    });
}

#[test]
fn a_complete_truncated_response_is_returned_without_plaintext_fallback() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let id = 0x5202;
        let expected = truncated_response_wire(id, 8);
        // One connection only: a TC response must never trigger a retry or a
        // plaintext TCP fallback.
        let server = TlsServer::start(&set.good, 1, reply_script(expected.clone(), 16));

        let upstream = verified_owner(&set, server.address, "dns.example");
        let response = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect("a complete TC response is a valid DoT result");
        assert!(response.truncated(), "the TC bit is reported");
        assert_eq!(response.wire(), expected.as_slice());
        assert_eq!(response.request_id(), id);
        assert_eq!(response.response_id(), id);

        server.join();
    });
}

#[test]
fn a_response_with_a_different_transaction_id_is_a_mismatch() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let id = 0x5203;
        // The server answers with a different ID while the wire stays valid.
        let server = TlsServer::start(&set.good, 1, reply_script(response_wire(id ^ 1, 9), 16));

        let upstream = verified_owner(&set, server.address, "dns.example");
        let error = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect_err("a wrong response ID must be rejected");
        assert_eq!(
            error,
            SecureError::Transport(UpstreamError::ResponseMismatch)
        );
        // The query frame was fully flushed before the response was read.
        assert_eq!(error.side_effect(), SideEffectState::Sent);

        server.join();
    });
}

#[test]
fn a_complete_frame_with_invalid_dns_wire_is_a_malformed_response() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let id = 0x5204;
        let server = TlsServer::start(
            &set.good,
            1,
            reply_script(invalid_dns_response_wire(id), 16),
        );

        let upstream = verified_owner(&set, server.address, "dns.example");
        let error = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect_err("an invalid DNS wire must be rejected");
        assert_eq!(
            error,
            SecureError::Transport(UpstreamError::MalformedResponse)
        );
        assert_eq!(error.side_effect(), SideEffectState::Sent);

        server.join();
    });
}

#[test]
fn a_zero_length_inbound_prefix_is_a_malformed_response() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let id = 0x5205;
        // A zero-length DNS frame cannot be a response; the server writes a
        // bare 0x0000 prefix.
        let server = TlsServer::start(&set.good, 1, |mut tls, _| async move {
            let _ = read_framed(&mut tls).await;
            tls.write_all(&[0x00, 0x00]).await.expect("write prefix");
            tls.flush().await.expect("flush prefix");
        });

        let upstream = verified_owner(&set, server.address, "dns.example");
        let error = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect_err("a zero-length frame is malformed");
        assert_eq!(
            error,
            SecureError::Transport(UpstreamError::MalformedResponse)
        );

        server.join();
    });
}

#[test]
fn eof_before_a_complete_prefix_is_a_truncated_frame() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let id = 0x5206;
        let server = TlsServer::start(&set.good, 1, |mut tls, _| async move {
            let _ = read_framed(&mut tls).await;
            // Half a length prefix, then close.
            tls.write_all(&[0x00]).await.expect("write half prefix");
            tls.flush().await.expect("flush half prefix");
            tls.shutdown().await.expect("close the server stream");
        });

        let upstream = verified_owner(&set, server.address, "dns.example");
        let error = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect_err("a partial prefix cannot complete a frame");
        assert_eq!(error, SecureError::Transport(UpstreamError::TruncatedFrame));
        assert_eq!(error.side_effect(), SideEffectState::Sent);

        server.join();
    });
}

#[test]
fn eof_during_the_declared_body_is_a_truncated_frame() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let id = 0x5207;
        let server = TlsServer::start(&set.good, 1, |mut tls, _| async move {
            let _ = read_framed(&mut tls).await;
            // Declare 8 body bytes but send only 3.
            tls.write_all(&[0x00, 0x08, 0x01, 0x02, 0x03])
                .await
                .expect("write partial body");
            tls.flush().await.expect("flush partial body");
            tls.shutdown().await.expect("close the server stream");
        });

        let upstream = verified_owner(&set, server.address, "dns.example");
        let error = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect_err("a partial body cannot complete a frame");
        assert_eq!(error, SecureError::Transport(UpstreamError::TruncatedFrame));

        server.join();
    });
}

#[test]
fn an_oversized_outbound_query_is_rejected_before_any_connection() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        // A query over u16::MAX cannot be represented by the two-byte DoT
        // prefix, so it must fail before any socket work. The endpoint points at
        // a bound-but-unused loopback address, so a connection attempt would be
        // observable.
        let (listener, address) = bind_listener();
        listener
            .set_nonblocking(true)
            .expect("listener non-blocking");

        let upstream = verified_owner(&set, address, "dns.example");
        let mut query = query_wire(0x5208);
        query[10..12].copy_from_slice(&1u16.to_be_bytes()); // ARCOUNT
        query.resize(usize::from(u16::MAX) + 1, 0);

        let error = exchange_owned(&upstream, &query, open_context())
            .await
            .expect_err("an unframeable query must be rejected");
        assert_eq!(error, SecureError::Transport(UpstreamError::FrameTooLarge));
        assert_eq!(error.side_effect(), SideEffectState::NotSent);

        match listener.accept() {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Ok(_) => panic!("an oversized query must never open a connection"),
            Err(error) => panic!("unexpected accept error: {error}"),
        }
        assert_eq!(upstream.in_flight_exchanges(), 0);
    });
}

// ---------------------------------------------------------------------------
// Control: owner close, caller cancellation, deadline
// ---------------------------------------------------------------------------

#[test]
fn owner_close_while_waiting_for_the_response_is_closed_with_sent() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let id = 0x5301;
        let (consumed_tx, consumed_rx) = oneshot::channel::<()>();
        let server = TlsServer::start(&set.good, 1, holding_script(consumed_tx));

        let upstream = Arc::new(verified_owner(&set, server.address, "dns.example"));
        let exchange = spawn_exchange(&upstream, &query_wire(id), open_context());

        timeout(TEST_TIMEOUT, consumed_rx)
            .await
            .expect("query consumption observed bounded")
            .expect("query consumed");
        assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);

        let error = timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("closed exchange bounded")
            .expect("exchange task joined")
            .expect_err("owner close terminates the response wait");
        assert_eq!(
            error,
            SecureError::Transport(UpstreamError::Closed(SideEffectState::Sent))
        );

        assert_eq!(upstream.close().await, CloseResult::Closed);
        assert_eq!(upstream.in_flight_exchanges(), 0);
        drop(server);
    });
}

#[test]
fn caller_cancellation_while_waiting_for_the_response_is_cancelled_with_sent() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let id = 0x5302;
        let (consumed_tx, consumed_rx) = oneshot::channel::<()>();
        let server = TlsServer::start(&set.good, 1, holding_script(consumed_tx));

        let upstream = Arc::new(verified_owner(&set, server.address, "dns.example"));
        let cancellation = TransportCancellation::new();
        let context = ExchangeContext::new(
            Instant::now() + Duration::from_secs(30),
            cancellation.clone(),
        );
        let exchange = spawn_exchange(&upstream, &query_wire(id), context);

        timeout(TEST_TIMEOUT, consumed_rx)
            .await
            .expect("query consumption observed bounded")
            .expect("query consumed");
        cancellation.cancel();

        let error = timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("cancelled exchange bounded")
            .expect("exchange task joined")
            .expect_err("caller cancellation terminates the response wait");
        assert_eq!(
            error,
            SecureError::Transport(UpstreamError::Cancelled(SideEffectState::Sent))
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);
        drop(server);
    });
}

#[test]
fn absolute_deadline_while_waiting_for_the_response_is_deadline_exceeded_with_sent() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let id = 0x5303;
        let (consumed_tx, consumed_rx) = oneshot::channel::<()>();
        let server = TlsServer::start(&set.good, 1, holding_script(consumed_tx));

        let upstream = Arc::new(verified_owner(&set, server.address, "dns.example"));
        // One absolute deadline for the whole exchange. Loopback connect,
        // handshake, write, and flush all finish far below it, so only the
        // response wait can be terminated by it.
        let context = ExchangeContext::new(
            Instant::now() + Duration::from_millis(750),
            TransportCancellation::new(),
        );
        let exchange = spawn_exchange(&upstream, &query_wire(id), context);

        timeout(TEST_TIMEOUT, consumed_rx)
            .await
            .expect("query consumption observed bounded")
            .expect("query consumed");

        let error = timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("deadline-terminated exchange bounded")
            .expect("exchange task joined")
            .expect_err("the absolute deadline terminates the response wait");
        assert_eq!(
            error,
            SecureError::Transport(UpstreamError::DeadlineExceeded(SideEffectState::Sent))
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);
        drop(server);
    });
}

#[test]
fn cancellation_after_the_tcp_connect_sends_no_query() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        // The server accepts the TCP connection and signals the test; it never
        // speaks TLS, so the client stays in its handshake. The test proceeds
        // only on that explicit signal, so ordering is deterministic and no
        // sleep stands in for "the client has connected".
        let (address, connected_rx, server) = silent_listener_signalled();

        let upstream = Arc::new(verified_owner(&set, address, "dns.example"));
        let cancellation = TransportCancellation::new();
        let context = ExchangeContext::new(
            Instant::now() + Duration::from_secs(30),
            cancellation.clone(),
        );
        let exchange = spawn_exchange(&upstream, &query_wire(0x5304), context);

        timeout(TEST_TIMEOUT, connected_rx)
            .await
            .expect("the client's TCP connection is established before cancellation")
            .expect("the listener observed the connection");
        assert_eq!(upstream.in_flight_exchanges(), 1);
        cancellation.cancel();

        let error = timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("cancelled handshake bounded")
            .expect("exchange task joined")
            .expect_err("cancellation terminates the handshake");
        // The handshake never completed, so no DNS byte was ever sent.
        assert_eq!(
            error,
            SecureError::Transport(UpstreamError::Cancelled(SideEffectState::NotSent))
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);
        drop(server);
    });
}

#[test]
fn owner_close_after_the_tcp_connect_sends_no_query() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let (address, connected_rx, server) = silent_listener_signalled();

        let upstream = Arc::new(verified_owner(&set, address, "dns.example"));
        let exchange = spawn_exchange(&upstream, &query_wire(0x5305), open_context());

        timeout(TEST_TIMEOUT, connected_rx)
            .await
            .expect("the client's TCP connection is established before close")
            .expect("the listener observed the connection");
        assert_eq!(upstream.in_flight_exchanges(), 1);
        assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);

        let error = timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("closed handshake bounded")
            .expect("exchange task joined")
            .expect_err("owner close terminates the handshake");
        assert_eq!(
            error,
            SecureError::Transport(UpstreamError::Closed(SideEffectState::NotSent))
        );

        assert_eq!(upstream.close().await, CloseResult::Closed);
        assert_eq!(upstream.in_flight_exchanges(), 0);
        drop(server);
    });
}

#[test]
fn a_dropped_exchange_future_releases_its_registration() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let (consumed_tx, consumed_rx) = oneshot::channel::<()>();
        let server = TlsServer::start(&set.good, 1, holding_script(consumed_tx));

        let upstream = Arc::new(verified_owner(&set, server.address, "dns.example"));
        let exchange = spawn_exchange(&upstream, &query_wire(0x5306), open_context());

        timeout(TEST_TIMEOUT, consumed_rx)
            .await
            .expect("query consumption observed bounded")
            .expect("query consumed");
        assert_eq!(upstream.in_flight_exchanges(), 1);

        // Aborting the future drops the prepared exchange, which releases the
        // RAII registration guard without any explicit cleanup step.
        exchange.abort();
        let _ = exchange.await;

        assert_eq!(
            upstream.in_flight_exchanges(),
            0,
            "an aborted exchange must release its registration"
        );
        // The owner still drains to Closed; no registration is left behind.
        assert_eq!(upstream.close().await, CloseResult::Closed);
        drop(server);
    });
}

#[test]
fn exchanges_after_close_are_rejected_without_opening_a_connection() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let (listener, address) = bind_listener();
        listener
            .set_nonblocking(true)
            .expect("listener non-blocking");

        let upstream = verified_owner(&set, address, "dns.example");
        assert_eq!(upstream.close().await, CloseResult::Closed);

        let error = exchange_owned(&upstream, &query_wire(0x5307), open_context())
            .await
            .expect_err("a closed owner rejects new exchanges");
        assert_eq!(
            error,
            SecureError::Transport(UpstreamError::Closed(SideEffectState::NotSent))
        );

        match listener.accept() {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Ok(_) => panic!("a closed owner must not open a connection"),
            Err(error) => panic!("unexpected accept error: {error}"),
        }
    });
}

// ---------------------------------------------------------------------------
// Fresh connection per exchange and concurrent isolation
// ---------------------------------------------------------------------------

#[test]
fn sequential_exchanges_open_a_fresh_authenticated_connection_each_time() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let first_id = 0x5401;
        let second_id = 0x5402;
        let first_expected = response_wire(first_id, 21);
        let second_expected = response_wire(second_id, 22);

        // Each connection is answered according to its index, and the ID the
        // server received must match that response, so a reused connection
        // carrying a stale exchange would be detected.
        let server = TlsServer::start(&set.good, 2, move |mut tls, index| {
            let reply = if index == 0 {
                first_expected.clone()
            } else {
                second_expected.clone()
            };
            async move {
                let received = read_framed(&mut tls).await;
                assert_eq!(
                    u16::from_be_bytes([received[0], received[1]]),
                    u16::from_be_bytes([reply[0], reply[1]])
                );
                write_framed_in_chunks(&mut tls, &reply, 4).await;
            }
        });

        let upstream = verified_owner(&set, server.address, "dns.example");

        let first = exchange_owned(&upstream, &query_wire(first_id), open_context())
            .await
            .expect("first exchange succeeds on its own connection");
        assert_eq!(first.wire(), response_wire(first_id, 21).as_slice());
        assert_eq!(upstream.in_flight_exchanges(), 0);

        let second = exchange_owned(&upstream, &query_wire(second_id), open_context())
            .await
            .expect("second exchange succeeds on a fresh connection");
        assert_eq!(second.wire(), response_wire(second_id, 22).as_slice());
        assert_eq!(upstream.in_flight_exchanges(), 0);

        server.join();
    });
}

#[test]
fn concurrent_exchanges_use_separate_connections_and_owned_responses() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let first_id = 0x5403;
        let second_id = 0x5404;
        // Answer with the response matching the ID this connection received, so
        // cross-talk between the two connections is observable as an ID
        // mismatch instead of silently passing.
        let server = TlsServer::start(&set.good, 2, |mut tls, _| async move {
            let received = read_framed(&mut tls).await;
            if u16::from_be_bytes([received[0], received[1]]) == 0x5403 {
                write_framed_in_chunks(&mut tls, &response_wire(0x5403, 31), 5).await;
            } else {
                write_framed_in_chunks(&mut tls, &response_wire(0x5404, 32), 5).await;
            }
        });

        let upstream = Arc::new(verified_owner(&set, server.address, "dns.example"));
        let first = spawn_exchange(&upstream, &query_wire(first_id), open_context());
        let second = spawn_exchange(&upstream, &query_wire(second_id), open_context());

        let first = timeout(TEST_TIMEOUT, first)
            .await
            .expect("first exchange bounded")
            .expect("first task joined")
            .expect("first exchange succeeds");
        let second = timeout(TEST_TIMEOUT, second)
            .await
            .expect("second exchange bounded")
            .expect("second task joined")
            .expect("second exchange succeeds");

        // Each caller receives exactly the response for its own request ID.
        assert_eq!(first.request_id(), first_id);
        assert_eq!(first.response_id(), first_id);
        assert_eq!(first.wire(), response_wire(first_id, 31).as_slice());
        assert_eq!(second.request_id(), second_id);
        assert_eq!(second.response_id(), second_id);
        assert_eq!(second.wire(), response_wire(second_id, 32).as_slice());
        assert_eq!(upstream.in_flight_exchanges(), 0);

        server.join();
    });
}

#[test]
fn owner_close_drains_a_concurrent_exchange_before_reporting_closed() {
    block_on(async {
        // Every test generates its own fresh fixture material; no key is
        // committed or shared between tests.
        let set = FixtureSet::generate();
        let (consumed_tx, consumed_rx) = oneshot::channel::<()>();
        let server = TlsServer::start(&set.good, 1, holding_script(consumed_tx));

        let upstream = Arc::new(verified_owner(&set, server.address, "dns.example"));
        let exchange = spawn_exchange(&upstream, &query_wire(0x5405), open_context());

        timeout(TEST_TIMEOUT, consumed_rx)
            .await
            .expect("query consumption observed bounded")
            .expect("query consumed");

        // The close must not report Closed while the exchange is registered.
        let close = {
            let upstream = Arc::clone(&upstream);
            tokio::spawn(async move { upstream.close().await })
        };
        let closed = timeout(TEST_TIMEOUT, close)
            .await
            .expect("close bounded")
            .expect("close task joined");
        assert_eq!(closed, CloseResult::Closed);
        assert_eq!(upstream.in_flight_exchanges(), 0);

        let error = timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("exchange bounded after close")
            .expect("exchange task joined")
            .expect_err("owner close terminates the exchange");
        assert_eq!(
            error,
            SecureError::Transport(UpstreamError::Closed(SideEffectState::Sent))
        );

        drop(server);
    });
}

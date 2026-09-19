//! Slice2 contract test for the one-shot DNS-over-HTTP/3 primitive.
//!
//! The test runs a deterministic in-process HTTP/3 server over an ephemeral IPv4
//! loopback port and drives real [`Doh3Upstream::exchange`] calls. The server
//! presents a synthetic leaf issued by the fixture's trusted root and offers
//! exactly the `h3` ALPN, so the client's `TlsPolicy`-derived configuration
//! performs a real certificate/identity verification against local material.
//!
//! The asserted public contract is the successful one-shot shape plus the `DoH`
//! response contract on the H3 transport:
//!
//! * the request is exactly one `GET` whose `:authority` is the endpoint
//!   authority and whose `:path` is byte-equal to `DohEndpoint::get_request_target`,
//!   with `Accept: application/dns-message`, no body, no `User-Agent`, and no
//!   `Content-Encoding`;
//! * the client sends the request send-side FIN, observed by the server reading
//!   the request stream to its end;
//! * the exchange reports `SecureTransport::Doh3` with `Some(Http3)` and the
//!   caller's original request/response ID restored;
//! * every exchange opens exactly one fresh connection (two exchanges are two
//!   accepted connections, one request each);
//! * the response must be `200`, `application/dns-message` (case-insensitive,
//!   parameters allowed), identity-encoded, at most 64 headers, at most 16 KiB
//!   of head, with a complete bounded body of at most 65535 bytes.
//!
//! Negative coverage: non-200, wrong/missing media type, non-identity encoding,
//! a declared or actual body above the DNS maximum, an incomplete body, too many
//! response headers, a peer that closes before any response head, and an
//! ALPN/TLS/identity handshake failure that must be `NotSent` with no fallback
//! to DoH/HTTP-2/HTTP-1 (a TCP listener sharing the QUIC port observes nothing).
//!
//! Control coverage is limited to the Slice2 ownership contract: caller
//! cancellation, owner close, a dropped exchange future, and an exchange after
//! close each leave zero in-flight registrations and no second connection. The
//! full deadline/cancellation precedence and stream-error-code matrix are
//! Slice 3 and are deliberately not asserted here.

mod fixtures;

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use fixtures::FixtureSet;
use hyper::body::{Buf as _, Bytes};
use mosdns_upstream_core::quic::{Doh3Upstream, H3_ALPN};
use mosdns_upstream_core::secure::{
    DohEndpoint, DohProtocolError, SecureError, SecureHttpVersion, SecureResponse, SecureTransport,
    TlsPolicy,
};
use mosdns_upstream_core::{
    CloseResult, CloseTransition, ExchangeContext, ExchangeRequest, SideEffectState,
    TransportCancellation, UpstreamError,
};
use quinn::crypto::rustls::{HandshakeData, QuicServerConfig};
use rustls::ServerConfig;
use tokio::sync::oneshot;
use tokio::time::timeout;

/// Bounds every socket phase so a broken path fails instead of hanging.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);
/// The bounded client deadline handed to the exchange.
const EXCHANGE_DEADLINE: Duration = Duration::from_secs(10);
/// A bounded probe after the first connection closes, used to prove the client
/// did not open a second connection.
const ACCEPT_PROBE: Duration = Duration::from_millis(200);

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

/// A scripted HTTP/3 response the fixture server should send.
#[derive(Clone, Debug)]
struct ScriptedH3Response {
    status: u16,
    /// The `content-type` header, omitted when `None`.
    content_type: Option<String>,
    /// Additional response headers, written verbatim.
    extra_headers: Vec<(String, String)>,
    /// The response body bytes.
    body: Vec<u8>,
    /// The `content-length` header, omitted when `None`.
    declared_length: Option<u64>,
}

impl ScriptedH3Response {
    /// A `200` response carrying `body` as `application/dns-message`.
    fn ok_dns(body: Vec<u8>) -> Self {
        let declared_length = u64::try_from(body.len()).expect("body length fits u64");
        Self {
            status: 200,
            content_type: Some("application/dns-message".to_owned()),
            extra_headers: Vec::new(),
            body,
            declared_length: Some(declared_length),
        }
    }
}

/// What the scripted fixture server does with the one request on a connection.
enum Behavior {
    /// Answer the request with a clone of this response.
    Respond(Box<ScriptedH3Response>),
    /// Read the request and then withhold every response byte until the client
    /// closes the connection.
    Hold,
    /// Read the request and then close the whole connection without a response.
    CloseWithoutResponse,
}

/// What the server observed from one accepted connection and its one request.
#[derive(Clone, Debug)]
struct H3Evidence {
    /// The ALPN protocol the handshake negotiated.
    alpn: Option<Vec<u8>>,
    method: String,
    authority: String,
    path: String,
    headers: Vec<(String, String)>,
    request_body: Vec<u8>,
    /// Whether the request stream was read to its end, which is the client's
    /// request send-side FIN observation.
    request_fin: bool,
}

impl H3Evidence {
    /// The first value of `name`, compared case-insensitively.
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Builds a QUIC server configuration presenting `cert`/`key` with exactly
/// `alpn`.
fn quic_server_config(
    cert: rustls::pki_types::CertificateDer<'static>,
    key: rustls::pki_types::PrivateKeyDer<'static>,
    alpn: Vec<Vec<u8>>,
) -> quinn::ServerConfig {
    let mut tls =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("ring provider supports TLS 1.3")
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .expect("synthetic certificate and key are consistent");
    tls.alpn_protocols = alpn;
    let quic = QuicServerConfig::try_from(tls).expect("a TLS1.3 config converts to QUIC");
    quinn::ServerConfig::with_crypto(Arc::new(quic))
}

/// The fixture's trusted server configuration offering exactly `h3`.
fn h3_server_config(set: &FixtureSet) -> quinn::ServerConfig {
    quic_server_config(
        set.good.cert.clone(),
        set.good.key.clone_key(),
        vec![H3_ALPN.to_vec()],
    )
}

/// Sends one scripted HTTP/3 response on `stream`.
async fn send_scripted(
    stream: &mut h3::server::RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>,
    response: &ScriptedH3Response,
) -> Result<(), h3::error::StreamError> {
    let mut builder = hyper::Response::builder().status(response.status);
    if let Some(content_type) = &response.content_type {
        builder = builder.header("content-type", content_type);
    }
    for (key, value) in &response.extra_headers {
        builder = builder.header(key, value);
    }
    if let Some(declared) = response.declared_length {
        builder = builder.header("content-length", declared);
    }
    let head = builder.body(()).expect("valid scripted response head");
    stream.send_response(head).await?;
    if !response.body.is_empty() {
        stream.send_data(Bytes::from(response.body.clone())).await?;
    }
    stream.finish().await
}

/// A scripted in-process HTTP/3 server driven on its own thread and runtime.
struct Doh3Server {
    address: SocketAddr,
    accepts: Arc<AtomicUsize>,
    /// A TCP listener bound to the QUIC port. A DoH/HTTP-1/HTTP-2 fallback would
    /// connect here, so an untouched listener is direct negative evidence.
    tcp_probe: Option<TcpListener>,
    handle: std::thread::JoinHandle<Vec<H3Evidence>>,
}

impl Doh3Server {
    /// Starts a server that answers one connection's one request.
    fn start(set: &FixtureSet, response: ScriptedH3Response) -> Self {
        Self::start_scripted(
            h3_server_config(set),
            Behavior::Respond(Box::new(response)),
            1,
            None,
            false,
        )
    }

    /// Starts a server that reads one request, signals `ready`, then withholds
    /// every response byte until the client closes.
    fn start_holding(set: &FixtureSet, ready: oneshot::Sender<()>) -> Self {
        Self::start_scripted(h3_server_config(set), Behavior::Hold, 1, Some(ready), false)
    }

    /// Starts a server that reads one request, then closes the connection
    /// without any response head.
    fn start_closing_without_response(set: &FixtureSet) -> Self {
        Self::start_scripted(
            h3_server_config(set),
            Behavior::CloseWithoutResponse,
            1,
            None,
            false,
        )
    }

    /// Starts a server with a caller-supplied ALPN list and a TCP probe on the
    /// QUIC port.
    fn start_with_alpn(set: &FixtureSet, alpn: Vec<Vec<u8>>) -> Self {
        Self::start_scripted(
            quic_server_config(set.good.cert.clone(), set.good.key.clone_key(), alpn),
            Behavior::CloseWithoutResponse,
            1,
            None,
            true,
        )
    }

    /// Starts a server presenting an untrusted or wrongly named leaf with the
    /// `h3` ALPN, for handshake-failure coverage.
    fn start_with_identity(
        cert: rustls::pki_types::CertificateDer<'static>,
        key: rustls::pki_types::PrivateKeyDer<'static>,
    ) -> Self {
        Self::start_scripted(
            quic_server_config(cert, key, vec![H3_ALPN.to_vec()]),
            Behavior::CloseWithoutResponse,
            1,
            None,
            false,
        )
    }

    /// Starts a server that answers two successive connections, proving each
    /// exchange opens a fresh connection and carries exactly one request.
    fn start_two_connections(set: &FixtureSet, response: ScriptedH3Response) -> Self {
        Self::start_scripted(
            h3_server_config(set),
            Behavior::Respond(Box::new(response)),
            2,
            None,
            false,
        )
    }

    /// Starts a scripted server with an explicit connection budget.
    #[allow(clippy::too_many_lines)]
    fn start_scripted(
        config: quinn::ServerConfig,
        behavior: Behavior,
        expected_connections: usize,
        mut ready: Option<oneshot::Sender<()>>,
        probe_tcp: bool,
    ) -> Self {
        // Reserving the port over TCP and then binding QUIC over UDP on the same
        // number lets the probe observe any TCP fallback attempt.
        let (tcp_probe, port) = if probe_tcp {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind tcp probe");
            listener
                .set_nonblocking(true)
                .expect("tcp probe non-blocking");
            let port = listener.local_addr().expect("tcp probe address").port();
            (Some(listener), port)
        } else {
            (None, 0)
        };

        let (address_tx, address_rx) = std::sync::mpsc::channel::<SocketAddr>();
        let accepts = Arc::new(AtomicUsize::new(0));
        let accepts_thread = Arc::clone(&accepts);

        let handle = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build server runtime");
            runtime.block_on(async move {
                let bind = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
                let endpoint = quinn::Endpoint::server(config, bind)
                    .expect("bind the ephemeral loopback QUIC endpoint");
                let address = endpoint.local_addr().expect("the bound address");
                address_tx
                    .send(address)
                    .expect("the test learns the address");

                let mut evidence = Vec::new();
                for _ in 0..expected_connections {
                    let Some(incoming) = timeout(TEST_TIMEOUT, endpoint.accept())
                        .await
                        .ok()
                        .flatten()
                    else {
                        break;
                    };
                    accepts_thread.fetch_add(1, Ordering::SeqCst);
                    let Ok(connection) = incoming.await else {
                        continue;
                    };

                    let alpn = connection
                        .handshake_data()
                        .and_then(|data| data.downcast::<HandshakeData>().ok())
                        .and_then(|data| data.protocol.clone());

                    let quinn_conn = connection.clone();
                    let Ok(mut server) = h3::server::builder()
                        .build::<_, Bytes>(h3_quinn::Connection::new(connection))
                        .await
                    else {
                        let _ = timeout(TEST_TIMEOUT, quinn_conn.closed()).await;
                        continue;
                    };
                    let Ok(Some(resolver)) = server.accept().await else {
                        let _ = timeout(TEST_TIMEOUT, quinn_conn.closed()).await;
                        continue;
                    };
                    let Ok((request, mut stream)) = resolver.resolve_request().await else {
                        let _ = timeout(TEST_TIMEOUT, quinn_conn.closed()).await;
                        continue;
                    };

                    let (parts, ()) = request.into_parts();
                    let method = parts.method.to_string();
                    let authority = parts
                        .uri
                        .authority()
                        .map(ToString::to_string)
                        .unwrap_or_default();
                    let path = parts
                        .uri
                        .path_and_query()
                        .map(ToString::to_string)
                        .unwrap_or_default();
                    let headers = parts
                        .headers
                        .iter()
                        .map(|(key, value)| {
                            (
                                key.as_str().to_owned(),
                                value.to_str().unwrap_or_default().to_owned(),
                            )
                        })
                        .collect::<Vec<_>>();

                    // `recv_data` only returns `Ok(None)` once the request stream
                    // ended, so its success is the request send-side FIN evidence.
                    let mut request_body = Vec::new();
                    let mut request_fin = false;
                    loop {
                        match stream.recv_data().await {
                            Ok(Some(data)) => {
                                request_body.extend_from_slice(data.chunk());
                            }
                            Ok(None) => {
                                request_fin = true;
                                break;
                            }
                            Err(_) => break,
                        }
                    }

                    if let Some(sender) = ready.take() {
                        let _ = sender.send(());
                    }

                    match &behavior {
                        Behavior::Respond(response) => {
                            let _ = send_scripted(&mut stream, response).await;
                        }
                        Behavior::Hold | Behavior::CloseWithoutResponse => {}
                    }
                    evidence.push(H3Evidence {
                        alpn,
                        method,
                        authority,
                        path,
                        headers,
                        request_body,
                        request_fin,
                    });

                    if matches!(behavior, Behavior::CloseWithoutResponse) {
                        quinn_conn.close(0u32.into(), b"");
                    }

                    // Keep the connection (and this h3 server connection) alive
                    // until the client closes, so the scripted response is
                    // delivered; the bound keeps a broken path from hanging.
                    let _ = timeout(TEST_TIMEOUT, quinn_conn.closed()).await;
                }

                // A second connection opened before the first closed would be
                // queued here; the bounded probe accounts for it.
                if timeout(ACCEPT_PROBE, endpoint.accept())
                    .await
                    .is_ok_and(|second| second.is_some())
                {
                    accepts_thread.fetch_add(1, Ordering::SeqCst);
                }

                evidence
            })
        });

        let address = address_rx
            .recv_timeout(TEST_TIMEOUT)
            .expect("the server binds within the bounded wait");
        Self {
            address,
            accepts,
            tcp_probe,
            handle,
        }
    }

    fn join(self) -> (usize, Vec<H3Evidence>) {
        let evidence = self.handle.join().expect("server thread joined");
        let accepts = self.accepts.load(Ordering::SeqCst);
        (accepts, evidence)
    }

    /// Asserts the shared TCP probe never accepted a fallback connection.
    fn assert_no_tcp_fallback(&self) {
        let listener = self.tcp_probe.as_ref().expect("a tcp probe was requested");
        match listener.accept() {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Ok(_) => panic!("a DoH/HTTP-1/HTTP-2 fallback must never dial TCP"),
            Err(error) => panic!("unexpected tcp probe error: {error}"),
        }
    }
}

/// A `DoH3` endpoint for `service_url` dialing the loopback `address`.
fn doh3_endpoint(address: SocketAddr, service_url: &str) -> DohEndpoint {
    DohEndpoint::new(service_url, address).expect("valid DoH endpoint")
}

/// A verified `DoH3` owner for the given endpoint.
fn verified_owner(set: &FixtureSet, address: SocketAddr, service_url: &str) -> Doh3Upstream {
    Doh3Upstream::new(
        doh3_endpoint(address, service_url),
        TlsPolicy::verified(set.root_store_a()).expect("verified policy"),
    )
    .expect("owner")
}

/// Runs one exchange for `query` and returns its typed outcome.
async fn exchange_owned(
    upstream: &Doh3Upstream,
    query: &[u8],
    context: ExchangeContext,
) -> Result<SecureResponse, SecureError> {
    let request = ExchangeRequest::new(query).expect("valid query");
    upstream.exchange(request, context).await
}

fn open_context() -> ExchangeContext {
    ExchangeContext::new(
        Instant::now() + EXCHANGE_DEADLINE,
        TransportCancellation::new(),
    )
}

// ---------------------------------------------------------------------------
// Wire-level request contract and the successful one-shot shape
// ---------------------------------------------------------------------------

#[test]
fn doh3_one_shot_get_succeeds_over_loopback() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x9001;
        let expected = response_wire(id, 1);
        let server = Doh3Server::start(&set, ScriptedH3Response::ok_dns(expected.clone()));
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let query = query_wire(id);
        let request = ExchangeRequest::new(&query).expect("valid query");
        let expected_target = upstream
            .endpoint()
            .get_request_target(request)
            .expect("the encoder produces one target");
        let expected_authority = upstream.endpoint().authority();

        let response = exchange_owned(&upstream, &query, open_context())
            .await
            .expect("a well-formed DoH3 response succeeds");

        // The frozen public shape: DoH3 transport over HTTP/3, with the caller's
        // original ID restored in both the metadata and the returned wire.
        assert_eq!(response.transport(), SecureTransport::Doh3);
        assert_eq!(response.http_version(), Some(SecureHttpVersion::Http3));
        assert_eq!(response.request_id(), id);
        assert_eq!(response.response_id(), id);
        assert_eq!(response.wire(), expected.as_slice());
        assert!(!response.truncated());
        // The RAII registration (and the tracked driver child) is gone once the
        // exchange future returns.
        assert_eq!(upstream.in_flight_exchanges(), 0);

        let (accepts, evidence) = server.join();
        server_probe(&upstream, accepts, &evidence);
        assert_eq!(accepts, 1, "exactly one connection is accepted");
        let observed = &evidence[0];
        assert_eq!(observed.alpn.as_deref(), Some(H3_ALPN));
        assert_eq!(observed.method, "GET", "DoH3 uses exactly one GET");
        assert_eq!(
            observed.authority, expected_authority,
            "the :authority must be the service authority, never the numeric dial"
        );
        assert!(
            !observed.authority.contains("127.0.0.1"),
            "the numeric dial address must never appear as :authority"
        );
        assert_eq!(
            observed.path, expected_target,
            "the :path must be byte-equal to the reused encoder output"
        );
        assert_eq!(
            observed.header("accept"),
            Some("application/dns-message"),
            "the request must accept the DNS media type"
        );
        assert!(
            observed.header("user-agent").is_none(),
            "no User-Agent may be added"
        );
        assert!(
            observed.header("content-encoding").is_none(),
            "no request Content-Encoding may be added"
        );
        assert!(
            observed.request_body.is_empty(),
            "a DoH GET carries no request body"
        );
        assert!(
            observed.request_fin,
            "the client must finish the request stream with a send-side FIN"
        );
    });
}

/// Asserts the observation shape shared by every successful exchange.
fn server_probe(upstream: &Doh3Upstream, accepts: usize, evidence: &[H3Evidence]) {
    assert_eq!(
        evidence.len(),
        1,
        "exactly one request is sent on the connection"
    );
    assert_eq!(accepts, 1, "exactly one connection is accepted");
    assert_eq!(upstream.in_flight_exchanges(), 0);
}

#[test]
fn doh3_authority_is_the_service_host_not_the_numeric_dial_address() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x9002;
        let server = Doh3Server::start(&set, ScriptedH3Response::ok_dns(response_wire(id, 2)));
        // The dial address is loopback; the service authority must stay the URL
        // host, including its explicit non-default port.
        let upstream = verified_owner(&set, server.address, "https://dns.example:8443/dns-query");

        let response = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect("the exchange succeeds");
        assert_eq!(response.wire(), response_wire(id, 2).as_slice());

        let (accepts, evidence) = server.join();
        server_probe(&upstream, accepts, &evidence);
        let observed = &evidence[0];
        assert_eq!(observed.authority, "dns.example:8443");
        assert!(
            observed.path.starts_with("/dns-query?dns="),
            "the path stays the service path plus the dns parameter, got {}",
            observed.path
        );
    });
}

/// The `dns` parameter value of an origin-form target.
fn dns_parameter(target: &str) -> String {
    target
        .split_once("dns=")
        .map(|(_, value)| value.to_owned())
        .expect("the target carries a dns parameter")
}

#[test]
fn doh3_media_type_is_matched_case_insensitively_with_parameters() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x9003;
        let mut response = ScriptedH3Response::ok_dns(response_wire(id, 3));
        response.content_type = Some("Application/DNS-Message; charset=binary".to_owned());
        let server = Doh3Server::start(&set, response);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let response = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect("the media type is case-insensitive and allows parameters");
        assert_eq!(response.wire(), response_wire(id, 3).as_slice());
        assert_eq!(response.transport(), SecureTransport::Doh3);

        let (accepts, evidence) = server.join();
        server_probe(&upstream, accepts, &evidence);
    });
}

#[test]
fn doh3_outbound_query_id_is_zeroed_and_the_caller_id_is_restored() {
    block_on(async {
        use base64::Engine as _;
        let set = FixtureSet::generate();
        let caller_id = 0xABCD;
        // The server answers with ID 0, as a real DoH resolver does.
        let server = Doh3Server::start(&set, ScriptedH3Response::ok_dns(response_wire(0, 4)));
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let query = query_wire(caller_id);
        let response = exchange_owned(&upstream, &query, open_context())
            .await
            .expect("the exchange succeeds");
        assert_eq!(response.request_id(), caller_id);
        assert_eq!(response.response_id(), caller_id);
        assert_eq!(
            u16::from_be_bytes([response.wire()[0], response.wire()[1]]),
            caller_id,
            "the caller's original ID must be restored into the returned wire"
        );

        let (accepts, evidence) = server.join();
        server_probe(&upstream, accepts, &evidence);
        let encoded = dns_parameter(&evidence[0].path);
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&encoded)
            .expect("the dns parameter is unpadded base64url");
        assert_eq!(
            &decoded[..2],
            &[0, 0],
            "the outbound copy's ID must be zeroed"
        );
        assert_eq!(
            &decoded[2..],
            &query[2..],
            "only the ID bytes may differ from the caller's query"
        );
        assert!(!encoded.contains('='), "the encoding must be unpadded");
    });
}

#[test]
fn doh3_each_exchange_opens_a_fresh_connection_with_one_request() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x9005;
        let server = Doh3Server::start_two_connections(
            &set,
            ScriptedH3Response::ok_dns(response_wire(id, 5)),
        );
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        for _ in 0..2 {
            let response = exchange_owned(&upstream, &query_wire(id), open_context())
                .await
                .expect("each fresh-connection exchange succeeds");
            assert_eq!(response.wire(), response_wire(id, 5).as_slice());
            assert_eq!(upstream.in_flight_exchanges(), 0);
        }

        let (accepts, evidence) = server.join();
        assert_eq!(accepts, 2, "each exchange opens its own connection");
        assert_eq!(evidence.len(), 2, "one request per connection");
        for observed in &evidence {
            assert_eq!(observed.method, "GET");
            assert!(observed.request_fin);
        }
    });
}

// ---------------------------------------------------------------------------
// Response status, media type, encoding, and body bounds
// ---------------------------------------------------------------------------

#[test]
fn doh3_non_200_status_is_a_typed_protocol_error() {
    block_on(async {
        let set = FixtureSet::generate();
        // The body is a perfectly valid DNS response, so only the status can be
        // the reason this is rejected.
        let mut response = ScriptedH3Response::ok_dns(response_wire(0, 6));
        response.status = 500;
        let server = Doh3Server::start(&set, response);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x9006), open_context())
            .await
            .expect_err("a 500 must not be accepted even with a valid DNS body");
        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::UnexpectedStatus { status: 500 }),
            "expected a typed status rejection"
        );
        assert_eq!(error.side_effect(), SideEffectState::Sent);
        assert_eq!(upstream.in_flight_exchanges(), 0);

        let (accepts, evidence) = server.join();
        assert_eq!(accepts, 1);
        assert_eq!(evidence.len(), 1);
    });
}

#[test]
fn doh3_missing_or_wrong_media_type_is_rejected() {
    block_on(async {
        let set = FixtureSet::generate();
        let mut missing = ScriptedH3Response::ok_dns(response_wire(0, 7));
        missing.content_type = None;
        let server = Doh3Server::start(&set, missing);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x9007), open_context())
            .await
            .expect_err("a response without a media type is rejected");
        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::MissingMediaType)
        );
        assert_eq!(error.side_effect(), SideEffectState::Sent);
        server.join();

        let mut wrong = ScriptedH3Response::ok_dns(response_wire(0, 8));
        wrong.content_type = Some("text/plain".to_owned());
        let server = Doh3Server::start(&set, wrong);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");
        let error = exchange_owned(&upstream, &query_wire(0x9008), open_context())
            .await
            .expect_err("a non-DNS media type is rejected");
        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::WrongMediaType)
        );
        server.join();
    });
}

#[test]
fn doh3_non_identity_content_encoding_is_rejected() {
    block_on(async {
        let set = FixtureSet::generate();
        let mut response = ScriptedH3Response::ok_dns(response_wire(0, 9));
        response
            .extra_headers
            .push(("content-encoding".to_owned(), "gzip".to_owned()));
        let server = Doh3Server::start(&set, response);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x9009), open_context())
            .await
            .expect_err("a compressed payload must never be read as DNS wire");
        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::ContentEncoding)
        );
        assert_eq!(error.side_effect(), SideEffectState::Sent);
        server.join();
    });
}

#[test]
fn doh3_declared_length_over_the_dns_maximum_is_rejected_at_the_head() {
    block_on(async {
        let set = FixtureSet::generate();
        // The declared length is above the DNS maximum, so the head alone must
        // reject it before any body byte is read.
        let mut response = ScriptedH3Response::ok_dns(vec![0u8; 16]);
        response.declared_length = Some(65_536);
        let server = Doh3Server::start(&set, response);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x9010), open_context())
            .await
            .expect_err("a declared body above the DNS maximum is rejected at the head");
        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::BodyTooLarge)
        );
        server.join();
    });
}

#[test]
fn doh3_oversized_body_is_rejected_by_the_incremental_gate() {
    block_on(async {
        let set = FixtureSet::generate();
        // No `content-length`, so only the incremental received-byte bound can
        // reject the 65536-byte body.
        let body = vec![0u8; 65_536];
        let mut response = ScriptedH3Response::ok_dns(body);
        response.declared_length = None;
        let server = Doh3Server::start(&set, response);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x9011), open_context())
            .await
            .expect_err("a body above the DNS maximum is rejected");
        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::BodyTooLarge)
        );
        assert_eq!(error.side_effect(), SideEffectState::Sent);
        server.join();
    });
}

#[test]
fn doh3_incomplete_body_is_a_typed_protocol_error() {
    block_on(async {
        let set = FixtureSet::generate();
        // The head declares 100 bytes but the stream is finished after 16, so
        // the body is an incomplete prefix rather than a silently accepted
        // shorter message.
        let mut response = ScriptedH3Response::ok_dns(vec![0u8; 16]);
        response.declared_length = Some(100);
        let server = Doh3Server::start(&set, response);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x9012), open_context())
            .await
            .expect_err("an early end of body is rejected");
        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::IncompleteBody)
        );
        assert_eq!(error.side_effect(), SideEffectState::Sent);
        server.join();
    });
}

#[test]
fn doh3_too_many_response_headers_is_rejected() {
    block_on(async {
        let set = FixtureSet::generate();
        // 80 small headers sit well under the 16 KiB byte bound, so only the
        // 64-header bound can reject this head.
        let mut response = ScriptedH3Response::ok_dns(response_wire(0, 10));
        for index in 0..80u16 {
            response
                .extra_headers
                .push((format!("x-pad-{index}"), "v".to_owned()));
        }
        let server = Doh3Server::start(&set, response);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x9013), open_context())
            .await
            .expect_err("a response head above the header-count bound is rejected");
        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::ResponseHeadTooLarge)
        );
        server.join();
    });
}

#[test]
fn doh3_close_before_any_response_head_is_not_reported_as_sent() {
    block_on(async {
        let set = FixtureSet::generate();
        let server = Doh3Server::start_closing_without_response(&set);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x9014), open_context())
            .await
            .expect_err("a connection closed before a head is a failure");
        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::ResponseHeadNotReceived),
            "a close before any head must be the head-not-received case"
        );
        assert_eq!(
            error.side_effect(),
            SideEffectState::MaybeSent,
            "an absent response head is conservatively MaybeSent"
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);
        server.join();
    });
}

// ---------------------------------------------------------------------------
// Transport selection: exact ALPN, verified identity, no fallback
// ---------------------------------------------------------------------------

#[test]
fn doh3_alpn_mismatch_is_not_sent_and_does_not_fall_back() {
    block_on(async {
        let set = FixtureSet::generate();
        // The server offers only `h2`. The client offers only `h3`, so the
        // handshake must fail before any HTTP request exists.
        let server = Doh3Server::start_with_alpn(&set, vec![b"h2".to_vec()]);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x9015), open_context())
            .await
            .expect_err("an ALPN mismatch must fail the handshake");
        assert!(
            matches!(error, SecureError::Tls(_)),
            "expected a typed TLS failure, got {error:?}"
        );
        assert_eq!(error.side_effect(), SideEffectState::NotSent);
        assert_eq!(upstream.in_flight_exchanges(), 0);

        server.assert_no_tcp_fallback();
        let (accepts, evidence) = server.join();
        assert_eq!(accepts, 1, "the single QUIC attempt was accepted");
        assert!(evidence.is_empty(), "no request may be decoded");
    });
}

#[test]
fn doh3_untrusted_certificate_is_not_sent() {
    block_on(async {
        let set = FixtureSet::generate();
        let server = Doh3Server::start_with_identity(
            set.unknown_issuer.cert.clone(),
            set.unknown_issuer.key.clone_key(),
        );
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x9016), open_context())
            .await
            .expect_err("a certificate from an untrusted root must fail");
        assert!(
            matches!(error, SecureError::Tls(_)),
            "expected a typed TLS failure, got {error:?}"
        );
        assert_eq!(error.side_effect(), SideEffectState::NotSent);

        let (accepts, evidence) = server.join();
        assert_eq!(accepts, 1);
        assert!(evidence.is_empty(), "no request may be decoded");
    });
}

#[test]
fn doh3_identity_mismatch_is_not_sent() {
    block_on(async {
        let set = FixtureSet::generate();
        // The certificate is trusted but only valid for `other.example`.
        let server = Doh3Server::start_with_identity(
            set.wrong_name.cert.clone(),
            set.wrong_name.key.clone_key(),
        );
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x9017), open_context())
            .await
            .expect_err("a name mismatch must fail the TLS handshake");
        assert!(
            matches!(error, SecureError::Tls(_)),
            "expected a typed TLS failure, got {error:?}"
        );
        assert_eq!(error.side_effect(), SideEffectState::NotSent);
        server.join();
    });
}

#[test]
fn doh3_exchanges_after_close_are_rejected_without_opening_a_connection() {
    block_on(async {
        let set = FixtureSet::generate();
        let (tcp_probe, port) = reserve_tcp_port();
        let upstream = verified_owner(
            &set,
            SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            "https://dns.example/dns-query",
        );
        assert_eq!(upstream.close().await, CloseResult::Closed);

        let error = exchange_owned(&upstream, &query_wire(0x9018), open_context())
            .await
            .expect_err("a closed owner rejects new exchanges");
        assert_eq!(
            error,
            SecureError::Transport(UpstreamError::Closed(SideEffectState::NotSent))
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);

        match tcp_probe.accept() {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Ok(_) => panic!("a closed owner must not open a connection"),
            Err(error) => panic!("unexpected accept error: {error}"),
        }
    });
}

/// Reserves an unused TCP port, so a numeric dial to it can be proven to be
/// untouched without binding a server.
fn reserve_tcp_port() -> (TcpListener, u16) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind tcp probe");
    listener
        .set_nonblocking(true)
        .expect("tcp probe non-blocking");
    let port = listener.local_addr().expect("tcp probe address").port();
    (listener, port)
}

// ---------------------------------------------------------------------------
// Slice2 ownership: no residue after close/cancel/drop
// ---------------------------------------------------------------------------

#[test]
fn doh3_owner_close_during_a_held_exchange_leaves_no_residue() {
    block_on(async {
        let set = FixtureSet::generate();
        let (ready_tx, ready_rx) = oneshot::channel::<()>();
        let server = Doh3Server::start_holding(&set, ready_tx);
        let upstream = Arc::new(verified_owner(
            &set,
            server.address,
            "https://dns.example/dns-query",
        ));
        let query = query_wire(0x9100);
        let exchange = {
            let upstream = Arc::clone(&upstream);
            let query = query.clone();
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                upstream.exchange(request, open_context()).await
            })
        };

        // Wait until the request was fully received, so the close lands while
        // the response is outstanding rather than racing the request send.
        timeout(TEST_TIMEOUT, ready_rx)
            .await
            .expect("the server observes the request within the bound")
            .expect("the readiness signal is delivered");

        assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);
        let error = timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("closed exchange bounded")
            .expect("exchange task joined")
            .expect_err("owner close terminates the exchange");
        assert!(
            matches!(error, SecureError::Transport(UpstreamError::Closed(_))),
            "owner close must report the typed Closed cause, got {error:?}"
        );

        assert_eq!(upstream.close().await, CloseResult::Closed);
        assert_eq!(upstream.in_flight_exchanges(), 0);

        let (accepts, evidence) = server.join();
        assert_eq!(
            accepts, 1,
            "a cancelled exchange opens no second connection"
        );
        assert_eq!(evidence.len(), 1);
    });
}

#[test]
fn doh3_caller_cancellation_during_a_held_exchange_leaves_no_residue() {
    block_on(async {
        let set = FixtureSet::generate();
        let (ready_tx, ready_rx) = oneshot::channel::<()>();
        let server = Doh3Server::start_holding(&set, ready_tx);
        let upstream = Arc::new(verified_owner(
            &set,
            server.address,
            "https://dns.example/dns-query",
        ));
        let cancellation = TransportCancellation::new();
        let context =
            ExchangeContext::new(Instant::now() + EXCHANGE_DEADLINE, cancellation.clone());
        let query = query_wire(0x9101);
        let exchange = {
            let upstream = Arc::clone(&upstream);
            let query = query.clone();
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                upstream.exchange(request, context).await
            })
        };

        timeout(TEST_TIMEOUT, ready_rx)
            .await
            .expect("the server observes the request within the bound")
            .expect("the readiness signal is delivered");

        cancellation.cancel();
        let error = timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("cancelled exchange bounded")
            .expect("exchange task joined")
            .expect_err("caller cancellation terminates the exchange");
        assert!(
            matches!(error, SecureError::Transport(UpstreamError::Cancelled(_))),
            "got {error:?}"
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);

        let (accepts, evidence) = server.join();
        assert_eq!(accepts, 1);
        assert_eq!(evidence.len(), 1);
    });
}

#[test]
fn doh3_dropped_exchange_future_leaves_no_residue() {
    block_on(async {
        let set = FixtureSet::generate();
        let (ready_tx, ready_rx) = oneshot::channel::<()>();
        let server = Doh3Server::start_holding(&set, ready_tx);
        let upstream = Arc::new(verified_owner(
            &set,
            server.address,
            "https://dns.example/dns-query",
        ));
        let query = query_wire(0x9102);
        let exchange = {
            let upstream = Arc::clone(&upstream);
            let query = query.clone();
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                upstream.exchange(request, open_context()).await
            })
        };

        timeout(TEST_TIMEOUT, ready_rx)
            .await
            .expect("the server observes the request within the bound")
            .expect("the readiness signal is delivered");

        // Abort the caller future. The tracked driver child holds a shared
        // registration, so the owner must stay non-drained until that child is
        // gone; the assertion is about the outcome, not about ordering.
        exchange.abort();
        let _ = exchange.await;
        assert_eq!(
            upstream.in_flight_exchanges(),
            0,
            "an aborted future must release its registration and its driver"
        );
        assert_eq!(upstream.close().await, CloseResult::Closed);

        let (accepts, evidence) = server.join();
        assert_eq!(accepts, 1, "an aborted exchange opens no second connection");
        assert_eq!(evidence.len(), 1);
    });
}

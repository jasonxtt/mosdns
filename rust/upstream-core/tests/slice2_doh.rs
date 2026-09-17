//! Slice2 contract tests for the bounded DoH-over-HTTP/1.1 primitive.
//!
//! Every test runs a scripted in-process HTTPS server on an ephemeral IPv4
//! loopback port and drives real `DohUpstream::exchange` calls through a real
//! rustls handshake. The server records the exact request it received, so the
//! assertions are about the bytes on the wire — request target, method, headers
//! and absence of a body — rather than about internal call order.
//!
//! Certificates are generated in memory per test by the shared `fixtures`
//! module; no key material is committed. Deliberately, no test depends on a
//! sleep to establish ordering: the server signals the request it has parsed and
//! the tests gate on that signal.

mod fixtures;

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fixtures::FixtureSet;
use mosdns_upstream_core::secure::{
    DohEndpoint, DohProtocolError, DohUpstream, SecureError, SecureResponse, SecureTransport,
    TlsPolicy,
};
use mosdns_upstream_core::{
    CloseResult, CloseTransition, ExchangeContext, ExchangeRequest, SideEffectState,
    TransportCancellation, UpstreamError,
};
use rustls::ServerConfig;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener as AsyncTcpListener, TcpStream};
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;

/// Bounds every exchange so a broken path fails instead of hanging.
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

/// The exact HTTP request head the scripted server received.
#[derive(Clone, Debug)]
struct ReceivedHead {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
}

impl ReceivedHead {
    /// The first value of `name`, compared case-insensitively.
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Reads one HTTP/1.1 request head plus any declared body, without a parser.
///
/// The tests care about the literal bytes, so this deliberately does not use a
/// shared parser: it splits the head on the first blank line and then consumes
/// exactly `Content-Length` body bytes if the header is present.
async fn read_request(stream: &mut TlsStream<TcpStream>) -> Option<(ReceivedHead, Vec<u8>)> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 512];
    // Find the head terminator.
    let head_end = loop {
        if let Some(index) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break index + 4;
        }
        if buffer.len() > 64 * 1024 {
            return None;
        }
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    };

    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split(' ');
    let method = parts.next()?.to_owned();
    let target = parts.next()?.to_owned();

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.push((key.trim().to_owned(), value.trim().to_owned()));
        }
    }

    let declared = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buffer[head_end..].to_vec();
    while body.len() < declared {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }

    Some((
        ReceivedHead {
            method,
            target,
            headers,
        },
        body,
    ))
}

/// A scripted HTTP/1.1 response the server should write back.
#[derive(Clone, Debug)]
struct ScriptedResponse {
    status: u16,
    reason: &'static str,
    content_type: Option<String>,
    extra_headers: Vec<(String, String)>,
    /// The body framing strategy.
    framing: BodyFraming,
    body: Vec<u8>,
    /// If set, close the connection abruptly instead of writing normally.
    truncate_after_head: bool,
}

/// How the scripted response frames its body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BodyFraming {
    /// Write `Content-Length: <body len>`.
    Length,
    /// Write `Transfer-Encoding: chunked` with one chunk.
    Chunked,
    /// Write `body` verbatim with no generated head at all.
    ///
    /// Used where the test itself must control the exact bytes, including a
    /// head that deliberately disagrees with the body it accompanies.
    Raw,
}

impl ScriptedResponse {
    /// A 200 response carrying `body` as `application/dns-message`.
    fn ok_dns(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            reason: "OK",
            content_type: Some("application/dns-message".to_owned()),
            extra_headers: Vec::new(),
            framing: BodyFraming::Length,
            body,
            truncate_after_head: false,
        }
    }
}

impl ScriptedResponse {
    /// Serializes the head and body according to the framing strategy.
    fn serialize(&self) -> Vec<u8> {
        if self.framing == BodyFraming::Raw {
            return self.body.clone();
        }
        let mut out = format!("HTTP/1.1 {} {}\r\n", self.status, self.reason).into_bytes();
        if let Some(content_type) = &self.content_type {
            out.extend_from_slice(format!("Content-Type: {content_type}\r\n").as_bytes());
        }
        for (key, value) in &self.extra_headers {
            out.extend_from_slice(format!("{key}: {value}\r\n").as_bytes());
        }
        match self.framing {
            BodyFraming::Length => {
                out.extend_from_slice(
                    format!("Content-Length: {}\r\n", self.body.len()).as_bytes(),
                );
            }
            BodyFraming::Chunked => {
                out.extend_from_slice(b"Transfer-Encoding: chunked\r\n");
            }
            // Unreachable: `Raw` returns the caller's bytes before any head is
            // generated.
            BodyFraming::Raw => {}
        }
        out.extend_from_slice(b"\r\n");
        if self.truncate_after_head {
            return out;
        }
        match self.framing {
            BodyFraming::Length => out.extend_from_slice(&self.body),
            BodyFraming::Chunked => {
                if !self.body.is_empty() {
                    out.extend_from_slice(format!("{:x}\r\n", self.body.len()).as_bytes());
                    out.extend_from_slice(&self.body);
                    out.extend_from_slice(b"\r\n");
                }
                out.extend_from_slice(b"0\r\n\r\n");
            }
            // Handled by the early return above.
            BodyFraming::Raw => {}
        }
        out
    }
}

/// Builds a TLS server configuration presenting `fixture`'s leaf chain.
fn server_config(set: &FixtureSet) -> Arc<ServerConfig> {
    ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("ring provider supports the safe default protocol versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![set.good.cert.clone(), set.root_chain()],
            set.good.key.clone_key(),
        )
        .map(Arc::new)
        .expect("generated certificate and key are consistent")
}

/// Binds a fresh blocking TCP listener on the IPv4 loopback.
fn bind_listener() -> (TcpListener, SocketAddr) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind ipv4 loopback");
    let address = listener.local_addr().expect("local address");
    (listener, address)
}

/// A `DoH` endpoint for `service_url` dialing the loopback `address`.
fn doh_endpoint(address: SocketAddr, service_url: &str) -> DohEndpoint {
    DohEndpoint::new(service_url, address).expect("valid DoH endpoint")
}

/// A verified `DoH` owner for the given endpoint.
fn verified_owner(set: &FixtureSet, address: SocketAddr, service_url: &str) -> DohUpstream {
    DohUpstream::new(
        doh_endpoint(address, service_url),
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

/// What the scripted server should do after reading one request.
enum Reply {
    /// Write this response, then drain until the client closes.
    Respond(Box<ScriptedResponse>),
    /// Close the connection immediately without writing anything.
    CloseImmediately,
    /// Hold the connection open without answering, until the client goes away.
    Hold,
    /// Write these exact bytes, then close the connection.
    ///
    /// This is how a test produces a genuinely truncated response: the client
    /// sees a clean TCP close mid-body rather than waiting for more bytes.
    WriteThenClose(Vec<u8>),
}

/// A scripted HTTPS server that serves exactly one connection.
struct HttpsServer {
    address: SocketAddr,
    handle: std::thread::JoinHandle<Option<(ReceivedHead, Vec<u8>)>>,
}

impl HttpsServer {
    /// Starts a server that writes `response` after reading one request.
    fn start(set: &FixtureSet, response: ScriptedResponse) -> Self {
        Self::start_with(set, move |_head, _body| {
            Reply::Respond(Box::new(response.clone()))
        })
    }

    /// Starts a server whose reply is computed from the received request.
    fn start_with<F>(set: &FixtureSet, reply: F) -> Self
    where
        F: FnOnce(ReceivedHead, Vec<u8>) -> Reply + Send + 'static,
    {
        let (listener, address) = bind_listener();
        listener
            .set_nonblocking(true)
            .expect("listener non-blocking");
        let config = server_config(set);
        let handle = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build server runtime");
            runtime.block_on(async move {
                let listener =
                    AsyncTcpListener::from_std(listener).expect("adopt the standard listener");
                let (stream, _) = timeout(TEST_TIMEOUT, listener.accept())
                    .await
                    .expect("a connection arrives")
                    .expect("accept succeeds");
                let acceptor = TlsAcceptor::from(config);
                let Ok(Ok(mut tls)) = timeout(TEST_TIMEOUT, acceptor.accept(stream)).await else {
                    return None;
                };
                let (head, body) = read_request(&mut tls).await?;
                match reply(head.clone(), body.clone()) {
                    Reply::Respond(response) => {
                        let bytes = response.serialize();
                        let _ = tls.write_all(&bytes).await;
                        let _ = tls.flush().await;
                        // Drain until the client closes, so joining cannot hang.
                        let mut scratch = [0u8; 64];
                        while let Ok(read) = tls.read(&mut scratch).await {
                            if read == 0 {
                                break;
                            }
                        }
                    }
                    Reply::CloseImmediately => {
                        // Drop the stream now: the client observes EOF.
                        let _ = tls.shutdown().await;
                    }
                    Reply::WriteThenClose(bytes) => {
                        // Write the partial response, then close: the client
                        // must classify the resulting early EOF itself.
                        let _ = tls.write_all(&bytes).await;
                        let _ = tls.flush().await;
                        let _ = tls.shutdown().await;
                    }
                    Reply::Hold => {
                        // Hold the connection open without answering; the
                        // client's own control must terminate the exchange.
                        let mut scratch = [0u8; 64];
                        while let Ok(read) = tls.read(&mut scratch).await {
                            if read == 0 {
                                break;
                            }
                        }
                    }
                }
                Some((head, body))
            })
        });
        Self { address, handle }
    }

    fn join(self) -> Option<(ReceivedHead, Vec<u8>)> {
        self.handle.join().expect("server thread joined")
    }
}

/// Runs one exchange for `query` and returns its typed outcome.
async fn exchange_owned(
    upstream: &DohUpstream,
    query: &[u8],
    context: ExchangeContext,
) -> Result<SecureResponse, SecureError> {
    let request = ExchangeRequest::new(query).expect("valid query");
    upstream.exchange(request, context).await
}

// ---------------------------------------------------------------------------
// Wire-level request contracts
// ---------------------------------------------------------------------------

#[test]
fn a_successful_exchange_sends_a_get_with_no_body_and_no_user_agent() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x7001;
        let expected = response_wire(id, 1);
        let server = HttpsServer::start(&set, ScriptedResponse::ok_dns(expected.clone()));
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let query = query_wire(id);
        let expected_query = query.clone();
        // The upstream DNS ID is deliberately different from the caller's, to
        // prove the response is associated by HTTP stream and not by ID.
        let response = exchange_owned(&upstream, &query, open_context())
            .await
            .expect("a well-formed DoH response succeeds");

        assert_eq!(response.transport(), SecureTransport::Doh);
        assert_eq!(response.request_id(), id);
        assert_eq!(response.response_id(), id, "the caller ID is restored");
        assert_eq!(response.wire(), expected.as_slice());
        assert_eq!(query, expected_query, "the caller query must not change");
        assert_eq!(upstream.in_flight_exchanges(), 0);

        let (head, body) = server.join().expect("the server received one request");
        assert_eq!(head.method, "GET", "DoH uses GET");
        // The request body is a closed `EmptyBody` type rather than a buffer,
        // and Hyper declares its length as exactly zero, so this asserts both
        // the observed bytes and the framing the client emitted.
        assert!(body.is_empty(), "a DoH GET has no request body");
        assert!(
            !head
                .headers
                .iter()
                .any(|(key, value)| key.eq_ignore_ascii_case("transfer-encoding")
                    || (key.eq_ignore_ascii_case("content-length") && value != "0")),
            "a bodyless GET must not declare a body framing, headers={:?}",
            head.headers
        );
        assert_eq!(head.header("accept"), Some("application/dns-message"));
        assert!(
            head.header("user-agent").is_none(),
            "no default User-Agent may be added"
        );
        assert!(
            head.header("content-encoding").is_none(),
            "a GET must not declare a request Content-Encoding"
        );
        assert!(
            head.target.starts_with("/dns-query?dns="),
            "the request target is the service path plus the dns parameter, got {}",
            head.target
        );
        assert!(
            !head.target.contains("0000"),
            "the padded zero-ID base64 must not appear in the target"
        );
    });
}

#[test]
fn the_authority_is_the_service_host_not_the_numeric_dial_address() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x7002;
        let server = HttpsServer::start(&set, ScriptedResponse::ok_dns(response_wire(id, 2)));
        // The dial address is loopback; the service authority must stay the URL
        // host, including its explicit non-default port.
        let upstream = verified_owner(&set, server.address, "https://dns.example:8443/dns-query");

        let response = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect("the exchange succeeds");
        assert_eq!(response.wire(), response_wire(id, 2).as_slice());

        let (head, _body) = server.join().expect("one request");
        let host = head.header("host").unwrap_or_default();
        assert_eq!(
            host, "dns.example:8443",
            "Host must be the service authority from the URL"
        );
        assert!(
            !host.contains("127.0.0.1"),
            "the numeric dial address must never appear as the authority"
        );
    });
}

#[test]
fn the_outbound_query_id_is_zeroed_while_the_caller_id_is_restored() {
    block_on(async {
        use base64::Engine as _;
        let set = FixtureSet::generate();
        let caller_id = 0xABCD;
        // The server answers with ID 0, as a real DoH resolver does.
        let server = HttpsServer::start(&set, ScriptedResponse::ok_dns(response_wire(0, 3)));
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let query = query_wire(caller_id);
        let response = exchange_owned(&upstream, &query, open_context())
            .await
            .expect("the exchange succeeds");
        assert_eq!(response.request_id(), caller_id);
        assert_eq!(
            response.response_id(),
            caller_id,
            "the caller's original ID must be restored into the returned wire"
        );
        assert_eq!(
            u16::from_be_bytes([response.wire()[0], response.wire()[1]]),
            caller_id
        );

        let (head, _body) = server.join().expect("one request");
        let encoded = head
            .target
            .split_once("dns=")
            .map(|(_, value)| value)
            .expect("the target carries a dns parameter");
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
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
fn unrelated_query_parameters_are_preserved_and_existing_dns_is_replaced() {
    block_on(async {
        use base64::Engine as _;
        let set = FixtureSet::generate();
        let id = 0x7003;
        let server = HttpsServer::start(&set, ScriptedResponse::ok_dns(response_wire(id, 4)));
        // A duplicate `dns` parameter plus an unrelated parameter.
        let upstream = verified_owner(
            &set,
            server.address,
            "https://dns.example/dns-query?dns=AAAA&ct=application%2Fdns-message&dns=BBBB",
        );

        let response = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect("the exchange succeeds");
        assert_eq!(response.wire(), response_wire(id, 4).as_slice());

        let (head, _body) = server.join().expect("one request");
        // Parse the pairs rather than substring-matching: a base64 payload can
        // itself contain any of the literal inputs, so only the decoded key and
        // value identify a `dns` parameter.
        let query = head
            .target
            .split_once('?')
            .map(|(_, query)| query.to_owned())
            .unwrap_or_default();
        // Decode the pairs, so a preserved value is compared by its decoded
        // meaning rather than by its re-encoded spelling.
        let pairs: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        let dns_values: Vec<&str> = pairs
            .iter()
            .filter(|(key, _)| key == "dns")
            .map(|(_, value)| value.as_str())
            .collect();
        assert_eq!(
            dns_values.len(),
            1,
            "every existing dns parameter must be replaced by exactly one, target={}",
            head.target
        );
        assert_ne!(dns_values[0], "AAAA");
        assert_ne!(dns_values[0], "BBBB");
        assert!(!dns_values[0].is_empty());
        assert_eq!(
            pairs
                .iter()
                .filter(|(key, _)| key == "ct")
                .map(|(_, value)| value.as_str())
                .collect::<Vec<_>>(),
            vec!["application/dns-message"],
            "the unrelated parameter must be preserved, target={}",
            head.target
        );
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(dns_values[0])
            .expect("the appended dns value decodes");
    });
}

// ---------------------------------------------------------------------------
// Status, MIME, framing and size
// ---------------------------------------------------------------------------

#[test]
fn a_non_200_status_is_a_typed_protocol_error() {
    block_on(async {
        let set = FixtureSet::generate();
        // The body is a perfectly valid DNS response, so only the status can
        // be the reason this is rejected. Without the 200 requirement the
        // exchange would succeed, which is what makes this test discriminating.
        let mut server_error = ScriptedResponse::ok_dns(response_wire(0, 1));
        server_error.status = 500;
        server_error.reason = "Internal Server Error";
        let server = HttpsServer::start(&set, server_error);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x7004), open_context())
            .await
            .expect_err("a 500 must not be accepted even with a valid DNS body");
        assert_eq!(
            error,
            SecureError::DohProtocol(
                mosdns_upstream_core::secure::DohProtocolError::UnexpectedStatus { status: 500 }
            ),
            "expected a typed status rejection"
        );
        // The request was transmitted, so the state is not NotSent.
        assert_eq!(error.side_effect(), SideEffectState::Sent);
        assert_eq!(upstream.in_flight_exchanges(), 0);
        server.join();
    });
}

#[test]
fn a_redirect_is_not_followed() {
    block_on(async {
        let set = FixtureSet::generate();
        // A valid DNS body again, so only the 3xx status can cause rejection.
        // Without the 200 requirement this exchange would otherwise succeed.
        let mut redirect = ScriptedResponse::ok_dns(response_wire(0, 2));
        redirect.status = 302;
        redirect.reason = "Found";
        redirect.extra_headers.push((
            "Location".to_owned(),
            "https://elsewhere.example/dns-query".to_owned(),
        ));
        let server = HttpsServer::start(&set, redirect);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x7005), open_context())
            .await
            .expect_err("a redirect must be terminal and must not be followed");
        assert_eq!(
            error,
            SecureError::DohProtocol(
                mosdns_upstream_core::secure::DohProtocolError::UnexpectedStatus { status: 302 }
            ),
            "a redirect must be reported as a status rejection, never followed"
        );
        // Exactly one request reached the server; no follow-up happened.
        server.join();
        assert_eq!(upstream.in_flight_exchanges(), 0);
    });
}

#[test]
fn a_missing_or_wrong_media_type_is_rejected() {
    block_on(async {
        let set = FixtureSet::generate();
        let body = response_wire(0x7006, 5);

        // Missing Content-Type.
        let mut missing = ScriptedResponse::ok_dns(body.clone());
        missing.content_type = None;
        let server = HttpsServer::start(&set, missing);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");
        let error = exchange_owned(&upstream, &query_wire(0x7006), open_context())
            .await
            .expect_err("a missing media type must be rejected");
        assert!(
            matches!(error, SecureError::DohProtocol(_)),
            "got {error:?}"
        );
        server.join();

        // Wrong Content-Type.
        let mut wrong = ScriptedResponse::ok_dns(body);
        wrong.content_type = Some("application/json".to_owned());
        let server = HttpsServer::start(&set, wrong);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");
        let error = exchange_owned(&upstream, &query_wire(0x7006), open_context())
            .await
            .expect_err("a wrong media type must be rejected");
        assert!(
            matches!(error, SecureError::DohProtocol(_)),
            "got {error:?}"
        );
        server.join();
    });
}

#[test]
fn the_media_type_is_matched_case_insensitively_with_parameters() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x7007;
        let expected = response_wire(id, 6);
        let mut response = ScriptedResponse::ok_dns(expected.clone());
        response.content_type = Some("Application/DNS-Message; charset=binary".to_owned());
        let server = HttpsServer::start(&set, response);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let result = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect("media type parameters and case must be tolerated");
        assert_eq!(result.wire(), expected.as_slice());
        server.join();
    });
}

#[test]
fn a_non_identity_content_encoding_is_rejected() {
    block_on(async {
        let set = FixtureSet::generate();
        let mut response = ScriptedResponse::ok_dns(response_wire(0x7008, 7));
        response
            .extra_headers
            .push(("Content-Encoding".to_owned(), "gzip".to_owned()));
        let server = HttpsServer::start(&set, response);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x7008), open_context())
            .await
            .expect_err("a compressed body must not be decompressed or accepted");
        assert!(
            matches!(error, SecureError::DohProtocol(_)),
            "got {error:?}"
        );
        server.join();
    });
}

#[test]
fn a_chunked_body_is_reassembled_and_validated() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x7009;
        let expected = response_wire(id, 8);
        let mut response = ScriptedResponse::ok_dns(expected.clone());
        response.framing = BodyFraming::Chunked;
        let server = HttpsServer::start(&set, response);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let result = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect("a chunked body is a valid framing");
        assert_eq!(result.wire(), expected.as_slice());
        server.join();
    });
}

#[test]
fn a_body_larger_than_the_dns_maximum_is_rejected() {
    block_on(async {
        let set = FixtureSet::generate();
        // 65536 bytes: exactly one over the DNS maximum of 65535.
        let oversized = vec![0u8; 65_536];
        let server = HttpsServer::start(&set, ScriptedResponse::ok_dns(oversized));
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x700A), open_context())
            .await
            .expect_err("an over-max body must be rejected");
        assert!(
            matches!(error, SecureError::DohProtocol(_)),
            "expected an oversize protocol error, got {error:?}"
        );
        server.join();
    });
}

#[test]
fn a_body_truncated_mid_header_is_an_incomplete_body() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x700B;
        // Declare a DNS-length body, send only a few bytes, then close.
        let server = HttpsServer::start_with(&set, |_head, _body| {
            let mut bytes =
                b"HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nContent-Length: 64\r\n\r\n"
                    .to_vec();
            bytes.extend_from_slice(&[0u8; 8]);
            Reply::WriteThenClose(bytes)
        });
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect_err("an incomplete body must not be accepted as a valid response");
        assert!(
            matches!(
                error,
                SecureError::DohProtocol(DohProtocolError::IncompleteBody)
            ),
            "got {error:?}"
        );
        server.join();
    });
}

#[test]
fn a_complete_body_with_invalid_dns_wire_is_rejected() {
    block_on(async {
        let set = FixtureSet::generate();
        // A QR-set header that declares one question and then ends.
        let mut invalid = Vec::new();
        invalid.extend_from_slice(&0u16.to_be_bytes());
        invalid.extend_from_slice(&[0x81, 0x80]);
        invalid.extend_from_slice(&1u16.to_be_bytes());
        invalid.extend_from_slice(&0u16.to_be_bytes());
        invalid.extend_from_slice(&0u16.to_be_bytes());
        invalid.extend_from_slice(&0u16.to_be_bytes());
        let server = HttpsServer::start(&set, ScriptedResponse::ok_dns(invalid));
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x700C), open_context())
            .await
            .expect_err("an invalid DNS body must be rejected");
        assert!(
            matches!(error, SecureError::Transport(_)),
            "expected a DNS-response failure, got {error:?}"
        );
        server.join();
    });
}

#[test]
fn a_body_shorter_than_a_dns_header_is_rejected() {
    block_on(async {
        let set = FixtureSet::generate();
        let server = HttpsServer::start(&set, ScriptedResponse::ok_dns(vec![0u8; 11]));
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x700D), open_context())
            .await
            .expect_err("an 11-byte body cannot be a DNS response");
        assert!(
            matches!(
                error,
                SecureError::DohProtocol(_) | SecureError::Transport(_)
            ),
            "got {error:?}"
        );
        server.join();
    });
}

// ---------------------------------------------------------------------------
// ALPN, header framing and length disagreement
// ---------------------------------------------------------------------------

/// Starts an HTTPS server that offers (or omits) an ALPN protocol.
fn alpn_server(
    set: &FixtureSet,
    alpn: Option<Vec<Vec<u8>>>,
    response: ScriptedResponse,
) -> (SocketAddr, std::thread::JoinHandle<Option<String>>) {
    let (listener, address) = bind_listener();
    listener
        .set_nonblocking(true)
        .expect("listener non-blocking");
    let mut config = (*server_config(set)).clone();
    config.alpn_protocols = alpn.unwrap_or_default();
    let config = Arc::new(config);
    let handle = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build server runtime");
        runtime.block_on(async move {
            let listener = AsyncTcpListener::from_std(listener).expect("adopt listener");
            let (stream, _) = timeout(TEST_TIMEOUT, listener.accept()).await.ok()?.ok()?;
            let acceptor = TlsAcceptor::from(config);
            let mut tls = timeout(TEST_TIMEOUT, acceptor.accept(stream))
                .await
                .ok()?
                .ok()?;
            // Record what the client offered and what the handshake selected.
            let (_io, session) = tls.get_ref();
            let negotiated = session
                .alpn_protocol()
                .map(|protocol| String::from_utf8_lossy(protocol).into_owned());
            let request = read_request(&mut tls).await;
            if request.is_some() {
                let bytes = response.serialize();
                let _ = tls.write_all(&bytes).await;
                let _ = tls.flush().await;
            }
            let mut scratch = [0u8; 64];
            while let Ok(read) = tls.read(&mut scratch).await {
                if read == 0 {
                    break;
                }
            }
            negotiated
        })
    });
    (address, handle)
}

#[test]
fn a_server_without_alpn_still_serves_http11_on_the_same_tls_stream() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x7200;
        let expected = response_wire(id, 11);
        // The server offers no ALPN at all. The client must not fail the
        // handshake; HTTP/1.1 is the default protocol on the established
        // stream, and no second connection may be opened.
        let (address, handle) = alpn_server(&set, None, ScriptedResponse::ok_dns(expected.clone()));
        let upstream = verified_owner(&set, address, "https://dns.example/dns-query");

        let response = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect("absent ALPN permits HTTP/1.1 on the same stream");
        assert_eq!(response.wire(), expected.as_slice());
        assert_eq!(
            handle.join().expect("server joined"),
            None,
            "no ALPN was negotiated"
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);
    });
}

#[test]
fn a_negotiated_http11_alpn_is_accepted() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x7201;
        let expected = response_wire(id, 12);
        let (address, handle) = alpn_server(
            &set,
            Some(vec![b"http/1.1".to_vec()]),
            ScriptedResponse::ok_dns(expected.clone()),
        );
        let upstream = verified_owner(&set, address, "https://dns.example/dns-query");

        let response = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect("a negotiated http/1.1 ALPN is the implemented protocol");
        assert_eq!(response.wire(), expected.as_slice());
        assert_eq!(
            handle.join().expect("server joined").as_deref(),
            Some("http/1.1")
        );
    });
}

#[test]
fn an_early_eof_with_a_larger_declared_length_is_an_incomplete_body() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x7202;
        let full = response_wire(id, 13);
        // Declare more bytes than are sent, then close. The client must
        // classify the early EOF as an incomplete body rather than accepting
        // the short prefix as a complete response.
        let declared = full.len() + 32;
        let body = full.clone();
        let server = HttpsServer::start_with(&set, move |_head, _body| {
            let mut bytes = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nContent-Length: {declared}\r\n\r\n"
            )
            .into_bytes();
            bytes.extend_from_slice(&body);
            Reply::WriteThenClose(bytes)
        });
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        // The server closes immediately, so the outcome is a body defect, not a
        // timeout: assert that specific classification.
        let error = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect_err("a body shorter than its declared length is incomplete");
        assert!(
            matches!(
                error,
                SecureError::DohProtocol(DohProtocolError::IncompleteBody)
            ),
            "an early EOF must be reported as an incomplete body, got {error:?}"
        );
        server.join();
    });
}

#[test]
fn a_truncated_chunked_body_is_an_incomplete_body() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x7203;
        let full = response_wire(id, 14);
        // Announce one chunk, write part of it, and never send the terminating
        // zero chunk. The body is incomplete, not a valid prefix.
        let body = full.clone();
        let server = HttpsServer::start_with(&set, move |_head, _body| {
            let mut bytes =
                b"HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nTransfer-Encoding: chunked\r\n\r\n"
                    .to_vec();
            bytes.extend_from_slice(format!("{:x}\r\n", body.len()).as_bytes());
            // Only half the announced chunk, then close without the terminator.
            bytes.extend_from_slice(&body[..body.len() / 2]);
            Reply::WriteThenClose(bytes)
        });
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect_err("an unterminated chunked body is incomplete");
        assert!(
            matches!(
                error,
                SecureError::DohProtocol(DohProtocolError::IncompleteBody)
            ),
            "a truncated chunked body must be incomplete, got {error:?}"
        );
        server.join();
    });
}

// ---------------------------------------------------------------------------
// Response preservation: the caller must receive the upstream's response with
// only the transaction ID rewritten
// ---------------------------------------------------------------------------

/// A valid response with RA explicitly cleared.
///
/// A real recursive resolver sets `RA`, but the `DoH` contract is that the caller
/// receives the upstream's response with only the ID fixed up, so clearing RA
/// exercises flag preservation directly.
fn response_wire_ra_clear(id: u16, marker: u8) -> Vec<u8> {
    let mut wire = response_wire(id, marker);
    wire[3] &= 0x7f; // clear RA
    wire
}

#[test]
fn a_ra_clear_response_keeps_ra_clear_and_only_rewrites_the_id() {
    block_on(async {
        let set = FixtureSet::generate();
        let caller_id = 0x7300;
        let upstream_wire = response_wire_ra_clear(0, 21);
        assert_eq!(
            upstream_wire[3] & 0x80,
            0,
            "the fixture must have RA clear before the exchange"
        );

        let server = HttpsServer::start(&set, ScriptedResponse::ok_dns(upstream_wire.clone()));
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let response = exchange_owned(&upstream, &query_wire(caller_id), open_context())
            .await
            .expect("a well-formed response succeeds");

        // RA must still be clear: the client restores the ID only.
        assert_eq!(
            response.wire()[3] & 0x80,
            0,
            "restoring the caller ID must not set RA"
        );
        // The ID is the caller's.
        assert_eq!(
            u16::from_be_bytes([response.wire()[0], response.wire()[1]]),
            caller_id
        );
        // Every other byte is exactly what the upstream sent.
        assert_eq!(&response.wire()[2..], &upstream_wire[2..]);
        // Metadata agrees with the returned wire.
        assert_eq!(response.request_id(), caller_id);
        assert_eq!(response.response_id(), caller_id);
        assert_eq!(
            u16::from_be_bytes([response.wire()[0], response.wire()[1]]),
            response.response_id(),
            "the reported response_id must equal the ID actually in the wire"
        );
        server.join();
    });
}

#[test]
fn an_ra_set_response_keeps_ra_set() {
    block_on(async {
        // The complement, so the assertion cannot pass by clearing flags.
        let set = FixtureSet::generate();
        let caller_id = 0x7301;
        let mut upstream_wire = response_wire(0, 22);
        upstream_wire[3] |= 0x80; // RA set
        let server = HttpsServer::start(&set, ScriptedResponse::ok_dns(upstream_wire.clone()));
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let response = exchange_owned(&upstream, &query_wire(caller_id), open_context())
            .await
            .expect("a well-formed response succeeds");

        assert_eq!(response.wire()[3] & 0x80, 0x80, "RA must stay set");
        assert_eq!(&response.wire()[2..], &upstream_wire[2..]);
        assert_eq!(response.response_id(), caller_id);
        server.join();
    });
}

// ---------------------------------------------------------------------------
// Response head and body bounds
// ---------------------------------------------------------------------------

#[test]
fn a_response_head_just_under_the_byte_bound_is_accepted() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x7400;
        let expected = response_wire(id, 31);
        // Size one header so the measured head lands one byte under 16 KiB.
        let mut response = ScriptedResponse::ok_dns(expected.clone());
        let base = "HTTP/1.1 200 OK\r\n".len()
            + "Content-Type: application/dns-message\r\n".len()
            + format!("Content-Length: {}\r\n", expected.len()).len()
            + 2; // terminating blank line
        let pad = MAX_HEAD_BYTES_FOR_TEST - base - "X-Pad: \r\n".len() - 1;
        response
            .extra_headers
            .push(("X-Pad".to_owned(), "a".repeat(pad)));
        let server = HttpsServer::start(&set, response);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let result = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect("a head just under the byte bound must be accepted");
        assert_eq!(result.wire(), expected.as_slice());
        server.join();
    });
}

#[test]
fn a_response_head_over_the_byte_bound_is_rejected() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x7401;
        // One header large enough to push the head past 16 KiB; a header-count
        // limit alone would not catch this.
        let mut response = ScriptedResponse::ok_dns(response_wire(id, 32));
        response
            .extra_headers
            .push(("X-Pad".to_owned(), "a".repeat(20 * 1024)));
        let server = HttpsServer::start(&set, response);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect_err("a head over the byte bound must be rejected");
        // A single header far larger than the whole allowed head is refused by
        // the raw parser bound while it is still being read, so no head is ever
        // handed to the exchange.
        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::ResponseHeadNotReceived),
            "an over-long head must never produce a parsed response head"
        );
        assert_eq!(error.side_effect(), SideEffectState::MaybeSent);
        server.join();
    });
}

#[test]
fn an_over_long_non_canonical_reason_phrase_is_rejected_on_the_raw_wire() {
    block_on(async {
        // The post-parse byte check reconstructs the head from parsed fields and
        // therefore cannot see a long reason phrase: Hyper keeps only the status
        // code and discards the rest of the status line. A head whose real wire
        // size is between 16 KiB and 32 KiB, made up almost entirely of the
        // reason phrase, would therefore slip past a post-parse-only bound.
        // This proves the *raw* parser bound rejects it in a real HTTP/1.1
        // loopback exchange.
        let set = FixtureSet::generate();
        let id = 0x7405;
        let body = response_wire(id, 35);
        let reason = "r".repeat(20 * 1024);
        assert!(
            reason.len() > 16 * 1024 && reason.len() < 32 * 1024,
            "the reason phrase must sit between the raw bound and the old buffer size"
        );
        let server = HttpsServer::start_with(&set, move |_head, _body| {
            let mut bytes = format!(
                "HTTP/1.1 200 {reason}\r\nContent-Type: application/dns-message\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .into_bytes();
            bytes.extend_from_slice(&body);
            // Hold the connection open after writing, so the client's failure
            // is decided by its own parser limit rather than by an EOF race.
            Reply::Respond(Box::new(ScriptedResponse {
                status: 200,
                reason: "OK",
                content_type: None,
                extra_headers: Vec::new(),
                framing: BodyFraming::Raw,
                body: bytes,
                truncate_after_head: false,
            }))
        });
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        // This is the discriminating assertion: under a 16 KiB parser buffer the
        // head cannot be assembled, and because the server keeps the connection
        // open the failure is decided by the client's own limit. If the buffer
        // were widened (to 32 KiB, or 1 MiB) the whole response would parse and
        // the exchange would SUCCEED, so `expect_err` would panic.
        let error = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect_err("a head over the raw 16 KiB bound must not be accepted");
        // The parser aborts the connection at its limit without ever handing a
        // head to the exchange, which is exactly the head-not-received case.
        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::ResponseHeadNotReceived),
            "an over-long raw head must never produce a parsed response head"
        );
        assert_eq!(error.side_effect(), SideEffectState::MaybeSent);
        server.join();
    });
}

#[test]
fn a_chunked_body_over_the_dns_maximum_is_rejected_by_the_incremental_gate() {
    block_on(async {
        // Chunked framing carries no Content-Length, so the head-stage check
        // cannot fire. The body must be stopped by the incremental bound in
        // `read_body` instead, which is what this exercises.
        let set = FixtureSet::generate();
        let id = 0x7406;
        let oversized = vec![0u8; 65_536]; // one byte over the DNS maximum
        let mut response = ScriptedResponse::ok_dns(oversized);
        response.framing = BodyFraming::Chunked;
        // Deliberately no Content-Length: chunked is the only framing signal.
        let server = HttpsServer::start(&set, response);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect_err("a chunked body over the maximum must be rejected");
        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::BodyTooLarge),
            "the incremental body bound must stop an over-max chunked body"
        );
        assert_eq!(error.side_effect(), SideEffectState::Sent);
        server.join();
    });
}

#[test]
fn a_close_delimited_body_over_the_dns_maximum_is_rejected_by_the_incremental_gate() {
    block_on(async {
        // A response with neither Content-Length nor chunked framing is
        // close-delimited, so again only the incremental bound can stop it.
        let set = FixtureSet::generate();
        let id = 0x7407;
        let oversized = vec![0u8; 65_536];
        let server = HttpsServer::start_with(&set, move |_head, _body| {
            let mut bytes =
                b"HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\n\r\n".to_vec();
            bytes.extend_from_slice(&oversized);
            Reply::WriteThenClose(bytes)
        });
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect_err("a close-delimited body over the maximum must be rejected");
        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::BodyTooLarge),
            "the incremental body bound must stop an over-max close-delimited body"
        );
        server.join();
    });
}

#[test]
fn a_declared_content_length_over_the_dns_maximum_fails_at_the_head() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x7402;
        let response = ScriptedResponse::ok_dns(response_wire(id, 33));
        let body = response.body.clone();
        let server = HttpsServer::start_with(&set, move |_head, _body| {
            // Declare 65536 while sending only a small body: the declared value
            // alone must be rejected, before any body byte is read.
            let mut bytes = b"HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nContent-Length: 65536\r\n\r\n"
                .to_vec();
            bytes.extend_from_slice(&body);
            Reply::WriteThenClose(bytes)
        });
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect_err("a declared body over the DNS maximum must be rejected");
        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::BodyTooLarge),
            "the declared length must fail at the head stage"
        );
        assert_eq!(error.side_effect(), SideEffectState::Sent);
        server.join();
    });
}

#[test]
fn a_body_of_exactly_the_dns_maximum_is_accepted() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x7403;
        let wire = dns_response_of_exact_len(id, 65_535);
        assert_eq!(wire.len(), 65_535);
        let server = HttpsServer::start(&set, ScriptedResponse::ok_dns(wire.clone()));
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let response = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect("a body of exactly the DNS maximum must be accepted");
        assert_eq!(response.wire().len(), 65_535);
        assert_eq!(&response.wire()[0..2], &wire[0..2]);
        server.join();
    });
}

#[test]
fn a_body_of_one_byte_over_the_dns_maximum_is_rejected() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x7404;
        let wire = dns_response_of_exact_len(id, 65_536);
        assert_eq!(wire.len(), 65_536);
        let server = HttpsServer::start(&set, ScriptedResponse::ok_dns(wire));
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(id), open_context())
            .await
            .expect_err("one byte over the maximum must be rejected");
        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::BodyTooLarge)
        );
        server.join();
    });
}

// ---------------------------------------------------------------------------
// Side-effect classification: an absent response head, and ALPN
// ---------------------------------------------------------------------------

#[test]
fn an_absent_response_head_is_never_reported_as_sent() {
    block_on(async {
        // The server accepts TLS, reads the request, then closes without writing
        // any response head. Whether the request reached the peer is unknown, so
        // claiming Sent would be false.
        let set = FixtureSet::generate();
        let server = HttpsServer::start_with(&set, |_head, _body| Reply::CloseImmediately);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x7410), open_context())
            .await
            .expect_err("a closed connection before a head is a failure");
        // Tightened from "not Sent" to the exact classification the contract
        // requires: the request was dispatched, but no head ever arrived, so
        // delivery is unknowable.
        assert_eq!(
            error,
            SecureError::DohProtocol(DohProtocolError::ResponseHeadNotReceived),
            "a clean close before any head must be the head-not-received case"
        );
        assert_eq!(
            error.side_effect(),
            SideEffectState::MaybeSent,
            "an absent response head is conservatively MaybeSent"
        );
        server.join();
    });
}

#[test]
fn a_peer_offering_only_h2_is_not_reinterpreted_as_http11() {
    block_on(async {
        // The server offers only h2. Slice3 now dispatches that protocol to
        // the scoped HTTP/2 driver; this deliberately HTTP/1.1-shaped test
        // peer cannot complete an h2 request, but the client must not retry it
        // as HTTP/1.1 or open another connection.
        let set = FixtureSet::generate();
        let (address, handle) = alpn_server(
            &set,
            Some(vec![b"h2".to_vec()]),
            ScriptedResponse::ok_dns(response_wire(0, 41)),
        );
        let upstream = verified_owner(&set, address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x7411), open_context())
            .await
            .expect_err("a peer that cannot speak HTTP/1.1 must not be used");
        assert_eq!(error.side_effect(), SideEffectState::MaybeSent);
        assert!(matches!(error, SecureError::Transport(_)), "got {error:?}");
        assert_eq!(
            error.side_effect(),
            SideEffectState::MaybeSent,
            "the h2 request was handed to the driver, but the peer did not return a response"
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);
        let _ = handle.join();
    });
}

#[test]
fn the_side_effect_classification_of_each_doh_defect_is_explicit() {
    // The classification is part of the contract, so it is asserted directly
    // rather than only through exchanges that happen to reach it.
    assert_eq!(
        DohProtocolError::UnexpectedAlpn.side_effect(),
        SideEffectState::NotSent,
        "ALPN is decided during the handshake, before any request exists"
    );
    assert_eq!(
        DohProtocolError::ResponseHeadNotReceived.side_effect(),
        SideEffectState::MaybeSent,
        "without a response head, delivery is unknowable"
    );
    for error in [
        DohProtocolError::UnexpectedStatus { status: 500 },
        DohProtocolError::WrongMediaType,
        DohProtocolError::MissingMediaType,
        DohProtocolError::ContentEncoding,
        DohProtocolError::ResponseHeadTooLarge,
        DohProtocolError::BodyTooLarge,
        DohProtocolError::IncompleteBody,
    ] {
        assert_eq!(
            error.side_effect(),
            SideEffectState::Sent,
            "{error:?} can only occur after a complete head arrived"
        );
    }
}

/// The response-head byte bound the tests size their fixtures against.
///
/// Kept in the test crate so a change to the production bound is caught by the
/// just-under/over pair rather than silently widening what they exercise.
const MAX_HEAD_BYTES_FOR_TEST: usize = 16 * 1024;

/// Builds a valid DNS response whose wire is exactly `len` bytes.
///
/// The response is padded with additional A records plus one padding label in
/// the question, chosen so the total is exactly `len`. The result is a genuinely
/// dns-core-valid response of a chosen size, which is what makes a 65535-byte
/// success and a 65536-byte rejection meaningful rather than a test of filler.
fn dns_response_of_exact_len(id: u16, len: usize) -> Vec<u8> {
    // Fixed per-record cost of a compressed-owner A record.
    const RECORD: usize = 16;
    // Header (12) + "example" (8) + "org" (4) + terminator (1) + type/class (4).
    const FIXED: usize = 12 + 8 + 4 + 1 + 4;

    // A padding label of `1 + p` bytes makes the remainder divide evenly by
    // RECORD. Searching for the smallest workable `p` keeps every label legal
    // (1..=63 bytes) and the arithmetic exact for any requested length.
    let (pad_len, answers) = (1..=63u8)
        .find_map(|p| {
            let used = FIXED + 1 + usize::from(p);
            let remaining = len.checked_sub(used)?;
            (remaining % RECORD == 0).then_some((p, remaining / RECORD))
        })
        .expect("a padding label exists for this length");
    let answers = u16::try_from(answers).expect("answer count fits u16");

    let mut wire = Vec::with_capacity(len);
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&[0x81, 0x80]);
    wire.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    wire.extend_from_slice(&answers.to_be_bytes()); // ANCOUNT
    wire.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    wire.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT
    wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
    wire.extend_from_slice(&[0x03, b'o', b'r', b'g']);
    wire.push(pad_len);
    wire.extend(std::iter::repeat_n(b'p', usize::from(pad_len)));
    wire.push(0x00);
    wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
    for index in 0..answers {
        wire.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]); // ptr, A IN
        wire.extend_from_slice(&60u32.to_be_bytes());
        let octet = u8::try_from(index % 256).expect("index mod 256 fits in a byte");
        wire.extend_from_slice(&[0x00, 0x04, 198, 51, 100, octet]);
    }
    assert_eq!(wire.len(), len, "the builder must hit the exact length");
    wire
}

/// A server that reads the request, then holds the connection without
/// answering.
///
/// The returned receiver exists so the control tests can be written uniformly;
/// it is already closed and carries no ordering signal. Ordering is established
/// by the deterministic phase seam in the module, never by a sleep here.
fn holding_server(set: &FixtureSet) -> (HttpsServer, std::sync::mpsc::Receiver<()>) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let server = HttpsServer::start_with(set, move |_head, _body| Reply::Hold);
    drop(tx);
    (server, rx)
}

#[test]
fn owner_close_before_the_response_is_closed_with_maybe_sent() {
    block_on(async {
        let set = FixtureSet::generate();
        let (server, _rx) = holding_server(&set);
        let upstream = Arc::new(verified_owner(
            &set,
            server.address,
            "https://dns.example/dns-query",
        ));
        let query = query_wire(0x7100);
        let exchange = {
            let upstream = Arc::clone(&upstream);
            let query = query.clone();
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                upstream.exchange(request, open_context()).await
            })
        };

        // Close while the request is in flight. The outcome must be the typed
        // owner-close cause; the exact side-effect state at a given phase is
        // proven deterministically by the in-crate phase matrix, because which
        // phase the close wins depends on scheduling rather than on a contract.
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
        drop(server);
    });
}

#[test]
fn caller_cancellation_before_the_response_is_cancelled_with_a_typed_state() {
    block_on(async {
        let set = FixtureSet::generate();
        let (server, _rx) = holding_server(&set);
        let upstream = Arc::new(verified_owner(
            &set,
            server.address,
            "https://dns.example/dns-query",
        ));
        let cancellation = TransportCancellation::new();
        let context = ExchangeContext::new(
            Instant::now() + Duration::from_secs(30),
            cancellation.clone(),
        );
        let query = query_wire(0x7101);
        let exchange = {
            let upstream = Arc::clone(&upstream);
            let query = query.clone();
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                upstream.exchange(request, context).await
            })
        };

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
        drop(server);
    });
}

#[test]
fn a_shared_absolute_deadline_terminates_the_exchange() {
    block_on(async {
        let set = FixtureSet::generate();
        let (server, _rx) = holding_server(&set);
        let upstream = Arc::new(verified_owner(
            &set,
            server.address,
            "https://dns.example/dns-query",
        ));
        let context = ExchangeContext::new(
            Instant::now() + Duration::from_millis(300),
            TransportCancellation::new(),
        );
        let query = query_wire(0x7102);
        let exchange = {
            let upstream = Arc::clone(&upstream);
            let query = query.clone();
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                upstream.exchange(request, context).await
            })
        };

        let error = timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("deadline-terminated exchange bounded")
            .expect("exchange task joined")
            .expect_err("the absolute deadline terminates the exchange");
        assert!(
            matches!(
                error,
                SecureError::Transport(UpstreamError::DeadlineExceeded(_))
            ),
            "got {error:?}"
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);
        drop(server);
    });
}

#[test]
fn a_dropped_exchange_future_releases_its_registration() {
    block_on(async {
        let set = FixtureSet::generate();
        let (server, _rx) = holding_server(&set);
        let upstream = Arc::new(verified_owner(
            &set,
            server.address,
            "https://dns.example/dns-query",
        ));
        let query = query_wire(0x7103);
        let exchange = {
            let upstream = Arc::clone(&upstream);
            let query = query.clone();
            tokio::spawn(async move {
                let request = ExchangeRequest::new(&query).expect("valid query");
                upstream.exchange(request, open_context()).await
            })
        };

        // Give the exchange a moment to register; then abort it. The assertion
        // is about the *outcome* (registration released), not about ordering.
        tokio::task::yield_now().await;
        exchange.abort();
        let _ = exchange.await;
        assert_eq!(
            upstream.in_flight_exchanges(),
            0,
            "an aborted future must release its registration"
        );
        assert_eq!(upstream.close().await, CloseResult::Closed);
        drop(server);
    });
}

#[test]
fn exchanges_after_close_are_rejected_without_opening_a_connection() {
    block_on(async {
        let (listener, address) = bind_listener();
        listener
            .set_nonblocking(true)
            .expect("listener non-blocking");
        let set = FixtureSet::generate();

        let upstream = verified_owner(&set, address, "https://dns.example/dns-query");
        assert_eq!(upstream.close().await, CloseResult::Closed);

        let error = exchange_owned(&upstream, &query_wire(0x7104), open_context())
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

#[test]
fn a_server_that_closes_without_responding_is_a_typed_failure() {
    block_on(async {
        let set = FixtureSet::generate();
        // Accept, complete TLS, read the request, then close with no response.
        let server = HttpsServer::start_with(&set, |_head, _body| Reply::CloseImmediately);
        let upstream = verified_owner(&set, server.address, "https://dns.example/dns-query");

        let error = exchange_owned(&upstream, &query_wire(0x7105), open_context())
            .await
            .expect_err("a connection closed before a response is a failure");
        assert!(
            matches!(
                error,
                SecureError::DohProtocol(_) | SecureError::Transport(_)
            ),
            "got {error:?}"
        );
        server.join();
        assert_eq!(upstream.in_flight_exchanges(), 0);
    });
}

#[test]
fn tls_verification_still_applies_to_the_doh_service_identity() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x7106;
        let server = HttpsServer::start(&set, ScriptedResponse::ok_dns(response_wire(id, 10)));

        // The certificate covers `dns.example`; asking for `other.example` must
        // fail authentication before any HTTP request is sent.
        let upstream = verified_owner(&set, server.address, "https://other.example/dns-query");
        let error = exchange_owned(&upstream, &query_wire(id), open_context())
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
fn an_oversized_outbound_query_is_rejected_before_any_connection() {
    block_on(async {
        let (listener, address) = bind_listener();
        listener
            .set_nonblocking(true)
            .expect("listener non-blocking");
        let set = FixtureSet::generate();
        let upstream = verified_owner(&set, address, "https://dns.example/dns-query");

        let mut query = query_wire(0x7107);
        query[10..12].copy_from_slice(&1u16.to_be_bytes()); // ARCOUNT
        query.resize(usize::from(u16::MAX) + 1, 0);

        let error = exchange_owned(&upstream, &query, open_context())
            .await
            .expect_err("a query beyond the DNS maximum must be rejected");
        assert_eq!(
            error,
            SecureError::DohRequest(mosdns_upstream_core::secure::DohRequestError::QueryTooLarge)
        );
        assert_eq!(error.side_effect(), SideEffectState::NotSent);

        match listener.accept() {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Ok(_) => panic!("an oversized query must never open a connection"),
            Err(error) => panic!("unexpected accept error: {error}"),
        }
    });
}

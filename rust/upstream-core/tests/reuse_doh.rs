//! Contract tests for `DoH` connection reuse — Slice 3 continuation.
//!
//! Scope: the `DoH` pooled owner retains and reuses an authenticated session —
//! both an HTTP/1.1 session and a negotiated HTTP/2 session — and refuses to
//! serve a request whose authority, identity, or policy differs from the session
//! it holds.
//!
//! Every test is deterministic: no wall-clock sleeps and no elapsed-time
//! polling. Reuse is proven by counting *accepted connections*; every await is
//! wrapped in a bounded guard.

mod fixtures;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use base64::Engine as _;
use fixtures::FixtureSet;
use mosdns_upstream_core::secure::SecureResponse;
use mosdns_upstream_core::{
    DohEndpoint, DohReuseOwner, ExchangeContext, ExchangeRequest, SecureError, TlsPolicy,
    TransportCancellation,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener as AsyncTcpListener;
use tokio_rustls::TlsAcceptor;

/// The upper bound on any single await: a deadlock guard only.
const DEADLINE: Duration = Duration::from_secs(20);

async fn bounded<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(DEADLINE, future)
        .await
        .expect("operation must complete within the test deadline")
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

fn context(seconds: u64) -> ExchangeContext {
    ExchangeContext::new(
        Instant::now() + Duration::from_secs(seconds),
        TransportCancellation::new(),
    )
}

/// The same context, but with a caller cancellation the test controls, and the
/// token handed back so the test can cancel it later.
fn context_with_token(seconds: u64, token: &TransportCancellation) -> ExchangeContext {
    ExchangeContext::new(Instant::now() + Duration::from_secs(seconds), token.clone())
}

/// Runs one pooled `DoH` exchange under the deadlock guard.
///
/// A pooled exchange owns a retained Hyper driver plus, for HTTP/2, the whole
/// child-tracking scope, so the future is inherently large; boxing it here keeps
/// every caller's own stack frame small instead of repeating `Box::pin` inline.
async fn pooled_exchange(
    owner: &DohReuseOwner,
    query: &[u8],
    context: ExchangeContext,
) -> Result<SecureResponse, SecureError> {
    let request = ExchangeRequest::new(query).expect("request");
    bounded(Box::pin(owner.exchange(request, context))).await
}

/// A minimal well-formed DNS query for `name`.
fn query_wire(id: u16, name: &str) -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&0x0100u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    for label in name.split('.') {
        wire.push(u8::try_from(label.len()).expect("label"));
        wire.extend_from_slice(label.as_bytes());
    }
    wire.push(0);
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire
}

/// A DNS answer echoing `query`'s ID and question.
fn a_response(query: &[u8], ip: [u8; 4]) -> Vec<u8> {
    let id = u16::from_be_bytes([query[0], query[1]]);
    let mut position = 12;
    while query[position] != 0 {
        position += 1 + usize::from(query[position]);
    }
    position += 1;
    let question = &query[12..position + 4];
    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&0x8180u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(&0u16.to_be_bytes());
    wire.extend_from_slice(question);
    wire.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
    wire.extend_from_slice(&60u32.to_be_bytes());
    wire.extend_from_slice(&[0x00, 0x04]);
    wire.extend_from_slice(&ip);
    wire
}

fn server_config(set: &FixtureSet) -> Arc<rustls::ServerConfig> {
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("ring provider supports the safe default protocol versions")
    .with_no_client_auth()
    .with_single_cert(
        vec![set.good.cert.clone(), set.root_chain()],
        set.good.key.clone_key(),
    )
    .expect("synthetic certificate and key are consistent");
    // Offer HTTP/1.1 only, so a retained session is the HTTP/1.1 pool path.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

/// A `DoH` server that serves `GET /dns-query?...` over HTTP/1.1 on each accepted
/// TLS connection, keeping a connection open across an idle gap.
struct DohServer {
    address: SocketAddr,
    accepts: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: std::thread::JoinHandle<()>,
}

impl DohServer {
    fn start(set: &FixtureSet, answer_ip: [u8; 4]) -> Self {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
        let address = listener.local_addr().expect("addr");
        listener.set_nonblocking(true).expect("nonblocking");
        let accepts = Arc::new(AtomicUsize::new(0));
        let accepts_in = Arc::clone(&accepts);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_in = Arc::clone(&stop);
        let config = server_config(set);

        let handle = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("server runtime");
            runtime.block_on(async move {
                let listener =
                    AsyncTcpListener::from_std(listener).expect("adopt the standard listener");
                while !stop_in.load(Ordering::SeqCst) {
                    let accepted = tokio::time::timeout(DEADLINE, listener.accept()).await;
                    let Ok(Ok((stream, _))) = accepted else {
                        continue;
                    };
                    if stop_in.load(Ordering::SeqCst) {
                        break;
                    }
                    accepts_in.fetch_add(1, Ordering::SeqCst);
                    let acceptor = TlsAcceptor::from(Arc::clone(&config));
                    let Ok(mut tls) = acceptor.accept(stream).await else {
                        continue;
                    };
                    // Serve requests on this connection until EOF or release.
                    loop {
                        if stop_in.load(Ordering::SeqCst) {
                            return;
                        }
                        let mut buffer = Vec::new();
                        let mut chunk = [0u8; 512];
                        let head_end = loop {
                            if let Some(index) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                                break index + 4;
                            }
                            if buffer.len() > 64 * 1024 {
                                return;
                            }
                            // The release flag must be observed inside this loop
                            // too: a client that holds the session idle between
                            // exchanges would otherwise keep this connection
                            // parked forever and `join` could never return.
                            if stop_in.load(Ordering::SeqCst) {
                                return;
                            }
                            match tokio::time::timeout(
                                Duration::from_millis(20),
                                tls.read(&mut chunk),
                            )
                            .await
                            {
                                // EOF and a read error both mean the peer is gone.
                                Ok(Ok(0) | Err(_)) => return,
                                Ok(Ok(read)) => buffer.extend_from_slice(&chunk[..read]),
                                // An idle gap keeps the session for a later reuse.
                                // The loop simply waits for the next byte.
                                Err(_) => {}
                            }
                        };

                        let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
                        let Some(target) =
                            head.lines().next().and_then(|line| line.split(' ').nth(1))
                        else {
                            return;
                        };
                        // The query travels base64url-encoded in the `dns` param,
                        // exactly as the reviewed encoder produces it.
                        let Some(encoded) = target.split("dns=").nth(1) else {
                            return;
                        };
                        let Ok(query) =
                            base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded)
                        else {
                            return;
                        };
                        let reply = a_response(&query, answer_ip);
                        let mut response = Vec::new();
                        response.extend_from_slice(b"HTTP/1.1 200 OK\r\n");
                        response.extend_from_slice(b"content-type: application/dns-message\r\n");
                        response.extend_from_slice(
                            format!("content-length: {}\r\n", reply.len()).as_bytes(),
                        );
                        response.extend_from_slice(b"\r\n");
                        response.extend_from_slice(&reply);
                        if tls.write_all(&response).await.is_err() || tls.flush().await.is_err() {
                            return;
                        }
                    }
                }
            });
        });

        Self {
            address,
            accepts,
            stop,
            handle,
        }
    }

    fn accepts(&self) -> usize {
        self.accepts.load(Ordering::SeqCst)
    }

    fn join(self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake a parked accept.
        let _ = std::net::TcpStream::connect(self.address);
        self.handle.join().expect("server thread joined");
    }
}

/// The same synthetic certificate, offering **only** `h2` via ALPN, so a
/// negotiated session on this listener is necessarily the HTTP/2 pool path.
fn h2_server_config(set: &FixtureSet) -> Arc<rustls::ServerConfig> {
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("ring provider supports the safe default protocol versions")
    .with_no_client_auth()
    .with_single_cert(
        vec![set.good.cert.clone(), set.root_chain()],
        set.good.key.clone_key(),
    )
    .expect("synthetic certificate and key are consistent");
    config.alpn_protocols = vec![b"h2".to_vec()];
    Arc::new(config)
}

/// A `DoH` server speaking HTTP/2 on each accepted TLS connection.
///
/// It serves every request that arrives on a connection, so a client that
/// reuses one session issues its later streams on the same accepted connection
/// and the accept count stays at one.
struct H2DohServer {
    address: SocketAddr,
    accepts: Arc<AtomicUsize>,
    streams: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: std::thread::JoinHandle<()>,
}

impl H2DohServer {
    fn start(set: &FixtureSet, answer_ip: [u8; 4]) -> Self {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
        let address = listener.local_addr().expect("addr");
        listener.set_nonblocking(true).expect("nonblocking");
        let accepts = Arc::new(AtomicUsize::new(0));
        let accepts_in = Arc::clone(&accepts);
        let streams = Arc::new(AtomicUsize::new(0));
        let streams_in = Arc::clone(&streams);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_in = Arc::clone(&stop);
        let config = h2_server_config(set);

        let handle = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("server runtime");
            runtime.block_on(async move {
                let listener =
                    AsyncTcpListener::from_std(listener).expect("adopt the standard listener");
                while !stop_in.load(Ordering::SeqCst) {
                    let accepted = tokio::time::timeout(DEADLINE, listener.accept()).await;
                    let Ok(Ok((stream, _))) = accepted else {
                        continue;
                    };
                    if stop_in.load(Ordering::SeqCst) {
                        break;
                    }
                    accepts_in.fetch_add(1, Ordering::SeqCst);
                    let acceptor = TlsAcceptor::from(Arc::clone(&config));
                    let Ok(tls) = acceptor.accept(stream).await else {
                        continue;
                    };
                    let Ok(mut connection) = h2::server::handshake(tls).await else {
                        continue;
                    };
                    // Serve streams on this one connection. A pooled client keeps
                    // the session between exchanges, so the gap between the two
                    // streams shows up here as an accept timeout, not as a close.
                    loop {
                        if stop_in.load(Ordering::SeqCst) {
                            return;
                        }
                        let next =
                            tokio::time::timeout(Duration::from_millis(20), connection.accept())
                                .await;
                        let (request, mut respond) = match next {
                            Ok(Some(Ok(pair))) => pair,
                            // An idle gap keeps the session for a later reuse.
                            Err(_) => continue,
                            // The peer closed or the connection errored.
                            Ok(Some(Err(_)) | None) => break,
                        };
                        let Some(path_and_query) = request.uri().path_and_query() else {
                            return;
                        };
                        let target = path_and_query.as_str();
                        let Some(encoded) = target.split("dns=").nth(1) else {
                            return;
                        };
                        let Ok(query) =
                            base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded)
                        else {
                            return;
                        };
                        let reply = a_response(&query, answer_ip);
                        let head = hyper::Response::builder()
                            .status(200)
                            .header("content-type", "application/dns-message")
                            .body(())
                            .expect("h2 response head");
                        let Ok(mut send) = respond.send_response(head, false) else {
                            return;
                        };
                        if send
                            .send_data(hyper::body::Bytes::from(reply), true)
                            .is_err()
                        {
                            return;
                        }
                        streams_in.fetch_add(1, Ordering::SeqCst);
                        // Frames queued by `send_data` reach the wire when the
                        // connection is polled again, which the next iteration's
                        // `accept` does.
                    }
                }
            });
        });

        Self {
            address,
            accepts,
            streams,
            stop,
            handle,
        }
    }

    fn accepts(&self) -> usize {
        self.accepts.load(Ordering::SeqCst)
    }

    fn streams(&self) -> usize {
        self.streams.load(Ordering::SeqCst)
    }

    fn join(self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake a parked accept.
        let _ = std::net::TcpStream::connect(self.address);
        self.handle.join().expect("server thread joined");
    }
}

fn doh_owner(set: &FixtureSet, address: SocketAddr, service_url: &str) -> DohReuseOwner {
    DohReuseOwner::new(
        DohEndpoint::new(service_url, address).expect("doh endpoint"),
        TlsPolicy::verified(set.root_store_a()).expect("verified policy"),
    )
    .expect("owner")
}

// ---------------------------------------------------------------------------
// DoH reuse
// ---------------------------------------------------------------------------

#[test]
fn a_second_doh_exchange_reuses_one_authenticated_session() {
    block_on(async {
        let set = FixtureSet::generate();
        let server = DohServer::start(&set, [192, 0, 2, 50]);
        let owner = doh_owner(&set, server.address, "https://dns.example/dns-query");
        let query = query_wire(0x7001, "doh-reuse.example.org");

        let first = pooled_exchange(&owner, &query, context(10))
            .await
            .expect("first doh exchange");
        assert_eq!(first.request_id(), 0x7001, "the original ID is preserved");
        assert_eq!(owner.idle_connections(), 1, "a session is retained");

        let second = pooled_exchange(&owner, &query, context(10))
            .await
            .expect("second doh exchange");
        assert_eq!(second.request_id(), 0x7001);

        assert_eq!(
            server.accepts(),
            1,
            "the second DoH exchange must reuse the authenticated session"
        );
        server.join();
    });
}

/// A different service URL on the same dial address is a different key, so it
/// must not be served by the retained session.
#[test]
fn a_different_service_authority_does_not_reuse_the_retained_session() {
    block_on(async {
        let set = FixtureSet::generate();
        let server = DohServer::start(&set, [192, 0, 2, 51]);
        let query = query_wire(0x7002, "authority.example.org");

        let first = doh_owner(&set, server.address, "https://dns.example/dns-query");
        pooled_exchange(&first, &query, context(10))
            .await
            .expect("first exchange");
        assert_eq!(server.accepts(), 1);

        // A second owner for a different authority on the same address holds a
        // different key, so it dials rather than borrowing the session.
        let other = doh_owner(&set, server.address, "https://other.example/dns-query");
        let outcome = pooled_exchange(&other, &query, context(10)).await;
        // The server presents `dns.example`, so the mismatched identity fails
        // verification; either way it must not have reused the first session.
        assert!(
            outcome.is_err(),
            "a different authority must not be served by the retained session"
        );
        assert_eq!(
            server.accepts(),
            1,
            "the retained session must not have been reused for a different \
             authority (the rejected handshake is not an accepted request)"
        );
        server.join();
    });
}

#[test]
fn doh_reuse_honors_an_expired_deadline_without_dialing() {
    block_on(async {
        let set = FixtureSet::generate();
        let server = DohServer::start(&set, [192, 0, 2, 52]);
        let owner = doh_owner(&set, server.address, "https://dns.example/dns-query");
        let query = query_wire(0x7003, "deadline.example.org");

        let Err(error) = pooled_exchange(&owner, &query, context(0)).await else {
            panic!("an expired deadline must be refused");
        };
        assert!(
            matches!(error, mosdns_upstream_core::SecureError::Transport(_)),
            "an expired deadline is a typed transport control error, got {error:?}"
        );
        assert_eq!(server.accepts(), 0, "an expired deadline must not dial");
        server.join();
    });
}

#[test]
fn doh_reuse_close_is_idempotent_and_drops_the_session() {
    block_on(async {
        let set = FixtureSet::generate();
        let server = DohServer::start(&set, [192, 0, 2, 53]);
        let owner = doh_owner(&set, server.address, "https://dns.example/dns-query");
        let query = query_wire(0x7004, "close.example.org");

        pooled_exchange(&owner, &query, context(10))
            .await
            .expect("exchange");
        assert_eq!(owner.idle_connections(), 1, "a session is retained");

        assert_eq!(
            owner.close().await,
            mosdns_upstream_core::CloseResult::Closed
        );
        assert_eq!(
            owner.close().await,
            mosdns_upstream_core::CloseResult::AlreadyClosed,
            "repeated close converges"
        );
        assert_eq!(owner.idle_connections(), 0, "close drops the session");
        assert_eq!(owner.in_flight_exchanges(), 0, "close drains registrations");
        server.join();
    });
}

/// A negotiated HTTP/2 session is retained and reused: two sequential exchanges
/// must ride the **same accepted connection** as two streams of one h2 session.
#[test]
fn a_second_doh_exchange_reuses_one_http2_session() {
    block_on(async {
        let set = FixtureSet::generate();
        let server = H2DohServer::start(&set, [192, 0, 2, 60]);
        let owner = doh_owner(&set, server.address, "https://dns.example/dns-query");
        let query = query_wire(0x7005, "h2-reuse.example.org");

        let first = pooled_exchange(&owner, &query, context(10))
            .await
            .expect("first h2 exchange");
        assert_eq!(first.request_id(), 0x7005, "the original ID is preserved");
        assert_eq!(
            owner.idle_connections(),
            1,
            "the negotiated h2 session is retained"
        );

        let second = pooled_exchange(&owner, &query, context(10))
            .await
            .expect("second h2 exchange");
        assert_eq!(second.request_id(), 0x7005);

        assert_eq!(
            server.streams(),
            2,
            "both exchanges must have been served as h2 streams"
        );
        assert_eq!(
            server.accepts(),
            1,
            "the second h2 exchange must reuse the retained session's connection"
        );
        server.join();
    });
}

/// The retained HTTP/2 driver must not be tied to the caller token of the
/// request that opened the session.
///
/// A pooled session outlives that request. If the pooled scope captured its
/// caller token, cancelling it after the first exchange succeeded would abort the
/// connection driver — which is itself a tracked child — and the second exchange
/// would have to dial again. This test cancels the first token and asserts the
/// second exchange still rides the original connection.
#[test]
fn a_cancelled_first_caller_token_does_not_kill_the_retained_http2_session() {
    block_on(async {
        let set = FixtureSet::generate();
        let server = H2DohServer::start(&set, [192, 0, 2, 61]);
        let owner = doh_owner(&set, server.address, "https://dns.example/dns-query");
        let query = query_wire(0x7006, "h2-cancel.example.org");

        let first_token = TransportCancellation::new();
        let first = pooled_exchange(&owner, &query, context_with_token(10, &first_token))
            .await
            .expect("first h2 exchange");
        assert_eq!(first.request_id(), 0x7006);
        assert_eq!(
            owner.idle_connections(),
            1,
            "the negotiated h2 session is retained"
        );

        // The first caller is done and goes away. Nothing about the retained
        // session may depend on this token any more.
        first_token.cancel();

        // Give the cancellation a chance to reach any child that (wrongly)
        // observed it before asserting the reuse still works.
        tokio::task::yield_now().await;

        let second_token = TransportCancellation::new();
        let second = pooled_exchange(&owner, &query, context_with_token(10, &second_token))
            .await
            .expect("second h2 exchange after the first caller cancelled");
        assert_eq!(second.request_id(), 0x7006);

        assert_eq!(
            server.streams(),
            2,
            "both exchanges must have been served as h2 streams"
        );
        assert_eq!(
            server.accepts(),
            1,
            "cancelling the first caller's token must not have forced a new \
             connection"
        );
        server.join();
    });
}

/// The endpoint keeps its numeric dial separate from the service authority and
/// path across reuse.
#[test]
fn doh_reuse_keeps_numeric_dial_separate_from_authority_and_path() {
    // A real bound loopback port, so the endpoint's dial address is concrete.
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
    let dial = listener.local_addr().expect("addr");
    drop(listener);

    let endpoint = DohEndpoint::new("https://dns.example/dns-query", dial).expect("endpoint");
    assert_eq!(endpoint.dial(), dial, "the dial address is the numeric one");
    assert_eq!(endpoint.host(), "dns.example", "authority is the service");
    assert_eq!(endpoint.path(), "/dns-query", "path is the service path");
    assert_eq!(
        endpoint.identity().as_str(),
        "dns.example",
        "identity comes from the URL host, never the dial address"
    );
}

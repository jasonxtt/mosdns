//! Contract tests for secure connection reuse — Slice 3.
//!
//! Scope: `DoT` reuse with strict service-identity separation, and the `DoH`
//! key-level ALPN/authority discrimination that governs reuse.
//!
//! Every test is deterministic: no wall-clock sleeps and no elapsed-time
//! polling. Reuse is proven by counting *accepted connections*; every await is
//! wrapped in a bounded guard.

mod fixtures;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use fixtures::{FixtureSet, ServerIdentityFixture};
use mosdns_upstream_core::{
    DotEndpoint, ExchangeContext, ExchangeRequest, ReuseKey, SecureKey, SecureKind,
    SecureReuseOwner, ServerIdentity, TlsPolicy, TransportCancellation,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener as AsyncTcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::client::TlsStream;

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

fn server_config(fixture: &ServerIdentityFixture) -> Arc<rustls::ServerConfig> {
    rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("ring provider supports the safe default protocol versions")
        .with_no_client_auth()
        .with_single_cert(vec![fixture.cert.clone()], fixture.key.clone_key())
        .map(Arc::new)
        .expect("synthetic certificate and key are consistent")
}

/// A scripted `DoT` server that counts accepted connections and serves framed
/// exchanges on each one until the test releases it.
///
/// It keeps a connection open across an idle gap so reuse is observable: a
/// timeout between exchanges is not a disconnect.
struct DotServer {
    address: SocketAddr,
    accepts: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: std::thread::JoinHandle<()>,
}

impl DotServer {
    /// Starts a `DoT` server presenting `fixture`'s certificate.
    fn start(fixture: &ServerIdentityFixture, answer_ip: [u8; 4]) -> Self {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
        let address = listener.local_addr().expect("addr");
        listener.set_nonblocking(true).expect("nonblocking");
        let accepts = Arc::new(AtomicUsize::new(0));
        let accepts_in = Arc::clone(&accepts);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_in = Arc::clone(&stop);
        let config = server_config(fixture);

        let handle = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("server runtime");
            runtime.block_on(async move {
                let listener =
                    AsyncTcpListener::from_std(listener).expect("adopt the standard listener");
                while !stop_in.load(Ordering::SeqCst) {
                    // The stop handshake wakes accept; a timeout is a poll
                    // boundary, not a failure.
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
                    // Serve framed exchanges on this TLS session until EOF or
                    // release. A read timeout between exchanges is an idle gap.
                    loop {
                        if stop_in.load(Ordering::SeqCst) {
                            return;
                        }
                        let mut prefix = [0u8; 2];
                        match tokio::time::timeout(
                            Duration::from_millis(20),
                            tls.read_exact(&mut prefix),
                        )
                        .await
                        {
                            Ok(Ok(_)) => {}
                            // Idle gap: keep the session open for a later reuse.
                            Ok(Err(_)) => break,
                            Err(_) => continue,
                        }
                        let length = usize::from(u16::from_be_bytes(prefix));
                        let mut body = vec![0u8; length];
                        if tls.read_exact(&mut body).await.is_err() {
                            break;
                        }
                        let reply = a_response(&body, answer_ip);
                        let mut framed =
                            Vec::from(u16::try_from(reply.len()).expect("len").to_be_bytes());
                        framed.extend_from_slice(&reply);
                        if tls.write_all(&framed).await.is_err() || tls.flush().await.is_err() {
                            break;
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

    /// Releases the server and joins its thread under a bounded wait.
    fn join(self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake a parked accept with one throwaway connection.
        let _ = std::net::TcpStream::connect(self.address);
        self.handle.join().expect("server thread joined");
    }
}

/// A verified `DoT` reuse owner dialing `address` as `identity` against root A.
fn dot_owner(set: &FixtureSet, address: SocketAddr, identity: &str) -> SecureReuseOwner {
    SecureReuseOwner::new(
        DotEndpoint::new(
            address,
            ServerIdentity::new(identity).expect("valid identity"),
        )
        .expect("dot endpoint"),
        TlsPolicy::verified(set.root_store_a()).expect("verified policy"),
    )
    .expect("owner")
}

// ---------------------------------------------------------------------------
// Slice 3 — DoT reuse with identity isolation
// ---------------------------------------------------------------------------

#[test]
fn a_second_dot_exchange_for_the_same_identity_reuses_one_tls_session() {
    block_on(async {
        let set = FixtureSet::generate();
        let server = DotServer::start(&set.good, [192, 0, 2, 30]);
        let owner = dot_owner(&set, server.address, "dns.example");
        let query = query_wire(0x1111, "dot-reuse.example.org");

        let first =
            bounded(owner.exchange(ExchangeRequest::new(&query).expect("request"), context(10)))
                .await
                .expect("first dot exchange");
        assert_eq!(first.response_id(), 0x1111, "the original ID is preserved");

        let second =
            bounded(owner.exchange(ExchangeRequest::new(&query).expect("request"), context(10)))
                .await
                .expect("second dot exchange");
        assert_eq!(second.response_id(), 0x1111);

        assert_eq!(
            server.accepts(),
            1,
            "the second DoT exchange must reuse the established TLS session"
        );
        server.join();
    });
}

/// The security-critical case: a TLS session authenticated as one identity is
/// never reused for a different identity, even on the same dial address.
#[test]
fn a_different_service_identity_never_reuses_an_authenticated_dot_session() {
    block_on(async {
        let set = FixtureSet::generate();
        // The server presents `dns.example`, so a request for `other.example`
        // must fail verification rather than reuse the trusted session.
        let server = DotServer::start(&set.good, [192, 0, 2, 31]);
        let good = dot_owner(&set, server.address, "dns.example");
        let query = query_wire(0x2222, "identity.example.org");

        bounded(good.exchange(ExchangeRequest::new(&query).expect("request"), context(10)))
            .await
            .expect("the matching identity authenticates");
        assert_eq!(server.accepts(), 1);

        // A distinct identity is a distinct key, so it must dial again and fail
        // its own handshake rather than borrow the authenticated session.
        let other = dot_owner(&set, server.address, "other.example");
        let outcome =
            bounded(other.exchange(ExchangeRequest::new(&query).expect("request"), context(10)))
                .await;
        assert!(
            outcome.is_err(),
            "a mismatched identity must not succeed on the reused session"
        );
        server.join();
    });
}

#[test]
fn a_different_dial_address_opens_its_own_dot_session() {
    block_on(async {
        let set = FixtureSet::generate();
        let first = DotServer::start(&set.good, [192, 0, 2, 32]);
        let second = DotServer::start(&set.good, [192, 0, 2, 33]);
        let query = query_wire(0x3333, "dial.example.org");

        let owner_a = dot_owner(&set, first.address, "dns.example");
        bounded(owner_a.exchange(ExchangeRequest::new(&query).expect("request"), context(10)))
            .await
            .expect("first dial");

        let owner_b = dot_owner(&set, second.address, "dns.example");
        bounded(owner_b.exchange(ExchangeRequest::new(&query).expect("request"), context(10)))
            .await
            .expect("second dial");

        assert_eq!(first.accepts(), 1);
        assert_eq!(second.accepts(), 1);
        first.join();
        second.join();
    });
}

#[test]
fn dot_reuse_honors_an_expired_deadline_without_dialing() {
    block_on(async {
        let set = FixtureSet::generate();
        let server = DotServer::start(&set.good, [192, 0, 2, 34]);
        let owner = dot_owner(&set, server.address, "dns.example");
        let query = query_wire(0x4444, "deadline.example.org");

        let Err(error) =
            bounded(owner.exchange(ExchangeRequest::new(&query).expect("request"), context(0)))
                .await
        else {
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
fn dot_reuse_close_is_idempotent_and_refuses_later_exchanges() {
    block_on(async {
        let set = FixtureSet::generate();
        let server = DotServer::start(&set.good, [192, 0, 2, 35]);
        let owner = dot_owner(&set, server.address, "dns.example");
        let query = query_wire(0x5555, "close.example.org");

        bounded(owner.exchange(ExchangeRequest::new(&query).expect("request"), context(10)))
            .await
            .expect("exchange");
        assert!(owner.idle_connections() >= 1, "a session is retained");

        assert_eq!(
            owner.close().await,
            mosdns_upstream_core::CloseResult::Closed
        );
        assert_eq!(
            owner.close().await,
            mosdns_upstream_core::CloseResult::AlreadyClosed,
            "repeated close converges"
        );
        assert_eq!(owner.idle_connections(), 0, "close drops idle sessions");
        server.join();
    });
}

// ---------------------------------------------------------------------------
// Slice 3 — DoH key discrimination (ALPN and authority)
// ---------------------------------------------------------------------------

/// The `DoH` key must separate HTTP/1.1 from HTTP/2 and must separate distinct
/// authorities, because a pooled HTTPS session is only valid for the protocol
/// and service it was established for.
#[test]
fn a_doh_reuse_key_separates_protocol_and_authority() {
    let dial = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 443);
    let key = |authority: &str, alpn: &str| {
        ReuseKey::plain_secure_endpoint(
            dial,
            SecureKey::new(SecureKind::Doh, "doh.example", Some(authority), false),
        )
        .expect("key")
        .with_negotiated_protocol(Some(alpn.to_owned()))
    };

    assert_ne!(
        key("doh.example", "h2"),
        key("doh.example", "http/1.1"),
        "HTTP/2 and HTTP/1.1 must not share a pooled session"
    );
    assert_ne!(
        key("doh.example", "h2"),
        key("doh.example:8443", "h2"),
        "a different authority must not share a pooled session"
    );
    assert_eq!(
        key("doh.example", "h2"),
        key("doh.example", "h2"),
        "an identical service and protocol is one key"
    );
}

/// A `DoH` owner keeps the numeric dial separate from the service authority, and
/// its endpoint retains the configured identity after reuse is enabled.
#[test]
fn a_doh_endpoint_keeps_numeric_dial_separate_from_the_service_authority() {
    let dial = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 443);
    let endpoint = mosdns_upstream_core::DohEndpoint::new("https://doh.example/dns-query", dial)
        .expect("doh endpoint");
    assert_eq!(endpoint.dial(), dial, "the dial address is the numeric one");
    assert_eq!(
        endpoint.host(),
        "doh.example",
        "the authority is the service"
    );
    assert_eq!(
        endpoint.path(),
        "/dns-query",
        "the path is the service path"
    );
    // The service identity is derived from the URL host, never the dial address.
    assert_eq!(endpoint.identity().as_str(), "doh.example");
    // And a reuse key over that endpoint carries the authority, not a hostname
    // in place of the dial address.
    let reuse = ReuseKey::plain_secure_endpoint(
        endpoint.dial(),
        SecureKey::new(
            SecureKind::Doh,
            endpoint.identity().as_str(),
            Some(&endpoint.authority()),
            false,
        ),
    )
    .expect("reuse key");
    assert_eq!(reuse.dial(), dial);
}

/// `TlsStream` is referenced by the server fixture; keep the import meaningful.
#[allow(dead_code)]
fn assert_tls_stream(_stream: &TlsStream<TcpStream>) {}

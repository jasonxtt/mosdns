//! Focused Slice 2 coverage for the real shared `DoH3` connection.
//!
//! This fixture deliberately accepts one QUIC connection and two concurrent
//! HTTP/3 request streams. The server waits until both request headers and
//! request FINs arrive before sending either response, proving that the client
//! multiplexes streams through one long-lived h3 driver instead of serializing
//! or opening a replacement connection.

mod fixtures;

use base64::Engine as _;
use std::collections::BTreeSet;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use fixtures::FixtureSet;
use hyper::body::{Buf as _, Bytes};
use mosdns_upstream_core::secure::{
    DohEndpoint, DohProtocolError, SecureError, SecureResponse, TlsPolicy,
};
use mosdns_upstream_core::{
    Doh3ReuseUpstream, ExchangeContext, ExchangeRequest, SideEffectState, TransportCancellation,
    UpstreamError,
};
use quinn::crypto::rustls::QuicServerConfig;
use rustls::ServerConfig;
use tokio::sync::{Barrier, oneshot};
use tokio::time::timeout;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build test runtime")
        .block_on(future)
}

fn query_wire(id: u16) -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&[0x01, 0x00]);
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
    wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0]);
    wire.extend_from_slice(&[0, 1, 0, 1]);
    wire
}

fn query_wire_with_marker(id: u16, marker: u8) -> Vec<u8> {
    let mut wire = query_wire(id);
    wire[13..20].copy_from_slice(&[b'm', b'a', b'r', b'k', b'e', b'r', b'0' + marker]);
    wire
}

fn marker_from_wire(wire: &[u8]) -> Option<u8> {
    (wire.get(13..19)? == b"marker")
        .then(|| wire.get(19).copied()?.checked_sub(b'0'))
        .flatten()
}

fn marker_from_path(path: &str) -> Option<u8> {
    let encoded = path.split_once("?dns=")?.1;
    let wire = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .ok()?;
    marker_from_wire(&wire)
}

fn response_wire(id: u16, marker: u8) -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&[0x81, 0x80]);
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&[0, 0, 0, 0]);
    wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
    wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0]);
    wire.extend_from_slice(&[0, 1, 0, 1]);
    wire.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1]);
    wire.extend_from_slice(&60u32.to_be_bytes());
    wire.extend_from_slice(&[0, 4, 192, 0, 2, marker]);
    wire
}

fn server_config(
    cert: rustls::pki_types::CertificateDer<'static>,
    key: rustls::pki_types::PrivateKeyDer<'static>,
    max_bidi: Option<u32>,
) -> quinn::ServerConfig {
    let mut tls =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("TLS 1.3 provider")
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .expect("synthetic certificate and key");
    tls.alpn_protocols = vec![b"h3".to_vec()];
    let quic = QuicServerConfig::try_from(tls).expect("QUIC TLS config");
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(quic));
    if let Some(max_bidi) = max_bidi {
        let mut transport = quinn::TransportConfig::default();
        transport.max_concurrent_bidi_streams(quinn::VarInt::from_u32(max_bidi));
        config.transport_config(Arc::new(transport));
    }
    config
}

#[derive(Debug)]
struct RequestEvidence {
    authority: String,
    path: String,
    request_body: Vec<u8>,
    request_fin: bool,
}

struct ReuseServer {
    address: SocketAddr,
    first_ready: Arc<tokio::sync::Notify>,
    release_first: Arc<tokio::sync::Notify>,
    handle: thread::JoinHandle<Vec<RequestEvidence>>,
}

impl ReuseServer {
    fn start(set: &FixtureSet) -> Self {
        Self::start_with_options(set, 2, None, false)
    }

    fn start_with_limit(set: &FixtureSet, streams: usize, max_bidi: Option<u32>) -> Self {
        Self::start_with_options(set, streams, max_bidi, false)
    }

    fn start_with_hold(set: &FixtureSet) -> Self {
        Self::start_with_options(set, 1, Some(1), true)
    }

    #[allow(clippy::too_many_lines)]
    fn start_with_options(
        set: &FixtureSet,
        streams: usize,
        max_bidi: Option<u32>,
        hold_first: bool,
    ) -> Self {
        let cert = set.good.cert.clone();
        let key = set.good.key.clone_key();
        let (address_tx, address_rx) = std::sync::mpsc::channel();
        let first_ready = Arc::new(tokio::sync::Notify::new());
        let first_ready_thread = Arc::clone(&first_ready);
        let release_first = Arc::new(tokio::sync::Notify::new());
        let release_first_thread = Arc::clone(&release_first);
        let handle = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build server runtime");
            runtime.block_on(async move {
                let endpoint = quinn::Endpoint::server(
                    server_config(cert, key, max_bidi),
                    SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
                )
                .expect("bind QUIC server");
                let address = endpoint.local_addr().expect("server address");
                address_tx.send(address).expect("publish server address");

                let incoming = timeout(TEST_TIMEOUT, endpoint.accept())
                    .await
                    .expect("accept bounded")
                    .expect("one connection");
                let connection = incoming.await.expect("QUIC handshake");
                let connection_for_wait = connection.clone();
                let mut h3 = h3::server::builder()
                    .build::<_, Bytes>(h3_quinn::Connection::new(connection))
                    .await
                    .expect("build server h3 connection");
                let barrier = (max_bidi != Some(1)).then(|| Arc::new(Barrier::new(streams)));
                let mut handlers = Vec::new();
                for index in 0..streams {
                    let marker = 0x20
                        + u8::try_from(index).expect("stress stream index fits response marker");
                    let resolver = timeout(TEST_TIMEOUT, h3.accept())
                        .await
                        .expect("accept request bounded")
                        .expect("accept request")
                        .expect("request stream remains open");
                    if hold_first && index == 0 {
                        first_ready_thread.notify_one();
                    }
                    let barrier = barrier.clone();
                    let release_first = Arc::clone(&release_first_thread);
                    handlers.push(tokio::spawn(async move {
                        let (request, mut stream) =
                            resolver.resolve_request().await.expect("resolve request");
                        let (parts, ()) = request.into_parts();
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
                        let mut request_body = Vec::new();
                        let request_fin = loop {
                            match stream.recv_data().await {
                                Ok(Some(data)) => request_body.extend_from_slice(data.chunk()),
                                Ok(None) => break true,
                                Err(_) => break false,
                            }
                        };
                        if let Some(barrier) = barrier {
                            barrier.wait().await;
                        }
                        if hold_first && index == 0 {
                            release_first.notified().await;
                        }

                        let marker = marker_from_path(&path).unwrap_or(marker);
                        let body = response_wire(0, marker);
                        let response = hyper::Response::builder()
                            .status(200)
                            .header("content-type", "application/dns-message")
                            .header("content-length", body.len())
                            .body(())
                            .expect("response head");
                        stream.send_response(response).await.expect("send head");
                        stream
                            .send_data(Bytes::from(body))
                            .await
                            .expect("send body");
                        stream.finish().await.expect("finish response");
                        RequestEvidence {
                            authority,
                            path,
                            request_body,
                            request_fin,
                        }
                    }));
                }

                let mut evidence = Vec::new();
                for handler in handlers {
                    evidence.push(handler.await.expect("request handler joined"));
                }
                let _ = timeout(TEST_TIMEOUT, connection_for_wait.closed()).await;
                evidence
            })
        });
        let address = address_rx
            .recv_timeout(TEST_TIMEOUT)
            .expect("server binds within timeout");
        Self {
            address,
            first_ready,
            release_first,
            handle,
        }
    }

    fn join(self) -> Vec<RequestEvidence> {
        self.handle.join().expect("server thread joined")
    }
}

fn owner(set: &FixtureSet, address: SocketAddr) -> Doh3ReuseUpstream {
    let endpoint =
        DohEndpoint::new("https://dns.example/dns-query", address).expect("valid DoH3 endpoint");
    Doh3ReuseUpstream::new(
        endpoint,
        TlsPolicy::verified(set.root_store_a()).expect("verified TLS policy"),
    )
    .expect("shared DoH3 owner")
}

async fn exchange(
    upstream: Arc<Doh3ReuseUpstream>,
    query: Vec<u8>,
) -> Result<SecureResponse, mosdns_upstream_core::secure::SecureError> {
    exchange_with_cancellation(upstream, query, TransportCancellation::new()).await
}

async fn exchange_with_cancellation(
    upstream: Arc<Doh3ReuseUpstream>,
    query: Vec<u8>,
    cancellation: TransportCancellation,
) -> Result<SecureResponse, mosdns_upstream_core::secure::SecureError> {
    let request = ExchangeRequest::new(&query).expect("valid query");
    upstream
        .exchange(
            request,
            ExchangeContext::new(Instant::now() + TEST_TIMEOUT, cancellation),
        )
        .await
}

async fn wait_for_entry_count(upstream: &Doh3ReuseUpstream, expected: usize) {
    timeout(TEST_TIMEOUT, async {
        while upstream.entry_count() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("entry count reaches expected value");
}

struct CancellationServer {
    address: SocketAddr,
    head_ready: Option<oneshot::Receiver<()>>,
    second_ready: Option<oneshot::Receiver<()>>,
    release_body: Option<oneshot::Sender<()>>,
    handle: thread::JoinHandle<()>,
}

impl CancellationServer {
    fn start(set: &FixtureSet) -> Self {
        let cert = set.good.cert.clone();
        let key = set.good.key.clone_key();
        let (address_tx, address_rx) = std::sync::mpsc::channel();
        let (head_tx, head_ready) = oneshot::channel();
        let (second_tx, second_ready) = oneshot::channel();
        let (release_body, release_rx) = oneshot::channel();
        let handle = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build server runtime");
            runtime.block_on(async move {
                let endpoint = quinn::Endpoint::server(
                    server_config(cert, key, None),
                    SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
                )
                .expect("bind QUIC server");
                let address = endpoint.local_addr().expect("server address");
                address_tx.send(address).expect("publish server address");

                let incoming = timeout(TEST_TIMEOUT, endpoint.accept())
                    .await
                    .expect("accept bounded")
                    .expect("one connection");
                let connection = incoming.await.expect("QUIC handshake");
                let connection_for_wait = connection.clone();
                let mut h3 = h3::server::builder()
                    .build::<_, Bytes>(h3_quinn::Connection::new(connection))
                    .await
                    .expect("build server h3 connection");

                let first = timeout(TEST_TIMEOUT, h3.accept())
                    .await
                    .expect("accept first request bounded")
                    .expect("accept first request")
                    .expect("first request stream remains open");
                let first_handler = tokio::spawn(async move {
                    let (request, mut stream) = first
                        .resolve_request()
                        .await
                        .expect("resolve first request");
                    let _ = request;
                    while let Ok(Some(_)) = stream.recv_data().await {}
                    let body = response_wire(0, 0x41);
                    let response = hyper::Response::builder()
                        .status(200)
                        .header("content-type", "application/dns-message")
                        .header("content-length", body.len())
                        .body(())
                        .expect("first response head");
                    stream
                        .send_response(response)
                        .await
                        .expect("send first head");
                    head_tx.send(()).expect("publish first response head");
                    let _ = release_rx.await;
                    let _ = stream.send_data(Bytes::from(body)).await;
                    let _ = stream.finish().await;
                });

                let second = timeout(TEST_TIMEOUT, h3.accept())
                    .await
                    .expect("accept second request bounded")
                    .expect("accept second request")
                    .expect("second request stream remains open");
                second_tx
                    .send(())
                    .expect("publish second request acceptance");
                let second_handler = tokio::spawn(async move {
                    let (request, mut stream) = second
                        .resolve_request()
                        .await
                        .expect("resolve second request");
                    let _ = request;
                    while let Ok(Some(_)) = stream.recv_data().await {}
                    let body = response_wire(0, 0x42);
                    let response = hyper::Response::builder()
                        .status(200)
                        .header("content-type", "application/dns-message")
                        .header("content-length", body.len())
                        .body(())
                        .expect("second response head");
                    let _ = stream.send_response(response).await;
                    let _ = stream.send_data(Bytes::from(body)).await;
                    let _ = stream.finish().await;
                });

                first_handler.await.expect("first request handler joined");
                second_handler.await.expect("second request handler joined");
                let _ = timeout(TEST_TIMEOUT, connection_for_wait.closed()).await;
            });
        });
        let address = address_rx
            .recv_timeout(TEST_TIMEOUT)
            .expect("server binds within timeout");
        Self {
            address,
            head_ready: Some(head_ready),
            second_ready: Some(second_ready),
            release_body: Some(release_body),
            handle,
        }
    }

    fn release(&mut self) {
        self.release_body
            .take()
            .expect("release sender available")
            .send(())
            .expect("release first response body");
    }

    fn join(self) {
        self.handle.join().expect("server thread joined");
    }
}

#[derive(Clone, Copy)]
enum ScriptedResponse {
    Reset,
    Malformed,
    Valid,
}

struct ScriptedServer {
    address: SocketAddr,
    handle: thread::JoinHandle<()>,
}

impl ScriptedServer {
    fn start(set: &FixtureSet, script: &'static [ScriptedResponse]) -> Self {
        let cert = set.good.cert.clone();
        let key = set.good.key.clone_key();
        let (address_tx, address_rx) = std::sync::mpsc::channel();
        let handle = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build server runtime");
            runtime.block_on(async move {
                let endpoint = quinn::Endpoint::server(
                    server_config(cert, key, None),
                    SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
                )
                .expect("bind QUIC server");
                let address = endpoint.local_addr().expect("server address");
                address_tx.send(address).expect("publish server address");

                let incoming = timeout(TEST_TIMEOUT, endpoint.accept())
                    .await
                    .expect("accept bounded")
                    .expect("one connection");
                let connection = incoming.await.expect("QUIC handshake");
                let connection_for_wait = connection.clone();
                let mut h3 = h3::server::builder()
                    .build::<_, Bytes>(h3_quinn::Connection::new(connection))
                    .await
                    .expect("build server h3 connection");
                let mut handlers = Vec::new();
                for mode in script.iter().copied() {
                    let resolver = timeout(TEST_TIMEOUT, h3.accept())
                        .await
                        .expect("accept scripted request bounded")
                        .expect("accept scripted request")
                        .expect("scripted request stream remains open");
                    handlers.push(tokio::spawn(async move {
                        let (request, mut stream) = resolver
                            .resolve_request()
                            .await
                            .expect("resolve scripted request");
                        let _ = request;
                        while let Ok(Some(_)) = stream.recv_data().await {}
                        match mode {
                            ScriptedResponse::Reset => {
                                stream.stop_stream(h3::error::Code::H3_INTERNAL_ERROR);
                            }
                            ScriptedResponse::Malformed => {
                                let body = response_wire(0, 0x51);
                                let response = hyper::Response::builder()
                                    .status(200)
                                    .header("content-type", "application/dns-message")
                                    .header("content-length", body.len() + 1)
                                    .body(())
                                    .expect("malformed response head");
                                stream
                                    .send_response(response)
                                    .await
                                    .expect("send malformed head");
                                stream
                                    .send_data(Bytes::from(body))
                                    .await
                                    .expect("send malformed body");
                                stream.finish().await.expect("finish malformed response");
                            }
                            ScriptedResponse::Valid => {
                                let body = response_wire(0, 0x52);
                                let response = hyper::Response::builder()
                                    .status(200)
                                    .header("content-type", "application/dns-message")
                                    .header("content-length", body.len())
                                    .body(())
                                    .expect("valid response head");
                                stream
                                    .send_response(response)
                                    .await
                                    .expect("send valid head");
                                stream
                                    .send_data(Bytes::from(body))
                                    .await
                                    .expect("send valid body");
                                stream.finish().await.expect("finish valid response");
                            }
                        }
                    }));
                }
                for handler in handlers {
                    handler.await.expect("scripted request handler joined");
                }
                let _ = timeout(TEST_TIMEOUT, connection_for_wait.closed()).await;
            });
        });
        let address = address_rx
            .recv_timeout(TEST_TIMEOUT)
            .expect("server binds within timeout");
        Self { address, handle }
    }

    fn join(self) {
        self.handle.join().expect("server thread joined");
    }
}

struct GoAwayServer {
    address: SocketAddr,
    shutdown_sent: Option<oneshot::Receiver<()>>,
    handle: thread::JoinHandle<()>,
}

impl GoAwayServer {
    fn start(set: &FixtureSet) -> Self {
        let cert = set.good.cert.clone();
        let key = set.good.key.clone_key();
        let (address_tx, address_rx) = std::sync::mpsc::channel();
        let (shutdown_tx, shutdown_sent) = oneshot::channel();
        let handle = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build server runtime");
            runtime.block_on(async move {
                let endpoint = quinn::Endpoint::server(
                    server_config(cert, key, None),
                    SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
                )
                .expect("bind QUIC server");
                let address = endpoint.local_addr().expect("server address");
                address_tx.send(address).expect("publish server address");
                let incoming = timeout(TEST_TIMEOUT, endpoint.accept())
                    .await
                    .expect("accept bounded")
                    .expect("one connection");
                let connection = incoming.await.expect("QUIC handshake");
                let connection_for_wait = connection.clone();
                let mut h3 = h3::server::builder()
                    .build::<_, Bytes>(h3_quinn::Connection::new(connection))
                    .await
                    .expect("build server h3 connection");
                let resolver = timeout(TEST_TIMEOUT, h3.accept())
                    .await
                    .expect("accept first request bounded")
                    .expect("accept first request")
                    .expect("first request stream remains open");
                let (request, mut stream) = resolver
                    .resolve_request()
                    .await
                    .expect("resolve first request");
                let _ = request;
                while let Ok(Some(_)) = stream.recv_data().await {}
                let body = response_wire(0, 0x61);
                let response = hyper::Response::builder()
                    .status(200)
                    .header("content-type", "application/dns-message")
                    .header("content-length", body.len())
                    .body(())
                    .expect("first response head");
                stream
                    .send_response(response)
                    .await
                    .expect("send first head");
                stream
                    .send_data(Bytes::from(body))
                    .await
                    .expect("send first body");
                stream.finish().await.expect("finish first response");
                h3.shutdown(0).await.expect("send GOAWAY");
                shutdown_tx.send(()).expect("publish GOAWAY");
                let _ = timeout(TEST_TIMEOUT, connection_for_wait.closed()).await;
            });
        });
        let address = address_rx
            .recv_timeout(TEST_TIMEOUT)
            .expect("server binds within timeout");
        Self {
            address,
            shutdown_sent: Some(shutdown_sent),
            handle,
        }
    }

    fn join(self) {
        self.handle.join().expect("server thread joined");
    }
}

#[test]
fn doh3_reuse_multiplexes_two_streams_on_one_connection() {
    block_on(async {
        let set = FixtureSet::generate();
        let server = ReuseServer::start(&set);
        let upstream = Arc::new(owner(&set, server.address));
        let first = tokio::spawn(exchange(Arc::clone(&upstream), query_wire(0x1201)));
        let second = tokio::spawn(exchange(Arc::clone(&upstream), query_wire(0x1202)));

        let first = timeout(TEST_TIMEOUT, first)
            .await
            .expect("first exchange bounded")
            .expect("first exchange task")
            .expect("first DoH3 response");
        let second = timeout(TEST_TIMEOUT, second)
            .await
            .expect("second exchange bounded")
            .expect("second exchange task")
            .expect("second DoH3 response");

        let ids = [first.request_id(), second.request_id()];
        assert!(ids.contains(&0x1201));
        assert!(ids.contains(&0x1202));
        assert_eq!(first.response_id(), first.request_id());
        assert_eq!(second.response_id(), second.request_id());
        assert_eq!(first.wire()[0..2], 0x1201u16.to_be_bytes());
        assert_eq!(second.wire()[0..2], 0x1202u16.to_be_bytes());
        assert_eq!(upstream.entry_count(), 1);
        assert_eq!(
            upstream.in_flight_exchanges(),
            1,
            "the active entry keeps its owner liveness registration"
        );

        upstream.close().await;
        assert_eq!(upstream.in_flight_exchanges(), 0);
        let evidence = server.join();
        assert_eq!(evidence.len(), 2);
        assert_eq!(evidence[0].authority, "dns.example");
        assert_eq!(evidence[1].authority, "dns.example");
        assert_eq!(evidence[0].path, evidence[1].path);
        assert!(evidence.iter().all(|item| item.request_body.is_empty()));
        assert!(evidence.iter().all(|item| item.request_fin));
    });
}

#[test]
fn doh3_peer_advertised_stream_limit_serializes_without_replacement() {
    block_on(async {
        let set = FixtureSet::generate();
        let streams = 2;
        let server = ReuseServer::start_with_limit(&set, streams, Some(1));
        let upstream = Arc::new(owner(&set, server.address));
        let first = tokio::spawn(exchange(Arc::clone(&upstream), query_wire(0x1251)));
        let second = tokio::spawn(exchange(Arc::clone(&upstream), query_wire(0x1252)));

        let first = timeout(TEST_TIMEOUT, first)
            .await
            .expect("first exchange bounded")
            .expect("first exchange task")
            .expect("first DoH3 response");
        let second = timeout(TEST_TIMEOUT, second)
            .await
            .expect("second exchange bounded")
            .expect("second exchange task")
            .expect("second DoH3 response");

        let ids = [first.request_id(), second.request_id()];
        assert!(ids.contains(&0x1251));
        assert!(ids.contains(&0x1252));
        assert_eq!(first.response_id(), first.request_id());
        assert_eq!(second.response_id(), second.request_id());
        assert_eq!(upstream.entry_count(), 1);
        assert_eq!(upstream.in_flight_exchanges(), 1);

        upstream.close().await;
        let evidence = server.join();
        assert_eq!(evidence.len(), streams);
        assert_eq!(
            evidence
                .iter()
                .map(|item| item.authority.as_str())
                .collect::<BTreeSet<_>>()
                .len(),
            1,
            "both requests use the configured authority"
        );
    });
}

#[test]
fn doh3_peer_stream_limit_honors_pending_open_cancellation_without_replacement() {
    block_on(async {
        let set = FixtureSet::generate();
        let server = ReuseServer::start_with_hold(&set);
        let upstream = Arc::new(owner(&set, server.address));
        let first = tokio::spawn(exchange(Arc::clone(&upstream), query_wire(0x1261)));
        timeout(TEST_TIMEOUT, server.first_ready.notified())
            .await
            .expect("first H3 stream reaches the held server");

        let second_cancellation = TransportCancellation::new();
        let second = tokio::spawn(exchange_with_cancellation(
            Arc::clone(&upstream),
            query_wire(0x1262),
            second_cancellation.clone(),
        ));
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        second_cancellation.cancel();
        let second = timeout(TEST_TIMEOUT, second)
            .await
            .expect("pending peer-credit exchange is bounded")
            .expect("second exchange task joins");
        assert!(matches!(
            second,
            Err(SecureError::Transport(UpstreamError::Cancelled(
                SideEffectState::MaybeSent,
            )))
        ));

        server.release_first.notify_one();
        let first = timeout(TEST_TIMEOUT, first)
            .await
            .expect("held first exchange is released")
            .expect("first exchange task joins")
            .expect("first response succeeds");
        assert_eq!(first.request_id(), 0x1261);
        assert_eq!(upstream.entry_count(), 1);

        upstream.close().await;
        let evidence = server.join();
        assert_eq!(
            evidence.len(),
            1,
            "the canceled pending stream never reaches the server"
        );
    });
}

#[test]
fn doh3_bounded_concurrent_stress_keeps_one_generation() {
    block_on(async {
        let set = FixtureSet::generate();
        let streams = 8;
        let server = ReuseServer::start_with_limit(&set, streams, None);
        let upstream = Arc::new(owner(&set, server.address));
        let mut tasks = Vec::new();
        for index in 0..streams {
            let upstream = Arc::clone(&upstream);
            tasks.push(tokio::spawn(async move {
                let id = 0x1300 + u16::try_from(index).expect("stress index fits u16");
                let marker = u8::try_from(index).expect("stress marker fits u8");
                (
                    marker,
                    exchange(upstream, query_wire_with_marker(id, marker)).await,
                )
            }));
        }

        let mut responses = Vec::new();
        for task in tasks {
            let (marker, response) = timeout(TEST_TIMEOUT, task)
                .await
                .expect("stress exchange bounded")
                .expect("stress exchange task");
            responses.push((marker, response.expect("stress DoH3 response")));
        }

        let mut ids = BTreeSet::new();
        let mut markers = BTreeSet::new();
        for (expected_marker, response) in responses {
            ids.insert(response.response_id());
            assert_eq!(
                response.wire().last().copied(),
                Some(expected_marker),
                "each caller receives the marker encoded in its own query"
            );
            markers.insert(expected_marker);
        }
        let expected_ids = (0..streams)
            .map(|index| 0x1300 + u16::try_from(index).expect("stress index fits u16"))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            ids, expected_ids,
            "stress responses keep independent DNS IDs"
        );
        assert_eq!(markers.len(), streams, "stress responses never cross-talk");
        assert_eq!(upstream.entry_count(), 1);
        assert_eq!(upstream.in_flight_exchanges(), 1);

        upstream.close().await;
        let evidence = server.join();
        assert_eq!(evidence.len(), streams);
        assert_eq!(
            evidence
                .iter()
                .map(|item| item.authority.as_str())
                .collect::<BTreeSet<_>>()
                .len(),
            1,
            "all stress streams use one authority"
        );
    });
}

#[test]
fn doh3_canceled_response_body_keeps_connection_reusable() {
    block_on(async {
        let set = FixtureSet::generate();
        let mut server = CancellationServer::start(&set);
        let upstream = Arc::new(owner(&set, server.address));
        let cancellation = TransportCancellation::new();
        let first = tokio::spawn(exchange_with_cancellation(
            Arc::clone(&upstream),
            query_wire(0x2201),
            cancellation.clone(),
        ));

        timeout(
            TEST_TIMEOUT,
            server
                .head_ready
                .take()
                .expect("response-head receiver available"),
        )
        .await
        .expect("response head bounded")
        .expect("response head signal");
        let second = tokio::spawn(exchange(Arc::clone(&upstream), query_wire(0x2202)));
        timeout(
            TEST_TIMEOUT,
            server
                .second_ready
                .take()
                .expect("second-request receiver available"),
        )
        .await
        .expect("second request acceptance bounded")
        .expect("second request acceptance signal");
        cancellation.cancel();
        let first = timeout(TEST_TIMEOUT, first)
            .await
            .expect("canceled exchange bounded")
            .expect("canceled exchange task");
        assert!(matches!(
            first,
            Err(SecureError::Transport(UpstreamError::Cancelled(
                SideEffectState::Sent,
            )))
        ));
        assert_eq!(upstream.entry_count(), 1);

        server.release();
        let second = timeout(TEST_TIMEOUT, second)
            .await
            .expect("reused exchange bounded")
            .expect("reused exchange task")
            .expect("reused response after cancellation");
        assert_eq!(second.request_id(), 0x2202);
        assert_eq!(second.response_id(), 0x2202);
        assert_eq!(upstream.entry_count(), 1);

        upstream.close().await;
        server.join();
    });
}

#[test]
fn doh3_stream_failures_keep_generation_active_for_next_request() {
    block_on(async {
        let set = FixtureSet::generate();
        let server = ScriptedServer::start(
            &set,
            &[
                ScriptedResponse::Reset,
                ScriptedResponse::Malformed,
                ScriptedResponse::Valid,
            ],
        );
        let upstream = Arc::new(owner(&set, server.address));

        let reset = exchange(Arc::clone(&upstream), query_wire(0x2301))
            .await
            .expect_err("stream reset must fail");
        assert!(matches!(
            reset,
            SecureError::DohProtocol(DohProtocolError::PeerStreamTerminated { .. })
        ));
        assert_eq!(upstream.entry_count(), 1);

        let malformed = exchange(Arc::clone(&upstream), query_wire(0x2302))
            .await
            .expect_err("declared body mismatch must fail");
        assert_eq!(
            malformed,
            SecureError::DohProtocol(DohProtocolError::IncompleteBody)
        );
        assert_eq!(upstream.entry_count(), 1);

        let valid = exchange(Arc::clone(&upstream), query_wire(0x2303))
            .await
            .expect("same connection remains reusable after stream failures");
        assert_eq!(valid.request_id(), 0x2303);
        assert_eq!(valid.response_id(), 0x2303);
        assert_eq!(upstream.entry_count(), 1);

        upstream.close().await;
        server.join();
    });
}

#[test]
fn doh3_owner_close_waits_for_held_connection_handle_before_removal() {
    block_on(async {
        let set = FixtureSet::generate();
        let server = ScriptedServer::start(&set, &[ScriptedResponse::Valid]);
        let upstream = Arc::new(owner(&set, server.address));
        let held = upstream
            .hold_connection_handle_for_test()
            .await
            .expect("acquire real caller-owned DoH3 handle");

        let response = exchange(Arc::clone(&upstream), query_wire(0x2351))
            .await
            .expect("request succeeds while the extra handle is held");
        assert_eq!(response.request_id(), 0x2351);
        assert_eq!(response.response_id(), 0x2351);
        assert_eq!(upstream.entry_count(), 1);
        assert_eq!(
            upstream.in_flight_exchanges(),
            2,
            "the held caller handle and entry liveness registration remain"
        );

        let close_upstream = Arc::clone(&upstream);
        let mut close = tokio::spawn(async move { close_upstream.close().await });
        assert!(
            timeout(Duration::from_millis(100), &mut close)
                .await
                .is_err(),
            "owner close must wait for the caller-owned H3 handle"
        );
        assert_eq!(upstream.entry_count(), 1);
        assert_eq!(
            upstream.in_flight_exchanges(),
            2,
            "close cannot drain while the caller-owned handle remains held"
        );

        drop(held);
        assert_eq!(
            timeout(TEST_TIMEOUT, &mut close)
                .await
                .expect("owner close completes after handle release")
                .expect("owner close task joined"),
            mosdns_upstream_core::CloseResult::Closed
        );
        assert_eq!(upstream.entry_count(), 0);
        assert_eq!(upstream.in_flight_exchanges(), 0);
        server.join();
    });
}

#[test]
fn doh3_goaway_send_error_deactivates_exact_generation() {
    block_on(async {
        let set = FixtureSet::generate();
        let mut server = GoAwayServer::start(&set);
        let upstream = Arc::new(owner(&set, server.address));

        let first = exchange(Arc::clone(&upstream), query_wire(0x2401))
            .await
            .expect("first response before GOAWAY");
        assert_eq!(first.request_id(), 0x2401);
        timeout(
            TEST_TIMEOUT,
            server
                .shutdown_sent
                .take()
                .expect("GOAWAY receiver available"),
        )
        .await
        .expect("GOAWAY bounded")
        .expect("GOAWAY signal");

        let second = exchange(Arc::clone(&upstream), query_wire(0x2402))
            .await
            .expect_err("GOAWAY must reject a new request");
        assert_eq!(
            second,
            SecureError::Transport(UpstreamError::Send(SideEffectState::MaybeSent))
        );
        wait_for_entry_count(&upstream, 0).await;

        server.join();
    });
}

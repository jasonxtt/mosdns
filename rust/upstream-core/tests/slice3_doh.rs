//! Slice3 contract tests for `DoH` HTTP/2 dispatch and scoped task ownership.
//!
//! The server is an in-process TLS+h2 peer. It records the request authority
//! and path, returns one valid DNS body, and then closes. No public DNS service
//! or shared connection pool is involved.

mod fixtures;

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fixtures::FixtureSet;
use h2::{Reason, server};
use hyper::Response;
use hyper::body::Bytes;
use mosdns_upstream_core::secure::{
    DohEndpoint, DohProtocolError, DohUpstream, SecureError, SecureHttpVersion, SecureTransport,
    TlsPolicy,
};
use mosdns_upstream_core::{
    CloseResult, ExchangeContext, ExchangeRequest, SideEffectState, TransportCancellation,
};
use rustls::ServerConfig;
use tokio::net::TcpListener as AsyncTcpListener;
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread test runtime")
        .block_on(future)
}

fn query_wire(id: u16) -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&[0x01, 0x00]);
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&[0u8; 6]);
    wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
    wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0x00]);
    wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
    wire
}

fn response_wire(id: u16) -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&[0x81, 0x80]);
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&[0u8; 4]);
    wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
    wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0x00]);
    wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
    wire.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
    wire.extend_from_slice(&60u32.to_be_bytes());
    wire.extend_from_slice(&[0x00, 0x04, 192, 0, 2, 53]);
    wire
}

fn server_config(set: &FixtureSet) -> Arc<ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("safe TLS versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![set.good.cert.clone(), set.root_chain()],
            set.good.key.clone_key(),
        )
        .expect("synthetic server certificate");
    config.alpn_protocols = vec![b"h2".to_vec()];
    Arc::new(config)
}

fn bind_listener() -> (TcpListener, SocketAddr) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback");
    let address = listener.local_addr().expect("listener address");
    (listener, address)
}

fn start_h2_server(
    set: &FixtureSet,
    expected_path: String,
    response: Vec<u8>,
) -> (SocketAddr, std::thread::JoinHandle<(String, String)>) {
    let (listener, address) = bind_listener();
    listener
        .set_nonblocking(true)
        .expect("listener nonblocking");
    let config = server_config(set);
    let handle = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build server runtime");
        runtime.block_on(async move {
            let listener = AsyncTcpListener::from_std(listener).expect("adopt listener");
            let (stream, _) = timeout(TEST_TIMEOUT, listener.accept())
                .await
                .expect("accept timeout")
                .expect("accept connection");
            let tls = TlsAcceptor::from(config)
                .accept(stream)
                .await
                .expect("TLS accept");
            let mut connection = server::handshake(tls).await.expect("h2 handshake");
            let Some(Ok((request, mut respond))) = connection.accept().await else {
                panic!("client did not send an h2 request");
            };
            let authority = request
                .headers()
                .get("host")
                .expect("h2 authority")
                .to_str()
                .expect("valid h2 authority")
                .to_owned();
            let path = request
                .uri()
                .path_and_query()
                .expect("h2 path")
                .as_str()
                .to_owned();
            assert_eq!(path, expected_path);
            let response_head = Response::builder()
                .status(200)
                .header("content-type", "application/dns-message")
                .body(())
                .expect("response head");
            let mut send = respond
                .send_response(response_head, false)
                .expect("send h2 response head");
            send.send_data(Bytes::from(response), true)
                .expect("send h2 response body");
            // Continue driving the server connection until the one-shot client
            // drops it; send_data only queues frames until the connection is
            // polled again.
            let _ = timeout(TEST_TIMEOUT, connection.accept()).await;
            (authority, path)
        })
    });
    (address, handle)
}

#[derive(Clone, Copy)]
enum H2Failure {
    RefusedStream,
    GoAway,
    Eof,
}

fn start_h2_failure_server(
    set: &FixtureSet,
    failure: H2Failure,
) -> (SocketAddr, std::thread::JoinHandle<usize>) {
    let (listener, address) = bind_listener();
    listener
        .set_nonblocking(true)
        .expect("listener nonblocking");
    let config = server_config(set);
    let handle = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build server runtime");
        runtime.block_on(async move {
            let listener = AsyncTcpListener::from_std(listener).expect("adopt listener");
            let (stream, _) = timeout(TEST_TIMEOUT, listener.accept())
                .await
                .expect("accept timeout")
                .expect("accept connection");
            let tls = TlsAcceptor::from(config)
                .accept(stream)
                .await
                .expect("TLS accept");
            let mut connection = server::handshake(tls).await.expect("h2 handshake");
            let Some(Ok((_request, mut respond))) = connection.accept().await else {
                panic!("client did not send an h2 request");
            };
            match failure {
                H2Failure::RefusedStream => {
                    respond.send_reset(Reason::REFUSED_STREAM);
                    let _ = timeout(TEST_TIMEOUT, connection.accept()).await;
                }
                H2Failure::GoAway => {
                    connection.abrupt_shutdown(Reason::NO_ERROR);
                    let _ = timeout(
                        TEST_TIMEOUT,
                        std::future::poll_fn(|cx| connection.poll_closed(cx)),
                    )
                    .await;
                }
                H2Failure::Eof => {
                    // Dropping the TLS+h2 connection immediately models a
                    // peer EOF before a response head.
                }
            }
            // Keep the listener alive briefly to make a forbidden retry or
            // HTTP/1.1 fallback observable as a second accepted connection.
            let second = timeout(Duration::from_millis(100), listener.accept()).await;
            if second.is_ok() { 2 } else { 1 }
        })
    });
    (address, handle)
}

fn start_h2_hanging_server(
    set: &FixtureSet,
    request_seen: tokio::sync::oneshot::Sender<()>,
) -> (SocketAddr, std::thread::JoinHandle<()>) {
    let (listener, address) = bind_listener();
    listener
        .set_nonblocking(true)
        .expect("listener nonblocking");
    let config = server_config(set);
    let handle = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build server runtime");
        runtime.block_on(async move {
            let listener = AsyncTcpListener::from_std(listener).expect("adopt listener");
            let (stream, _) = timeout(TEST_TIMEOUT, listener.accept())
                .await
                .expect("accept timeout")
                .expect("accept connection");
            let tls = TlsAcceptor::from(config)
                .accept(stream)
                .await
                .expect("TLS accept");
            let mut connection = server::handshake(tls).await.expect("h2 handshake");
            let Some(Ok((_request, _respond))) = connection.accept().await else {
                panic!("client did not send an h2 request");
            };
            request_seen.send(()).expect("request observer alive");
            let _ = timeout(TEST_TIMEOUT, connection.accept()).await;
        });
    });
    (address, handle)
}

fn verified_upstream(set: &FixtureSet, address: SocketAddr) -> DohUpstream {
    DohUpstream::new(
        DohEndpoint::new("https://dns.example/dns-query?foo=bar", address).expect("endpoint"),
        TlsPolicy::verified(set.root_store_a()).expect("verified roots"),
    )
    .expect("upstream")
}

#[test]
fn negotiated_h2_serves_one_doh_get_with_service_authority_and_path() {
    block_on(async {
        let set = FixtureSet::generate();
        let id = 0x8301;
        let query = query_wire(id);
        let expected_target = DohEndpoint::new(
            "https://dns.example/dns-query?foo=bar",
            SocketAddr::from((Ipv4Addr::LOCALHOST, 1)),
        )
        .expect("endpoint")
        .get_request_target(ExchangeRequest::new(&query).expect("query"))
        .expect("request target");
        let (address, server) = start_h2_server(&set, expected_target.clone(), response_wire(0));
        let endpoint =
            DohEndpoint::new("https://dns.example/dns-query?foo=bar", address).expect("endpoint");
        let upstream = DohUpstream::new(
            endpoint,
            TlsPolicy::verified(set.root_store_a()).expect("verified roots"),
        )
        .expect("upstream");

        let response = upstream
            .exchange(
                ExchangeRequest::new(&query).expect("query"),
                ExchangeContext::new(Instant::now() + TEST_TIMEOUT, TransportCancellation::new()),
            )
            .await
            .expect("h2 DoH exchange");
        assert_eq!(response.transport(), SecureTransport::Doh);
        assert_eq!(response.http_version(), Some(SecureHttpVersion::Http2));
        assert_eq!(response.request_id(), id);
        assert_eq!(&response.wire()[0..2], &id.to_be_bytes());
        assert_eq!(upstream.in_flight_exchanges(), 0);
        let (authority, path) = server.join().expect("server joined");
        assert_eq!(authority, "dns.example");
        assert_eq!(path, expected_target);
    });
}

#[test]
fn independent_h2_owners_use_independent_fresh_connections() {
    block_on(async {
        let first_set = FixtureSet::generate();
        let second_set = FixtureSet::generate();
        let first_target = DohEndpoint::new(
            "https://dns.example/dns-query?foo=bar",
            SocketAddr::from((Ipv4Addr::LOCALHOST, 1)),
        )
        .expect("endpoint")
        .get_request_target(ExchangeRequest::new(&query_wire(0x8302)).expect("query"))
        .expect("target");
        let second_target = DohEndpoint::new(
            "https://dns.example/dns-query?foo=bar",
            SocketAddr::from((Ipv4Addr::LOCALHOST, 1)),
        )
        .expect("endpoint")
        .get_request_target(ExchangeRequest::new(&query_wire(0x8303)).expect("query"))
        .expect("target");
        let (first_address, first_server) =
            start_h2_server(&first_set, first_target, response_wire(0));
        let (second_address, second_server) =
            start_h2_server(&second_set, second_target, response_wire(0));
        let first = verified_upstream(&first_set, first_address);
        let second = verified_upstream(&second_set, second_address);
        let first_query = query_wire(0x8302);
        let second_query = query_wire(0x8303);

        let (first_response, second_response) = tokio::join!(
            first.exchange(
                ExchangeRequest::new(&first_query).expect("query"),
                ExchangeContext::new(Instant::now() + TEST_TIMEOUT, TransportCancellation::new(),),
            ),
            second.exchange(
                ExchangeRequest::new(&second_query).expect("query"),
                ExchangeContext::new(Instant::now() + TEST_TIMEOUT, TransportCancellation::new(),),
            ),
        );
        assert_eq!(
            first_response.expect("first h2 response").request_id(),
            0x8302
        );
        assert_eq!(
            second_response.expect("second h2 response").request_id(),
            0x8303
        );
        assert_eq!(first.in_flight_exchanges(), 0);
        assert_eq!(second.in_flight_exchanges(), 0);
        first_server.join().expect("first server joined");
        second_server.join().expect("second server joined");
    });
}

#[test]
fn h2_reset_goaway_and_eof_are_terminal_maybe_sent_failures() {
    for failure in [H2Failure::RefusedStream, H2Failure::GoAway, H2Failure::Eof] {
        block_on(async move {
            let set = FixtureSet::generate();
            let (address, server) = start_h2_failure_server(&set, failure);
            let upstream = verified_upstream(&set, address);
            let query = query_wire(0x8304);
            let error = upstream
                .exchange(
                    ExchangeRequest::new(&query).expect("query"),
                    ExchangeContext::new(
                        Instant::now() + TEST_TIMEOUT,
                        TransportCancellation::new(),
                    ),
                )
                .await
                .expect_err("h2 peer failure is terminal");
            assert_eq!(error.side_effect(), SideEffectState::MaybeSent);
            assert!(
                matches!(
                    error,
                    SecureError::Transport(_)
                        | SecureError::DohProtocol(DohProtocolError::ResponseHeadNotReceived)
                ),
                "got {error:?}"
            );
            assert_eq!(upstream.in_flight_exchanges(), 0);
            assert_eq!(
                server.join().expect("failure server joined"),
                1,
                "one h2 failure must not trigger a retry or protocol fallback"
            );
        });
    }
}

#[test]
fn aborting_after_h2_handoff_drains_children_before_close_returns() {
    block_on(async {
        let set = FixtureSet::generate();
        let (request_seen, request_received) = tokio::sync::oneshot::channel();
        let (address, server) = start_h2_hanging_server(&set, request_seen);
        let upstream = Arc::new(verified_upstream(&set, address));
        let task_upstream = Arc::clone(&upstream);
        let query = query_wire(0x8305);
        let task = tokio::spawn(async move {
            task_upstream
                .exchange(
                    ExchangeRequest::new(&query).expect("query"),
                    ExchangeContext::new(
                        Instant::now() + Duration::from_secs(30),
                        TransportCancellation::new(),
                    ),
                )
                .await
        });
        timeout(TEST_TIMEOUT, request_received)
            .await
            .expect("h2 request handoff observed")
            .expect("request observer alive");
        task.abort();
        assert_eq!(
            timeout(TEST_TIMEOUT, upstream.close())
                .await
                .expect("close timeout"),
            CloseResult::Closed
        );
        assert_eq!(upstream.in_flight_exchanges(), 0);
        server.join().expect("hanging server joined");
    });
}

#[test]
fn caller_cancellation_after_h2_handoff_is_maybe_sent_and_drains_children() {
    block_on(async {
        let set = FixtureSet::generate();
        let (request_seen, request_received) = tokio::sync::oneshot::channel();
        let (address, server) = start_h2_hanging_server(&set, request_seen);
        let upstream = Arc::new(verified_upstream(&set, address));
        let caller_cancellation = TransportCancellation::new();
        let task_upstream = Arc::clone(&upstream);
        let query = query_wire(0x8306);
        let task_cancellation = caller_cancellation.clone();
        let task = tokio::spawn(async move {
            task_upstream
                .exchange(
                    ExchangeRequest::new(&query).expect("query"),
                    ExchangeContext::new(
                        Instant::now() + Duration::from_secs(30),
                        task_cancellation,
                    ),
                )
                .await
        });
        timeout(TEST_TIMEOUT, request_received)
            .await
            .expect("h2 request handoff observed")
            .expect("request observer alive");
        caller_cancellation.cancel();
        let error = timeout(TEST_TIMEOUT, task)
            .await
            .expect("exchange timeout")
            .expect("exchange task joined")
            .expect_err("caller cancellation must terminate the exchange");
        assert_eq!(error.side_effect(), SideEffectState::MaybeSent);
        assert_eq!(upstream.in_flight_exchanges(), 0);
        assert_eq!(upstream.close().await, CloseResult::Closed);
        server.join().expect("hanging server joined");
    });
}

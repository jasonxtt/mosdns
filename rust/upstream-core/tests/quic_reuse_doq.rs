//! Slice 1 real `DoQ` reuse tests.
//!
//! These fixtures deliberately exercise the shared connection boundary rather
//! than the existing one-shot `DoqUpstream`: one QUIC accept must serve several
//! independent bidirectional streams, and a connection failure must retire only
//! that generation so a later exchange can establish one replacement.

mod fixtures;

use std::collections::BTreeSet;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use fixtures::FixtureSet;
use mosdns_upstream_core::quic::DOQ_ALPN;
use mosdns_upstream_core::quic_reuse::DoqReuseUpstream;
use mosdns_upstream_core::secure::{SecureError, SecureResponse, TlsPolicy};
use mosdns_upstream_core::{
    ExchangeContext, ExchangeRequest, ServerIdentity, SideEffectState, TransportCancellation,
    UpstreamError,
};
use quinn::crypto::rustls::QuicServerConfig;
use rustls::ServerConfig;
use tokio::sync::oneshot;
use tokio::time::timeout;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);
const EXCHANGE_DEADLINE: Duration = Duration::from_secs(10);
const MAX_DOQ_MESSAGE: usize = 65_537;

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
    wire.extend_from_slice(&[0; 6]);
    wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
    wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0x00]);
    wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
    wire
}

fn response_wire(id: u16, marker: u8) -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend_from_slice(&id.to_be_bytes());
    wire.extend_from_slice(&[0x81, 0x80]);
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&1u16.to_be_bytes());
    wire.extend_from_slice(&[0; 4]);
    wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
    wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0x00]);
    wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
    wire.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
    wire.extend_from_slice(&60u32.to_be_bytes());
    wire.extend_from_slice(&[0x00, 0x04, 192, 0, 2, marker]);
    wire
}

fn frame(body: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(body.len() + 2);
    let length = u16::try_from(body.len()).expect("test response fits the DoQ frame");
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(body);
    bytes
}

fn server_config(set: &FixtureSet) -> quinn::ServerConfig {
    let mut tls =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("TLS 1.3 is supported")
            .with_no_client_auth()
            .with_single_cert(vec![set.good.cert.clone()], set.good.key.clone_key())
            .expect("synthetic certificate and key match");
    tls.alpn_protocols = vec![DOQ_ALPN.to_vec()];
    let quic = QuicServerConfig::try_from(tls).expect("TLS config converts to QUIC");
    quinn::ServerConfig::with_crypto(Arc::new(quic))
}

struct DoqServer {
    address: SocketAddr,
    accepts: Arc<AtomicUsize>,
    ready: Arc<tokio::sync::Notify>,
    evidence: oneshot::Receiver<Vec<(u64, Vec<u8>)>>,
    handle: std::thread::JoinHandle<()>,
}

#[derive(Clone, Copy)]
enum FirstStreamMode {
    Normal,
    Hold,
    Reset,
    Malformed,
    Close,
}

impl DoqServer {
    fn start(set: &FixtureSet, streams: usize, mode: FirstStreamMode) -> Self {
        let config = server_config(set);
        let (address_tx, address_rx) = std::sync::mpsc::channel();
        let (evidence_tx, evidence_rx) = oneshot::channel();
        let accepts = Arc::new(AtomicUsize::new(0));
        let accepts_thread = Arc::clone(&accepts);
        let ready = Arc::new(tokio::sync::Notify::new());
        let ready_thread = Arc::clone(&ready);
        let handle = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build server runtime");
            runtime.block_on(async move {
                let endpoint =
                    quinn::Endpoint::server(config, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                        .expect("bind loopback QUIC endpoint");
                address_tx
                    .send(endpoint.local_addr().expect("read loopback address"))
                    .expect("send server address");

                let mut evidence = Vec::new();
                let connection_rounds = if matches!(mode, FirstStreamMode::Close) {
                    2
                } else {
                    1
                };
                for connection_index in 0..connection_rounds {
                    let Some(incoming) = endpoint.accept().await else {
                        return;
                    };
                    accepts_thread.fetch_add(1, Ordering::SeqCst);
                    let Ok(connection) = incoming.await else {
                        continue;
                    };

                    let mut tasks = Vec::new();
                    for index in 0..streams {
                        let Ok((mut send, mut recv)) = connection.accept_bi().await else {
                            return;
                        };
                        let stream_id = send.id().index();
                        let connection_for_task = connection.clone();
                        let ready_for_task = Arc::clone(&ready_thread);
                        let task = tokio::spawn(async move {
                            let request = recv.read_to_end(MAX_DOQ_MESSAGE).await.ok()?;
                            if connection_index == 0 && index == 0 {
                                ready_for_task.notify_one();
                            }
                            if connection_index == 0 && index == 0 {
                                match mode {
                                    FirstStreamMode::Close => {
                                        connection_for_task
                                            .close(quinn::VarInt::from_u32(0x2), b"test failure");
                                        return Some((stream_id, request));
                                    }
                                    FirstStreamMode::Hold => {
                                        let _ = send.stopped().await;
                                        return Some((stream_id, request));
                                    }
                                    FirstStreamMode::Reset => {
                                        send.reset(quinn::VarInt::from_u32(0x2)).ok()?;
                                        return Some((stream_id, request));
                                    }
                                    FirstStreamMode::Malformed => {
                                        send.write_all(&[0, 0]).await.ok()?;
                                        send.finish().ok()?;
                                        return Some((stream_id, request));
                                    }
                                    FirstStreamMode::Normal => {}
                                }
                            }
                            let marker = u8::try_from(index).expect("test stream index fits u8");
                            let response = frame(&response_wire(0, marker));
                            send.write_all(&response).await.ok()?;
                            send.finish().ok()?;
                            Some((stream_id, request))
                        });
                        tasks.push(task);
                    }
                    for task in tasks {
                        if let Ok(Some(item)) = task.await {
                            evidence.push(item);
                        }
                    }
                    let _ = timeout(TEST_TIMEOUT, connection.closed()).await;
                }
                endpoint.wait_idle().await;
                let _ = evidence_tx.send(evidence);
            });
        });
        let address = address_rx
            .recv_timeout(TEST_TIMEOUT)
            .expect("server binds within the test bound");
        Self {
            address,
            accepts,
            ready,
            evidence: evidence_rx,
            handle,
        }
    }

    fn join(self) -> (usize, Vec<(u64, Vec<u8>)>) {
        self.handle.join().expect("server thread joins");
        let accepts = self.accepts.load(Ordering::SeqCst);
        let evidence = self
            .evidence
            .blocking_recv()
            .expect("server returns stream evidence");
        (accepts, evidence)
    }
}

fn upstream_with_tls(address: SocketAddr, tls: TlsPolicy) -> DoqReuseUpstream {
    let endpoint = mosdns_upstream_core::quic::DoqEndpoint::new(
        address,
        ServerIdentity::new("dns.example").expect("valid identity"),
    )
    .expect("valid DoQ endpoint");
    DoqReuseUpstream::new(endpoint, tls).expect("shared DoQ owner constructs")
}

fn upstream(set: &FixtureSet, address: SocketAddr) -> DoqReuseUpstream {
    upstream_with_tls(
        address,
        TlsPolicy::verified(set.root_store_a()).expect("verified TLS policy"),
    )
}

#[test]
fn doq_reuse_multiplexes_concurrent_queries_on_one_connection() {
    let set = FixtureSet::generate();
    let streams = 4;
    let server = DoqServer::start(&set, streams, FirstStreamMode::Normal);
    let upstream = Arc::new(upstream(&set, server.address));

    let results = block_on(async {
        let mut tasks = Vec::new();
        for index in 0..streams {
            let upstream = Arc::clone(&upstream);
            tasks.push(tokio::spawn(async move {
                let index = u16::try_from(index).expect("test stream index fits u16");
                let query = query_wire(0x1000 + index);
                let request = ExchangeRequest::new(&query).expect("valid query");
                let context = ExchangeContext::new(
                    Instant::now() + EXCHANGE_DEADLINE,
                    TransportCancellation::new(),
                );
                upstream.exchange(request, context).await
            }));
        }
        let mut results = Vec::new();
        for task in tasks {
            results.push(task.await.expect("query task joins"));
        }
        results
    });

    for (index, result) in results.into_iter().enumerate() {
        let response: SecureResponse = result.expect("shared DoQ exchange succeeds");
        let index = u16::try_from(index).expect("test stream index fits u16");
        assert_eq!(response.wire()[0..2], (0x1000 + index).to_be_bytes());
        assert_eq!(
            response.wire()[response.wire().len() - 1],
            u8::try_from(index).expect("test stream index fits u8")
        );
    }
    block_on(upstream.close());
    let (accepts, evidence) = server.join();
    assert_eq!(accepts, 1, "all streams share one QUIC accept");
    assert_eq!(evidence.len(), streams, "each query reached one stream FIN");
    assert_eq!(
        evidence
            .iter()
            .map(|(id, _)| *id)
            .collect::<BTreeSet<_>>()
            .len(),
        streams,
        "each query received an independent bidirectional stream ID"
    );
    for (_, request) in evidence {
        assert_eq!(&request[2..4], &[0, 0], "wire DNS IDs are zeroed");
    }
}

#[test]
fn doq_stream_cancellation_does_not_kill_healthy_connection() {
    let set = FixtureSet::generate();
    let server = DoqServer::start(&set, 2, FirstStreamMode::Hold);
    let upstream = Arc::new(upstream(&set, server.address));
    let cancellation = TransportCancellation::new();
    let first_upstream = Arc::clone(&upstream);
    let second_upstream = Arc::clone(&upstream);
    let ready = Arc::clone(&server.ready);

    let (first, second) = block_on(async {
        let first_query = query_wire(0x2001);
        let first_context =
            ExchangeContext::new(Instant::now() + EXCHANGE_DEADLINE, cancellation.clone());
        let first_task = tokio::spawn(async move {
            let first_request = ExchangeRequest::new(&first_query).expect("valid query");
            first_upstream.exchange(first_request, first_context).await
        });
        timeout(TEST_TIMEOUT, ready.notified())
            .await
            .expect("the first request reaches the server");
        cancellation.cancel();
        let second_query = query_wire(0x2002);
        let second_request = ExchangeRequest::new(&second_query).expect("valid query");
        let second_context = ExchangeContext::new(
            Instant::now() + EXCHANGE_DEADLINE,
            TransportCancellation::new(),
        );
        let second = second_upstream.exchange(second_request, second_context);
        let first = first_task.await.expect("first task joins");
        (first, second.await)
    });

    assert!(matches!(
        first,
        Err(SecureError::Transport(UpstreamError::Cancelled(
            SideEffectState::Sent,
        )))
    ));
    let response = second.expect("the second stream remains healthy");
    assert_eq!(&response.wire()[0..2], &[0x20, 0x02]);
    block_on(upstream.close());
    let (accepts, _) = server.join();
    assert_eq!(
        accepts, 1,
        "stream cancellation does not create a replacement"
    );
}

fn assert_stream_local_failure_is_reusable(reset: bool) {
    let set = FixtureSet::generate();
    let mode = if reset {
        FirstStreamMode::Reset
    } else {
        FirstStreamMode::Malformed
    };
    let server = DoqServer::start(&set, 2, mode);
    let upstream = Arc::new(upstream(&set, server.address));

    let results = block_on(async {
        let mut tasks = Vec::new();
        for index in 0..2 {
            let upstream = Arc::clone(&upstream);
            tasks.push(tokio::spawn(async move {
                let query = query_wire(0x2800 + index);
                let request = ExchangeRequest::new(&query).expect("valid query");
                let context = ExchangeContext::new(
                    Instant::now() + EXCHANGE_DEADLINE,
                    TransportCancellation::new(),
                );
                upstream.exchange(request, context).await
            }));
        }
        let mut results = Vec::new();
        for task in tasks {
            results.push(task.await.expect("stream task joins"));
        }
        upstream.close().await;
        results
    });

    let mut successes = 0;
    let mut failures = 0;
    for result in results {
        match result {
            Ok(_) => successes += 1,
            Err(error) => {
                failures += 1;
                if reset {
                    assert_eq!(error, SecureError::DoqProtocolMissingResponseFin);
                } else {
                    assert_eq!(
                        error,
                        SecureError::Transport(UpstreamError::MalformedResponse)
                    );
                }
            }
        }
    }
    assert_eq!(successes, 1, "the healthy stream still completes");
    assert_eq!(failures, 1, "only the faulty stream fails");
    let (accepts, _) = server.join();
    assert_eq!(
        accepts, 1,
        "a stream-local failure does not replace the connection"
    );
}

#[test]
fn doq_stream_reset_is_stream_local_on_a_healthy_connection() {
    assert_stream_local_failure_is_reusable(true);
}

#[test]
fn doq_malformed_response_is_stream_local_on_a_healthy_connection() {
    assert_stream_local_failure_is_reusable(false);
}

#[test]
fn doq_connection_failure_replaces_only_after_terminal_removal() {
    let set = FixtureSet::generate();
    let server = DoqServer::start(&set, 1, FirstStreamMode::Close);
    let upstream = upstream(&set, server.address);
    let (first, replacement) = block_on(async {
        let first_query = query_wire(0x3001);
        let request = ExchangeRequest::new(&first_query).expect("valid query");
        let context = ExchangeContext::new(
            Instant::now() + EXCHANGE_DEADLINE,
            TransportCancellation::new(),
        );
        let first = upstream.exchange(request, context).await;

        let second_query = query_wire(0x3002);
        let request = ExchangeRequest::new(&second_query).expect("valid query");
        let context = ExchangeContext::new(
            Instant::now() + EXCHANGE_DEADLINE,
            TransportCancellation::new(),
        );
        let second = upstream.exchange(request, context).await;
        assert!(matches!(
            second,
            Err(SecureError::Transport(UpstreamError::Closed(
                SideEffectState::NotSent,
            )))
        ));

        timeout(TEST_TIMEOUT, async {
            while upstream.entry_count() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("failed generation reaches terminal removal");

        let request = ExchangeRequest::new(&second_query).expect("valid query");
        let context = ExchangeContext::new(
            Instant::now() + EXCHANGE_DEADLINE,
            TransportCancellation::new(),
        );
        let replacement = upstream.exchange(request, context).await;
        upstream.close().await;
        (first, replacement)
    });
    assert!(matches!(
        first,
        Err(SecureError::DoqProtocolMissingResponseFin)
    ));
    assert!(
        replacement.is_ok(),
        "the next independent exchange replaces it"
    );
    let (accepts, _) = server.join();
    assert_eq!(accepts, 2, "only the replacement opens a second connection");
}

#[test]
fn doq_initialization_failure_returns_connect_and_removes_failed_generation() {
    let set = FixtureSet::generate();
    let server = DoqServer::start(&set, 1, FirstStreamMode::Normal);
    let upstream = Arc::new(upstream_with_tls(
        server.address,
        TlsPolicy::verified(set.root_store_b()).expect("verified TLS policy"),
    ));

    let results = block_on(async {
        let mut tasks = Vec::new();
        for id in [0x3801, 0x3802] {
            let upstream = Arc::clone(&upstream);
            tasks.push(tokio::spawn(async move {
                let query = query_wire(id);
                let request = ExchangeRequest::new(&query).expect("valid query");
                let context = ExchangeContext::new(
                    Instant::now() + EXCHANGE_DEADLINE,
                    TransportCancellation::new(),
                );
                upstream.exchange(request, context).await
            }));
        }
        let mut results = Vec::new();
        for task in tasks {
            results.push(task.await.expect("initializer waiter joins"));
        }
        results
    });
    assert!(
        results
            .into_iter()
            .all(|result| matches!(result, Err(SecureError::Transport(UpstreamError::Connect))))
    );
    assert_eq!(upstream.entry_count(), 0, "failed generation is removed");
    assert_eq!(
        upstream.in_flight_exchanges(),
        0,
        "no lifecycle residue remains"
    );

    block_on(upstream.close());
    let (accepts, evidence) = server.join();
    assert_eq!(accepts, 1, "the failed generation attempted one connection");
    assert!(evidence.is_empty(), "TLS failure sends no DoQ stream");
}

#[test]
fn doq_owner_close_waits_for_outstanding_stream_drain() {
    let set = FixtureSet::generate();
    let server = DoqServer::start(&set, 1, FirstStreamMode::Hold);
    let upstream = Arc::new(upstream(&set, server.address));
    let query = query_wire(0x3901);
    let exchange_upstream = Arc::clone(&upstream);

    let result = block_on(async {
        let exchange = tokio::spawn(async move {
            let request = ExchangeRequest::new(&query).expect("valid query");
            let context = ExchangeContext::new(
                Instant::now() + EXCHANGE_DEADLINE,
                TransportCancellation::new(),
            );
            exchange_upstream.exchange(request, context).await
        });
        timeout(TEST_TIMEOUT, server.ready.notified())
            .await
            .expect("the held request reaches the server");

        timeout(TEST_TIMEOUT, upstream.close())
            .await
            .expect("owner close waits for the real DoQ stream drain");
        assert_eq!(upstream.entry_count(), 0, "terminal removal follows drain");
        assert_eq!(
            upstream.in_flight_exchanges(),
            0,
            "close leaves no liveness"
        );
        timeout(TEST_TIMEOUT, exchange)
            .await
            .expect("the force-closed exchange returns")
            .expect("the exchange task joins")
    });
    assert!(
        result.is_err(),
        "owner close cannot report a held query success"
    );

    let (accepts, _) = server.join();
    assert_eq!(accepts, 1, "close drains the existing generation in place");
}

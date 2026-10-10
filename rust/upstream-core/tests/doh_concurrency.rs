//! Local trusted TLS peer: acceptance counts expose reuse, not timing guesses.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use base64::Engine as _;
use mosdns_upstream_core::secure::{DohEndpoint, TlsPolicy};
use mosdns_upstream_core::{
    DohReuseOwner, ExchangeContext, ExchangeRequest, MAX_DOH_CONNECTIONS, TransportCancellation,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;

fn runtime<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

fn query(id: u16) -> Vec<u8> {
    let mut wire = vec![
        0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 3, b'd', b'n', b's', 7, b'e', b'x', b'a', b'm', b'p',
        b'l', b'e', 0, 0, 1, 0, 1,
    ];
    wire[..2].copy_from_slice(&id.to_be_bytes());
    wire[15] = b'a' + u8::try_from(id % 26).expect("label index");
    wire
}

fn answer(target: &str) -> Vec<u8> {
    let encoded = target.split("dns=").nth(1).expect("DNS GET");
    let mut wire = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .expect("query");
    wire[2] = 0x81;
    wire[3] = 0x80;
    wire
}

struct Peer {
    address: std::net::SocketAddr,
    policy: TlsPolicy,
    accepts: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    ended: Arc<AtomicUsize>,
    control: tokio::sync::broadcast::Sender<bool>,
    stop: TransportCancellation,
    task: tokio::task::JoinHandle<()>,
}

struct CountDown(Arc<AtomicUsize>);
impl Drop for CountDown {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
struct CountEnd(Arc<AtomicUsize>);
impl Drop for CountEnd {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

async fn serve_h1(
    mut stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    requests: Arc<AtomicUsize>,
) {
    let mut head = Vec::new();
    loop {
        let mut byte = [0];
        match stream.read_exact(&mut byte).await {
            Ok(_) => head.push(byte[0]),
            Err(_) => return,
        }
        if !head.ends_with(b"\r\n\r\n") {
            continue;
        }
        let text = std::str::from_utf8(&head).expect("head");
        let target = text.split_whitespace().nth(1).expect("target");
        let body = answer(target);
        requests.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(40)).await;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        if stream.write_all(response.as_bytes()).await.is_err()
            || stream.write_all(&body).await.is_err()
        {
            return;
        }
        head.clear();
    }
}

fn tls_config(h2: bool) -> (TlsPolicy, rustls::ServerConfig) {
    let identity =
        rcgen::generate_simple_self_signed(vec!["dns.example".into()]).expect("identity");
    let der = identity.cert.der().clone();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(der.clone()).expect("root");
    let policy = TlsPolicy::verified(roots).expect("verified policy");
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("protocols")
    .with_no_client_auth()
    .with_single_cert(
        vec![der],
        rustls::pki_types::PrivateKeyDer::try_from(identity.signing_key.serialize_der())
            .expect("key"),
    )
    .expect("server TLS");
    config.alpn_protocols = vec![if h2 {
        b"h2".to_vec()
    } else {
        b"http/1.1".to_vec()
    }];
    (policy, config)
}

impl Peer {
    async fn h1() -> Self {
        Self::start(false, 64).await
    }

    async fn start(h2: bool, stream_limit: u32) -> Self {
        let (policy, config) = tls_config(h2);
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("listen");
        let address = listener.local_addr().expect("address");
        let accepts = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let ended = Arc::new(AtomicUsize::new(0));
        let (control, _) = tokio::sync::broadcast::channel(8);
        let control_sender = control.clone();
        let req_count = Arc::clone(&requests);
        let active_count = Arc::clone(&active);
        let peak_count = Arc::clone(&peak);
        let end_count = Arc::clone(&ended);
        let count = Arc::clone(&accepts);
        let stop = TransportCancellation::new();
        let stopping = stop.clone();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let task = tokio::spawn(async move {
            let mut children = JoinSet::new();
            loop {
                tokio::select! {
                    () = stopping.cancelled() => break,
                    accepted = listener.accept() => {
                        let (socket,_) = accepted.expect("accept");
                        count.fetch_add(1,Ordering::SeqCst);
                        let acceptor = acceptor.clone();
                        let mut controls=control_sender.subscribe();
                        let req_count=Arc::clone(&req_count);
                        let active_count=Arc::clone(&active_count);
                        let peak_count=Arc::clone(&peak_count);
                        let end_count=Arc::clone(&end_count);
                        children.spawn(async move {
                            let _ended=CountEnd(end_count);
                            let Ok(stream) = acceptor.accept(socket).await else {return};
                            if h2 {
                                let mut builder = h2::server::Builder::new();
                                builder.max_concurrent_streams(stream_limit);
                                let Ok(mut connection) = builder.handshake(stream).await else {return};
                                let mut responses = JoinSet::new();
                                loop {
                                    let incoming = tokio::select! {
                                        signal=controls.recv() => {
                                            if signal==Ok(true) {connection.graceful_shutdown();continue}
                                            break;
                                        },
                                        incoming=connection.accept() => match incoming {Some(v)=>v,None=>break},
                                    };
                                    let Ok((request,mut send)) = incoming else {break};
                                    req_count.fetch_add(1,Ordering::SeqCst);
                                    let current=active_count.fetch_add(1,Ordering::SeqCst)+1;
                                    peak_count.fetch_max(current,Ordering::SeqCst);
                                    let active=Arc::clone(&active_count);
                                    let body = answer(&request.uri().to_string());
                                    responses.spawn(async move {
                                        let _active=CountDown(active);
                                        // Delay headers too; requests must actually overlap.
                                        tokio::time::sleep(Duration::from_millis(40)).await;
                                        let head = hyper::Response::builder().status(200).header("content-type","application/dns-message").body(()).expect("response");
                                        if let Ok(mut stream) = send.send_response(head,false) {
                                            let _ = stream.send_data(hyper::body::Bytes::from(body),true);
                                        }
                                    });
                                    while responses.try_join_next().is_some() {}
                                }
                                return;
                            }
                            serve_h1(stream,req_count).await;
                        });
                    },
                    _ = children.join_next(), if !children.is_empty() => {},
                }
            }
            children.abort_all();
            while children.join_next().await.is_some() {}
        });
        Self {
            address,
            policy,
            accepts,
            requests,
            active,
            peak,
            ended,
            control,
            stop,
            task,
        }
    }

    fn owner(&self) -> Arc<DohReuseOwner> {
        Arc::new(
            DohReuseOwner::new(
                DohEndpoint::new("https://dns.example/dns-query", self.address).expect("endpoint"),
                self.policy.clone(),
            )
            .expect("owner"),
        )
    }

    async fn close(self) {
        self.stop.cancel();
        self.task.await.expect("server stopped");
    }
}

async fn wait_count(count: &AtomicUsize, expected: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while count.load(Ordering::SeqCst) < expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("peer reaches observable boundary");
}

struct Proxy {
    address: std::net::SocketAddr,
    accepts: Arc<AtomicUsize>,
    stop: TransportCancellation,
    task: tokio::task::JoinHandle<()>,
}

impl Proxy {
    async fn start(destination: std::net::SocketAddr) -> Self {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("proxy listen");
        let address = listener.local_addr().expect("proxy address");
        let accepts = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&accepts);
        let stop = TransportCancellation::new();
        let stopping = stop.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    ()=stopping.cancelled() => break,
                    accepted=listener.accept() => {
                        let (mut client,_)=accepted.expect("proxy accept");
                        count.fetch_add(1,Ordering::SeqCst);
                        connections.spawn(async move {
                            let mut greeting=[0;3];
                            client.read_exact(&mut greeting).await.expect("greeting");
                            assert_eq!(greeting,[5,1,0]);
                            client.write_all(&[5,0]).await.expect("method");
                            let mut connect=[0;10];
                            client.read_exact(&mut connect).await.expect("CONNECT");
                            assert_eq!(&connect[..8],&[5,1,0,1,127,0,0,2],"immutable SOCKS target");
                            assert_eq!(u16::from_be_bytes([connect[8],connect[9]]),destination.port());
                            let mut upstream=tokio::net::TcpStream::connect(destination).await.expect("restricted local destination");
                            client.write_all(&[5,0,0,1,127,0,0,1,0,0]).await.expect("connected");
                            let _=tokio::io::copy_bidirectional(&mut client,&mut upstream).await;
                        });
                    },
                    joined=connections.join_next(),if !connections.is_empty()=>{joined.expect("proxy task").expect("proxy assertions");},
                }
            }
            connections.abort_all();
            while connections.join_next().await.is_some() {}
        });
        Self {
            address,
            accepts,
            stop,
            task,
        }
    }

    async fn close(self) {
        self.stop.cancel();
        self.task.await.expect("proxy stopped");
    }
}

#[test]
fn both_protocols_keep_the_same_socks_route_for_every_reused_connection() {
    runtime(async {
        for h2 in [false, true] {
            let peer = Peer::start(h2, 64).await;
            let proxy = Proxy::start(peer.address).await;
            // A direct dial to127.0.0.2 cannot reach this127.0.0.1 listener.
            let destination = std::net::SocketAddr::from(([127, 0, 0, 2], peer.address.port()));
            let route = mosdns_upstream_core::socks5::Route::new(&proxy.address.to_string(), None);
            let endpoint = DohEndpoint::new("https://dns.example/dns-query", destination)
                .expect("endpoint")
                .with_socks5(route);
            let owner = Arc::new(DohReuseOwner::new(endpoint, peer.policy.clone()).expect("owner"));
            let mut requests = JoinSet::new();
            for id in 1..=24 {
                requests.spawn(exchange(Arc::clone(&owner), id));
            }
            while let Some(result) = requests.join_next().await {
                result.expect("task").expect("SOCKS reuse");
            }
            let connections = proxy.accepts.load(Ordering::SeqCst);
            assert_eq!(connections, peer.accepts.load(Ordering::SeqCst));
            if h2 {
                assert_eq!(connections, 1);
            } else {
                assert_eq!(connections, MAX_DOH_CONNECTIONS);
            }
            owner.close().await;
            wait_count(&peer.ended, connections).await;
            assert_eq!(owner.stats().connections, 0);
            peer.close().await;
            proxy.close().await;
        }
    });
}

#[test]
fn goaway_drains_inflight_streams_but_connection_failure_does_not_replay_them() {
    runtime(async {
        for graceful in [true, false] {
            let peer = Peer::start(true, 64).await;
            let owner = peer.owner();
            let first = tokio::spawn(exchange(Arc::clone(&owner), 1));
            let second = tokio::spawn(exchange(Arc::clone(&owner), 2));
            wait_count(&peer.requests, 2).await;
            peer.control.send(graceful).expect("signal peer");
            for task in [first, second] {
                let result = task.await.expect("exchange task");
                if graceful {
                    result.expect("GOAWAY keeps accepted streams");
                } else {
                    assert!(result.is_err(), "on-wire requests fail without replay");
                }
            }
            assert_eq!(peer.requests.load(Ordering::SeqCst), 2);
            assert_eq!(
                peer.accepts.load(Ordering::SeqCst),
                1,
                "no hidden retry after handoff"
            );
            wait_count(&peer.ended, 1).await;
            exchange(Arc::clone(&owner), 3)
                .await
                .expect("subsequent fresh session");
            assert_eq!(peer.accepts.load(Ordering::SeqCst), 2);
            owner.close().await;
            peer.close().await;
        }
    });
}

#[test]
fn h2_peer_stream_limit_and_local_lease_limit_are_bounded() {
    runtime(async {
        let peer = Peer::start(true, 2).await;
        let owner = peer.owner();
        let mut requests = JoinSet::new();
        for id in 1..=96 {
            requests.spawn(exchange(Arc::clone(&owner), id));
        }
        while let Some(result) = requests.join_next().await {
            result.expect("task").expect("bounded streams complete");
        }
        assert_eq!(peer.accepts.load(Ordering::SeqCst), 1);
        assert_eq!(
            peer.peak.load(Ordering::SeqCst),
            2,
            "peer SETTINGS is respected on wire"
        );
        assert_eq!(peer.active.load(Ordering::SeqCst), 0);
        assert_eq!(
            owner.stats().peak_leases,
            64,
            "local leases include peer-ready waits"
        );
        assert_eq!(owner.stats().active_leases, 0);
        owner.close().await;
        peer.close().await;
    });
}

#[test]
fn cancelling_one_request_keeps_other_requests_and_h2_connection_alive() {
    runtime(async {
        for h2 in [false, true] {
            let peer = Peer::start(h2, 64).await;
            let owner = peer.owner();
            let cancellation = TransportCancellation::new();
            let cancelled = {
                let owner = Arc::clone(&owner);
                let token = cancellation.clone();
                tokio::spawn(async move {
                    let wire = query(1);
                    owner
                        .exchange(
                            ExchangeRequest::new(&wire).expect("request"),
                            ExchangeContext::new(Instant::now() + Duration::from_secs(3), token),
                        )
                        .await
                })
            };
            let other = tokio::spawn(exchange(Arc::clone(&owner), 2));
            wait_count(&peer.requests, 2).await;
            cancellation.cancel();
            let result = cancelled.await.expect("cancelled task");
            assert!(matches!(
                result,
                Err(mosdns_upstream_core::secure::SecureError::Transport(
                    mosdns_upstream_core::UpstreamError::Cancelled(_)
                ))
            ));
            assert_eq!(
                other.await.expect("other task").expect("sibling survives"),
                2
            );
            assert_eq!(
                exchange(Arc::clone(&owner), 3)
                    .await
                    .expect("continued reuse"),
                3
            );
            if h2 {
                assert_eq!(
                    peer.accepts.load(Ordering::SeqCst),
                    1,
                    "stream cancellation keeps h2 connection"
                );
            } else {
                assert_eq!(
                    owner.stats().connections,
                    1,
                    "cancelled H1 socket discarded"
                );
            }
            owner.close().await;
            peer.close().await;
        }
    });
}

#[test]
fn full_h1_pool_waits_only_until_original_deadline_without_sending() {
    runtime(async {
        let peer = Peer::h1().await;
        let owner = peer.owner();
        let mut requests = JoinSet::new();
        for id in 1..=u16::try_from(MAX_DOH_CONNECTIONS).expect("bounded pool") {
            requests.spawn(exchange(Arc::clone(&owner), id));
        }
        wait_count(&peer.requests, MAX_DOH_CONNECTIONS).await;
        let wire = query(u16::try_from(MAX_DOH_CONNECTIONS + 1).expect("next query"));
        let result = owner
            .exchange(
                ExchangeRequest::new(&wire).expect("request"),
                ExchangeContext::new(
                    Instant::now() + Duration::from_millis(10),
                    TransportCancellation::new(),
                ),
            )
            .await;
        assert!(matches!(
            result,
            Err(mosdns_upstream_core::secure::SecureError::Transport(
                mosdns_upstream_core::UpstreamError::DeadlineExceeded(
                    mosdns_upstream_core::SideEffectState::NotSent
                )
            ))
        ));
        assert_eq!(
            peer.requests.load(Ordering::SeqCst),
            MAX_DOH_CONNECTIONS,
            "waiting request never sent"
        );
        while let Some(result) = requests.join_next().await {
            result.expect("task").expect("existing lease unaffected");
        }
        owner.close().await;
        peer.close().await;
    });
}

#[test]
fn h2_goaway_and_peer_disconnect_allow_bounded_reconnection() {
    runtime(async {
        for graceful in [true, false] {
            let peer = Peer::start(true, 64).await;
            let owner = peer.owner();
            exchange(Arc::clone(&owner), 1)
                .await
                .expect("initial stream");
            peer.control
                .send(graceful)
                .expect("signal current connection");
            wait_count(&peer.ended, 1).await;
            exchange(Arc::clone(&owner), 2)
                .await
                .expect("reconnect after peer retirement");
            assert_eq!(peer.accepts.load(Ordering::SeqCst), 2);
            assert_eq!(owner.stats().connect_attempts, 2);
            owner.close().await;
            peer.close().await;
        }
    });
}

async fn exchange(
    owner: Arc<DohReuseOwner>,
    id: u16,
) -> Result<u16, mosdns_upstream_core::secure::SecureError> {
    let wire = query(id);
    owner
        .exchange(
            ExchangeRequest::new(&wire).expect("request"),
            ExchangeContext::new(
                Instant::now() + Duration::from_secs(3),
                TransportCancellation::new(),
            ),
        )
        .await
        .map(|r| {
            assert_eq!(
                &r.wire()[12..],
                &wire[12..],
                "response belongs to the original stream question"
            );
            r.request_id()
        })
}

#[test]
fn h2_concurrent_callers_share_one_connection_with_independent_responses() {
    runtime(async {
        let peer = Peer::start(true, 64).await;
        let owner = peer.owner();
        let mut requests = JoinSet::new();
        for id in 1..=24 {
            requests.spawn(exchange(Arc::clone(&owner), id));
        }
        let mut ids = Vec::new();
        while let Some(result) = requests.join_next().await {
            ids.push(result.expect("task").expect("h2 stream succeeds"));
        }
        ids.sort_unstable();
        assert_eq!(ids, (1..=24).collect::<Vec<_>>());
        assert_eq!(
            peer.accepts.load(Ordering::SeqCst),
            1,
            "h2 streams share one authenticated connection"
        );
        owner.close().await;
        assert_eq!(owner.in_flight_exchanges(), 0);
        assert_eq!(owner.idle_connections(), 0);
        peer.close().await;
    });
}

#[test]
fn h1_concurrent_callers_reuse_a_bounded_pool_instead_of_busy_fallback() {
    runtime(async {
        let peer = Peer::h1().await;
        let owner = peer.owner();
        let mut requests = JoinSet::new();
        for id in 1..=24 {
            requests.spawn(exchange(Arc::clone(&owner), id));
        }
        let mut ids = Vec::new();
        while let Some(result) = requests.join_next().await {
            ids.push(
                result
                    .expect("task")
                    .expect("bounded pool waits rather than Busy"),
            );
        }
        ids.sort_unstable();
        assert_eq!(ids, (1..=24).collect::<Vec<_>>());
        assert!(
            peer.accepts.load(Ordering::SeqCst) <= MAX_DOH_CONNECTIONS,
            "bounded connections"
        );
        let before = peer.accepts.load(Ordering::SeqCst);
        assert_eq!(exchange(Arc::clone(&owner), 25).await.expect("reuse"), 25);
        assert_eq!(
            peer.accepts.load(Ordering::SeqCst),
            before,
            "reuse after burst"
        );
        owner.close().await;
        assert_eq!(owner.in_flight_exchanges(), 0);
        assert_eq!(owner.idle_connections(), 0);
        peer.close().await;
    });
}

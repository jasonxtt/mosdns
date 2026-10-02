//! Same-key followers cannot replace the first lazy refresh's state or wire.
use hickory_proto::op::{Message, MessageType, Query};
use hickory_proto::rr::{Name, RData, Record, RecordType, rdata::A};
use mosdns_native_host::{CacheTestClock, HostAssembly, HostOptions, compile_yaml};
use mosdns_upstream_core::TransportCancellation;
use std::net::UdpSocket;
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn query(marker: u8) -> Vec<u8> {
    let mut message = Message::new();
    message
        .set_id(77)
        .set_recursion_desired(true)
        .add_query(Query::query(
            Name::from_ascii("refresh.example.").unwrap(),
            RecordType::A,
        ));
    let mut wire = message.to_vec().unwrap();
    wire[11] = 1;
    let body = [0, 8, 0, 7, 0, 1, 24, 0, 203, 0, 113, 253, 233, 0, 1, marker];
    wire.extend_from_slice(&[0, 0, 41, 4, 208, 0, 0, 0, 0]);
    wire.extend_from_slice(&u16::try_from(body.len()).unwrap().to_be_bytes());
    wire.extend_from_slice(&body);
    wire
}
async fn tcp_send(address: std::net::SocketAddr, peer: &str, marker: u8) -> tokio::net::TcpStream {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind(format!("{peer}:0").parse().unwrap()).unwrap();
    let mut stream = socket.connect(address).await.unwrap();
    let wire = query(marker);
    stream
        .write_all(&u16::try_from(wire.len()).unwrap().to_be_bytes())
        .await
        .unwrap();
    stream.write_all(&wire).await.unwrap();
    stream
}
async fn dns(address: std::net::SocketAddr, peer: &str, marker: u8, tcp: bool) -> Message {
    if tcp {
        let mut stream = tcp_send(address, peer, marker).await;
        let size = stream.read_u16().await.unwrap();
        let mut wire = vec![0; usize::from(size)];
        stream.read_exact(&mut wire).await.unwrap();
        return Message::from_vec(&wire).unwrap();
    }
    let socket = tokio::net::UdpSocket::bind((peer, 0)).await.unwrap();
    socket.send_to(&query(marker), address).await.unwrap();
    let mut buffer = [0; 4096];
    let (len, _) = tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut buffer))
        .await
        .unwrap()
        .unwrap();
    Message::from_vec(&buffer[..len]).unwrap()
}
fn peer_loop(
    primary: &UdpSocket,
    worker_started: &AtomicUsize,
    gate: &std::sync::mpsc::Receiver<()>,
) -> Vec<Vec<u8>> {
    let mut captured = Vec::new();
    for index in 0..2 {
        let mut buffer = [0; 4096];
        let (len, peer) = primary.recv_from(&mut buffer).unwrap();
        captured.push(buffer[..len].to_vec());
        worker_started.store(index + 1, Ordering::SeqCst);
        if index == 1 {
            gate.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        let request = Message::from_vec(&buffer[..len]).unwrap();
        let mut response = Message::new();
        response
            .set_id(request.id())
            .set_message_type(MessageType::Response)
            .set_recursion_available(true)
            .add_query(request.queries()[0].clone());
        response.add_answer(Record::from_rdata(
            request.queries()[0].name().clone(),
            if index == 0 { 1 } else { 60 },
            RData::A(A::new(192, 0, 2, 20 + u8::try_from(index).unwrap())),
        ));
        primary.send_to(&response.to_vec().unwrap(), peer).unwrap();
    }
    captured
}
#[test]
fn first_refresh_keeps_peer_derived_state_and_current_query_options() {
    run(false);
}
#[test]
fn tcp_disconnect_does_not_replace_first_refresh_with_followers() {
    run(true);
}
fn run(tcp: bool) {
    let primary = UdpSocket::bind("127.0.0.1:0").unwrap();
    primary
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let primary_addr = primary.local_addr().unwrap();
    let follower = UdpSocket::bind("127.0.0.1:0").unwrap();
    follower.set_nonblocking(true).unwrap();
    let follower_addr = follower.local_addr().unwrap();
    let started = Arc::new(AtomicUsize::new(0));
    let worker_started = started.clone();
    let (release, gate) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || peer_loop(&primary, &worker_started, &gate));
    let yaml = format!(
        "log: {{level: error}}\nplugins:\n  - tag: ecs\n    type: ecs_handler\n    args: {{forward: true}}\n  - tag: cache\n    type: cache\n    args: {{enable_ecs: true, lazy_cache_ttl: 900}}\n  - tag: primary\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://{primary_addr}'}}]}}\n  - tag: follower\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://{follower_addr}'}}]}}\n  - tag: main\n    type: sequence\n    args:\n      - matches: client_ip 127.0.0.2\n        exec: fast_mark 1\n      - matches: client_ip 127.0.0.3\n        exec: fast_mark 2\n      - exec: $ecs\n      - exec: $cache\n      - matches: fast_mark 1\n        exec: $primary\n      - matches: fast_mark 2\n        exec: $follower\n  - tag: dns\n    type: udp_server\n    args: {{entry: main, listen: '127.0.0.1:19909', enable_audit: false}}\n"
    );
    let yaml = if tcp {
        yaml.replace("type: udp_server", "type: tcp_server")
            .replace(
                "enable_audit: false}",
                "enable_audit: false, idle_timeout: 10}",
            )
    } else {
        yaml
    };
    let clock = CacheTestClock::new(1000);
    let host = HostAssembly::with_options(
        compile_yaml(&yaml).unwrap(),
        HostOptions::default().with_cache_clock(Rc::new(clock.clone())),
    )
    .unwrap();
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let address = bound.dns_addr();
        let stop = TransportCancellation::new();
        let scope = stop.clone();
        let serving = tokio::task::spawn_local(async move { bound.serve(scope).await });
        assert_eq!(
            dns(address, "127.0.0.2", 11, tcp).await.answers()[0]
                .data()
                .to_string(),
            "192.0.2.20"
        );
        clock.advance(2);
        if tcp {
            drop(tcp_send(address, "127.0.0.2", 42).await);
        } else {
            assert_eq!(
                dns(address, "127.0.0.2", 42, false).await.answers()[0]
                    .data()
                    .to_string(),
                "192.0.2.20"
            );
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            while started.load(Ordering::SeqCst) != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            dns(address, "127.0.0.3", 99, tcp).await.answers()[0]
                .data()
                .to_string(),
            "192.0.2.20"
        );
        release.send(()).unwrap();
        let cache = host.cache().get(mosdns_native_host::CacheId(0)).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if cache.lookup(&query(42)).unwrap().is_some_and(|wire| {
                    Message::from_vec(&wire).unwrap().answers()[0]
                        .data()
                        .to_string()
                        == "192.0.2.21"
                }) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            dns(address, "127.0.0.3", 99, tcp).await.answers()[0]
                .data()
                .to_string(),
            "192.0.2.21"
        );
        stop.cancel();
        serving.await.unwrap().unwrap();
    });
    let captured = worker.join().unwrap();
    assert!(captured[1].windows(5).any(|s| s == [253, 233, 0, 1, 42]));
    assert!(
        captured[1]
            .windows(11)
            .any(|s| s == [0, 8, 0, 7, 0, 1, 24, 0, 203, 0, 113])
    );
    assert!(follower.recv_from(&mut [0; 4096]).is_err());
}

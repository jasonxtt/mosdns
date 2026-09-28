use std::net::{SocketAddr, UdpSocket as StdUdpSocket};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use mosdns_dns_core::{inspect_response_header, parse_query, validate_response};
use mosdns_native_host::{HostAssembly, HostOptions, UdpServer, compile_yaml};
use mosdns_upstream_core::TransportCancellation;

struct Peer {
    address: SocketAddr,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Peer {
    fn start(delay: Duration) -> Self {
        let socket = StdUdpSocket::bind("127.0.0.1:0").expect("peer bind");
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .expect("peer timeout");
        let address = socket.local_addr().expect("peer address");
        let requests = Arc::new(AtomicUsize::new(0));
        let thread_requests = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            let mut packet = vec![0_u8; 65535];
            while !thread_stop.load(Ordering::SeqCst) {
                let Ok((length, client)) = socket.recv_from(&mut packet) else {
                    continue;
                };
                thread_requests.fetch_add(1, Ordering::SeqCst);
                if !delay.is_zero() {
                    thread::sleep(delay);
                }
                let query = &packet[..length];
                let (_, question) = parse_query(query).expect("peer query");
                let response = response_for(query, &question);
                socket.send_to(&response, client).expect("peer response");
            }
        });
        Self {
            address,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread
            .take()
            .expect("peer thread")
            .join()
            .expect("peer join");
    }
}

fn response_for(query: &[u8], question: &mosdns_dns_core::QuestionInfo) -> Vec<u8> {
    let (header, _) = parse_query(query).expect("query");
    let mut response = Vec::new();
    response.extend_from_slice(&header.id.to_be_bytes());
    response.extend_from_slice(&0x8180_u16.to_be_bytes());
    response.extend_from_slice(&1_u16.to_be_bytes());
    response.extend_from_slice(&1_u16.to_be_bytes());
    response.extend_from_slice(&[0, 0, 0, 0]);
    response.extend_from_slice(&question.qname_wire);
    response.extend_from_slice(&question.qtype.to_be_bytes());
    response.extend_from_slice(&question.qclass.to_be_bytes());
    response.extend_from_slice(&[
        0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3c, 0x00, 0x04, 192, 0, 2, 44,
    ]);
    response
}

fn query(id: u16, name: &str) -> Vec<u8> {
    let mut packet = Vec::from([
        (id >> 8) as u8,
        u8::try_from(id & 0x00ff).expect("low ID byte"),
        0x01,
        0x00,
        0x00,
        0x01,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
    ]);
    for label in name.trim_end_matches('.').split('.') {
        packet.push(u8::try_from(label.len()).expect("DNS label"));
        packet.extend_from_slice(label.as_bytes());
    }
    packet.extend_from_slice(&[0, 0, 1, 0, 1]);
    packet
}

fn client_request(listener: SocketAddr, request: &[u8]) -> Vec<u8> {
    let socket = StdUdpSocket::bind("127.0.0.1:0").expect("client bind");
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("client timeout");
    socket.send_to(request, listener).expect("client send");
    let mut response = vec![0_u8; 65535];
    let (length, _) = socket.recv_from(&mut response).expect("client response");
    response[..length].to_vec()
}

fn free_addr() -> SocketAddr {
    StdUdpSocket::bind("127.0.0.1:0")
        .expect("free address")
        .local_addr()
        .expect("free socket address")
}

fn branch_yaml(peer: SocketAddr, listener: SocketAddr) -> String {
    format!(
        r#"
log: {{ level: error }}
plugins:
  - tag: entry
    type: sequence
    args:
      - exec: fast_mark 1
      - matches: qname full:flagged.test
        exec: fast_mark 7
      - matches: fast_mark 7 63
        exec: reject 3
      - exec: $forward
  - tag: forward
    type: forward
    args: {{ upstreams: [ {{ addr: "udp://{peer}" }} ] }}
  - tag: listener
    type: udp_server
    args: {{ entry: entry, listen: "{listener}", enable_audit: true }}
"#
    )
}

fn flow_yaml(peer: SocketAddr, listener: SocketAddr) -> String {
    format!(
        r#"
log: {{ level: error }}
plugins:
  - tag: entry
    type: sequence
    args:
      - exec: $setter
      - exec: $forward
  - tag: setter
    type: flow_setter
    args: {{ matched_group: configured_group, final_sequence: configured_sequence, final_upstream: configured_upstream }}
  - tag: forward
    type: forward
    args: {{ upstreams: [ {{ tag: actual_peer, addr: "udp://{peer}" }} ] }}
  - tag: listener
    type: udp_server
    args: {{ entry: entry, listen: "{listener}", enable_audit: true }}
"#
    )
}

#[test]
fn live_listener_flags_branch_and_isolate_each_request() {
    let peer = Peer::start(Duration::ZERO);
    let listener_address = free_addr();
    let assembly = HostAssembly::with_options(
        compile_yaml(&branch_yaml(peer.address, listener_address)).expect("branch config"),
        HostOptions::default(),
    )
    .expect("branch assembly");
    let server = assembly
        .block_on(UdpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let (flagged, normal) = assembly.block_on(async {
        let task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let flagged = tokio::task::spawn_blocking(move || {
            client_request(listener, &query(0x5101, "flagged.test."))
        })
        .await
        .expect("flagged client");
        let normal = tokio::task::spawn_blocking(move || {
            client_request(listener, &query(0x5102, "normal.test."))
        })
        .await
        .expect("normal client");
        shutdown.cancel();
        task.await.expect("server task").expect("server shutdown");
        (flagged, normal)
    });

    assert_eq!(u16::from_be_bytes([flagged[2], flagged[3]]) & 0x000f, 3);
    assert_eq!(
        inspect_response_header(&normal)
            .expect("normal response")
            .id,
        0x5102
    );
    validate_response(&normal).expect("normal response validates");
    assert_eq!(peer.requests.load(Ordering::SeqCst), 1);
    assert_eq!(assembly.audit_snapshot().records.len(), 2);
    peer.stop();
}

#[test]
fn delayed_forward_preserves_configured_flow_metadata_and_precedence() {
    let peer = Peer::start(Duration::from_millis(50));
    let listener_address = free_addr();
    let assembly = HostAssembly::with_options(
        compile_yaml(&flow_yaml(peer.address, listener_address)).expect("flow config"),
        HostOptions::default(),
    )
    .expect("flow assembly");
    let server = assembly
        .block_on(UdpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let response = assembly.block_on(async {
        let task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let response = tokio::task::spawn_blocking(move || {
            client_request(listener, &query(0x5201, "delayed.test."))
        })
        .await
        .expect("flow client");
        shutdown.cancel();
        task.await.expect("server task").expect("server shutdown");
        response
    });

    assert_eq!(
        inspect_response_header(&response).expect("response").id,
        0x5201
    );
    let audit = assembly.audit_snapshot();
    let record = audit.records.last().expect("flow audit record");
    assert_eq!(record.matched_group.as_deref(), Some("configured_group"));
    assert_eq!(
        record.final_sequence.as_deref(),
        Some("configured_sequence")
    );
    assert_eq!(
        record.final_upstream.as_deref(),
        Some("configured_upstream")
    );
    peer.stop();
}

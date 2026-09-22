use std::net::{SocketAddr, UdpSocket as StdUdpSocket};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use mosdns_dns_core::{inspect_response_header, parse_query, validate_response};
use mosdns_native_host::{HostAssembly, HostOptions, UdpServer, compile_yaml};
use mosdns_upstream_core::TransportCancellation;

const UDP_CONFIG: &str = include_str!("../../../tests/phase5a-baseline/configs/forward-udp.yaml");

struct MockUpstream {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MockUpstream {
    fn start() -> Self {
        let socket = StdUdpSocket::bind("127.0.0.1:0").expect("mock upstream bind");
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .expect("mock timeout");
        let address = socket.local_addr().expect("mock local address");
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            let mut input = vec![0_u8; 65535];
            while !thread_stop.load(Ordering::SeqCst) {
                let Ok((length, peer)) = socket.recv_from(&mut input) else {
                    continue;
                };
                let query = &input[..length];
                let nx = query.windows(3).any(|window| window == [2, b'n', b'x']);
                let response = response_for(query, nx).expect("mock query shape");
                socket.send_to(&response, peer).expect("mock response");
            }
        });
        Self {
            address,
            stop,
            thread: Some(thread),
        }
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("mock thread join");
        }
    }
}

fn response_for(query: &[u8], nxdomain: bool) -> Result<Vec<u8>, String> {
    let (header, question) = parse_query(query).map_err(|error| format!("{error:?}"))?;
    let flags = if nxdomain { 0x8183_u16 } else { 0x8180_u16 };
    let answer_count = u16::from(!nxdomain);
    let mut response = Vec::new();
    response.extend_from_slice(&header.id.to_be_bytes());
    response.extend_from_slice(&flags.to_be_bytes());
    response.extend_from_slice(&1_u16.to_be_bytes());
    response.extend_from_slice(&answer_count.to_be_bytes());
    response.extend_from_slice(&0_u16.to_be_bytes());
    response.extend_from_slice(&0_u16.to_be_bytes());
    response.extend_from_slice(&question.qname_wire);
    response.extend_from_slice(&question.qtype.to_be_bytes());
    response.extend_from_slice(&question.qclass.to_be_bytes());
    if !nxdomain {
        response.extend_from_slice(&[
            0xc0, 0x0c, // owner pointer to the single question
            0x00, 0x01, // A
            0x00, 0x01, // IN
            0x00, 0x00, 0x00, 0x3c, // TTL 60
            0x00, 0x04, // RDLENGTH
            192, 0, 2, 1,
        ]);
    }
    Ok(response)
}

fn query(id: u16, labels: &[&str]) -> Vec<u8> {
    let mut packet = Vec::from([
        u8::try_from(id >> 8).expect("high ID byte"),
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
    for label in labels {
        packet.push(u8::try_from(label.len()).expect("DNS label length"));
        packet.extend_from_slice(label.as_bytes());
    }
    packet.extend_from_slice(&[0, 0, 1, 0, 1]);
    packet
}

fn client_request(listener: SocketAddr, request: &[u8], timeout: Duration) -> Vec<u8> {
    let socket = StdUdpSocket::bind("127.0.0.1:0").expect("client bind");
    socket
        .set_read_timeout(Some(timeout))
        .expect("client timeout");
    socket.send_to(request, listener).expect("client send");
    let mut response = vec![0_u8; 65535];
    let (length, _) = socket.recv_from(&mut response).expect("client response");
    response[..length].to_vec()
}

fn assembly_for(upstream: SocketAddr, options: HostOptions) -> HostAssembly {
    let yaml = UDP_CONFIG.replace("udp://127.0.0.1:15453", &format!("udp://{upstream}"));
    let config = compile_yaml(&yaml).expect("test config compile");
    HostAssembly::with_options(config, options).expect("test assembly")
}

#[test]
fn udp_answers_positive_nxdomain_and_concurrent_distinct_queries() {
    let mock = MockUpstream::start();
    let assembly = assembly_for(mock.address, HostOptions::default());
    let server = assembly
        .block_on(UdpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("UDP listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let requests = vec![
        (0x1001, vec!["www", "example"]),
        (0x1002, vec!["nx", "example"]),
        (0x1003, vec!["api", "example"]),
    ];

    let result = assembly.block_on(async {
        let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let mut clients = Vec::new();
        for (id, labels) in requests {
            let request = query(id, &labels);
            clients.push(tokio::task::spawn_blocking(move || {
                (
                    id,
                    client_request(listener, &request, Duration::from_secs(2)),
                )
            }));
        }

        let mut responses = Vec::new();
        for client in clients {
            responses.push(client.await.expect("client task"));
        }
        shutdown.cancel();
        let server_result = server_task.await.expect("server task");
        (responses, server_result)
    });

    let (responses, server_result) = result;
    server_result.expect("server shutdown");
    for (id, response) in responses {
        let header = inspect_response_header(&response).expect("response header");
        assert_eq!(header.id, id);
        validate_response(&response).expect("valid DNS response");
        let rcode = u16::from_be_bytes([response[2], response[3]]) & 0x000f;
        if id == 0x1002 {
            assert_eq!(rcode, 3, "NXDOMAIN must remain a normal response");
        } else {
            assert_eq!(rcode, 0);
        }
    }
    mock.stop();
}

#[test]
fn malformed_datagram_is_dropped_and_listener_stays_available() {
    let mock = MockUpstream::start();
    let assembly = assembly_for(mock.address, HostOptions::default());
    let server = assembly
        .block_on(UdpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("UDP listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let result = assembly.block_on(async {
        let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let malformed = StdUdpSocket::bind("127.0.0.1:0").expect("malformed client bind");
        malformed
            .set_read_timeout(Some(Duration::from_millis(150)))
            .expect("malformed timeout");
        malformed
            .send_to(&[1, 2, 3], listener)
            .expect("malformed send");
        let mut dropped = [0_u8; 64];
        assert!(malformed.recv_from(&mut dropped).is_err());

        let response = tokio::task::spawn_blocking(move || {
            client_request(
                listener,
                &query(0x2001, &["after", "malformed"]),
                Duration::from_secs(2),
            )
        })
        .await
        .expect("valid client task");
        shutdown.cancel();
        let server_result = server_task.await.expect("server task");
        (response, server_result)
    });
    let (response, server_result) = result;
    server_result.expect("server shutdown");
    assert_eq!(
        inspect_response_header(&response).expect("response").id,
        0x2001
    );
    mock.stop();
}

#[test]
fn stalled_upstream_maps_to_servfail_and_shutdown_allows_rebind() {
    let blackhole = StdUdpSocket::bind("127.0.0.1:0").expect("blackhole bind");
    let assembly = assembly_for(
        blackhole.local_addr().expect("blackhole address"),
        HostOptions::with_deadline(Duration::from_millis(60)),
    );
    let server = assembly
        .block_on(UdpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("UDP listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let (response, server_result) = assembly.block_on(async {
        let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let response = tokio::task::spawn_blocking(move || {
            client_request(
                listener,
                &query(0x3001, &["timeout", "example"]),
                Duration::from_secs(2),
            )
        })
        .await
        .expect("timeout client task");
        shutdown.cancel();
        let server_result = server_task.await.expect("server task");
        (response, server_result)
    });
    server_result.expect("server shutdown");
    assert_eq!(u16::from_be_bytes([response[2], response[3]]) & 0x000f, 2);
    validate_response(&response).expect("SERVFAIL response");

    let rebound = assembly
        .block_on(UdpServer::bind(&assembly, listener))
        .expect("listener must rebind after shutdown");
    drop(rebound);
}

#[test]
fn cancellation_stops_pending_request_without_a_late_response() {
    let blackhole = StdUdpSocket::bind("127.0.0.1:0").expect("blackhole bind");
    let assembly = assembly_for(
        blackhole.local_addr().expect("blackhole address"),
        HostOptions::default(),
    );
    let server = assembly
        .block_on(UdpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("UDP listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let server_result = assembly.block_on(async {
        let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let client = tokio::task::spawn_blocking(move || {
            let socket = StdUdpSocket::bind("127.0.0.1:0").expect("client bind");
            socket
                .set_read_timeout(Some(Duration::from_millis(300)))
                .expect("client timeout");
            socket
                .send_to(&query(0x4001, &["cancel", "example"]), listener)
                .expect("client send");
            let mut response = [0_u8; 512];
            socket.recv_from(&mut response).is_ok()
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        shutdown.cancel();
        let got_response = client.await.expect("cancellation client task");
        let server_result = server_task.await.expect("server task");
        (got_response, server_result)
    });
    assert!(
        !server_result.0,
        "cancelled request must not write late response"
    );
    server_result.1.expect("server shutdown");
}

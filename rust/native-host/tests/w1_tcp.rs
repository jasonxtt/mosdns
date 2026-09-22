use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener as StdTcpListener, TcpStream as StdTcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use mosdns_dns_core::{inspect_response_header, parse_query, validate_response};
use mosdns_native_host::{HostAssembly, HostOptions, TcpServer, compile_yaml};
use mosdns_upstream_core::TransportCancellation;

const TCP_CONFIG: &str = include_str!("../../../tests/phase5a-baseline/configs/forward-tcp.yaml");

struct MockTcpUpstream {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MockTcpUpstream {
    fn start() -> Self {
        let listener = StdTcpListener::bind("127.0.0.1:0").expect("mock TCP upstream bind");
        listener
            .set_nonblocking(true)
            .expect("mock listener nonblocking");
        let address = listener.local_addr().expect("mock upstream address");
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
                        let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
                        if let Ok(query) = read_frame(&mut stream) {
                            let nx = query.windows(3).any(|window| window == [2, b'n', b'x']);
                            if let Ok(response) = response_for(&query, nx) {
                                let length = u16::try_from(response.len()).expect("response fits");
                                let prefix = length.to_be_bytes();
                                let _ = stream.write_all(&prefix);
                                let split = response.len() / 2;
                                let _ = stream.write_all(&response[..split]);
                                thread::sleep(Duration::from_millis(2));
                                let _ = stream.write_all(&response[split..]);
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
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
            thread.join().expect("mock upstream thread");
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
            0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3c, 0x00, 0x04, 192, 0, 2, 1,
        ]);
    }
    Ok(response)
}

fn read_frame(stream: &mut StdTcpStream) -> std::io::Result<Vec<u8>> {
    let mut prefix = [0_u8; 2];
    stream.read_exact(&mut prefix)?;
    let length = usize::from(u16::from_be_bytes(prefix));
    if length == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "zero-length DNS frame",
        ));
    }
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body)?;
    Ok(body)
}

fn write_frame_in_fragments(stream: &mut StdTcpStream, body: &[u8]) {
    let length = u16::try_from(body.len()).expect("query fits TCP frame");
    let prefix = length.to_be_bytes();
    stream.write_all(&prefix[..1]).expect("first prefix byte");
    thread::sleep(Duration::from_millis(1));
    stream.write_all(&prefix[1..]).expect("second prefix byte");
    for chunk in body.chunks(3) {
        stream.write_all(chunk).expect("fragmented query body");
    }
}

fn read_response(stream: &mut StdTcpStream) -> Vec<u8> {
    read_frame(stream).expect("TCP response frame")
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

fn assembly_for(upstream: SocketAddr, options: HostOptions) -> HostAssembly {
    let yaml = TCP_CONFIG.replace("tcp://127.0.0.1:15454", &format!("tcp://{upstream}"));
    let config = compile_yaml(&yaml).expect("test config compile");
    HostAssembly::with_options(config, options).expect("test assembly")
}

fn response_id_rcode(response: &[u8]) -> (u16, u16) {
    let header = inspect_response_header(response).expect("response header");
    validate_response(response).expect("valid DNS response");
    (
        header.id,
        u16::from_be_bytes([response[2], response[3]]) & 0x000f,
    )
}

#[test]
fn tcp_answers_fragmented_positive_nxdomain_sequential_and_concurrent_requests() {
    let mock = MockTcpUpstream::start();
    let assembly = assembly_for(mock.address, HostOptions::default());
    let server = assembly
        .block_on(TcpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("TCP listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let result = assembly.block_on(async {
        let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let sequential = tokio::task::spawn_blocking(move || {
            let mut stream = StdTcpStream::connect(listener).expect("sequential client");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("sequential timeout");
            let first = query(0x5101, &["www", "example"]);
            write_frame_in_fragments(&mut stream, &first);
            let first_response = read_response(&mut stream);
            let second = query(0x5102, &["nx", "example"]);
            write_frame_in_fragments(&mut stream, &second);
            let second_response = read_response(&mut stream);
            (
                response_id_rcode(&first_response),
                response_id_rcode(&second_response),
            )
        });
        let concurrent = tokio::task::spawn_blocking(move || {
            let mut stream = StdTcpStream::connect(listener).expect("concurrent client");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("concurrent timeout");
            let request = query(0x5103, &["api", "example"]);
            write_frame_in_fragments(&mut stream, &request);
            response_id_rcode(&read_response(&mut stream))
        });
        let result = (
            sequential.await.expect("sequential client task"),
            concurrent.await.expect("concurrent client task"),
        );
        shutdown.cancel();
        let server_result = server_task.await.expect("server task");
        (result, server_result)
    });
    let (responses, server_result) = result;
    server_result.expect("server shutdown");
    assert_eq!(responses.0.0, (0x5101, 0));
    assert_eq!(responses.0.1, (0x5102, 3));
    assert_eq!(responses.1, (0x5103, 0));
    mock.stop();
}

#[test]
fn partial_frame_eof_closes_only_that_connection_and_listener_stays_available() {
    let mock = MockTcpUpstream::start();
    let assembly = assembly_for(mock.address, HostOptions::default());
    let server = assembly
        .block_on(TcpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("TCP listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let result = assembly.block_on(async {
        let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let malformed = tokio::task::spawn_blocking(move || {
            let mut stream = StdTcpStream::connect(listener).expect("partial client");
            stream.write_all(&[0, 8, 1, 2]).expect("partial frame");
            stream.shutdown(Shutdown::Write).expect("half close");
            let mut response = [0_u8; 16];
            stream.read(&mut response).expect("partial close read")
        });
        assert_eq!(malformed.await.expect("partial client task"), 0);
        let valid = tokio::task::spawn_blocking(move || {
            let mut stream = StdTcpStream::connect(listener).expect("valid client");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("valid timeout");
            let request = query(0x5201, &["after", "partial"]);
            write_frame_in_fragments(&mut stream, &request);
            response_id_rcode(&read_response(&mut stream))
        });
        let valid_result = valid.await.expect("valid client task");
        shutdown.cancel();
        let server_result = server_task.await.expect("server task");
        (valid_result, server_result)
    });
    let (valid_result, server_result) = result;
    server_result.expect("server shutdown");
    assert_eq!(valid_result, (0x5201, 0));
    mock.stop();
}

#[test]
fn idle_timeout_closes_inactive_connection() {
    let mock = MockTcpUpstream::start();
    let assembly = assembly_for(mock.address, HostOptions::default());
    let server = assembly
        .block_on(TcpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("TCP listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let result = assembly.block_on(async {
        let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let closed = tokio::task::spawn_blocking(move || {
            let mut stream = StdTcpStream::connect(listener).expect("idle client");
            stream
                .set_read_timeout(Some(Duration::from_secs(4)))
                .expect("idle timeout");
            let mut response = [0_u8; 16];
            stream.read(&mut response).expect("idle close read")
        });
        let bytes = closed.await.expect("idle client task");
        shutdown.cancel();
        let server_result = server_task.await.expect("server task");
        (bytes, server_result)
    });
    assert_eq!(result.0, 0);
    result.1.expect("server shutdown");
    mock.stop();
}

#[test]
fn stalled_upstream_maps_to_servfail_and_disconnect_shutdown_allows_rebind() {
    let blackhole = StdTcpListener::bind("127.0.0.1:0").expect("blackhole bind");
    blackhole
        .set_nonblocking(true)
        .expect("blackhole nonblocking");
    let assembly = assembly_for(
        blackhole.local_addr().expect("blackhole address"),
        HostOptions::with_deadline(Duration::from_millis(60)),
    );
    let server = assembly
        .block_on(TcpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("TCP listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let result = assembly.block_on(async {
        let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let response = tokio::task::spawn_blocking(move || {
            let mut stream = StdTcpStream::connect(listener).expect("stalled client");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("stalled timeout");
            let request = query(0x5301, &["timeout", "example"]);
            write_frame_in_fragments(&mut stream, &request);
            response_id_rcode(&read_response(&mut stream))
        });
        let response = response.await.expect("stalled client task");
        shutdown.cancel();
        let server_result = server_task.await.expect("server task");
        (response, server_result)
    });
    assert_eq!(result.0, (0x5301, 2));
    result.1.expect("server shutdown");

    let rebound = assembly
        .block_on(TcpServer::bind(&assembly, listener))
        .expect("listener must rebind after shutdown");
    drop(rebound);
}

#[test]
fn client_disconnect_isolated_and_shutdown_releases_connection_tasks() {
    let mock = MockTcpUpstream::start();
    let assembly = assembly_for(mock.address, HostOptions::default());
    let server = assembly
        .block_on(TcpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("TCP listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    assembly.block_on(async {
        let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        tokio::task::spawn_blocking(move || {
            let mut stream = StdTcpStream::connect(listener).expect("disconnect client");
            let request = query(0x5401, &["disconnect", "example"]);
            write_frame_in_fragments(&mut stream, &request);
            drop(stream);
        })
        .await
        .expect("disconnect client task");
        tokio::time::sleep(Duration::from_millis(20)).await;
        shutdown.cancel();
        server_task
            .await
            .expect("server task")
            .expect("server shutdown");
    });

    let rebound = assembly
        .block_on(TcpServer::bind(&assembly, listener))
        .expect("listener must rebind after connection cleanup");
    drop(rebound);
    mock.stop();
}

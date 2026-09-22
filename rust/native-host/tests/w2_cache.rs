use std::net::{SocketAddr, UdpSocket as StdUdpSocket};
use std::rc::Rc;
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use mosdns_dns_core::{inspect_response_header, parse_query, validate_response};
use mosdns_native_host::{CacheTestClock, HostAssembly, HostOptions, UdpServer, compile_yaml};
use mosdns_upstream_core::TransportCancellation;

const CACHE_CONFIG: &str = include_str!("../../../tests/phase5a-baseline/configs/cache.yaml");

struct MockUpstream {
    address: SocketAddr,
    count: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MockUpstream {
    fn start(hold_two: bool) -> Self {
        let socket = StdUdpSocket::bind("127.0.0.1:0").expect("mock upstream bind");
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .expect("mock timeout");
        let address = socket.local_addr().expect("mock local address");
        let count = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_count = Arc::clone(&count);
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            let mut input = vec![0_u8; 65535];
            let mut pending = Vec::new();
            while !thread_stop.load(Ordering::SeqCst) {
                let Ok((length, peer)) = socket.recv_from(&mut input) else {
                    continue;
                };
                thread_count.fetch_add(1, Ordering::SeqCst);
                if hold_two {
                    pending.push((input[..length].to_vec(), peer));
                    if pending.len() < 2 {
                        continue;
                    }
                    for (query, peer) in pending.drain(..) {
                        if query_has_label(&query, "blackhole") {
                            continue;
                        }
                        socket
                            .send_to(&response_for(&query), peer)
                            .expect("mock response");
                    }
                    continue;
                }
                let query = &input[..length];
                if query_has_label(query, "blackhole") {
                    continue;
                }
                let response = response_for(query);
                socket.send_to(&response, peer).expect("mock response");
            }
        });
        Self {
            address,
            count,
            stop,
            thread: Some(thread),
        }
    }

    fn requests(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("mock thread join");
        }
    }
}

fn query_has_label(query: &[u8], label: &str) -> bool {
    let (_, question) = parse_query(query).expect("test query");
    question.qname_wire.windows(label.len() + 1).any(|window| {
        window[0] == u8::try_from(label.len()).expect("test label length")
            && &window[1..] == label.as_bytes()
    })
}

fn response_for(query: &[u8]) -> Vec<u8> {
    if query_has_label(query, "malformed") {
        return vec![1, 2, 3];
    }
    let (header, question) = parse_query(query).expect("test query");
    let truncated = query_has_label(query, "truncated");
    let opt = query_has_label(query, "opt");
    let mismatch = query_has_label(query, "mismatch");
    let nxdomain = query_has_label(query, "nxdomain");
    let servfail = query_has_label(query, "servfail");
    let empty = query_has_label(query, "empty");
    let answer_count = u16::from(!nxdomain && !servfail && !empty);
    let rcode = if nxdomain {
        3
    } else if servfail {
        2
    } else {
        0
    };
    let flags = 0x8180_u16 | rcode | u16::from(truncated) << 9;
    let additional_count = u16::from(opt);
    let mut response = Vec::new();
    response.extend_from_slice(&header.id.to_be_bytes());
    response.extend_from_slice(&flags.to_be_bytes());
    response.extend_from_slice(&1_u16.to_be_bytes());
    response.extend_from_slice(&answer_count.to_be_bytes());
    response.extend_from_slice(&0_u16.to_be_bytes());
    response.extend_from_slice(&additional_count.to_be_bytes());
    if mismatch {
        response.extend_from_slice(&[5, b'o', b't', b'h', b'e', b'r', 0]);
        response.extend_from_slice(&question.qtype.to_be_bytes());
        response.extend_from_slice(&question.qclass.to_be_bytes());
    } else {
        response.extend_from_slice(&question.qname_wire);
        response.extend_from_slice(&question.qtype.to_be_bytes());
        response.extend_from_slice(&question.qclass.to_be_bytes());
    }
    if answer_count != 0 {
        let ttl = u32::from(query_has_label(query, "expire"));
        response.extend_from_slice(&[
            0xc0, 0x0c, // owner pointer to the response question
        ]);
        response.extend_from_slice(&question.qtype.to_be_bytes());
        response.extend_from_slice(&question.qclass.to_be_bytes());
        response.extend_from_slice(&if ttl == 0 { 60 } else { ttl }.to_be_bytes());
        response.extend_from_slice(&[0, 4, 192, 0, 2, 1]);
    }
    if opt {
        response.extend_from_slice(&[0, 0, 41, 0x04, 0xd0, 0, 0, 0, 0, 0, 0]);
    }
    response
}

fn query(id: u16, name: &str, qtype: u16, qclass: u16, edns: bool) -> Vec<u8> {
    let [id_high, id_low] = id.to_be_bytes();
    let mut packet = Vec::from([
        id_high,
        id_low,
        0x01,
        0x00,
        0x00,
        0x01,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        u8::from(edns),
    ]);
    for label in name.split('.') {
        packet.push(u8::try_from(label.len()).expect("DNS label length"));
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&qtype.to_be_bytes());
    packet.extend_from_slice(&qclass.to_be_bytes());
    if edns {
        packet.extend_from_slice(&[0, 0, 41, 0x04, 0xd0, 0, 0, 0, 0, 0, 0]);
    }
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
    let yaml = CACHE_CONFIG.replace("udp://127.0.0.1:15455", &format!("udp://{upstream}"));
    let config = compile_yaml(&yaml).expect("frozen W2 config must compile");
    HostAssembly::with_options(config, options).expect("W2 assembly")
}

fn response_id(response: &[u8]) -> u16 {
    inspect_response_header(response)
        .expect("response header")
        .id
}

#[test]
fn warm_hits_skip_upstream_and_isolate_ids_and_buffers() {
    let mock = MockUpstream::start(false);
    let clock = CacheTestClock::new(100);
    let assembly = assembly_for(
        mock.address,
        HostOptions::default().with_cache_clock(Rc::new(clock)),
    );
    let server = assembly
        .block_on(UdpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("UDP listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let result = assembly.block_on(async {
        let task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let first_request = query(0x1001, "warm.example", 1, 1, false);
        let first = tokio::task::spawn_blocking(move || {
            client_request(listener, &first_request, Duration::from_secs(2))
        })
        .await
        .expect("first client");
        let second_request = query(0x1002, "warm.example", 1, 1, false);
        let second = tokio::task::spawn_blocking(move || {
            client_request(listener, &second_request, Duration::from_secs(2))
        })
        .await
        .expect("second client");
        let third_request = query(0x1003, "warm.example", 1, 1, false);
        let third = tokio::task::spawn_blocking(move || {
            client_request(listener, &third_request, Duration::from_secs(2))
        })
        .await
        .expect("third client");
        shutdown.cancel();
        let server_result = task.await.expect("server task");
        (first, second, third, server_result)
    });
    let (first, second, third, server_result) = result;
    server_result.expect("server shutdown");
    assert_eq!(mock.requests(), 1, "warm requests must not reach upstream");
    assert_eq!(response_id(&first), 0x1001);
    assert_eq!(response_id(&second), 0x1002);
    assert_eq!(response_id(&third), 0x1003);
    validate_response(&second).expect("cached response");
    let mut mutated = first;
    mutated[12] ^= 0xff;
    assert_eq!(response_id(&third), 0x1003, "cache owns response bytes");
    mock.stop();
}

#[test]
fn cold_concurrent_misses_are_independent_and_warm_concurrency_is_free() {
    let mock = MockUpstream::start(true);
    let assembly = assembly_for(mock.address, HostOptions::default());
    let server = assembly
        .block_on(UdpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("UDP listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let client_barrier = Arc::new(Barrier::new(2));
    let result = assembly.block_on(async {
        let task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let mut clients = Vec::new();
        for id in [0x2001_u16, 0x2002] {
            let request = query(id, "cold.example", 1, 1, false);
            let client_barrier = Arc::clone(&client_barrier);
            clients.push(tokio::task::spawn_blocking(move || {
                client_barrier.wait();
                client_request(listener, &request, Duration::from_secs(2))
            }));
        }
        let first = clients.remove(0).await.expect("cold client one");
        let second = clients.remove(0).await.expect("cold client two");
        shutdown.cancel();
        let server_result = task.await.expect("server task");
        (first, second, server_result)
    });
    let (first, second, server_result) = result;
    server_result.expect("server shutdown");
    assert_eq!(mock.requests(), 2, "cold misses must not singleflight");
    assert_eq!(response_id(&first), 0x2001);
    assert_eq!(response_id(&second), 0x2002);
    validate_response(&first).expect("cold response one");
    validate_response(&second).expect("cold response two");
    mock.stop();

    let mock = MockUpstream::start(false);
    let assembly = assembly_for(mock.address, HostOptions::default());
    let server = assembly
        .block_on(UdpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("UDP listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let result = assembly.block_on(async {
        let task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let warm = query(0x2100, "warm-concurrent.example", 1, 1, false);
        let _ = tokio::task::spawn_blocking(move || {
            client_request(listener, &warm, Duration::from_secs(2))
        })
        .await
        .expect("warm prefill");
        let mut clients = Vec::new();
        for id in [0x2101_u16, 0x2102] {
            let request = query(id, "warm-concurrent.example", 1, 1, false);
            clients.push(tokio::task::spawn_blocking(move || {
                client_request(listener, &request, Duration::from_secs(2))
            }));
        }
        let first = clients.remove(0).await.expect("warm client one");
        let second = clients.remove(0).await.expect("warm client two");
        shutdown.cancel();
        let server_result = task.await.expect("server task");
        (first, second, server_result)
    });
    let (first, second, server_result) = result;
    server_result.expect("server shutdown");
    assert_eq!(mock.requests(), 1, "warm concurrency must hit one entry");
    assert_eq!(response_id(&first), 0x2101);
    assert_eq!(response_id(&second), 0x2102);
    mock.stop();
}

#[test]
fn edns_and_non_in_queries_bypass_lookup_and_publication() {
    let mock = MockUpstream::start(false);
    let assembly = assembly_for(mock.address, HostOptions::default());
    let server = assembly
        .block_on(UdpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("UDP listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let result = assembly.block_on(async {
        let task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        for request in [
            query(0x3001, "bypass.example", 1, 1, false),
            query(0x3002, "bypass.example", 1, 1, true),
            query(0x3003, "bypass.example", 1, 1, true),
            query(0x3004, "bypass.example", 1, 3, false),
            query(0x3005, "bypass.example", 1, 3, false),
        ] {
            let response = tokio::task::spawn_blocking(move || {
                client_request(listener, &request, Duration::from_secs(2))
            })
            .await
            .expect("bypass client");
            validate_response(&response).expect("W1 bypass response");
        }
        shutdown.cancel();
        task.await.expect("server task")
    });
    result.expect("server shutdown");
    assert_eq!(mock.requests(), 5, "EDNS and non-IN must not publish");
    mock.stop();
}

#[test]
fn expiry_and_invalid_or_opt_responses_never_publish() {
    let mock = MockUpstream::start(false);
    let clock = CacheTestClock::new(500);
    let assembly = assembly_for(
        mock.address,
        HostOptions::default().with_cache_clock(Rc::new(clock.clone())),
    );
    let server = assembly
        .block_on(UdpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("UDP listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let result = assembly.block_on(async {
        let task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let request = query(0x4001, "expire.example", 1, 1, false);
        let response = tokio::task::spawn_blocking(move || {
            client_request(listener, &request, Duration::from_secs(2))
        })
        .await
        .expect("expiry warm");
        validate_response(&response).expect("expiry response");
        clock.advance(1);
        let request = query(0x4002, "expire.example", 1, 1, false);
        let response = tokio::task::spawn_blocking(move || {
            client_request(listener, &request, Duration::from_secs(2))
        })
        .await
        .expect("expiry miss");
        validate_response(&response).expect("expiry response after miss");

        for (index, name) in [
            "mismatch.example",
            "truncated.example",
            "opt.example",
            "malformed.example",
        ]
        .iter()
        .enumerate()
        {
            let offset = u16::try_from(index).expect("test index");
            let first_request = query(0x4100 + offset, name, 1, 1, false);
            let first = tokio::task::spawn_blocking(move || {
                client_request(listener, &first_request, Duration::from_secs(2))
            })
            .await
            .expect("invalid response one");
            let second_request = query(0x4200 + offset, name, 1, 1, false);
            let second = tokio::task::spawn_blocking(move || {
                client_request(listener, &second_request, Duration::from_secs(2))
            })
            .await
            .expect("invalid response two");
            assert_eq!(response_id(&first), 0x4100 + offset);
            assert_eq!(response_id(&second), 0x4200 + offset);
        }
        let first_request = query(0x4301, "servfail.example", 1, 1, false);
        let _ = tokio::task::spawn_blocking(move || {
            client_request(listener, &first_request, Duration::from_secs(2))
        })
        .await
        .expect("SERVFAIL one");
        let second_request = query(0x4302, "servfail.example", 1, 1, false);
        let second = tokio::task::spawn_blocking(move || {
            client_request(listener, &second_request, Duration::from_secs(2))
        })
        .await
        .expect("SERVFAIL two");
        shutdown.cancel();
        let server_result = task.await.expect("server task");
        (second, server_result)
    });
    let (second, server_result) = result;
    server_result.expect("server shutdown");
    assert_eq!(response_id(&second), 0x4302);
    assert_eq!(mock.requests(), 2 + 8 + 1, "only valid SERVFAIL is cached");
    mock.stop();
}

#[test]
fn shutdown_cancels_w2_request_without_publication_and_allows_rebind() {
    let blackhole = StdUdpSocket::bind("127.0.0.1:0").expect("blackhole bind");
    let assembly = assembly_for(
        blackhole.local_addr().expect("blackhole address"),
        HostOptions::with_deadline(Duration::from_secs(2)),
    );
    let server = assembly
        .block_on(UdpServer::bind(&assembly, "127.0.0.1:0".parse().unwrap()))
        .expect("UDP listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();
    let (received, server_result) = assembly.block_on(async {
        let task = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let client = tokio::task::spawn_blocking(move || {
            let socket = StdUdpSocket::bind("127.0.0.1:0").expect("client bind");
            socket
                .set_read_timeout(Some(Duration::from_millis(300)))
                .expect("client timeout");
            let request = query(0x5001, "blackhole.example", 1, 1, false);
            socket.send_to(&request, listener).expect("client send");
            let mut response = [0_u8; 512];
            socket.recv_from(&mut response).is_ok()
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        shutdown.cancel();
        let received = client.await.expect("cancelled client");
        let server_result = task.await.expect("server task");
        (received, server_result)
    });
    assert!(!received, "shutdown must suppress late response");
    server_result.expect("server shutdown");
    assert!(
        assembly.cache().is_empty(),
        "cancelled request must not publish"
    );
    let rebound = assembly
        .block_on(UdpServer::bind(&assembly, listener))
        .expect("W2 listener must rebind after shutdown");
    drop(rebound);
}

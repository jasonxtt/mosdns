//! S3 admission-semantics proof: real DNS datagrams and TCP frames capture
//! one immutable switch snapshot, keep it through branches and lazy
//! successors, managed apply drains and rebinds the latest values, and a
//! fatal post-rename ambiguity stops the host.

#![allow(clippy::too_many_lines)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::HostAssembly;

struct Root {
    path: PathBuf,
}

impl Root {
    fn new(name: &str) -> Self {
        static ID: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "switch-admit-{}-{name}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("test root");
        Self { path }
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// One UDP upstream that counts requests, answers with the configured final
/// address octet, and optionally holds each request until released.
struct CountingUpstream {
    address: SocketAddr,
    entered: Arc<AtomicUsize>,
    release: Option<mpsc::Sender<()>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl CountingUpstream {
    fn start(answer_octet: u8) -> Self {
        Self::start_with_gate(answer_octet, false, 60)
    }

    fn gated(answer_octet: u8) -> Self {
        Self::start_with_gate(answer_octet, true, 60)
    }

    fn start_with_gate(answer_octet: u8, gated: bool, ttl: u32) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("upstream bind");
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .expect("upstream timeout");
        let address = socket.local_addr().expect("upstream address");
        let entered = Arc::new(AtomicUsize::new(0));
        let thread_entered = Arc::clone(&entered);
        let (release, releases) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            let mut input = vec![0_u8; 65535];
            while !thread_stop.load(Ordering::SeqCst) {
                let Ok((length, peer)) = socket.recv_from(&mut input) else {
                    continue;
                };
                thread_entered.fetch_add(1, Ordering::SeqCst);
                if gated {
                    while releases.recv_timeout(Duration::from_millis(20)).is_err() {
                        if thread_stop.load(Ordering::SeqCst) {
                            return;
                        }
                    }
                }
                let response = upstream_response_with_ttl(&input[..length], answer_octet, ttl);
                socket.send_to(&response, peer).expect("upstream response");
            }
        });
        Self {
            address,
            entered,
            release: gated.then_some(release),
            stop,
            thread: Some(thread),
        }
    }

    fn release_one(&self) {
        if let Some(release) = &self.release {
            let _ = release.send(());
        }
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(release) = &self.release {
            let _ = release.send(());
        }
        if let Some(thread) = self.thread.take() {
            thread.join().expect("upstream join");
        }
    }
}

fn upstream_response_with_ttl(query: &[u8], answer_octet: u8, ttl: u32) -> Vec<u8> {
    let mut response = query.to_vec();
    response[2] |= 0x80; // QR
    response[2] |= 0x80; // RA
    response[3] = 0; // no error
    response[7] = 1; // one answer
    response.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
    response.extend_from_slice(&ttl.to_be_bytes());
    response.extend_from_slice(&[0x00, 0x04, 192, 0, 2, answer_octet]);
    response
}

fn query(id: u16, name: &str) -> Vec<u8> {
    let mut packet = vec![0_u8; 12];
    packet[0] = u8::try_from(id >> 8).expect("id high");
    packet[1] = u8::try_from(id & 0xff).expect("id low");
    packet[2] = 0x01; // recursion desired
    packet[5] = 1; // one question
    for label in name.split('.') {
        packet.push(u8::try_from(label.len()).expect("label"));
        packet.extend_from_slice(label.as_bytes());
    }
    packet.extend_from_slice(&[0, 0, 1, 0, 1]);
    packet
}

fn rcode(response: &[u8]) -> u8 {
    response[3] & 0x0f
}

fn client_udp(listener: SocketAddr, request: &[u8]) -> Vec<u8> {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("client bind");
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("client timeout");
    socket.send_to(request, listener).expect("client send");
    let mut response = vec![0_u8; 65535];
    let (length, _) = socket.recv_from(&mut response).expect("client response");
    response[..length].to_vec()
}

fn write_tcp_frame(stream: &mut TcpStream, body: &[u8]) {
    let length = u16::try_from(body.len()).expect("frame length");
    stream
        .write_all(&length.to_be_bytes())
        .expect("frame prefix");
    stream.write_all(body).expect("frame body");
    stream.flush().expect("frame flush");
}

fn read_tcp_frame(stream: &mut TcpStream) -> Vec<u8> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("frame timeout");
    let mut prefix = [0_u8; 2];
    stream.read_exact(&mut prefix).expect("frame prefix");
    let length = u16::from_be_bytes(prefix) as usize;
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).expect("frame body");
    body
}

async fn wait_for_count(counter: &AtomicUsize, count: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while counter.load(Ordering::SeqCst) < count {
        assert!(
            tokio::time::Instant::now() < deadline,
            "peer never reached {count} requests"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

async fn wait_for_gate(gate: &crate::managed::PersistGate) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !gate.arrived() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "held commit never reached the gate"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

async fn post_and_await(assembly: &HostAssembly, tag: &str, value: &str) {
    assembly
        .control()
        .view()
        .switches
        .post(tag, value.to_owned())
        .expect("mutation is accepted")
        .complete()
        .await
        .expect("mutation commits");
}

fn entry_config(root: &Root, peer: SocketAddr, state_file: &str, initial: &str) -> String {
    entry_config_transport(root, peer, state_file, initial, "udp")
}

fn entry_config_transport(
    root: &Root,
    peer: SocketAddr,
    state_file: &str,
    initial: &str,
    transport: &str,
) -> String {
    let file = root.path.join(state_file);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).expect("state parent");
    }
    std::fs::write(&file, initial).expect("state file");
    let server = if transport == "tcp" {
        "tcp_server"
    } else {
        "udp_server"
    };
    let idle_timeout = if transport == "tcp" {
        "\n      idle_timeout: 2"
    } else {
        ""
    };
    format!(
        "log:\n  level: error\n\nplugins:\n  - tag: fwd\n    type: forward\n    args:\n      \
         upstreams:\n        - addr: \"udp://{peer}\"\n  - tag: sw2\n    type: switch2\n    \
         args:\n      initial_value: \"{state_file}\"\n  - tag: entry\n    type: sequence\n    \
         args:\n      - matches: switch2 B\n        exec: reject\n      - exec: $fwd\n  - tag: \
         srv\n    type: {server}\n    args:\n      entry: entry{idle_timeout}\n      listen: \
         \"127.0.0.1:25390\"\n      enable_audit: false\n"
    )
}

/// Writes the YAML under the test root and assembles from the file, so
/// relative switch state files resolve against the root.
fn assemble_from(root: &Root, yaml: &str) -> HostAssembly {
    let path = root.path.join("config.yaml");
    std::fs::write(&path, yaml).expect("config file");
    HostAssembly::from_config_file(&path).expect("assembly from file")
}

type TransportCancellation = mosdns_upstream_core::TransportCancellation;

#[test]
fn an_old_admitted_udp_query_keeps_its_switch_snapshot_across_a_post() {
    let root = Root::new("udp-old-new");
    let peer = CountingUpstream::gated(1);
    let assembly = assemble_from(&root, &entry_config(&root, peer.address, "sw2.txt", "A"));
    let server = assembly
        .block_on(crate::UdpServer::bind(
            &assembly,
            "127.0.0.1:0".parse().unwrap(),
        ))
        .expect("listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();

    // The first query is admitted and forwarded before the switch changes.
    let first = query(1, "old.example");
    assembly.block_on(async {
        let serve = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let first = first.clone();
        let client = tokio::task::spawn_blocking(move || client_udp(listener, &first));
        wait_for_count(&peer.entered, 1).await;
        // The switch POST lands while the first query still awaits its
        // upstream response. Only future admissions see the new value.
        post_and_await(&assembly, "sw2", "B").await;
        peer.release_one();
        let first_response = client.await.expect("client join");
        // The old query kept its old snapshot: it was forwarded, not refused.
        assert_eq!(rcode(&first_response), 0, "old query is answered upstream");

        // The next datagram is admitted with the new value and rejected.
        let second = query(2, "new.example");
        let second_response = tokio::task::spawn_blocking(move || client_udp(listener, &second))
            .await
            .expect("second client join");
        assert_eq!(rcode(&second_response), 5, "new query sees the new value");

        shutdown.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(2), serve).await;
    });
    peer.stop();
}

#[test]
fn tcp_frames_on_one_connection_admit_independent_switch_snapshots() {
    let root = Root::new("tcp-frames");
    let peer = CountingUpstream::gated(2);
    let assembly = assemble_from(
        &root,
        &entry_config_transport(&root, peer.address, "sw2.txt", "A", "tcp"),
    );
    let server = assembly
        .block_on(crate::TcpServer::bind(
            &assembly,
            "127.0.0.1:0".parse().unwrap(),
        ))
        .expect("tcp bind");
    let listener = server.local_addr().expect("tcp listener");
    let shutdown = TransportCancellation::new();

    assembly.block_on(async {
        let serve = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let stream = Arc::new(std::sync::Mutex::new(
            tokio::task::spawn_blocking(move || {
                TcpStream::connect_timeout(&listener, Duration::from_secs(5)).expect("tcp connect")
            })
            .await
            .expect("tcp connect join"),
        ));

        // Frame one is admitted with the old value and held at the peer.
        let frame_one = query(3, "frame-one.example");
        let write_stream = Arc::clone(&stream);
        tokio::task::spawn_blocking(move || {
            write_tcp_frame(&mut write_stream.lock().expect("stream lock"), &frame_one);
        })
        .await
        .expect("frame write");
        wait_for_count(&peer.entered, 1).await;
        post_and_await(&assembly, "sw2", "B").await;
        peer.release_one();
        let read_stream = Arc::clone(&stream);
        let first = tokio::task::spawn_blocking(move || {
            read_tcp_frame(&mut read_stream.lock().expect("stream lock"))
        })
        .await
        .expect("first frame read");
        assert_eq!(rcode(&first), 0, "first frame keeps its old snapshot");

        // The second frame on the same connection admits the new value.
        let frame_two = query(4, "frame-two.example");
        let write_stream = Arc::clone(&stream);
        tokio::task::spawn_blocking(move || {
            write_tcp_frame(&mut write_stream.lock().expect("stream lock"), &frame_two);
        })
        .await
        .expect("second frame write");
        let read_stream = Arc::clone(&stream);
        let second = tokio::task::spawn_blocking(move || {
            read_tcp_frame(&mut read_stream.lock().expect("stream lock"))
        })
        .await
        .expect("second frame read");
        assert_eq!(rcode(&second), 5, "second frame sees the new value");

        drop(stream);
        shutdown.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(2), serve).await;
    });
    peer.stop();
}

#[test]
fn every_switch_type_routes_real_dns_on_its_admitted_value() {
    let root = Root::new("all17");
    for type_number in 1..=17u8 {
        for (value, expect_refused) in [("A", true), ("X", false)] {
            let peer = CountingUpstream::start(3);
            let file = format!("state{type_number}.txt");
            std::fs::write(root.path.join(&file), value).expect("state file");
            let yaml = format!(
                "log:\n  level: error\n\nplugins:\n  - tag: fwd\n    type: forward\n    \
                 args:\n      upstreams:\n        - addr: \"udp://{peer}\"\n  - tag: \
                 sw{type_number}\n    type: switch{type_number}\n    args:\n      \
                 initial_value: \"{file}\"\n  - tag: entry\n    type: sequence\n    args:\n      \
                 - matches: switch{type_number} A\n        exec: reject\n      - exec: $fwd\n  \
                 - tag: srv\n    type: udp_server\n    args:\n      entry: entry\n      listen: \
                 \"127.0.0.1:25390\"\n      enable_audit: false\n",
                peer = peer.address
            );
            let assembly = assemble_from(&root, &yaml);
            let server = assembly
                .block_on(crate::UdpServer::bind(
                    &assembly,
                    "127.0.0.1:0".parse().unwrap(),
                ))
                .expect("listener bind");
            let listener = server.local_addr().expect("listener address");
            let shutdown = TransportCancellation::new();
            let request = query(10, "route.example");
            let response = assembly.block_on(async {
                let serve = tokio::task::spawn_local(server.serve(shutdown.clone()));
                let response = tokio::task::spawn_blocking(move || client_udp(listener, &request))
                    .await
                    .expect("client join");
                shutdown.cancel();
                let _ = tokio::time::timeout(Duration::from_secs(2), serve).await;
                response
            });
            assert_eq!(
                rcode(&response),
                if expect_refused { 5 } else { 0 },
                "switch{type_number} value {value}"
            );
            peer.stop();
        }
    }
}

#[test]
fn a_query_local_fast_mark_only_satisfies_bit_backed_switches() {
    let root = Root::new("fast-mark");
    // switch3 owns bit 33: a query-local fast_mark satisfies `switch3 A`
    // even though the admitted value is B. The bitless switch15 never reads
    // a fast flag.
    for (type_number, file, matcher, expect_refused) in [
        (3_u8, "s3.txt", "switch3 A", true),
        (15_u8, "s15.txt", "switch15 A", false),
    ] {
        let peer = CountingUpstream::start(4);
        std::fs::write(root.path.join(file), "B").expect("state file");
        let yaml = format!(
            "log:\n  level: error\n\nplugins:\n  - tag: fwd\n    type: forward\n    args:\n      \
             upstreams:\n        - addr: \"udp://{peer}\"\n  - tag: sw\n    type: \
             switch{type_number}\n    args:\n      initial_value: \"{file}\"\n  - tag: entry\n  \
             \x20 type: sequence\n    args:\n      - exec: fast_mark 34\n      - matches: \
             {matcher}\n        exec: reject\n      - exec: $fwd\n  - tag: srv\n    type: \
             udp_server\n    args:\n      entry: entry\n      listen: \"127.0.0.1:25390\"\n      \
             enable_audit: false\n",
            peer = peer.address
        );
        let assembly = assemble_from(&root, &yaml);
        let server = assembly
            .block_on(crate::UdpServer::bind(
                &assembly,
                "127.0.0.1:0".parse().unwrap(),
            ))
            .expect("listener bind");
        let listener = server.local_addr().expect("listener address");
        let shutdown = TransportCancellation::new();
        let request = query(11, "mark.example");
        let response = assembly.block_on(async {
            let serve = tokio::task::spawn_local(server.serve(shutdown.clone()));
            let response = tokio::task::spawn_blocking(move || client_udp(listener, &request))
                .await
                .expect("client join");
            shutdown.cancel();
            let _ = tokio::time::timeout(Duration::from_secs(2), serve).await;
            response
        });
        assert_eq!(
            rcode(&response),
            if expect_refused { 5 } else { 0 },
            "type {type_number} fast_mark 34"
        );
        peer.stop();
    }
}

#[test]
fn admission_values_survive_redirect_and_sequence_branches() {
    let root = Root::new("branches");
    for branch in ["redirect", "jump"] {
        let peer = CountingUpstream::start(6);
        std::fs::write(root.path.join("sw2.txt"), "B").expect("state file");
        let entry_args = if branch == "redirect" {
            "      - exec: $rewrite\n      - matches: switch2 B\n        exec: reject\n      - \
             exec: $fwd"
        } else {
            "      - exec: jump $child\n      - matches: switch2 B\n        exec: reject\n      \
             - exec: $fwd"
        };
        let child = if branch == "jump" {
            "  - tag: child\n    type: sequence\n    args:\n      - matches: switch2 B\n        \
             exec: reject\n"
        } else {
            ""
        };
        let redirect = if branch == "redirect" {
            "  - tag: rewrite\n    type: redirect\n    args:\n      rules:\n        - \
             'branch.example target.example'\n"
        } else {
            ""
        };
        let yaml = format!(
            "log:\n  level: error\n\nplugins:\n  - tag: fwd\n    type: forward\n    args:\n      \
             upstreams:\n        - addr: \"udp://{peer}\"\n{redirect}  - tag: sw2\n    type: \
             switch2\n    args:\n      initial_value: \"sw2.txt\"\n{child}  - tag: entry\n    \
             type: sequence\n    args:\n{entry_args}\n  - tag: srv\n    type: udp_server\n    \
             args:\n      entry: entry\n      listen: \"127.0.0.1:25390\"\n      enable_audit: \
             false\n",
            peer = peer.address
        );
        let assembly = assemble_from(&root, &yaml);
        let server = assembly
            .block_on(crate::UdpServer::bind(
                &assembly,
                "127.0.0.1:0".parse().unwrap(),
            ))
            .expect("listener bind");
        let listener = server.local_addr().expect("listener address");
        let shutdown = TransportCancellation::new();
        let request = query(12, "branch.example");
        let response = assembly.block_on(async {
            let serve = tokio::task::spawn_local(server.serve(shutdown.clone()));
            let response = tokio::task::spawn_blocking(move || client_udp(listener, &request))
                .await
                .expect("client join");
            shutdown.cancel();
            let _ = tokio::time::timeout(Duration::from_secs(2), serve).await;
            response
        });
        assert_eq!(
            rcode(&response),
            5,
            "the admitted value reaches the {branch} branch"
        );
        peer.stop();
    }
}

#[test]
fn lazy_refresh_publishes_old_policy_while_new_queries_use_new_values() {
    let root = Root::new("lazy-refresh");
    // The cached policy is observable through which upstream answered: the
    // entry routes to `a` when switch2 is not B and to `b` when it is.
    let peer_a = CountingUpstream::start_with_gate(11, true, 1);
    let peer_b = CountingUpstream::start(22);
    std::fs::write(root.path.join("sw2.txt"), "A").expect("state file");
    let yaml = format!(
        "log:\n  level: error\n\nplugins:\n  - tag: fwd_a\n    type: forward\n    args:\n      \
         upstreams:\n        - addr: \"udp://{a}\"\n  - tag: fwd_b\n    type: forward\n    \
         args:\n      upstreams:\n        - addr: \"udp://{b}\"\n  - tag: c1\n    type: \
         cache\n    args:\n      size: 64\n      lazy_cache_ttl: 60\n  - tag: sw2\n    type: \
         switch2\n    args:\n      initial_value: \"sw2.txt\"\n  - tag: entry\n    type: \
         sequence\n    args:\n      - exec: $c1\n      - matches: switch2 B\n        exec: \
         $fwd_b\n      - matches: '!has_resp'\n        exec: $fwd_a\n  - tag: srv\n    type: \
         udp_server\n    args:\n      \
         entry: entry\n      listen: \"127.0.0.1:25390\"\n      enable_audit: true\n",
        a = peer_a.address,
        b = peer_b.address
    );
    let assembly = assemble_from(&root, &yaml);
    let server = assembly
        .block_on(crate::UdpServer::bind(
            &assembly,
            "127.0.0.1:0".parse().unwrap(),
        ))
        .expect("listener bind");
    let listener = server.local_addr().expect("listener address");
    let shutdown = TransportCancellation::new();

    assembly.block_on(async {
        let serve = tokio::task::spawn_local(server.serve(shutdown.clone()));

        // Miss under the old value: peer A answers and the result is cached.
        let cached_name = "cached.example";
        let request = query(20, cached_name);
        let first = tokio::task::spawn_blocking(move || client_udp(listener, &request));
        wait_for_count(&peer_a.entered, 1).await;
        peer_a.release_one();
        let first_response = first.await.expect("first client join");
        assert_eq!(rcode(&first_response), 0);
        assert_eq!(&first_response[first_response.len() - 1..], &[11]);

        // Let the entry cross from fresh into its lazy window.
        tokio::time::sleep(Duration::from_millis(1200)).await;

        // The repeat is a lazy hit: the old-policy answer is served and the
        // captured successor refresh starts toward peer A, where it parks.
        let request = query(21, cached_name);
        let second = tokio::task::spawn_blocking(move || client_udp(listener, &request))
            .await
            .expect("second client join");
        assert_eq!(rcode(&second), 0);
        assert_eq!(
            &second[second.len() - 1..],
            &[11],
            "lazy hit keeps old policy"
        );
        wait_for_count(&peer_a.entered, 2).await;

        // The switch flips while the refresh is still parked at peer A.
        post_and_await(&assembly, "sw2", "B").await;
        // The refresh captured before the POST completes against its captured
        // routing and publishes normally; no switch fence cancels it.
        peer_a.release_one();
        tokio::time::sleep(Duration::from_millis(200)).await;

        // A fresh, uncached name admits the new value and routes to peer B.
        let request = query(22, "fresh.example");
        let fresh = tokio::task::spawn_blocking(move || client_udp(listener, &request))
            .await
            .expect("fresh client join");
        assert_eq!(rcode(&fresh), 0);
        assert_eq!(&fresh[fresh.len() - 1..], &[22], "new query uses new state");
        assert_eq!(peer_b.entered.load(Ordering::SeqCst), 1);
        assert_eq!(
            peer_a.entered.load(Ordering::SeqCst),
            2,
            "the refresh kept its captured old-policy routing"
        );

        shutdown.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(2), serve).await;
    });
    peer_a.stop();
    peer_b.stop();
}

#[test]
fn post_flips_between_a_and_non_a_reseed_real_dns_admissions() {
    // Real DNS proof that publication updates both the aggregate and the
    // A fast bits: `switch2 A` matches after an A POST and stops matching
    // after a non-A POST.
    for (initial, after_post, expect_refused_after) in [("B", "A", true), ("A", "B", false)] {
        let root = Root::new("a-flip-dns");
        let peer = CountingUpstream::start(7);
        let yaml = entry_config_a_matcher(&root, peer.address, "sw2.txt", initial);
        let assembly = assemble_from(&root, &yaml);
        let server = assembly
            .block_on(crate::UdpServer::bind(
                &assembly,
                "127.0.0.1:0".parse().unwrap(),
            ))
            .expect("listener bind");
        let listener = server.local_addr().expect("listener address");
        let shutdown = TransportCancellation::new();

        let (initial_response, after) = assembly.block_on(async {
            let serve = tokio::task::spawn_local(server.serve(shutdown.clone()));
            let request = query(30, "flip.example");
            let initial_response =
                tokio::task::spawn_blocking(move || client_udp(listener, &request))
                    .await
                    .expect("initial client join");
            post_and_await(&assembly, "sw2", after_post).await;
            let request = query(31, "flip.example");
            let after = tokio::task::spawn_blocking(move || client_udp(listener, &request))
                .await
                .expect("second client join");
            shutdown.cancel();
            let _ = tokio::time::timeout(Duration::from_secs(2), serve).await;
            (initial_response, after)
        });
        assert_eq!(
            rcode(&initial_response),
            if expect_refused_after { 0 } else { 5 },
            "initial value {initial} routes before the POST"
        );
        assert_eq!(
            rcode(&after),
            if expect_refused_after { 5 } else { 0 },
            "value {after_post} must reseed both the aggregate and the A bits"
        );
        peer.stop();
    }
}

/// The `switch2 A` variant of the entry config.
fn entry_config_a_matcher(
    root: &Root,
    peer: SocketAddr,
    state_file: &str,
    initial: &str,
) -> String {
    let yaml = entry_config(root, peer, state_file, initial);
    yaml.replace("matches: switch2 B", "matches: switch2 A")
}

#[test]
fn a_changed_owner_identity_on_the_same_path_re_admits_after_the_drain() {
    // P1-3 race: an old owner's held POST commits to the shared state file
    // while a candidate changes the owning identity on that same path. The
    // candidate must admit the file's post-drain state, not a stale
    // preparation-time read.
    let yaml = "log: {level: error}
native_management: {special_groups: true}
plugins:
  - tag: default_upstream
    type: forward
    args: {upstreams: [{addr: \"udp://127.0.0.1:25458\"}]}
  - tag: sw2
    type: switch2
    args: {initial_value: \"shared.txt\"}
  - tag: main_entry
    type: sequence
    args:
      - exec: $special_upstream_matcher
      - exec: $default_upstream
  - tag: main_udp
    type: udp_server
    args: {entry: main_entry, listen: \"127.0.0.1:25358\", enable_audit: false}
";
    let (assembly, root) = managed_fixture(yaml, &[("shared.txt", "X")]);
    let control = assembly.control();

    // Hold a POST on the old owner (sw2, shared.txt) across the apply.
    let gate = crate::managed::PersistGate::new();
    control.view().switches.inject_commit_hold(gate.clone());
    let config_path = root.path.join("config.yaml");
    assembly.block_on(async {
        let ticket = control
            .view()
            .switches
            .post("sw2", "Y".to_owned())
            .expect("post is accepted");
        wait_for_gate(&gate).await;

        // The candidate changes both the type and the tag while keeping the
        // same state-file path.
        let replacement = std::fs::read_to_string(&config_path)
            .unwrap()
            .replace("tag: sw2", "tag: sw3")
            .replace("type: switch2", "type: switch3");
        std::fs::write(&config_path, &replacement).expect("stage replacement config");
        let candidate = control
            .compile_managed_candidate(&config_path, Vec::new())
            .expect("candidate compiles");
        let control_task = control.clone();
        let applied =
            tokio::task::spawn_local(async move { control_task.apply_candidate(candidate).await });
        tokio::time::sleep(Duration::from_millis(50)).await;

        // The held old-owner POST commits Y durably before the drain
        // completes.
        gate.release();
        ticket.complete().await.expect("post completed");
        applied.await.expect("apply task").expect("apply succeeds");

        // The changed identity was admitted after the drain: its value and
        // fingerprint match the file the old owner just committed, so a
        // subsequent write succeeds instead of conflicting.
        let view = control.view();
        assert_eq!(view.switches.show("sw3").as_deref(), Some("Y"));
        assert_eq!(
            std::fs::read(root.path.join("shared.txt")).expect("file"),
            b"Y".to_vec()
        );
        let ticket = view
            .switches
            .post("sw3", "Z".to_owned())
            .expect("the changed owner's fingerprint matches the post-drain file");
        ticket.complete().await.expect("subsequent commit");
        assert_eq!(
            std::fs::read(root.path.join("shared.txt")).expect("file"),
            b"Z".to_vec()
        );
    });
}

fn managed_fixture(yaml: &str, state_files: &[(&str, &str)]) -> (HostAssembly, Root) {
    let root = Root::new("managed");
    std::fs::create_dir_all(root.path.join("webinfo")).expect("webinfo");
    std::fs::create_dir_all(root.path.join("cache")).expect("cache");
    for (name, value) in state_files {
        std::fs::write(root.path.join(name), value).expect("state file");
    }
    let config_path = root.path.join("config.yaml");
    std::fs::write(&config_path, yaml).expect("config");
    std::fs::write(
        root.path.join("webinfo/special_upstream_groups.json"),
        b"[]\n",
    )
    .unwrap();
    std::fs::write(root.path.join("webinfo/upstream_overrides.json"), b"{}\n").unwrap();
    (
        HostAssembly::from_config_file(&config_path).expect("assembly"),
        root,
    )
}

#[test]
fn managed_apply_drains_a_held_switch_post_and_rebinds_the_latest_value() {
    let yaml = "log: {level: error}
native_management: {special_groups: true}
plugins:
  - tag: default_upstream
    type: forward
    args: {upstreams: [{addr: \"udp://127.0.0.1:25455\"}]}
  - tag: sw2
    type: switch2
    args: {initial_value: \"sw2.txt\"}
  - tag: main_entry
    type: sequence
    args:
      - exec: $special_upstream_matcher
      - exec: $default_upstream
  - tag: main_udp
    type: udp_server
    args: {entry: main_entry, listen: \"127.0.0.1:25355\", enable_audit: false}
";
    let (assembly, root) = managed_fixture(yaml, &[("sw2.txt", "A")]);
    let control = assembly.control();

    let gate = crate::managed::PersistGate::new();
    control.view().switches.inject_commit_hold(gate.clone());
    let generation_before = control.generation();
    let generation = assembly.block_on(async {
        let ticket = control
            .view()
            .switches
            .post("sw2", "B".to_owned())
            .expect("post is accepted");
        wait_for_gate(&gate).await;

        // The apply pauses admission and waits for the accepted mutation's
        // durable completion before publishing the new generation.
        let candidate = control
            .compile_managed_candidate(&root.path.join("config.yaml"), Vec::new())
            .expect("candidate compiles");
        let control_task = control.clone();
        let applied =
            tokio::task::spawn_local(async move { control_task.apply_candidate(candidate).await });
        // Yield so the apply reaches its admission pause and management
        // drain before the held POST is released.
        tokio::time::sleep(Duration::from_millis(50)).await;
        gate.release();
        ticket.complete().await.expect("post completed");
        let generation = applied.await.expect("apply task").expect("apply succeeds");
        // The candidate has the same switch identity, so the owner carried
        // over and the new view publishes its latest value.
        assert_eq!(control.view().switches.show("sw2").as_deref(), Some("B"));
        generation
    });
    assert!(generation > generation_before);
    assert_eq!(
        std::fs::read(root.path.join("sw2.txt")).expect("state file"),
        b"B".to_vec()
    );
}

#[test]
fn a_stale_same_tag_different_path_write_cannot_reach_a_new_owner() {
    let yaml = "log: {level: error}
native_management: {special_groups: true}
plugins:
  - tag: default_upstream
    type: forward
    args: {upstreams: [{addr: \"udp://127.0.0.1:25456\"}]}
  - tag: sw2
    type: switch2
    args: {initial_value: \"old.txt\"}
  - tag: main_entry
    type: sequence
    args:
      - exec: $special_upstream_matcher
      - exec: $default_upstream
  - tag: main_udp
    type: udp_server
    args: {entry: main_entry, listen: \"127.0.0.1:25356\", enable_audit: false}
";
    let (assembly, root) = managed_fixture(yaml, &[("old.txt", "A"), ("new.txt", "C")]);
    let control = assembly.control();

    // Hold a write to the old owner's file across the managed swap.
    let gate = crate::managed::PersistGate::new();
    control.view().switches.inject_commit_hold(gate.clone());
    let config_path = root.path.join("config.yaml");
    assembly.block_on(async {
        let ticket = control
            .view()
            .switches
            .post("sw2", "B".to_owned())
            .expect("post is accepted");
        wait_for_gate(&gate).await;

        // The candidate moves the same tag to a new state file. The main
        // configuration is an ordinary disk input to candidate compilation.
        let replacement = std::fs::read_to_string(&config_path)
            .unwrap()
            .replace("old.txt", "new.txt");
        std::fs::write(&config_path, &replacement).expect("stage replacement config");
        let candidate = control
            .compile_managed_candidate(&config_path, Vec::new())
            .expect("candidate compiles");
        let control_task = control.clone();
        let applied =
            tokio::task::spawn_local(async move { control_task.apply_candidate(candidate).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        gate.release();
        ticket.complete().await.expect("post completed");
        applied.await.expect("apply task").expect("apply succeeds");

        // The late writer finished against its own captured file only; the
        // new owner's file and value are untouched.
        assert_eq!(
            std::fs::read(root.path.join("old.txt")).expect("old file"),
            b"B".to_vec(),
            "the lease-bound old mutation wrote its captured target"
        );
        assert_eq!(
            std::fs::read(root.path.join("new.txt")).expect("new file"),
            b"C".to_vec(),
            "the new owner's file is unreachable by the stale writer"
        );
        assert_eq!(control.view().switches.show("sw2").as_deref(), Some("C"));
    });
}

#[test]
fn a_fatal_post_rename_ambiguity_fences_admission_and_stops_the_host() {
    let root = Root::new("fatal");
    let assembly = assemble_from(
        &root,
        &entry_config(&root, "127.0.0.1:25457".parse().unwrap(), "sw2.txt", "A"),
    );
    let control = assembly.control();
    assert!(control.admission_open());
    let view = control.view();
    view.switches
        .inject_fault(crate::switch_state::SwitchCommitFault::DirSync);
    let error = assembly.block_on(async {
        view.switches
            .post("sw2", "B".to_owned())
            .expect("accepted")
            .complete()
            .await
            .expect_err("fatal ambiguity")
    });
    assert!(
        matches!(
            error,
            crate::switch_state::SwitchMutationError::RecoveryRequired
        ),
        "{error:?}"
    );
    assert!(control.recovery_required());
    assert!(!control.admission_open(), "admission is fenced");
    assert_eq!(control.lifecycle_state(), "recovery_required");
    // The final file is preserved with its new, complete content.
    assert_eq!(
        std::fs::read(root.path.join("sw2.txt")).expect("final file"),
        b"B".to_vec()
    );
}

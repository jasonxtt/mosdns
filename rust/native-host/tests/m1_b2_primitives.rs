//! M1-b2: the request-local primitives `rcode`, `tag_setter`, `drop_resp` and
//! `sleep`, driven through a real UDP listener, a real upstream socket, and the
//! audit ring, plus a bare-tag `fallback`.

use std::net::{SocketAddr, UdpSocket as StdUdpSocket};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mosdns_native_host::{HostAssembly, QueryTerminalOutcome};
use mosdns_upstream_core::TransportCancellation;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("m1-b2-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn free_port() -> u16 {
    StdUdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn wire_name(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for label in name.trim_end_matches('.').split('.') {
        out.push(u8::try_from(label.len()).unwrap());
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    out
}

fn query(id: u16, name: &str) -> Vec<u8> {
    let mut packet = vec![
        (id >> 8) as u8,
        (id & 0xff) as u8,
        0x01,
        0x00,
        0,
        1,
        0,
        0,
        0,
        0,
        0,
        0,
    ];
    packet.extend_from_slice(&wire_name(name));
    packet.extend_from_slice(&[0, 1, 0, 1]);
    packet
}

/// The upstream the sequences forward to. The first label of the query name
/// picks the answer: `nx` NXDOMAIN, `servfail` SERVFAIL, `badvers` NOERROR in
/// the header but extended RCODE 1 in an OPT record (full code 16), anything
/// else NOERROR with one A record.
struct Upstream {
    address: SocketAddr,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Upstream {
    fn start() -> Self {
        let socket = StdUdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let address = socket.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen, stopped) = (Arc::clone(&requests), Arc::clone(&stop));
        let thread = thread::spawn(move || {
            let mut packet = vec![0_u8; 4096];
            while !stopped.load(Ordering::SeqCst) {
                let Ok((length, client)) = socket.recv_from(&mut packet) else {
                    continue;
                };
                seen.fetch_add(1, Ordering::SeqCst);
                socket
                    .send_to(&upstream_answer(&packet[..length]), client)
                    .unwrap();
            }
        });
        Self {
            address,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    fn seen(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn upstream_answer(query: &[u8]) -> Vec<u8> {
    let name_end = 12 + query[12..].iter().position(|byte| *byte == 0).unwrap() + 1;
    let first_label = &query[13..13 + usize::from(query[12])];
    let question = &query[12..name_end + 4];
    let mut response = vec![query[0], query[1], 0x81, 0x80, 0, 1, 0, 0, 0, 0, 0, 0];
    response.extend_from_slice(question);
    match first_label {
        b"nx" => response[3] |= 3,
        b"servfail" => response[3] |= 2,
        b"badvers" => {
            response[11] = 1;
            response.extend_from_slice(&[0, 0, 41, 0x10, 0, 1, 0, 0, 0, 0, 0]);
        }
        _ => {
            response[7] = 1;
            response.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 192, 0, 2, 44]);
        }
    }
    response
}

async fn exchange(target: SocketAddr, packet: &[u8], wait: Duration) -> Option<Vec<u8>> {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.send_to(packet, target).await.unwrap();
    let mut response = vec![0; 4096];
    let (length, _) = tokio::time::timeout(wait, socket.recv_from(&mut response))
        .await
        .ok()?
        .unwrap();
    response.truncate(length);
    Some(response)
}

fn rcode(response: &[u8]) -> u8 {
    response[3] & 0x0f
}

fn ancount(response: &[u8]) -> u16 {
    u16::from_be_bytes([response[6], response[7]])
}

fn config(upstream: SocketAddr, port: u16, sequence: &str, extra: &str) -> String {
    format!(
        "log: {{ level: error }}\nplugins:\n  - tag: forward\n    type: forward\n    args: {{ upstreams: [{{ addr: \"udp://{upstream}\" }}] }}\n{extra}  - tag: entry\n    type: sequence\n    args:\n{sequence}  - tag: listener\n    type: udp_server\n    args: {{ entry: entry, listen: \"127.0.0.1:{port}\", enable_audit: true }}\n"
    )
}

fn assemble(dir: &TempDir, yaml: &str) -> HostAssembly {
    let path = dir.0.join("config.yaml");
    std::fs::write(&path, yaml).unwrap();
    HostAssembly::from_config_file(&path).expect("assembles")
}

/// The production shapes in one sequence: `sleep`, `rcode` + `resp_ip`,
/// `drop_resp`, `tag_setter` (set, overwrite, clear), `reject`. As in the
/// production sub-configs, `reject` inside an exec list only leaves that
/// list, so a marked rule ends the request with a later `exit`.
const SEQUENCE: &str = "      - matches: qname full:sleepy.test\n        exec: sleep 150\n      - matches: qname full:clear.test\n        exec:\n          - tag_setter temporary\n          - tag_setter\n          - $forward\n          - exit\n      - matches: qname full:direct.test\n        exec:\n          - $forward\n          - tag_setter 指定客户端直连\n          - exit\n      - exec: $forward\n      - matches: rcode 16\n        exec:\n          - tag_setter badvers\n          - exit\n      - matches:\n          - rcode 0 3\n          - \"!resp_ip 0.0.0.0/0\"\n        exec: fast_mark 20\n      - matches: fast_mark 20\n        exec:\n          - tag_setter noip\n          - drop_resp\n          - reject 0\n      - matches: fast_mark 20\n        exec: exit\n      - matches: \"!rcode 0 3\"\n        exec:\n          - tag_setter failed\n          - drop_resp\n          - exit\n      - exec: tag_setter answered\n";

#[test]
#[allow(clippy::too_many_lines)] // one scenario table over one running host
fn real_queries_run_rcode_tag_setter_drop_resp_and_sleep() {
    let upstream = Upstream::start();
    let port = free_port();
    let dir = TempDir::new("udp");
    let host = assemble(&dir, &config(upstream.address, port, SEQUENCE, ""));
    host.block_on(async {
        let bound = host.bind_host().await.expect("binds");
        let shutdown = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let target: SocketAddr = ([127, 0, 0, 1], port).into();
        let wait = Duration::from_secs(3);

        // (name, rcode, answers, audit domain_set)
        let cases: &[(&str, u8, u16, Option<&str>)] = &[
            // Plain answer: `rcode 0 3` is true but `!resp_ip` is not.
            ("ok.test", 0, 1, Some("answered")),
            // NXDOMAIN without an address: `rcode 3` and `!resp_ip` match;
            // drop_resp + `reject 0` turn it into an empty NOERROR.
            ("nx.test", 0, 0, Some("noip")),
            // SERVFAIL: `!rcode 0 3`; `drop_resp` then `exit` leaves no
            // response at all, which the listener answers with REFUSED.
            ("servfail.test", 5, 0, Some("failed")),
            // NOERROR in the header but BADVERS (16) through the OPT record.
            ("badvers.test", 0, 0, Some("badvers")),
            // A tag set and then cleared before the forward: nothing remains.
            ("clear.test", 0, 1, None),
            // A tag set after the forward answered (the production pattern).
            ("direct.test", 0, 1, Some("指定客户端直连")),
        ];
        for (index, (name, expected_rcode, answers, _)) in cases.iter().enumerate() {
            let id = u16::try_from(index + 1).unwrap();
            let response = exchange(target, &query(id, name), wait)
                .await
                .unwrap_or_else(|| panic!("{name} must be answered"));
            assert_eq!(rcode(&response), *expected_rcode, "{name}");
            assert_eq!(ancount(&response), *answers, "{name}");
        }

        // `sleep 150` delays the answer by at least that long.
        let started = Instant::now();
        let response = exchange(target, &query(100, "sleepy.test"), wait)
            .await
            .expect("sleepy.test is answered");
        assert!(started.elapsed() >= Duration::from_millis(150));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!((rcode(&response), ancount(&response)), (0, 1));

        let records = host.audit_snapshot().records;
        assert_eq!(records.len(), cases.len() + 1);
        for (record, (name, _, _, expected_set)) in records.iter().zip(cases) {
            assert_eq!(record.qname.trim_end_matches('.'), *name);
            assert_eq!(
                record.domain_set.as_deref(),
                *expected_set,
                "{name}: audit domain_set"
            );
        }
        // The production label also reaches the effective tag.
        assert_eq!(
            records[5].effective_tag.as_deref(),
            Some("指定客户端直连"),
            "direct.test effective_tag"
        );
        // A dropped response leaves no upstream behind: the REFUSED the
        // entry sends is local, and neither a final nor a selected upstream
        // is reported for it.
        let refused = &records[2];
        assert_eq!(
            refused.response,
            mosdns_native_host::ResponseState::Dns {
                rcode: 5,
                source: mosdns_native_host::ResponseSource::Local
            },
            "{refused:?}"
        );
        assert!(refused.final_upstream.is_none(), "{refused:?}");
        assert!(refused.selected_upstream.is_none(), "{refused:?}");
        assert!(
            refused
                .upstream_diagnostics
                .as_ref()
                .is_none_or(|diagnostics| diagnostics.selected.is_none()),
            "{refused:?}"
        );
        // The query did reach the upstream, and that attempt stays on record.
        assert_eq!(refused.upstream_attempts.len(), 1, "{refused:?}");
        // `drop_resp` followed by `reject 0` in the same exec list: the final
        // NOERROR is local too, with no supplier or selected endpoint.
        let emptied = &records[1];
        assert_eq!(
            emptied.response,
            mosdns_native_host::ResponseState::Dns {
                rcode: 0,
                source: mosdns_native_host::ResponseSource::Local
            },
            "{emptied:?}"
        );
        assert!(emptied.final_upstream.is_none(), "{emptied:?}");
        assert!(emptied.selected_upstream.is_none(), "{emptied:?}");
        // A cleared tag leaves nothing to derive an effective tag from.
        assert_eq!(records[4].domain_set, None);

        shutdown.cancel();
        serving.await.unwrap().expect("clean shutdown");
    });
    // 6 table queries + sleepy, each forwarded once.
    assert_eq!(upstream.seen(), 7);
}

#[test]
fn a_query_sleeping_at_shutdown_is_released_promptly_without_forwarding() {
    let upstream = Upstream::start();
    let port = free_port();
    let dir = TempDir::new("cancel");
    let host = assemble(
        &dir,
        &config(
            upstream.address,
            port,
            "      - exec: sleep 60000\n      - exec: $forward\n",
            "",
        ),
    );
    host.block_on(async {
        let bound = host.bind_host().await.expect("binds");
        let shutdown = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let target: SocketAddr = ([127, 0, 0, 1], port).into();

        // The query is admitted and sleeps; nothing answers it.
        assert!(
            exchange(target, &query(1, "ok.test"), Duration::from_millis(300))
                .await
                .is_none()
        );
        let started = Instant::now();
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), serving)
            .await
            .expect("shutdown does not wait out the sleep")
            .unwrap()
            .expect("clean shutdown");
        assert!(started.elapsed() < Duration::from_secs(2));

        let records = host.audit_snapshot().records;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].terminal_outcome, QueryTerminalOutcome::Canceled);
    });
    assert_eq!(upstream.seen(), 0, "the sleeping query never forwarded");
}

#[test]
fn a_sleep_past_the_request_budget_answers_servfail() {
    // The default budget is 5 s; a 60 s sleep ends at the budget, the entry
    // answers SERVFAIL, and the forward after it is never reached.
    let upstream = Upstream::start();
    let port = free_port();
    let dir = TempDir::new("budget");
    let host = assemble(
        &dir,
        &config(
            upstream.address,
            port,
            "      - exec: sleep 60000\n      - exec: $forward\n",
            "",
        ),
    );
    host.block_on(async {
        let bound = host.bind_host().await.expect("binds");
        let shutdown = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let target: SocketAddr = ([127, 0, 0, 1], port).into();
        let started = Instant::now();
        let response = exchange(target, &query(1, "ok.test"), Duration::from_secs(9))
            .await
            .expect("answered at the budget");
        assert_eq!(rcode(&response), 2, "SERVFAIL");
        assert!(started.elapsed() >= Duration::from_secs(4));
        assert!(started.elapsed() < Duration::from_secs(8));
        shutdown.cancel();
        serving.await.unwrap().expect("clean shutdown");
    });
    assert_eq!(upstream.seen(), 0);
}

#[test]
fn a_fallback_names_its_targets_bare_or_with_a_dollar() {
    for (primary, secondary) in [("primary", "secondary"), ("$primary", "$secondary")] {
        let upstream = Upstream::start();
        let port = free_port();
        let dir = TempDir::new("fallback");
        let extra = format!(
            "  - tag: primary\n    type: sequence\n    args: [ {{ exec: sleep 400 }}, {{ exec: \"$forward\" }} ]\n  - tag: secondary\n    type: sequence\n    args: [ {{ exec: \"$forward\" }}, {{ exec: tag_setter from-secondary }} ]\n  - tag: fb\n    type: fallback\n    args: {{ primary: \"{primary}\", secondary: \"{secondary}\", threshold: 50 }}\n"
        );
        let host = assemble(
            &dir,
            &config(upstream.address, port, "      - exec: $fb\n", &extra),
        );
        host.block_on(async {
            let bound = host.bind_host().await.expect("binds");
            let shutdown = TransportCancellation::new();
            let serving = tokio::task::spawn_local(bound.serve(shutdown.clone()));
            let target: SocketAddr = ([127, 0, 0, 1], port).into();
            let started = Instant::now();
            let response = exchange(target, &query(1, "ok.test"), Duration::from_secs(3))
                .await
                .expect("answered");
            // The slow primary is overtaken by the secondary after 50 ms.
            assert_eq!((rcode(&response), ancount(&response)), (0, 1));
            assert!(started.elapsed() < Duration::from_millis(350));
            let records = host.audit_snapshot().records;
            assert_eq!(records[0].domain_set.as_deref(), Some("from-secondary"));
            shutdown.cancel();
            serving.await.unwrap().expect("clean shutdown");
        });
    }
}

/// A label set after the response was committed must reach the audit even when
/// something dispatches in between: a `sleep`, a cache scope boundary (the
/// production `$sequence_local` is a cached child), or a response policy.
#[test]
fn a_tag_set_after_the_response_reaches_the_audit_across_dispatches() {
    let upstream = Upstream::start();
    let port = free_port();
    let dir = TempDir::new("late-tag");
    let extra = "  - tag: cached_child\n    type: sequence\n    args: [ { exec: $cache_x }, { exec: $forward } ]\n  - tag: cache_x\n    type: cache\n    args: { size: 64 }\n";
    let sequence = "      - matches: qname full:sleep.test\n        exec:\n          - $forward\n          - sleep 10\n          - tag_setter after-sleep\n          - exit\n      - matches: qname full:cached.test\n        exec:\n          - $cached_child\n          - tag_setter after-cache\n          - exit\n      - matches: qname full:policy.test\n        exec:\n          - $forward\n          - ttl 30\n          - tag_setter after-policy\n          - exit\n      - matches: qname full:flow.test\n        exec:\n          - $forward\n          - sleep 10\n          - flow_setter upstream=late-upstream\n          - exit\n      - exec: $forward\n";
    let host = assemble(&dir, &config(upstream.address, port, sequence, extra));
    host.block_on(async {
        let bound = host.bind_host().await.expect("binds");
        let shutdown = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let target: SocketAddr = ([127, 0, 0, 1], port).into();
        // cached.test twice: a cache miss, then a cache hit.
        for (id, name) in [
            (1, "sleep.test"),
            (2, "cached.test"),
            (3, "cached.test"),
            (4, "policy.test"),
            (5, "flow.test"),
        ] {
            let response = exchange(target, &query(id, name), Duration::from_secs(3))
                .await
                .expect("answered");
            assert_eq!(ancount(&response), 1, "{name}");
        }
        let records = host.audit_snapshot().records;
        let sets: Vec<_> = records
            .iter()
            .map(|record| record.domain_set.as_deref())
            .collect();
        assert_eq!(
            sets,
            [
                Some("after-sleep"),
                Some("after-cache"),
                Some("after-cache"),
                Some("after-policy"),
                None
            ],
            "{records:#?}"
        );
        assert_eq!(
            records[4].final_upstream.as_deref(),
            Some("late-upstream"),
            "a flow_setter after a dispatch also reaches the audit"
        );
        shutdown.cancel();
        serving.await.unwrap().expect("clean shutdown");
    });
}

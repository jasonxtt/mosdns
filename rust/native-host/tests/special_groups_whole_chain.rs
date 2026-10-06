//! Slice 7 whole-chain proof for ordered managed routing, per-group listeners,
//! cache invalidation, and public audit provenance.
//!
//! Every peer and listener binds an ephemeral high loopback port. The test
//! drives real UDP DNS, a TCP upstream, management HTTP, and audit HTTP.

use std::cell::Cell;
use std::fs;
use std::net::{SocketAddr, TcpListener as StdTcpListener, UdpSocket as StdUdpSocket};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use hickory_proto::op::{Message, MessageType, Query};
use hickory_proto::rr::{Name, RData, Record, RecordType, rdata};
use mosdns_native_host::HostAssembly;
use mosdns_upstream_core::TransportCancellation;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "special-groups-whole-chain-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        for directory in ["webinfo", "srs", "rule", "cache"] {
            fs::create_dir_all(root.join(directory)).expect("fixture directory");
        }
        Self(root)
    }

    fn write(&self, relative: &str, value: impl AsRef<[u8]>) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture parent");
        fs::write(path, value).expect("fixture file");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn free_udp_socket() -> StdUdpSocket {
    let socket = StdUdpSocket::bind("127.0.0.1:0").expect("reserve UDP port");
    socket.set_nonblocking(true).expect("nonblocking UDP peer");
    socket
}

fn free_tcp_listener() -> StdTcpListener {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("reserve TCP port");
    listener
        .set_nonblocking(true)
        .expect("nonblocking TCP peer");
    listener
}

fn free_pair() -> u16 {
    loop {
        let tcp = StdTcpListener::bind("127.0.0.1:0").expect("probe TCP port");
        let port = tcp.local_addr().expect("TCP address").port();
        if let Ok(udp) = StdUdpSocket::bind(("127.0.0.1", port)) {
            drop(udp);
            return port;
        }
    }
}

fn dns_query(id: u16, name: &str) -> Vec<u8> {
    let mut query = Message::new();
    query.set_id(id).add_query(Query::query(
        Name::from_ascii(format!("{name}.")).expect("valid test name"),
        RecordType::A,
    ));
    query.to_vec().expect("encode DNS query")
}

fn dns_response(request: &[u8], answer: &str) -> Vec<u8> {
    let mut response = Message::from_vec(request).expect("parse controlled DNS query");
    let name = response.queries()[0].name().clone();
    response
        .set_message_type(MessageType::Response)
        .set_recursion_available(true)
        .add_answer(Record::from_rdata(
            name,
            60,
            RData::A(rdata::A(answer.parse().expect("valid test IPv4 address"))),
        ));
    response.to_vec().expect("encode controlled DNS answer")
}

fn answer(response: &Message) -> String {
    response
        .answers()
        .iter()
        .find_map(|record| match record.data() {
            RData::A(value) => Some(value.to_string()),
            _ => None,
        })
        .expect("A answer")
}

async fn udp_dns(port: u16, id: u16, name: &str) -> Message {
    let socket = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("DNS client socket");
    socket
        .send_to(&dns_query(id, name), ("127.0.0.1", port))
        .await
        .expect("send DNS query");
    let mut buffer = vec![0; 4096];
    let (length, _) = tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut buffer))
        .await
        .expect("bounded DNS response")
        .expect("receive DNS response");
    Message::from_vec(&buffer[..length]).expect("parse DNS response")
}

fn serve_udp_peer(
    socket: StdUdpSocket,
    answer_ip: &'static str,
    requests: Rc<Cell<usize>>,
    shutdown: TransportCancellation,
) -> tokio::task::JoinHandle<()> {
    let socket = UdpSocket::from_std(socket).expect("adopt UDP peer");
    tokio::task::spawn_local(async move {
        let mut buffer = vec![0; 4096];
        loop {
            let received = tokio::select! {
                () = shutdown.cancelled() => break,
                received = socket.recv_from(&mut buffer) => received,
            };
            let Ok((length, peer)) = received else {
                break;
            };
            requests.set(requests.get() + 1);
            let response = dns_response(&buffer[..length], answer_ip);
            if socket.send_to(&response, peer).await.is_err() {
                break;
            }
        }
    })
}

fn serve_tcp_peer(
    listener: StdTcpListener,
    answer_ip: &'static str,
    requests: Rc<Cell<usize>>,
    shutdown: TransportCancellation,
) -> tokio::task::JoinHandle<()> {
    let listener = TcpListener::from_std(listener).expect("adopt TCP peer");
    tokio::task::spawn_local(async move {
        loop {
            let accepted = tokio::select! {
                () = shutdown.cancelled() => break,
                accepted = listener.accept() => accepted,
            };
            let Ok((mut stream, _)) = accepted else {
                break;
            };
            loop {
                let mut prefix = [0; 2];
                if stream.read_exact(&mut prefix).await.is_err() {
                    break;
                }
                let mut query = vec![0; usize::from(u16::from_be_bytes(prefix))];
                if stream.read_exact(&mut query).await.is_err() {
                    break;
                }
                requests.set(requests.get() + 1);
                let response = dns_response(&query, answer_ip);
                let Ok(length) = u16::try_from(response.len()) else {
                    break;
                };
                if stream.write_all(&length.to_be_bytes()).await.is_err()
                    || stream.write_all(&response).await.is_err()
                {
                    break;
                }
            }
        }
    })
}

struct HttpResponse {
    status: u16,
    body: String,
}

async fn http_request(address: SocketAddr, method: &str, path: &str, body: &str) -> HttpResponse {
    let mut stream = TcpStream::connect(address).await.expect("HTTP connection");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: native\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("HTTP request");
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.expect("HTTP response");
    let text = String::from_utf8(bytes).expect("HTTP is UTF-8");
    let (head, body) = text
        .split_once("\r\n\r\n")
        .expect("HTTP response separator");
    let status = head
        .lines()
        .next()
        .expect("HTTP status line")
        .split_whitespace()
        .nth(1)
        .expect("HTTP status")
        .parse()
        .expect("numeric HTTP status");
    HttpResponse {
        status,
        body: body.to_owned(),
    }
}

fn catalog(slot: u32, file: &str) -> Value {
    json!({
        "local": {
            "name": "local",
            "type": format!("special_{slot}"),
            "enabled": true,
            "files": file
        }
    })
}

#[test]
#[allow(clippy::too_many_lines)] // One integrated HTTP/DNS/cache/audit proof.
fn whole_chain_proves_ordered_groups_cache_publication_and_http_provenance() {
    let fixture = Fixture::new();
    let api_port = free_pair();
    let main_port = free_udp_socket().local_addr().unwrap().port();
    let group50_port = free_pair();
    let group51_port = free_pair();
    let group52_port = free_pair();

    let default_peer = free_udp_socket();
    let group50_peer = free_udp_socket();
    let group51_peer = free_tcp_listener();
    let group52_peer = free_udp_socket();
    let replacement_peer = free_udp_socket();
    let default_addr = default_peer.local_addr().unwrap();
    let group50_addr = group50_peer.local_addr().unwrap();
    let group51_addr = group51_peer.local_addr().unwrap();
    let group52_addr = group52_peer.local_addr().unwrap();
    let replacement_addr = replacement_peer.local_addr().unwrap();

    fixture.write(
        "config.yaml",
        format!(
            "log: {{level: error}}\nnative_management: {{special_groups: true}}\napi: {{http: '127.0.0.1:{api_port}'}}\nplugins:\n  - tag: default_upstream\n    type: forward\n    args:\n      upstreams: [{{tag: default_supplier, addr: 'udp://{default_addr}'}}]\n  - tag: main_entry\n    type: sequence\n    args: [{{exec: $special_upstream_matcher}}, {{exec: $default_upstream}}]\n  - tag: main_udp\n    type: udp_server\n    args: {{entry: main_entry, listen: '127.0.0.1:{main_port}', enable_audit: true}}\n"
        ),
    );
    fixture.write(
        "webinfo/special_upstream_groups.json",
        serde_json::to_vec(&json!([
            {"slot":50,"name":"lower","listen_port":group50_port,"custom_port_only":false},
            {"slot":51,"name":"higher","listen_port":group51_port,"custom_port_only":false},
            {"slot":52,"name":"custom-only","listen_port":group52_port,"custom_port_only":true}
        ]))
        .unwrap(),
    );
    fixture.write(
        "webinfo/upstream_overrides.json",
        serde_json::to_vec(&json!({
            "special_upstream_50": [{"tag":"lower_supplier","enabled":true,"protocol":"udp","addr":format!("udp://{group50_addr}")}],
            "special_upstream_51": [{"tag":"higher_supplier","enabled":true,"protocol":"tcp","addr":format!("tcp://{group51_addr}")}],
            "special_upstream_52": [{"tag":"isolated_supplier","enabled":true,"protocol":"udp","addr":format!("udp://{group52_addr}")}]
        }))
        .unwrap(),
    );
    for (slot, file, rule) in [
        (50, "rule/catalog-50.txt", "full:shared.example\n"),
        (51, "rule/catalog-51.txt", "full:shared.example\n"),
        (52, "rule/catalog-52.txt", "full:exclusive.example\n"),
    ] {
        fixture.write(file, rule);
        fixture.write(
            &format!("srs/special_{slot}.json"),
            serde_json::to_vec(&catalog(slot, file)).unwrap(),
        );
    }

    let host = HostAssembly::from_config_file(&fixture.0.join("config.yaml"))
        .expect("compile managed host from isolated fixture");
    host.block_on(async {
        let default_count = Rc::new(Cell::new(0));
        let group50_count = Rc::new(Cell::new(0));
        let group51_count = Rc::new(Cell::new(0));
        let group52_count = Rc::new(Cell::new(0));
        let replacement_count = Rc::new(Cell::new(0));
        let peer_shutdown = TransportCancellation::new();
        let mut peers = vec![
            serve_udp_peer(
                default_peer,
                "192.0.2.90",
                default_count.clone(),
                peer_shutdown.clone(),
            ),
            serve_udp_peer(
                group50_peer,
                "192.0.2.50",
                group50_count.clone(),
                peer_shutdown.clone(),
            ),
            serve_tcp_peer(
                group51_peer,
                "192.0.2.51",
                group51_count.clone(),
                peer_shutdown.clone(),
            ),
            serve_udp_peer(
                group52_peer,
                "192.0.2.52",
                group52_count.clone(),
                peer_shutdown.clone(),
            ),
            serve_udp_peer(
                replacement_peer,
                "192.0.2.55",
                replacement_count.clone(),
                peer_shutdown.clone(),
            ),
        ];

        let bound = host.bind_host().await.expect("bind isolated listeners");
        let api = bound.api_addr().expect("management API address");
        let shutdown = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(shutdown.clone()));

        assert_eq!(
            answer(&udp_dns(main_port, 1, "shared.example").await),
            "192.0.2.50"
        );
        assert_eq!(
            answer(&udp_dns(group50_port, 2, "shared.example").await),
            "192.0.2.50",
            "group 50 custom listener must use its own provider"
        );
        assert_eq!(
            answer(&udp_dns(group51_port, 3, "shared.example").await),
            "192.0.2.51",
            "the same qname on group 51 custom listener must be isolated"
        );
        assert_eq!(group50_count.get(), 1, "group 50's second query is cached");
        assert_eq!(group51_count.get(), 1, "group 51 used its TCP peer once");

        assert_eq!(
            answer(&udp_dns(main_port, 4, "exclusive.example").await),
            "192.0.2.90",
            "custom-only rules are excluded from main routing"
        );
        assert_eq!(
            answer(&udp_dns(group52_port, 5, "exclusive.example").await),
            "192.0.2.52",
            "custom-only group remains available on its dedicated listener"
        );
        assert_eq!(
            answer(&udp_dns(main_port, 6, "no-match.example").await),
            "192.0.2.90",
            "unmatched traffic follows the configured default entry"
        );
        assert_eq!(default_count.get(), 2);

        let initial_metrics = http_request(api, "GET", "/metrics", "").await;
        assert_eq!(initial_metrics.status, 200);
        assert!(
            initial_metrics
                .body
                .contains("mosdns_cache_hit_total{tag=\"cache_special_50\"} 1"),
            "per-group cache hit is visible: {}",
            initial_metrics.body
        );

        let saved = json!({
            "plugin_tag": "special_upstream_50",
            "upstreams": [{
                "tag": "lower_supplier_v2",
                "enabled": true,
                "protocol": "udp",
                "addr": format!("udp://{replacement_addr}")
            }]
        })
        .to_string();
        let response = http_request(api, "POST", "/api/v1/upstream/config", &saved).await;
        assert_eq!(response.status, 200, "{}", response.body);
        assert_eq!(
            answer(&udp_dns(main_port, 7, "shared.example").await),
            "192.0.2.55",
            "next query sees committed provider and invalidated group cache"
        );
        assert_eq!(replacement_count.get(), 1);
        assert_eq!(group50_count.get(), 1);
        assert_eq!(
            answer(&udp_dns(group51_port, 8, "shared.example").await),
            "192.0.2.51"
        );
        assert_eq!(
            group51_count.get(),
            1,
            "unaffected cache and TCP provider remain warm"
        );

        let rejected = json!({
            "plugin_tag": "special_upstream_50",
            "upstreams": [{
                "tag": "unsupported",
                "enabled": true,
                "protocol": "quic",
                "addr": "quic://127.0.0.1:26053"
            }]
        })
        .to_string();
        let failed = http_request(api, "POST", "/api/v1/upstream/config", &rejected).await;
        assert_eq!(failed.status, 400, "unsupported protocol fails closed");
        let runtime = http_request(
            api,
            "GET",
            "/api/v1/upstream/runtime/special_upstream_50",
            "",
        )
        .await;
        assert_eq!(runtime.status, 200);
        let runtime: Value = serde_json::from_str(&runtime.body).expect("runtime JSON");
        assert_eq!(runtime["override_config"][0]["tag"], "lower_supplier_v2");
        assert_eq!(
            answer(&udp_dns(main_port, 9, "shared.example").await),
            "192.0.2.55",
            "rejected save leaves the last committed generation active"
        );

        let logs = http_request(api, "GET", "/api/v2/audit/logs?page=1&limit=100", "").await;
        assert_eq!(logs.status, 200, "{}", logs.body);
        let logs: Value = serde_json::from_str(&logs.body).expect("audit JSON");
        let record = logs["logs"]
            .as_array()
            .expect("audit logs array")
            .iter()
            .find(|record| {
                record["query_name"] == "shared.example"
                    && record["final_upstream"] == "lower_supplier_v2"
                    && record["upstream_diagnostics"]["attempts"]
                        .as_array()
                        .is_some_and(|attempts| !attempts.is_empty())
            })
            .expect("post-save main-entry audit record");
        assert_eq!(record["matched_group"], "special_50");
        assert_eq!(record["matched_rule_source"], "domain_set:special_route_50");
        assert_eq!(record["domain_set"], "special_route_50");
        assert_eq!(record["effective_tag"], "特殊上游50");
        assert_eq!(record["final_sequence"], "sequence_special_50");
        let selected = &record["upstream_diagnostics"]["selected"];
        assert_eq!(selected["entry"], "lower_supplier_v2");
        assert_eq!(selected["peer"], replacement_addr.to_string());
        assert_eq!(selected["transport"], "udp");
        let attempt = &record["upstream_diagnostics"]["attempts"][0];
        assert_eq!(attempt["ordinal"], 0);
        assert_eq!(attempt["entry"], "lower_supplier_v2");
        assert_eq!(attempt["peer"], replacement_addr.to_string());
        assert_eq!(attempt["transport"], "udp");
        assert_eq!(attempt["outcome"], "response");

        for (entry, peer, transport) in [
            ("lower_supplier_v2", replacement_addr, "udp"),
            ("higher_supplier", group51_addr, "tcp"),
        ] {
            let hit = logs["logs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|record| {
                    record["final_upstream"] == entry
                        && record["upstream_diagnostics"]["attempts"]
                            .as_array()
                            .is_some_and(Vec::is_empty)
                })
                .expect("cache hit retains supplier through invalidation and disjoint owner reuse");
            assert_eq!(hit["upstream_diagnostics"]["selected"]["entry"], entry);
            assert_eq!(
                hit["upstream_diagnostics"]["selected"]["peer"],
                peer.to_string()
            );
            assert_eq!(
                hit["upstream_diagnostics"]["selected"]["transport"],
                transport
            );
        }

        shutdown.cancel();
        serving
            .await
            .expect("supervisor task")
            .expect("clean close");
        peer_shutdown.cancel();
        for peer in peers.drain(..) {
            peer.await.expect("controlled peer stopped");
        }
        for port in [main_port, group50_port, group51_port, group52_port] {
            assert!(
                StdUdpSocket::bind(("127.0.0.1", port)).is_ok(),
                "UDP listener {port} released on close"
            );
            assert!(
                StdTcpListener::bind(("127.0.0.1", port)).is_ok(),
                "TCP listener {port} released on close"
            );
        }
    });
}

#[test]
fn audit_off_keeps_forward_metrics_without_retaining_query_records() {
    let fixture = Fixture::new();
    let dns_port = free_udp_socket().local_addr().unwrap().port();
    let peer = free_udp_socket();
    let peer_addr = peer.local_addr().unwrap();
    fixture.write(
        "config.yaml",
        format!(
            "log: {{level: error}}\nplugins:\n  - tag: default\n    type: forward\n    args: {{upstreams: [{{tag: measured_peer, addr: 'udp://{peer_addr}'}}]}}\n  - tag: main_entry\n    type: sequence\n    args: [{{exec: $default}}]\n  - tag: main\n    type: udp_server\n    args: {{entry: main_entry, listen: '127.0.0.1:{dns_port}', enable_audit: false}}\n"
        ),
    );
    let host = HostAssembly::from_config_file(&fixture.0.join("config.yaml"))
        .expect("compile audit-off fixture");
    host.block_on(async {
        let count = Rc::new(Cell::new(0));
        let peer_shutdown = TransportCancellation::new();
        let peer_task = serve_udp_peer(peer, "192.0.2.60", count.clone(), peer_shutdown.clone());
        let bound = host.bind_host().await.expect("bind audit-off host");
        let port = bound.dns_addr().port();
        let shutdown = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        assert_eq!(
            answer(&udp_dns(port, 10, "metrics.example").await),
            "192.0.2.60"
        );
        assert_eq!(count.get(), 1);
        assert!(host.audit_snapshot().records.is_empty());
        let metrics = host.metrics_snapshot();
        assert_eq!(metrics.admitted_total, 1);
        assert_eq!(metrics.completed_total, 1);
        assert_eq!(
            metrics.forward_attempts_by_upstream["measured_peer"].responses_total,
            1
        );
        shutdown.cancel();
        serving.await.expect("host task").expect("clean close");
        peer_shutdown.cancel();
        peer_task.await.expect("controlled peer stopped");
    });
}

#[test]
fn group_cache_supplier_survives_shutdown_restart_without_new_attempt() {
    let fixture = Fixture::new();
    let peer = free_udp_socket();
    let peer_addr = peer.local_addr().unwrap();
    let main_port = free_pair();
    fixture.write(
        "webinfo/special_upstream_groups.json",
        br#"[{"slot":50,"name":"persisted"}]"#,
    );
    fixture.write("rule/special_50.txt", b"full:persisted.example\n");
    fixture.write("webinfo/upstream_overrides.json", serde_json::to_vec(&json!({"special_upstream_50":[{"tag":"persisted_supplier","enabled":true,"protocol":"udp","addr":format!("udp://{peer_addr}")}]})).unwrap());
    fixture.write("config.yaml", format!("log: {{level: error}}\nnative_management: {{special_groups: true}}\nplugins:\n  - tag: default\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://{peer_addr}'}}]}}\n  - tag: main_entry\n    type: sequence\n    args: [{{exec: $special_upstream_matcher}}, {{exec: $default}}]\n  - tag: main\n    type: udp_server\n    args: {{entry: main_entry, listen: '127.0.0.1:{main_port}', enable_audit: true}}\n"));
    let count = Rc::new(Cell::new(0));
    for restarted in [false, true] {
        let config = mosdns_native_host::load_and_compile(&fixture.0.join("config.yaml")).unwrap();
        let host = HostAssembly::from_config(config).unwrap();
        host.block_on(async {
            let peer_shutdown = TransportCancellation::new();
            let peer_task = serve_udp_peer(
                peer.try_clone().unwrap(),
                "192.0.2.50",
                count.clone(),
                peer_shutdown.clone(),
            );
            let bound = host.bind_host().await.unwrap();
            let shutdown = TransportCancellation::new();
            let serving = tokio::task::spawn_local(bound.serve(shutdown.clone()));
            let response = udp_dns(main_port, 41, "persisted.example").await;
            assert_eq!(answer(&response), "192.0.2.50");
            assert!(
                response.extensions().is_none(),
                "origin does not grant ECS echo"
            );
            let audit = host.audit_snapshot();
            let record = audit.records.last().unwrap();
            assert_eq!(record.final_upstream.as_deref(), Some("persisted_supplier"));
            assert_eq!(
                record.selected_upstream.as_deref(),
                Some(peer_addr.to_string().as_str())
            );
            let selected = record
                .upstream_diagnostics
                .as_ref()
                .unwrap()
                .selected
                .as_ref()
                .unwrap();
            assert_eq!(selected.entry, "persisted_supplier");
            assert_eq!(selected.peer, peer_addr);
            assert_eq!(
                selected.transport,
                mosdns_native_host::UpstreamTransport::Udp
            );
            if restarted {
                assert_eq!(record.cache_status, mosdns_native_host::CacheStatus::Hit);
                assert!(record.upstream_attempts.is_empty());
                assert!(
                    record
                        .upstream_diagnostics
                        .as_ref()
                        .unwrap()
                        .attempts
                        .is_empty()
                );
            }
            assert_eq!(count.get(), 1, "restart hit creates no peer request");
            shutdown.cancel();
            serving.await.unwrap().unwrap();
            peer_shutdown.cancel();
            peer_task.await.unwrap();
        });
        assert!(fixture.0.join("cache/cache_special_50.dump").exists());
    }
}

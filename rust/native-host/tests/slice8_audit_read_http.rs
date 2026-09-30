//! Slice 2: retained-ring v2 audit projections over real HTTP and DNS.

use std::io::{Read, Write};
use std::net::{
    SocketAddr, TcpListener as StdTcpListener, TcpStream as StdTcpStream, UdpSocket as StdUdpSocket,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mosdns_dns_core::validate_response;
use mosdns_native_host::{AuditTestClock, HostAssembly, HostOptions};
use mosdns_upstream_core::TransportCancellation;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn free_tcp_port() -> u16 {
    let listener = StdTcpListener::bind("127.0.0.1:0").expect("probe TCP port");
    let port = listener.local_addr().expect("probe TCP address").port();
    drop(listener);
    port
}

fn free_udp_port() -> u16 {
    let socket = StdUdpSocket::bind("127.0.0.1:0").expect("probe UDP port");
    let port = socket.local_addr().expect("probe UDP address").port();
    drop(socket);
    port
}

fn dns_query(id: u16, name: &str) -> Vec<u8> {
    dns_query_type(id, name, 1)
}

fn dns_query_type(id: u16, name: &str, qtype: u16) -> Vec<u8> {
    let mut packet = vec![
        u8::try_from(id >> 8).expect("high DNS ID byte"),
        u8::try_from(id & 0x00ff).expect("low DNS ID byte"),
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
    ];
    for label in name.trim_end_matches('.').split('.') {
        packet.push(u8::try_from(label.len()).expect("DNS label length"));
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&qtype.to_be_bytes());
    packet.extend_from_slice(&[0, 1]);
    packet
}

fn udp_query(listener: SocketAddr, request: &[u8]) -> Vec<u8> {
    let socket = StdUdpSocket::bind("127.0.0.1:0").expect("DNS client bind");
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("DNS client timeout");
    socket.send_to(request, listener).expect("DNS client send");
    let mut response = vec![0_u8; 65535];
    let (length, _) = socket
        .recv_from(&mut response)
        .expect("DNS client response");
    response[..length].to_vec()
}

fn tcp_query(listener: SocketAddr, request: &[u8]) -> Vec<u8> {
    let mut stream = StdTcpStream::connect(listener).expect("DNS TCP connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("DNS TCP timeout");
    stream
        .write_all(
            &u16::try_from(request.len())
                .expect("DNS TCP request length")
                .to_be_bytes(),
        )
        .expect("DNS TCP length write");
    stream.write_all(request).expect("DNS TCP request write");
    let mut length = [0_u8; 2];
    stream.read_exact(&mut length).expect("DNS TCP length read");
    let mut response = vec![0_u8; usize::from(u16::from_be_bytes(length))];
    stream
        .read_exact(&mut response)
        .expect("DNS TCP response read");
    response
}

fn fill_real_ring(listener: SocketAddr, count: u32) {
    let socket = StdUdpSocket::bind("127.0.0.1:0").expect("bulk DNS client bind");
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("bulk DNS client timeout");
    let mut response = [0_u8; 2048];
    for id in 0..count {
        let request = dns_query(
            u16::try_from(id % u32::from(u16::MAX)).expect("bulk DNS ID"),
            "bulk.example.",
        );
        socket.send_to(&request, listener).expect("bulk DNS send");
        socket.recv_from(&mut response).expect("bulk DNS response");
    }
}

fn rich_response(query: &[u8]) -> Vec<u8> {
    let mut response = query[..2].to_vec();
    response.extend_from_slice(&[0x81, 0x80, 0, 1, 0, 2, 0, 0, 0, 0]);
    response.extend_from_slice(&query[12..]);
    let cname = b"\x05alias\x07example\x00";
    response.extend_from_slice(&[0xc0, 0x0c, 0, 5, 0, 1, 0, 0, 0, 60]);
    response.extend_from_slice(
        &u16::try_from(cname.len())
            .expect("CNAME length")
            .to_be_bytes(),
    );
    response.extend_from_slice(cname);
    let qtype_offset = query
        .iter()
        .enumerate()
        .skip(12)
        .find_map(|(index, byte)| (*byte == 0).then_some(index + 1))
        .expect("rich qname terminator");
    let qtype = u16::from_be_bytes([query[qtype_offset], query[qtype_offset + 1]]);
    if qtype == 28 {
        response.extend_from_slice(&[
            0xc0, 0x0c, 0, 28, 0, 1, 0, 0, 0, 60, 0, 16, 0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0x55,
        ]);
    } else {
        response.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 192, 0, 2, 55]);
    }
    response
}

fn run_rich_upstream(
    socket: &std::net::UdpSocket,
    stopped: &Arc<AtomicBool>,
    served: &Arc<AtomicUsize>,
) {
    socket
        .set_read_timeout(Some(Duration::from_millis(50)))
        .expect("rich upstream timeout");
    let mut query = [0_u8; 4096];
    while !stopped.load(Ordering::Acquire) {
        let Ok((length, peer)) = socket.recv_from(&mut query) else {
            continue;
        };
        served.fetch_add(1, Ordering::Relaxed);
        let response = rich_response(&query[..length]);
        socket
            .send_to(&response, peer)
            .expect("rich upstream response");
    }
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    body: String,
}

async fn http_request(address: SocketAddr, method: &str, target: &str) -> HttpResponse {
    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("HTTP connect");
    let request =
        format!("{method} {target} HTTP/1.1\r\nHost: native\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("HTTP write");
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.expect("HTTP read");
    let text = String::from_utf8(bytes).expect("HTTP UTF-8");
    let (head, body) = text.split_once("\r\n\r\n").expect("HTTP response split");
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("HTTP status")
        .parse()
        .expect("HTTP status integer");
    HttpResponse {
        status,
        body: body.to_owned(),
    }
}

async fn http_post(address: SocketAddr, target: &str, body: &str) -> HttpResponse {
    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("HTTP POST connect");
    let request = format!(
        "POST {target} HTTP/1.1\r\nHost: native\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("HTTP POST write");
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .await
        .expect("HTTP POST read");
    let text = String::from_utf8(bytes).expect("HTTP POST UTF-8");
    let (head, body) = text
        .split_once("\r\n\r\n")
        .expect("HTTP POST response split");
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("HTTP POST status")
        .parse()
        .expect("HTTP POST status integer");
    HttpResponse {
        status,
        body: body.to_owned(),
    }
}

fn config(
    root: &std::path::Path,
    dns_port: u16,
    api_port: u16,
    upstream_port: u16,
    listener_type: &str,
) -> mosdns_native_host::CompiledConfig {
    let idle_timeout = if listener_type == "tcp_server" {
        "      idle_timeout: 5\n"
    } else {
        ""
    };
    let yaml = format!(
        r#"log:
  level: error
api:
  http: "127.0.0.1:{api_port}"
plugins:
  - tag: sequence_main
    type: sequence
    args:
      - matches: "!qname $rich_rules"
        exec: reject 3
      - matches: qname $rich_rules
        exec: $forward_main
  - tag: rich_rules
    type: domain_set
    args:
      exps: ["full:rich.example."]
  - tag: forward_main
    type: forward
    args:
      upstreams:
        - addr: "udp://127.0.0.1:{upstream_port}"
  - tag: listener
    type: {listener_type}
    args:
      entry: sequence_main
      listen: "127.0.0.1:{dns_port}"
      enable_audit: true
{idle_timeout}
"#
    );
    let path = root.join("config.yaml");
    std::fs::write(&path, yaml).expect("write audit read config");
    mosdns_native_host::load_and_compile(&path).expect("compile audit read config")
}

fn at(seconds: u64, nanos: u32) -> SystemTime {
    UNIX_EPOCH + Duration::new(seconds, nanos)
}

async fn send_dns(
    dns: SocketAddr,
    clock: &AuditTestClock,
    timestamp: SystemTime,
    id: u16,
    name: &str,
) {
    clock.set(timestamp);
    let name = name.to_owned();
    let response = tokio::task::spawn_blocking(move || udp_query(dns, &dns_query(id, &name)))
        .await
        .expect("DNS client task");
    assert!(!response.is_empty(), "DNS response must be real");
}

#[test]
#[allow(clippy::too_many_lines)]
fn v2_stats_windows_and_logs_use_retained_real_dns_records() {
    let root = std::env::temp_dir().join(format!("mosdns-audit-read-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("audit read root");
    let dns_port = free_udp_port();
    let api_port = free_tcp_port();
    let upstream_port = free_udp_port();
    let upstream_socket =
        std::net::UdpSocket::bind(("127.0.0.1", upstream_port)).expect("rich upstream bind");
    let upstream_stopped = Arc::new(AtomicBool::new(false));
    let upstream_served = Arc::new(AtomicUsize::new(0));
    let upstream_stopped_for_thread = Arc::clone(&upstream_stopped);
    let upstream_served_for_thread = Arc::clone(&upstream_served);
    let upstream_task = std::thread::spawn(move || {
        run_rich_upstream(
            &upstream_socket,
            &upstream_stopped_for_thread,
            &upstream_served_for_thread,
        );
    });
    let clock = AuditTestClock::new(at(1_700_000_000, 0));
    let host = HostAssembly::with_options_and_state_root(
        config(&root, dns_port, api_port, upstream_port, "udp_server"),
        HostOptions::default()
            .with_audit_capacity(8)
            .with_audit_clock(Arc::new(clock.clone())),
        &root,
    )
    .expect("assemble audit read host");
    let bound = host
        .block_on(host.bind_host())
        .expect("bind audit read host");
    let dns = bound.dns_addr();
    let api = bound.api_addr().expect("audit read API");
    let shutdown = TransportCancellation::new();

    let result = host.block_on(async {
        let task = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let origin = at(1_700_000_000, 0);
        send_dns(
            dns,
            &clock,
            origin.checked_sub(Duration::from_secs(604_800)).expect("7d timestamp"),
            0x8101,
            "old.example.",
        )
        .await;
        send_dns(
            dns,
            &clock,
            origin.checked_sub(Duration::from_secs(3_600)).expect("1h timestamp"),
            0x8102,
            "edge.example.",
        )
        .await;
        send_dns(
            dns,
            &clock,
            origin
                .checked_sub(Duration::from_nanos(123_456_789))
                .expect("nanosecond timestamp"),
            0x8103,
            "new.example.",
        )
        .await;
        clock.set(origin);

        let stats = http_request(api, "GET", "/api/v2/audit/stats").await;
        assert_eq!(stats.status, 200);
        let stats_json: Value = serde_json::from_str(&stats.body).expect("stats JSON");
        assert_eq!(stats_json["total_queries"], 3);
        assert!(stats_json["average_duration_ms"].as_f64().is_some());

        let windows = http_request(api, "GET", "/api/v2/audit/stats/windows").await;
        assert_eq!(windows.status, 200);
        let windows_json: Value = serde_json::from_str(&windows.body).expect("windows JSON");
        assert_eq!(windows_json["generated_at"], "2023-11-14T22:13:20Z");
        assert_eq!(windows_json["items"].as_array().expect("window items").len(), 5);
        assert_eq!(windows_json["items"][0]["key"], "1h");
        assert_eq!(windows_json["items"][0]["request_count"], 2);
        assert_eq!(windows_json["items"][0]["complete"], true);
        assert_eq!(windows_json["items"][4]["complete"], true);
        assert_eq!(windows_json["items"][0]["coverage_start"], "2023-11-07T22:13:20Z");

        let logs = http_request(api, "GET", "/api/v2/audit/logs").await;
        assert_eq!(logs.status, 200);
        let logs_json: Value = serde_json::from_str(&logs.body).expect("logs JSON");
        assert_eq!(logs_json["pagination"]["total_items"], 3);
        assert_eq!(logs_json["pagination"]["total_pages"], 1);
        assert_eq!(logs_json["logs"][0]["query_name"], "new.example");
        assert_eq!(logs_json["logs"][0]["query_time"], "2023-11-14T22:13:19.876543211Z");
        assert_eq!(logs_json["logs"][0]["query_type"], "A");
        assert_eq!(logs_json["logs"][0]["client_ip"], "127.0.0.1");
        let trace_id = logs_json["logs"][0]["trace_id"].as_str().expect("trace id");
        assert_eq!(trace_id.len(), 51);
        assert!(trace_id.starts_with("n-"));
        assert_eq!(logs_json["logs"][0]["response_code"], "NXDOMAIN");
        assert_eq!(logs_json["logs"][0]["answer_details_status"], "complete");
        assert!(logs_json["logs"][0]["response_flags"]["RA"].is_boolean());

        let page_one = http_request(api, "GET", "/api/v2/audit/logs?page=1&limit=2").await;
        let page_one_json: Value = serde_json::from_str(&page_one.body).expect("page one JSON");
        assert_eq!(page_one_json["pagination"]["total_pages"], 2);
        assert_eq!(page_one_json["logs"].as_array().expect("page one logs").len(), 2);
        let page_two = http_request(api, "GET", "/api/v2/audit/logs?page=2&limit=2").await;
        let page_two_json: Value = serde_json::from_str(&page_two.body).expect("page two JSON");
        assert_eq!(page_two_json["logs"].as_array().expect("page two logs").len(), 1);
        let page_three = http_request(api, "GET", "/api/v2/audit/logs?page=3&limit=2").await;
        let page_three_json: Value = serde_json::from_str(&page_three.body).expect("page three JSON");
        assert!(page_three_json["logs"].as_array().expect("page three logs").is_empty());

        for limit in [160, 500] {
            assert_eq!(
                http_request(api, "GET", &format!("/api/v2/audit/logs?limit={limit}")).await.status,
                200
            );
        }
        let too_large = http_request(api, "GET", "/api/v2/audit/logs?limit=501").await;
        assert_eq!(too_large.status, 400);
        assert_eq!(too_large.body, "audit log limit must be between 1 and 500\n");
        let filtered = http_request(api, "GET", "/api/v2/audit/logs?q=example").await;
        assert_eq!(filtered.status, 200);
        let filtered_json: Value = serde_json::from_str(&filtered.body).expect("filtered JSON");
        assert_eq!(filtered_json["pagination"]["total_items"], 3);
        let exact = http_request(api, "GET", "/api/v2/audit/logs?q=new.example&exact=true").await;
        assert_eq!(exact.status, 200);
        let exact_json: Value = serde_json::from_str(&exact.body).expect("exact JSON");
        assert_eq!(exact_json["pagination"]["total_items"], 1);
        let repeated_client_ip = http_request(
            api,
            "GET",
            "/api/v2/audit/logs?client_ip=127.0.0.2&client_ip=::ffff:127.0.0.1",
        )
        .await;
        assert_eq!(repeated_client_ip.status, 200);
        assert_eq!(serde_json::from_str::<Value>(&repeated_client_ip.body).expect("repeated client JSON")["pagination"]["total_items"], 3);
        let no_client_match = http_request(api, "GET", "/api/v2/audit/logs?client_ip=127.0.0.2").await;
        assert_eq!(no_client_match.status, 200);
        assert_eq!(serde_json::from_str::<Value>(&no_client_match.body).expect("no client JSON")["pagination"]["total_items"], 0);
        let rich = tokio::task::spawn_blocking(move || udp_query(dns, &dns_query(0x8109, "rich.example.")))
            .await
            .expect("rich DNS client");
        assert!(!rich.is_empty());
        let rich_filters = [
            ("domain=rich.example", 1),
            ("answer_ip=192.0.2.55", 1),
            ("cname=alias.example.", 1),
            ("domain_set=rich_rules", 1),
            ("effective_tag=rich_rules", 1),
            ("client_ip=127.0.0.1&answer_ip=192.0.2.55", 1),
        ];
        for (query, expected) in rich_filters {
            let response = http_request(api, "GET", &format!("/api/v2/audit/logs?{query}")).await;
            assert_eq!(response.status, 200, "filter {query}");
            assert_eq!(serde_json::from_str::<Value>(&response.body).expect("rich filter JSON")["pagination"]["total_items"], expected, "filter {query}");
        }
        let overflow = http_request(
            api,
            "GET",
            "/api/v2/audit/logs?page=9223372036854775808&limit=9223372036854775808",
        )
        .await;
        assert_eq!(overflow.status, 200);
        assert!(serde_json::from_str::<Value>(&overflow.body).expect("overflow JSON")["logs"].as_array().expect("overflow logs").len() <= 50);
        let malformed = http_request(api, "GET", "/api/v2/audit/logs?q=%FF").await;
        assert_eq!(malformed.status, 400);
        assert_eq!(malformed.body, "invalid audit query encoding\n");
        let unknown = http_request(api, "GET", "/api/v2/audit/logs?bogus=1").await;
        assert_eq!(unknown.status, 400);
        assert_eq!(unknown.body, "unsupported audit query parameter\n");
        let rank = http_request(api, "GET", "/api/v2/audit/rank/domain?limit=20").await;
        assert_eq!(rank.status, 200);
        let rank_json: Value = serde_json::from_str(&rank.body).expect("rank JSON");
        assert_eq!(rank_json[0]["key"], "edge.example");
        let domain_logs = http_request(
            api,
            "GET",
            "/api/v2/audit/logs/domain?domain=edge.example&limit=50",
        )
        .await;
        assert_eq!(domain_logs.status, 200);
        let domain_logs_json: Value = serde_json::from_str(&domain_logs.body).expect("domain logs JSON");
        assert_eq!(domain_logs_json["pagination"]["total_items"], 1);
        for (route, expected_key, expected_count) in [
            ("rank/client", "127.0.0.1", 4),
            ("rank/domain_set", "rich_rules", 1),
            ("rank/effective", "rich_rules", 1),
        ] {
            let response = http_request(api, "GET", &format!("/api/v2/audit/{route}?limit=20")).await;
            assert_eq!(response.status, 200, "{route}");
            let values: Value = serde_json::from_str(&response.body).expect("rank JSON");
            assert_eq!(values[0]["key"], expected_key, "{route}");
            assert_eq!(values[0]["count"], expected_count, "{route}");
        }
        let slowest = http_request(api, "GET", "/api/v2/audit/rank/slowest?limit=300").await;
        assert_eq!(slowest.status, 200);
        let slowest_values: Value = serde_json::from_str(&slowest.body).expect("slowest JSON");
        assert!(slowest_values.as_array().expect("slowest values").iter().any(|item| item["query_name"] == "rich.example"));

        let resized = http_post(api, "/api/v1/audit/capacity", r#"{"capacity":2}"#).await;
        assert_eq!(resized.status, 200);
        send_dns(
            dns,
            &clock,
            origin.checked_sub(Duration::from_secs(604_800)).expect("eviction old timestamp"),
            0x8105,
            "evict-old.example.",
        )
        .await;
        send_dns(
            dns,
            &clock,
            origin.checked_sub(Duration::from_secs(1_800)).expect("eviction middle timestamp"),
            0x8106,
            "evict-middle.example.",
        )
        .await;
        send_dns(dns, &clock, origin, 0x8107, "evict-new.example.").await;
        let evicted_stats = http_request(api, "GET", "/api/v2/audit/stats").await;
        assert_eq!(serde_json::from_str::<Value>(&evicted_stats.body).expect("evicted stats")["total_queries"], 2);
        let evicted_windows = http_request(api, "GET", "/api/v2/audit/stats/windows").await;
        let evicted_windows_json: Value = serde_json::from_str(&evicted_windows.body).expect("evicted windows");
        assert_eq!(evicted_windows_json["items"][0]["complete"], false);
        assert_eq!(evicted_windows_json["items"][4]["complete"], false);
        let evicted_logs = http_request(api, "GET", "/api/v2/audit/logs").await;
        let evicted_logs_json: Value = serde_json::from_str(&evicted_logs.body).expect("evicted logs");
        assert_eq!(evicted_logs_json["logs"].as_array().expect("evicted log list").len(), 2);
        assert_eq!(evicted_logs_json["logs"][0]["query_name"], "evict-new.example");

        for id in 0x8200..0x8210 {
            clock.set(origin);
            let query_name = format!("load-{id}.example.");
            let dns_task = tokio::task::spawn_blocking(move || {
                udp_query(dns, &dns_query(id, &query_name))
            });
            assert_eq!(
                http_request(api, "GET", "/api/v2/audit/stats").await.status,
                200
            );
            assert_eq!(
                http_request(api, "GET", "/api/v2/audit/stats/windows").await.status,
                200
            );
            assert_eq!(
                http_request(api, "GET", "/api/v2/audit/logs?limit=160")
                    .await
                    .status,
                200
            );
            assert!(!dns_task.await.expect("load DNS client").is_empty());
        }

        let metrics_before_stop = host.metrics_snapshot().completed_total;
        host.stop_audit();
        send_dns(
            dns,
            &clock,
            origin.checked_add(Duration::from_secs(1)).expect("after stop timestamp"),
            0x8104,
            "stopped.example.",
        )
        .await;
        assert!(host.metrics_snapshot().completed_total > metrics_before_stop);
        let stats_after_stop = http_request(api, "GET", "/api/v2/audit/stats").await;
        assert_eq!(serde_json::from_str::<Value>(&stats_after_stop.body).expect("stop stats")["total_queries"], 2);

        let clear = http_request(api, "POST", "/api/v1/audit/clear").await;
        assert_eq!(clear.status, 200);
        let empty_stats = http_request(api, "GET", "/api/v2/audit/stats").await;
        assert_eq!(serde_json::from_str::<Value>(&empty_stats.body).expect("empty stats")["total_queries"], 0);
        let empty_windows = http_request(api, "GET", "/api/v2/audit/stats/windows").await;
        let empty_windows_json: Value = serde_json::from_str(&empty_windows.body).expect("empty windows");
        assert!(empty_windows_json["items"][0].get("coverage_start").is_none());
        let empty_logs = http_request(api, "GET", "/api/v2/audit/logs").await;
        let empty_logs_json: Value = serde_json::from_str(&empty_logs.body).expect("empty logs");
        assert_eq!(empty_logs_json["pagination"]["total_pages"], 0);
        assert!(empty_logs_json["logs"].as_array().expect("empty log list").is_empty());

        let started = http_request(api, "POST", "/api/v1/audit/start").await;
        assert_eq!(started.status, 200);
        send_dns(
            dns,
            &clock,
            origin.checked_sub(Duration::from_secs(1_800)).expect("incomplete timestamp"),
            0x8108,
            "incomplete.example.",
        )
        .await;
        clock.set(origin);
        let incomplete_windows = http_request(api, "GET", "/api/v2/audit/stats/windows").await;
        let incomplete_windows_json: Value = serde_json::from_str(&incomplete_windows.body).expect("incomplete windows");
        assert_eq!(incomplete_windows_json["items"][0]["complete"], false);
        assert_eq!(incomplete_windows_json["items"][4]["complete"], false);

        let full_capacity =
            http_post(api, "/api/v1/audit/capacity", r#"{"capacity":400000}"#).await;
        assert_eq!(full_capacity.status, 200);
        clock.set(origin);
        tokio::task::spawn_blocking(move || fill_real_ring(dns, 400_000))
            .await
            .expect("near-full DNS fill task");
        let full_stats = http_request(api, "GET", "/api/v2/audit/stats").await;
        assert_eq!(
            serde_json::from_str::<Value>(&full_stats.body).expect("full stats")["total_queries"],
            400_000
        );
        let full_logs_task = tokio::spawn(http_request(
            api,
            "GET",
            "/api/v2/audit/logs?limit=500",
        ));
        let full_slowest_task = tokio::spawn(http_request(
            api,
            "GET",
            "/api/v2/audit/rank/slowest?limit=300",
        ));
        tokio::task::yield_now().await;
        assert!(
            !full_logs_task.is_finished() || !full_slowest_task.is_finished(),
            "at least one bounded read must still be active before progress probes"
        );
        let mut dns_progress_during_reads = 0;
        let mut active_read_probe = false;
        for id in 0x9001..0x9005 {
            let read_active = !full_logs_task.is_finished() || !full_slowest_task.is_finished();
            active_read_probe |= read_active;
            let response = tokio::task::spawn_blocking(move || {
                udp_query(dns, &dns_query(id, "during-full-read.example."))
            })
            .await
            .expect("during-read DNS client");
            if read_active && !response.is_empty() {
                dns_progress_during_reads += 1;
            }
        }
        assert!(active_read_probe, "DNS probes must observe a live HTTP read");
        assert!(dns_progress_during_reads > 0, "DNS must progress during HTTP reads");
        let full_logs = full_logs_task.await.expect("full logs read");
        let full_slowest = full_slowest_task.await.expect("full slowest read");
        assert_eq!(full_logs.status, 200);
        assert_eq!(full_slowest.status, 200);
        let full_logs_json: Value = serde_json::from_str(&full_logs.body).expect("full logs");
        let projected_records = full_logs_json["logs"]
            .as_array()
            .expect("full logs records")
            .len();
        assert_eq!(projected_records, 500);
        let slowest_records: Value =
            serde_json::from_str(&full_slowest.body).expect("full slowest records");
        assert!(slowest_records
            .as_array()
            .expect("slowest list")
            .len()
            <= 300);
        let body_bytes = u64::try_from(full_logs.body.len()).expect("body length fits u64");
        let record_count = u64::try_from(projected_records).expect("record count fits u64");
        let bytes_per_record_hundredths =
            (u128::from(body_bytes) * 100) / u128::from(record_count);
        assert!(bytes_per_record_hundredths > 0);
        println!(
            "full-ring projection: retained=400000 concurrent_reads=2 dns_progress_during_reads={} logs=500 slowest_max=300 logs_bytes={} logs_bytes_per_record={}.{:02}",
            dns_progress_during_reads,
            full_logs.body.len(),
            bytes_per_record_hundredths / 100,
            bytes_per_record_hundredths % 100
        );
        for _ in 0..3 {
            let dns_task = tokio::task::spawn_blocking(move || {
                udp_query(dns, &dns_query(0x9001, "after-full.example."))
            });
            assert_eq!(
                http_request(api, "GET", "/api/v2/audit/stats")
                    .await
                    .status,
                200
            );
            assert_eq!(
                http_request(api, "GET", "/api/v2/audit/stats/windows")
                    .await
                    .status,
                200
            );
            assert_eq!(
                http_request(api, "GET", "/api/v2/audit/logs?limit=160")
                    .await
                    .status,
                200
            );
            assert!(!dns_task.await.expect("post-full DNS client").is_empty());
        }

        shutdown.cancel();
        task.await.expect("audit read supervisor")
    });
    result.expect("audit read supervisor shutdown");
    upstream_stopped.store(true, Ordering::Release);
    upstream_task.join().expect("rich upstream shutdown");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn tcp_real_listener_preserves_positive_cname_and_audit_projection() {
    validate_response(&rich_response(&dns_query_type(0x9101, "rich.example.", 28)))
        .expect("valid rich AAAA response");
    let root = std::env::temp_dir().join(format!("mosdns-audit-tcp-read-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("TCP audit read root");
    let dns_port = free_tcp_port();
    let api_port = free_tcp_port();
    let upstream_port = free_udp_port();
    let upstream_socket =
        std::net::UdpSocket::bind(("127.0.0.1", upstream_port)).expect("TCP rich upstream bind");
    let upstream_stopped = Arc::new(AtomicBool::new(false));
    let upstream_served = Arc::new(AtomicUsize::new(0));
    let upstream_stopped_for_thread = Arc::clone(&upstream_stopped);
    let upstream_served_for_thread = Arc::clone(&upstream_served);
    let upstream_task = std::thread::spawn(move || {
        run_rich_upstream(
            &upstream_socket,
            &upstream_stopped_for_thread,
            &upstream_served_for_thread,
        );
    });
    let clock = AuditTestClock::new(at(1_700_000_000, 0));
    let host = HostAssembly::with_options_and_state_root(
        config(&root, dns_port, api_port, upstream_port, "tcp_server"),
        HostOptions::default()
            .with_audit_capacity(8)
            .with_audit_clock(Arc::new(clock)),
        &root,
    )
    .expect("assemble TCP audit read host");
    let bound = host
        .block_on(host.bind_host())
        .expect("bind TCP audit read host");
    let dns = bound.dns_addr();
    let api = bound.api_addr().expect("TCP audit read API");
    let shutdown = TransportCancellation::new();

    let result = host.block_on(async {
        let task = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let response = tokio::task::spawn_blocking(move || {
            tcp_query(dns, &dns_query_type(0x9101, "rich.example.", 28))
        })
        .await
        .expect("TCP DNS client");
        assert!(!response.is_empty());
        assert_eq!(upstream_served.load(Ordering::Relaxed), 1);
        let logs = http_request(api, "GET", "/api/v2/audit/logs?domain=rich.example").await;
        assert_eq!(logs.status, 200);
        let body: Value = serde_json::from_str(&logs.body).expect("TCP audit logs JSON");
        assert_eq!(body["pagination"]["total_items"], 1);
        assert_eq!(body["logs"][0]["query_type"], "AAAA");
        assert_eq!(body["logs"][0]["response_code"], "NOERROR");
        assert_eq!(
            body["logs"][0]["answers"]
                .as_array()
                .expect("TCP answers")
                .len(),
            2
        );
        assert_eq!(body["logs"][0]["answers"][0]["type"], "CNAME");
        assert_eq!(body["logs"][0]["answers"][1]["data"], "2001:db8::55");
        shutdown.cancel();
        task.await.expect("TCP audit read supervisor")
    });
    result.expect("TCP audit read supervisor shutdown");
    upstream_stopped.store(true, Ordering::Release);
    upstream_task.join().expect("TCP rich upstream shutdown");
    let _ = std::fs::remove_dir_all(root);
}

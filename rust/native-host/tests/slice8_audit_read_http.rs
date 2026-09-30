//! Slice 2: retained-ring v2 audit projections over real HTTP and DNS.

use std::net::{SocketAddr, TcpListener as StdTcpListener, UdpSocket as StdUdpSocket};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    packet.extend_from_slice(&[0, 0, 1, 0, 1]);
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
) -> mosdns_native_host::CompiledConfig {
    let yaml = format!(
        r#"log:
  level: error
api:
  http: "127.0.0.1:{api_port}"
plugins:
  - tag: sequence_main
    type: sequence
    args:
      - exec: reject 3
  - tag: forward_main
    type: forward
    args:
      upstreams:
        - addr: "udp://127.0.0.1:25999"
  - tag: listener
    type: udp_server
    args:
      entry: sequence_main
      listen: "127.0.0.1:{dns_port}"
      enable_audit: true
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
    let clock = AuditTestClock::new(at(1_700_000_000, 0));
    let host = HostAssembly::with_options_and_state_root(
        config(&root, dns_port, api_port),
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
        assert!(logs_json["logs"][0].get("trace_id").is_none());

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
        let unsupported = http_request(api, "GET", "/api/v2/audit/logs?q=example").await;
        assert_eq!(unsupported.status, 400);
        assert_eq!(unsupported.body, "unsupported audit log parameter: q\n");

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

        shutdown.cancel();
        task.await.expect("audit read supervisor")
    });
    result.expect("audit read supervisor shutdown");
    let _ = std::fs::remove_dir_all(root);
}

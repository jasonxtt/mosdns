//! F-2 diagnostics, against the real `mosdns` binary: the log capture routes
//! (`/api/v1/capture/*`: contract, per-query lines, expiry, restart of the
//! window, level restoration), the `mosdns_aliapi_*` upstream series of
//! `/metrics` with exact counts, and the absence of `/debug/pprof`. No public
//! network: the upstream is a local UDP stub.
#![cfg(unix)]
#![allow(clippy::pedantic)]

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn dns_query(name: &str, id: u16) -> Vec<u8> {
    let mut wire = vec![
        (id >> 8) as u8,
        id as u8,
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
    for label in name.split('.') {
        wire.push(label.len() as u8);
        wire.extend_from_slice(label.as_bytes());
    }
    wire.extend_from_slice(&[0, 0, 1, 0, 1]);
    wire
}

/// The query echoed back as an answer carrying 192.0.2.1 (TTL 60).
fn dns_answer(query: &[u8]) -> Vec<u8> {
    let mut wire = query.to_vec();
    wire[2] = 0x81;
    wire[3] = 0x80;
    wire[7] = 1;
    wire.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 192, 0, 2, 1]);
    wire
}

struct Stub {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Stub {
    fn start() -> (Self, u16) {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        socket
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::spawn(move || {
            let mut buffer = [0_u8; 512];
            while !flag.load(Ordering::SeqCst) {
                if let Ok((length, peer)) = socket.recv_from(&mut buffer) {
                    let _ = socket.send_to(&dns_answer(&buffer[..length]), peer);
                }
            }
        });
        (
            Self {
                stop,
                handle: Some(handle),
            },
            port,
        )
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

fn ask(dns: u16, name: &str) -> Option<Vec<u8>> {
    ask_type(dns, name, 1)
}

fn ask_type(dns: u16, name: &str, qtype: u16) -> Option<Vec<u8>> {
    let socket = UdpSocket::bind("127.0.0.1:0").ok()?;
    socket
        .set_read_timeout(Some(Duration::from_millis(1500)))
        .ok()?;
    let mut query = dns_query(name, 0x4242);
    let at = query.len() - 4;
    query[at..at + 2].copy_from_slice(&qtype.to_be_bytes());
    socket.send_to(&query, ("127.0.0.1", dns)).ok()?;
    let mut buffer = [0_u8; 512];
    let (length, _) = socket.recv_from(&mut buffer).ok()?;
    Some(buffer[..length].to_vec())
}

struct Reply {
    status: u16,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl Reply {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|_| panic!("JSON: {:?}", String::from_utf8_lossy(&self.body)))
    }
}

fn http(port: u16, method: &str, path: &str, body: &[u8]) -> Option<Reply> {
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(500),
    )
    .ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).ok()?;
    stream.write_all(body).ok()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).ok()?;
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&response[..split]).ok()?;
    let status = head.split(' ').nth(1)?.parse().ok()?;
    let content_type = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-type")
            .then(|| value.trim().to_owned())
    });
    Some(Reply {
        status,
        content_type,
        body: response[split + 4..].to_vec(),
    })
}

struct Host {
    root: PathBuf,
    child: Child,
    stderr: Receiver<String>,
    api: u16,
    dns: u16,
    upstream: u16,
    stub: Stub,
}

impl Host {
    fn start(name: &str) -> Self {
        common::ports::retry_addr_in_use(|| Self::try_start(name))
    }

    fn try_start(name: &str) -> Result<Self, String> {
        let root = std::env::temp_dir().join(format!("f2-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let exe = root.join("mosdns");
        std::fs::copy(env!("CARGO_BIN_EXE_mosdns"), &exe).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (stub, upstream) = Stub::start();
        let (api, dns) = (free_port(), free_udp_port());
        // `log.level: error`: debug lines reach stderr only while a capture runs.
        std::fs::write(
            root.join("config.yaml"),
            format!(
                "log: {{level: error}}\napi: {{http: '127.0.0.1:{api}'}}\nplugins:\n  - tag: cache_main\n    type: cache\n    args: {{}}\n  - tag: normal\n    type: aliapi\n    args:\n      upstreams: [{{tag: original, addr: 'udp://127.0.0.1:{upstream}', upstream_query_timeout: 400}}]\n  - tag: sub\n    type: sequence\n    args: [{{exec: $cache_main}}, {{exec: $normal}}]\n  - tag: pri\n    type: sequence\n    args: [{{matches: ['_true'], exec: $normal}}]\n  - tag: sec\n    type: sequence\n    args: [{{exec: $normal}}]\n  - tag: fb\n    type: fallback\n    args: {{primary: pri, secondary: sec}}\n  - tag: main\n    type: sequence\n    args:\n      - {{matches: ['qtype 28'], exec: 'reject 3'}}\n      - {{matches: ['qtype 15'], exec: $fb}}\n      - {{matches: ['!_false', 'qtype 1'], exec: $sub}}\n      - {{exec: accept}}\n  - tag: dns\n    type: udp_server\n    args: {{listen: '127.0.0.1:{dns}', entry: main, enable_audit: true}}\n"
            ),
        )
        .unwrap();
        // Another test thread copying its own binary at the same moment can leave this file
        // briefly open for writing in a forked child ("Text file busy"): retry the spawn.
        let mut attempt = 0;
        let mut child = loop {
            match Command::new(&exe)
                .args(["start", "-d"])
                .arg(&root)
                .arg("-c")
                .arg(root.join("config.yaml"))
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
            {
                Ok(child) => break child,
                Err(error) if error.raw_os_error() == Some(26) && attempt < 20 => {
                    attempt += 1;
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) => panic!("spawn failed: {error}"),
            }
        };
        let (sender, stderr) = channel();
        let pipe = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut host = Self {
            root,
            child,
            stderr,
            api,
            dns,
            upstream,
            stub,
        };
        common::ports::wait_ready(
            &mut host.child,
            &host.stderr,
            Duration::from_secs(20),
            || {
                http(api, "GET", "/api/v1/system/health", b"")
                    .is_some_and(|reply| reply.status == 200)
            },
        )?;
        Ok(host)
    }

    fn api(&self, method: &str, path: &str, body: &[u8]) -> Reply {
        http(self.api, method, path, body).unwrap_or_else(|| panic!("{method} {path}: no answer"))
    }

    fn metrics(&self) -> String {
        self.api("GET", "/metrics", b"").text()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.stub.stop();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out: {what}");
}

/// The value of the sample `name{labels}` in a Prometheus text document.
fn sample(text: &str, name: &str, extra: &str) -> Option<f64> {
    let prefix = format!("{name}{{");
    text.lines()
        .filter(|line| line.starts_with(&prefix) && line.contains(extra))
        .find_map(|line| line.rsplit_once(' ')?.1.parse().ok())
}

fn captured(host: &Host) -> Vec<serde_json::Value> {
    let reply = host.api("GET", "/api/v1/capture/logs", b"");
    assert_eq!(reply.status, 200);
    reply.json().as_array().unwrap().clone()
}

#[test]
fn log_capture_follows_the_reference_contract() {
    let host = Host::start("capture");
    // Capability and the absent Go profiler.
    let capabilities = host.api("GET", "/api/v1/capabilities", b"").json();
    assert_eq!(
        capabilities["ui_operations"]["capture.logs"]["supported"],
        true
    );
    for path in [
        "/debug/pprof",
        "/debug/pprof/",
        "/debug/pprof/profile",
        "/debug/pprof/cmdline",
    ] {
        assert_eq!(host.api("GET", path, b"").status, 404, "{path}");
    }

    // Methods, bounds and bodies of `start`.
    assert_eq!(host.api("GET", "/api/v1/capture/start", b"").status, 405);
    assert_eq!(host.api("POST", "/api/v1/capture/logs", b"").status, 405);
    for (body, what) in [
        (&br#"{"duration_seconds": 0}"#[..], "zero"),
        (br#"{"duration_seconds": 601}"#, "too long"),
        (br#"{"duration_seconds": -5}"#, "negative"),
    ] {
        let reply = host.api("POST", "/api/v1/capture/start", body);
        assert_eq!(reply.status, 400, "{what}");
        assert_eq!(
            reply.text(),
            "Duration must be between 1 and 600 seconds\n",
            "{what}"
        );
    }
    for body in [
        &b"{"[..],
        br#"{"duration_seconds": 1.5}"#,
        br#"{"duration_seconds": "9"}"#,
    ] {
        let reply = host.api("POST", "/api/v1/capture/start", body);
        assert_eq!(reply.status, 400, "{body:?}");
        assert!(
            reply.text().starts_with("Invalid request body: "),
            "{body:?}"
        );
    }
    // Nothing was started by the refused requests; logs are an empty array.
    let empty = host.api("GET", "/api/v1/capture/logs", b"");
    assert_eq!(
        (empty.status, empty.content_type.as_deref(), empty.text()),
        (200, Some("application/json"), "[]\n".to_owned())
    );

    // No body, `null` and `{}` keep the default of 120 seconds.
    for body in [&b""[..], b"null", b"{}", br#"{"duration_seconds": null}"#] {
        let reply = host.api("POST", "/api/v1/capture/start", body);
        assert_eq!(reply.status, 200, "{body:?}");
        assert_eq!(
            reply.text(),
            "Log capture started for 120 seconds. Log level set to DEBUG.",
            "{body:?}"
        );
    }
    // A started capture reports its length, and traces audited queries.
    let started = host.api(
        "POST",
        "/api/v1/capture/start",
        br#"{"duration_seconds": 30}"#,
    );
    assert_eq!(started.status, 200);
    assert_eq!(
        started.text(),
        "Log capture started for 30 seconds. Log level set to DEBUG."
    );
    assert!(ask(host.dns, "traced.example.com").is_some());
    wait_until("the query line", || {
        let reply = host.api("GET", "/api/v1/capture/logs", b"");
        let lines = reply.json();
        lines
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line["domain"] == "traced.example.com")
    });
    // Reading emptied the buffer (read once).
    assert_eq!(host.api("GET", "/api/v1/capture/logs", b"").text(), "[]\n");

    // The query line carries what the UI groups and shows.
    assert!(ask(host.dns, "second.example.com").is_some());
    wait_until("the second query line", || {
        host.api("GET", "/api/v1/capture/logs", b"")
            .text()
            .contains("second.example.com")
    });
    assert!(ask(host.dns, "third.example.com").is_some());
    let mut line = None;
    wait_until("the third query line", || {
        line = captured(&host).into_iter().find(|line| {
            line["domain"] == "third.example.com" && line["msg"] == "dns query completed"
        });
        line.is_some()
    });
    let line = line.unwrap();
    assert_eq!(line["level"], "debug");
    assert_eq!(line["msg"], "dns query completed");
    assert_eq!(line["logger_name"], "query");
    assert!(!line["trace_id"].as_str().unwrap().is_empty());
    assert_eq!(line["qtype"], 1);
    assert_eq!(line["rcode"], 0);
    assert_eq!(line["cache"], "miss");
    assert!(line["time"].as_str().unwrap().contains('T'));
    assert!(line["client"].as_str().unwrap().starts_with("127.0.0.1:"));

    // A second `start` empties the buffer and restarts the window.
    assert!(ask(host.dns, "old.example.com").is_some());
    std::thread::sleep(Duration::from_millis(300));
    host.api(
        "POST",
        "/api/v1/capture/start",
        br#"{"duration_seconds": 30}"#,
    );
    assert_eq!(host.api("GET", "/api/v1/capture/logs", b"").text(), "[]\n");
}

#[test]
fn a_capture_ends_at_its_deadline_and_the_log_level_comes_back() {
    let host = Host::start("expiry");
    // Debug lines are below `log.level: error`: none on stderr yet.
    host.api("GET", "/metrics", b"");
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        !host
            .stderr
            .try_iter()
            .any(|line| line.contains("Metrics endpoint accessed")),
        "debug stays below the configured level"
    );
    // During a capture the threshold drops to debug.
    host.api(
        "POST",
        "/api/v1/capture/start",
        br#"{"duration_seconds": 1}"#,
    );
    assert!(ask(host.dns, "inside.example.com").is_some());
    host.api("GET", "/metrics", b"");
    wait_until("the debug line reaches stderr", || {
        host.stderr
            .try_iter()
            .any(|line| line.contains("Metrics endpoint accessed"))
    });
    // After the deadline the configured level applies again and no line is kept.
    std::thread::sleep(Duration::from_millis(1300));
    assert!(ask(host.dns, "outside.example.com").is_some());
    host.api("GET", "/metrics", b"");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !host
            .stderr
            .try_iter()
            .any(|line| line.contains("Metrics endpoint accessed")),
        "the level was restored"
    );
    let lines = captured(&host);
    assert!(
        lines
            .iter()
            .any(|line| line["domain"] == "inside.example.com")
    );
    assert!(
        !lines
            .iter()
            .any(|line| line["domain"] == "outside.example.com")
    );
}

#[test]
fn the_upstream_series_count_attempts_errors_winners_and_latency() {
    let mut host = Host::start("series");
    let labels = format!(
        "addr=\"udp://127.0.0.1:{}\",metrics_tag=\"normal\",tag=\"original\"",
        host.upstream
    );
    // Series exist at zero before any query, with the reference's help and type lines.
    let initial = host.metrics();
    assert_eq!(
        sample(&initial, "mosdns_aliapi_query_total", &labels),
        Some(0.0)
    );
    assert!(initial.contains("# TYPE mosdns_aliapi_response_latency_millisecond histogram\n"));
    assert!(initial.contains("# HELP mosdns_aliapi_upstream_winner_total "));
    for absent in [
        "inflight_queries",
        "conn_opened_total",
        "conn_closed_total",
        "go_goroutines",
    ] {
        assert!(!initial.contains(absent), "{absent}");
    }
    assert!(initial.contains("mosdns_cache_query_total{tag=\"cache_main\"}"));

    // Three distinct names miss the cache and reach the upstream, which answers each.
    for index in 0..3 {
        assert!(ask(host.dns, &format!("ok{index}.example.com")).is_some());
    }
    wait_until("three attempts counted", || {
        sample(&host.metrics(), "mosdns_aliapi_query_total", &labels) == Some(3.0)
    });
    let text = host.metrics();
    assert_eq!(
        sample(&text, "mosdns_aliapi_error_total", &labels),
        Some(0.0)
    );
    assert_eq!(
        sample(&text, "mosdns_aliapi_upstream_winner_total", &labels),
        Some(3.0)
    );
    assert_eq!(
        sample(
            &text,
            "mosdns_aliapi_response_latency_millisecond_count",
            &labels
        ),
        Some(3.0)
    );
    assert_eq!(
        sample(
            &text,
            "mosdns_aliapi_response_latency_millisecond_bucket",
            &format!("{labels},le=\"+Inf\"")
        ),
        Some(3.0)
    );
    // A local loopback stub answers well under 5 s: every observation is in a finite bucket.
    assert_eq!(
        sample(
            &text,
            "mosdns_aliapi_response_latency_millisecond_bucket",
            &format!("{labels},le=\"5000\"")
        ),
        Some(3.0)
    );

    // A cache hit makes no upstream attempt.
    assert!(ask(host.dns, "ok0.example.com").is_some());
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        sample(&host.metrics(), "mosdns_aliapi_query_total", &labels),
        Some(3.0)
    );

    // With the upstream gone, attempts fail: counted as queries and errors, never as winners.
    host.stub.stop();
    for index in 0..2 {
        let _ = ask(host.dns, &format!("bad{index}.example.com"));
    }
    wait_until("two failed attempts counted", || {
        sample(&host.metrics(), "mosdns_aliapi_query_total", &labels) == Some(5.0)
    });
    let text = host.metrics();
    assert_eq!(
        sample(&text, "mosdns_aliapi_error_total", &labels),
        Some(2.0)
    );
    assert_eq!(
        sample(&text, "mosdns_aliapi_upstream_winner_total", &labels),
        Some(3.0)
    );
    assert_eq!(
        sample(
            &text,
            "mosdns_aliapi_response_latency_millisecond_count",
            &labels
        ),
        Some(3.0),
        "only successful attempts are observed"
    );
}

/// The `(logger_name, kind, name, result)` steps of one query's lines, in time order, from the
/// reference's two step messages.
fn steps(lines: &[serde_json::Value], domain: &str) -> Vec<String> {
    let mut mine: Vec<&serde_json::Value> = lines
        .iter()
        .filter(|line| line["domain"] == domain)
        .collect();
    mine.sort_by_key(|line| line["time"].as_str().unwrap().to_owned());
    mine.iter()
        .filter_map(|line| match line["msg"].as_str().unwrap() {
            "dns query flows through matcher" => Some(format!(
                "{}|matcher|{}|{}",
                line["logger_name"].as_str().unwrap(),
                line["matcher_name"].as_str().unwrap(),
                line["match_result"]
            )),
            "dns query flows through plugin" => Some(format!(
                "{}|plugin|{}",
                line["logger_name"].as_str().unwrap(),
                line["plugin_name"].as_str().unwrap()
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn a_capture_reports_every_matcher_and_action_in_the_reference_form() {
    let host = Host::start("steps");
    // Before a capture: no lines at all (and nothing is kept).
    assert!(ask(host.dns, "before.example.com").is_some());
    std::thread::sleep(Duration::from_millis(200));
    host.api(
        "POST",
        "/api/v1/capture/start",
        br#"{"duration_seconds": 30}"#,
    );
    assert!(ask(host.dns, "steps.example.com").is_some());
    let mut lines = Vec::new();
    wait_until("the summary line of the query", || {
        lines.extend(captured(&host));
        lines.iter().any(|line| {
            line["domain"] == "steps.example.com" && line["msg"] == "dns query completed"
        })
    });
    // The first query misses the cache and goes upstream: every rule of both sequences.
    assert_eq!(
        steps(&lines, "steps.example.com"),
        [
            "main|matcher|anonymous_match(qtype: 28)|false",
            "main|matcher|anonymous_match(qtype: 15)|false",
            "main|matcher|not(anonymous_match(_false: ))|true",
            "main|matcher|anonymous_match(qtype: 1)|true",
            "main|plugin|sub",
            "sub|plugin|cache_main",
            "sub|plugin|normal",
            "main|plugin|anonymous_exec(accept: )",
        ]
    );
    // One trace id ties the steps to the summary line; steps carry the query id and no `time` skew.
    let ids: std::collections::BTreeSet<_> = lines
        .iter()
        .filter(|line| line["domain"] == "steps.example.com")
        .map(|line| line["trace_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids.len(), 1, "{ids:?}");
    let plugin = lines
        .iter()
        .find(|line| line["plugin_name"] == "sub" && line["domain"] == "steps.example.com")
        .unwrap();
    assert_eq!(plugin["query_id"], 0x4242);
    assert_eq!(plugin["level"], "debug");
    assert!(
        lines
            .iter()
            .filter(|line| line["domain"] == "steps.example.com")
            .all(|line| line["time"].as_str().unwrap().contains('T'))
    );
    // Nothing from the query made before the capture.
    assert!(
        lines
            .iter()
            .all(|line| line["domain"] != "before.example.com")
    );

    // A second ask of the same name is a cache hit: the cache step is reported, the upstream one is not.
    assert!(ask(host.dns, "steps.example.com").is_some());
    let mut again = Vec::new();
    wait_until("the second summary line", || {
        again.extend(captured(&host));
        again.iter().any(|line| {
            line["domain"] == "steps.example.com" && line["msg"] == "dns query completed"
        })
    });
    let second = steps(&again, "steps.example.com");
    assert!(
        second.contains(&"sub|plugin|cache_main".to_owned()),
        "{second:?}"
    );
    assert!(
        !second.contains(&"sub|plugin|normal".to_owned()),
        "{second:?}"
    );

    // An AAAA query takes the first rule's action, and a name with a trailing dot form is the same name.
    let reply = ask_type(host.dns, "v6.example.com", 28).unwrap();
    assert_eq!(reply[3] & 0x0f, 3, "reject 3 answers NXDOMAIN");
    let mut v6 = Vec::new();
    wait_until("the AAAA summary line", || {
        v6.extend(captured(&host));
        v6.iter()
            .any(|line| line["domain"] == "v6.example.com" && line["msg"] == "dns query completed")
    });
    assert_eq!(
        steps(&v6, "v6.example.com"),
        [
            "main|matcher|anonymous_match(qtype: 28)|true",
            "main|plugin|anonymous_exec(reject: 3)",
        ]
    );

    // A fallback runs its primary branch as a nested sequence under the same trace id.
    assert!(ask_type(host.dns, "mx.example.com", 15).is_some());
    let mut mx = Vec::new();
    wait_until("the MX summary line", || {
        mx.extend(captured(&host));
        mx.iter()
            .any(|line| line["domain"] == "mx.example.com" && line["msg"] == "dns query completed")
    });
    let flow = steps(&mx, "mx.example.com");
    for expected in [
        "main|matcher|anonymous_match(qtype: 15)|true",
        "main|plugin|fb",
        "pri|matcher|anonymous_match(_true: )|true",
        "pri|plugin|normal",
    ] {
        assert!(flow.contains(&expected.to_owned()), "{expected}: {flow:?}");
    }
    let ids: std::collections::BTreeSet<_> = mx
        .iter()
        .filter(|line| line["domain"] == "mx.example.com")
        .map(|line| line["trace_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids.len(), 1, "{ids:?}");
}

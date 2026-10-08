//! M1-c2: the native `requery` plugin through real sockets: the job's steps
//! (save, merge, flush, rate-limited re-query through the host's own listener,
//! save), conflict and cancellation, restart recovery, the scheduler, the API
//! and the reference file format. No test reaches the public network.

#![allow(clippy::too_many_lines)]

use std::net::UdpSocket as StdUdpSocket;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mosdns_native_host::{BoundHost, HostAssembly, HostOptions};
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// ----- fixtures ---------------------------------------------------------------

struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("m1-c2-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn text(&self, relative: &str) -> String {
        std::fs::read_to_string(self.0.join(relative)).unwrap_or_default()
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Ports {
    dns: u16,
    api: u16,
}

fn ports() -> Ports {
    Ports {
        dns: free_port(),
        api: free_port(),
    }
}

/// A UDP DNS upstream on a thread: answers every A / AAAA query and records
/// `(name, qtype, flags)`.
struct Upstream {
    address: String,
    seen: Arc<Mutex<Vec<(String, u16, u8)>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

fn question(packet: &[u8]) -> (String, u16, usize) {
    let mut position = 12;
    let mut labels = Vec::new();
    while packet[position] != 0 {
        let length = usize::from(packet[position]);
        labels.push(
            String::from_utf8_lossy(&packet[position + 1..position + 1 + length]).into_owned(),
        );
        position += 1 + length;
    }
    let end = position + 5;
    let qtype = u16::from_be_bytes([packet[end - 4], packet[end - 3]]);
    (labels.join("."), qtype, end)
}

impl Upstream {
    fn start() -> Self {
        let socket = StdUdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let address = socket.local_addr().unwrap().to_string();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (record, stopped) = (Arc::clone(&seen), Arc::clone(&stop));
        let thread = thread::spawn(move || {
            let mut packet = vec![0_u8; 4096];
            while !stopped.load(Ordering::SeqCst) {
                let Ok((length, client)) = socket.recv_from(&mut packet) else {
                    continue;
                };
                let query = &packet[..length];
                let (name, qtype, end) = question(query);
                record.lock().unwrap().push((name, qtype, query[3]));
                let mut reply = vec![query[0], query[1], 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0];
                reply.extend_from_slice(&query[12..end]);
                reply.extend_from_slice(&[0xc0, 0x0c]);
                reply.extend_from_slice(&qtype.to_be_bytes());
                reply.extend_from_slice(&[0, 1, 0, 0, 0, 60]);
                let data: Vec<u8> = if qtype == 28 {
                    let mut data = vec![0x20, 0x01, 0x0d, 0xb8];
                    data.extend([0; 11]);
                    data.push(1);
                    data
                } else {
                    vec![192, 0, 2, 1]
                };
                reply.extend_from_slice(&u16::try_from(data.len()).unwrap().to_be_bytes());
                reply.extend_from_slice(&data);
                let _ = socket.send_to(&reply, client);
            }
        });
        Self {
            address,
            seen,
            stop,
            thread: Some(thread),
        }
    }

    fn queries(&self) -> Vec<(String, u16, u8)> {
        self.seen.lock().unwrap().clone()
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

/// The host: `$top_domains` then `$cache` then the upstream; the plugin under
/// test is `requery`, its state file `webinfo/requeryconfig.json`.
fn config(tree: &Tree, ports: &Ports, upstream: &Upstream) {
    let yaml = format!(
        r#"log: {{level: error}}
api: {{http: "127.0.0.1:{api}"}}
plugins:
  - tag: fwd
    type: forward
    args: {{upstreams: [{{addr: "udp://{upstream}"}}]}}
  - tag: cache
    type: cache
    args: {{dump_file: "cache/c.gz"}}
  - tag: top_domains
    type: domain_output
    args: {{file_stat: gen/top.txt, dump_interval: 3600}}
  - tag: requery
    type: requery
    args: {{file: webinfo/requeryconfig.json}}
  - tag: entry
    type: sequence
    args:
      - exec: $top_domains
      - exec: $cache
      - exec: $fwd
  - tag: listener
    type: udp_server
    args: {{entry: entry, listen: "127.0.0.1:{dns}"}}
"#,
        api = ports.api,
        dns = ports.dns,
        upstream = upstream.address,
    );
    tree.write("config.yaml", &yaml);
    // The cache saves into this directory.
    std::fs::create_dir_all(tree.path("cache")).unwrap();
}

/// A requery state file: save/flush the stats and the cache, ask the host's
/// own listener, `qps` names per second.
fn state(
    tree: &Tree,
    ports: &Ports,
    qps: i64,
    sources: &[(&str, &str)],
    scheduler: &str,
    status: &str,
) {
    let sources: Vec<String> = sources
        .iter()
        .map(|(alias, path)| format!(r#"{{"alias": "{alias}", "path": "{path}"}}"#))
        .collect();
    let api = ports.api;
    let text = format!(
        r#"{{
  "domain_processing": {{"source_files": [{sources}]}},
  "url_actions": {{
    "save_rules": ["http://127.0.0.1:9099/plugins/top_domains/save", "http://localhost:{api}/plugins/cache/save"],
    "flush_rules": ["http://127.0.0.1:{api}/plugins/top_domains/flush", "http://127.0.0.1:{api}/plugins/cache/flush"]
  }},
  "scheduler": {scheduler},
  "execution_settings": {{"queries_per_second": {qps}, "resolver_address": "127.0.0.1:{dns}", "url_call_delay_ms": 5, "date_range_days": 30}},
  "status": {status}
}}"#,
        sources = sources.join(", "),
        dns = ports.dns,
    );
    tree.write("webinfo/requeryconfig.json", &text);
}

const NO_SCHEDULER: &str = r#"{"enabled": false, "start_datetime": "", "interval_minutes": 0}"#;
const IDLE: &str = r#"{"task_state": "idle", "progress": {"processed": 0, "total": 0}}"#;

fn host(tree: &Tree) -> HostAssembly {
    let config = mosdns_native_host::load_and_compile(&tree.path("config.yaml")).expect("compiles");
    HostAssembly::with_options(config, HostOptions::default()).expect("assembles")
}

async fn serve(host: &HostAssembly) -> (TransportCancellation, tokio::task::JoinHandle<()>) {
    let bound: BoundHost = host.bind_host().await.expect("binds");
    let shutdown = TransportCancellation::new();
    let scope = shutdown.clone();
    let task = tokio::task::spawn_local(async move {
        bound.serve(scope).await.expect("clean shutdown");
    });
    (shutdown, task)
}

struct Http {
    status: u16,
    body: String,
}

async fn api(port: u16, method: &str, path: &str, body: &str) -> Http {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: native\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.unwrap();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").expect("http response");
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    Http {
        status,
        body: body.to_owned(),
    }
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("{error}: {text}"))
}

async fn status(ports: &Ports) -> serde_json::Value {
    let reply = api(ports.api, "GET", "/plugins/requery/status", "").await;
    assert_eq!(reply.status, 200);
    json(&reply.body)
}

async fn until_state(ports: &Ports, wanted: &str) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let current = status(ports).await;
        if current["task_state"] == wanted {
            return current;
        }
        assert!(
            Instant::now() < deadline,
            "never reached {wanted}; now {current}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` of a Unix second.
fn rfc3339(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86400).unwrap();
    let rest = seconds % 86400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

// ----- tests ------------------------------------------------------------------

#[test]
fn a_task_saves_merges_flushes_requeries_and_saves_again() {
    let tree = Tree::new("run");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream);
    tree.write(
        "gen/top.txt",
        "0000000003 9999-12-31 a.example\n0000000002 9999-12-31 b.example|AD\n0000000001 2000-01-01 old.example\n",
    );
    tree.write("gen/extra.txt", "full:c.example\n");
    // The statistics file is saved by the first step and read by the second.
    state(
        &tree,
        &ports,
        200,
        &[("stats", "gen/top.txt"), ("extra", "gen/extra.txt")],
        NO_SCHEDULER,
        IDLE,
    );
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let started = api(ports.api, "POST", "/plugins/requery/trigger", "").await;
        assert_eq!(started.status, 200);
        assert_eq!(
            started.body,
            "{\"message\":\"A new task has been started.\",\"status\":\"success\"}\n"
        );
        let done = until_state(&ports, "idle").await;
        assert_eq!(done["last_run_domain_count"], 3);
        assert_eq!(
            done["progress"],
            serde_json::json!({"processed": 3, "total": 3})
        );
        assert!(done.get("last_error").is_none(), "{done}");
        assert_ne!(done["last_run_start_time"], "0001-01-01T00:00:00Z");
        assert_ne!(done["last_run_end_time"], "0001-01-01T00:00:00Z");

        // Only the fresh names were asked for, each as A and AAAA, after the
        // flush; AD of the flagged one came along.
        let mut asked = upstream.queries();
        asked.sort();
        assert_eq!(
            asked
                .iter()
                .map(|q| (q.0.as_str(), q.1))
                .collect::<Vec<_>>(),
            [
                ("a.example", 1),
                ("a.example", 28),
                ("b.example", 1),
                ("b.example", 28),
                ("c.example", 1),
                ("c.example", 28)
            ]
        );
        assert!(
            asked
                .iter()
                .filter(|q| q.0 == "b.example")
                .all(|q| q.2 & 0x20 != 0)
        );
        assert!(
            asked
                .iter()
                .filter(|q| q.0 != "b.example")
                .all(|q| q.2 & 0x20 == 0)
        );

        // The final save wrote the statistics the re-queries produced (the
        // flush emptied them first, so the old name is gone), and the cache.
        let stat = tree.text("gen/top.txt");
        let keys: Vec<&str> = stat.lines().filter_map(|l| l.split(' ').nth(2)).collect();
        assert_eq!(keys, ["a.example", "b.example", "c.example"]);
        assert!(stat.lines().all(|l| l.starts_with("0000000002 ")), "{stat}");
        assert!(tree.path("cache/c.gz").exists());
        // The state file holds the settled state in the reference format.
        let saved = json(&tree.text("webinfo/requeryconfig.json"));
        assert_eq!(saved["status"]["task_state"], "idle");
        assert_eq!(saved["status"]["progress"]["total"], 3);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn a_second_trigger_conflicts_and_cancel_stops_the_task_at_once() {
    let tree = Tree::new("cancel");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream);
    tree.write(
        "gen/extra.txt",
        "full:a.example\nfull:b.example\nfull:c.example\nfull:d.example\nfull:e.example\nfull:f.example\n",
    );
    state(
        &tree,
        &ports,
        4,
        &[("extra", "gen/extra.txt")],
        NO_SCHEDULER,
        IDLE,
    );
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        assert_eq!(
            api(ports.api, "POST", "/plugins/requery/cancel", "")
                .await
                .status,
            404
        );
        assert_eq!(
            api(ports.api, "POST", "/plugins/requery/trigger", "")
                .await
                .status,
            200
        );
        // Wait until names are being asked for.
        let deadline = Instant::now() + Duration::from_secs(10);
        while upstream.queries().len() < 2 {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let conflict = api(ports.api, "POST", "/plugins/requery/trigger", "").await;
        assert_eq!(conflict.status, 409);
        assert_eq!(
            conflict.body,
            "{\"message\":\"A task is already running.\",\"status\":\"error\"}\n"
        );
        let cancel = api(ports.api, "POST", "/plugins/requery/cancel", "").await;
        assert_eq!(cancel.status, 200);
        assert_eq!(
            cancel.body,
            "{\"message\":\"Task cancellation initiated.\",\"status\":\"success\"}\n"
        );
        let stopped = until_state(&ports, "cancelled").await;
        assert_eq!(stopped["last_error"], "task cancelled by user");
        assert!(stopped["progress"]["processed"].as_i64().unwrap() < 6);
        // Nothing is asked after the cancellation.
        let after = upstream.queries().len();
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert_eq!(upstream.queries().len(), after);
        assert_eq!(
            api(ports.api, "POST", "/plugins/requery/cancel", "")
                .await
                .body,
            "{\"message\":\"No running task to cancel.\",\"status\":\"error\"}\n"
        );
        // A new task can start again. Cancelling it while it still saves
        // fails it (the reference does the same); during the queries it is
        // "cancelled".
        let before = upstream.queries().len();
        assert_eq!(
            api(ports.api, "POST", "/plugins/requery/trigger", "")
                .await
                .status,
            200
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while upstream.queries().len() < before + 2 {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        api(ports.api, "POST", "/plugins/requery/cancel", "").await;
        until_state(&ports, "cancelled").await;
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn a_task_found_running_at_start_is_marked_failed_and_defaults_are_filled() {
    let tree = Tree::new("recover");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream);
    tree.write(
        "webinfo/requeryconfig.json",
        r#"{"execution_settings": {"queries_per_second": 0, "url_call_delay_ms": 0}, "status": {"task_state": "running", "progress": {"processed": 5, "total": 9}}}"#,
    );
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let current = status(&ports).await;
        assert_eq!(current["task_state"], "failed");
        assert_ne!(current["last_run_end_time"], "0001-01-01T00:00:00Z");
        assert_eq!(current["progress"]["processed"], 5);
        let everything = json(&api(ports.api, "GET", "/plugins/requery/", "").await.body);
        assert_eq!(everything["execution_settings"]["queries_per_second"], 100);
        assert_eq!(everything["execution_settings"]["url_call_delay_ms"], 50);
        assert_eq!(everything["execution_settings"]["date_range_days"], 30);
        // The file was rewritten with both.
        let saved = json(&tree.text("webinfo/requeryconfig.json"));
        assert_eq!(saved["status"]["task_state"], "failed");
        assert_eq!(saved["execution_settings"]["queries_per_second"], 100);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn a_missing_state_file_is_created_with_the_reference_defaults() {
    let tree = Tree::new("missing");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream);
    let host = host(&tree);
    // Compiling alone writes nothing.
    assert!(!tree.path("webinfo/requeryconfig.json").exists());
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !tree.path("webinfo/requeryconfig.json").exists() {
            assert!(Instant::now() < deadline, "the file was never created");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let saved = json(&tree.text("webinfo/requeryconfig.json"));
        assert_eq!(saved["status"]["task_state"], "idle");
        assert_eq!(saved["execution_settings"]["date_range_days"], 30);
        assert_eq!(
            saved["domain_processing"]["source_files"],
            serde_json::Value::Null
        );
        assert_eq!(saved["scheduler"]["enabled"], false);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn the_scheduler_starts_a_task_at_the_start_time_and_follows_its_configuration() {
    let tree = Tree::new("schedule");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream);
    tree.write("gen/extra.txt", "full:s.example\n");
    let begin = rfc3339(now_seconds() + 2);
    state(
        &tree,
        &ports,
        100,
        &[("extra", "gen/extra.txt")],
        &format!(r#"{{"enabled": true, "start_datetime": "{begin}", "interval_minutes": 1440}}"#),
        IDLE,
    );
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        assert_eq!(status(&ports).await["last_run_start_time"], "0001-01-01T00:00:00Z");
        // Fires at the start time, once.
        let deadline = Instant::now() + Duration::from_secs(10);
        let ran = loop {
            let current = status(&ports).await;
            if current["last_run_start_time"] != "0001-01-01T00:00:00Z"
                && current["task_state"] == "idle"
            {
                break current;
            }
            assert!(Instant::now() < deadline, "never ran: {current}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        assert_eq!(ran["last_run_domain_count"], 1);
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(
            upstream.queries().iter().filter(|q| q.0 == "s.example").count(),
            2,
            "A and AAAA once"
        );

        // The API replaces the scheduler section and the range.
        let bad = api(ports.api, "POST", "/plugins/requery/scheduler/config", "nope").await;
        assert_eq!(bad.status, 400);
        assert_eq!(bad.body, "{\"message\":\"Invalid JSON body\",\"status\":\"error\"}\n");
        let update = api(
            ports.api,
            "POST",
            "/plugins/requery/scheduler/config",
            r#"{"enabled": false, "start_datetime": "2030-01-01T00:00:00Z", "interval_minutes": 7, "date_range_days": 12}"#,
        )
        .await;
        assert_eq!(update.status, 200);
        assert_eq!(
            update.body,
            "{\"message\":\"Scheduler configuration updated successfully.\",\"status\":\"success\"}\n"
        );
        let saved = json(&tree.text("webinfo/requeryconfig.json"));
        assert_eq!(saved["scheduler"]["enabled"], false);
        assert_eq!(saved["scheduler"]["interval_minutes"], 7);
        assert_eq!(saved["execution_settings"]["date_range_days"], 12);
        // A zero range leaves the old one.
        api(
            ports.api,
            "POST",
            "/plugins/requery/scheduler/config",
            r#"{"enabled": true, "start_datetime": "2030-01-01T00:00:00Z", "interval_minutes": 7}"#,
        )
        .await;
        let saved = json(&tree.text("webinfo/requeryconfig.json"));
        assert_eq!(saved["execution_settings"]["date_range_days"], 12);
        assert_eq!(saved["scheduler"]["enabled"], true);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn a_failing_url_action_fails_the_task_with_the_reference_message() {
    let tree = Tree::new("failure");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream);
    tree.write("gen/top.txt", "full:a.example\n");
    state(
        &tree,
        &ports,
        100,
        &[("all", "gen/top.txt")],
        NO_SCHEDULER,
        IDLE,
    );
    // Replace the save list by a plugin that does not exist.
    let text = tree
        .text("webinfo/requeryconfig.json")
        .replace("plugins/top_domains/save", "plugins/nothing/save");
    tree.write("webinfo/requeryconfig.json", &text);
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        api(ports.api, "POST", "/plugins/requery/trigger", "").await;
        let failed = until_state(&ports, "failed").await;
        assert_eq!(
            failed["last_error"],
            format!(
                "failed during save_rules step: bad response from URL http://127.0.0.1:{}/plugins/nothing/save: status 404, body: 404 page not found\n",
                ports.api
            )
        );
        // The failure came before any flush or query.
        assert!(upstream.queries().is_empty());
        // The count endpoint saves first, so it fails the same way.
        let counts = api(ports.api, "GET", "/plugins/requery/stats/source_file_counts", "").await;
        assert_eq!(counts.status, 500);
        assert!(counts.body.contains("Failed to save rules before counting: bad response from URL"), "{}", counts.body);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn source_file_counts_save_first_and_count_full_lines() {
    let tree = Tree::new("counts");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream);
    tree.write(
        "gen/extra.txt",
        "full:a.example\nfull:b.example\n0000000001 9999-12-31 c.example\n  full:d.example\n",
    );
    state(
        &tree,
        &ports,
        100,
        &[("all", "gen/extra.txt"), ("gone", "gen/none.txt")],
        NO_SCHEDULER,
        IDLE,
    );
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let counts = api(ports.api, "GET", "/plugins/requery/stats/source_file_counts", "").await;
        assert_eq!(counts.status, 200);
        assert_eq!(
            counts.body,
            "{\"data\":[{\"alias\":\"all\",\"count\":2},{\"alias\":\"gone\",\"count\":0}],\"status\":\"success\"}\n"
        );
        assert!(tree.path("cache/c.gz").exists(), "the cache was saved first");
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn an_other_hosts_url_is_called_over_http() {
    let tree = Tree::new("http");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream);
    tree.write("gen/top.txt", "full:h.example\n");
    state(
        &tree,
        &ports,
        100,
        &[("all", "gen/top.txt")],
        NO_SCHEDULER,
        IDLE,
    );
    let listener = std::net::TcpListener::bind("127.0.0.2:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let record = Arc::clone(&seen);
    let server = thread::spawn(move || {
        use std::io::{Read, Write};
        // One call per save step (two) is enough; answer 200 each time.
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 2048];
            let length = stream.read(&mut buffer).unwrap_or(0);
            record.lock().unwrap().push(
                String::from_utf8_lossy(&buffer[..length])
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_owned(),
            );
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        }
    });
    // 127.0.0.2 is not one of the hosts the reference treats as itself.
    let text = tree.text("webinfo/requeryconfig.json").replace(
        "\"http://127.0.0.1:9099/plugins/top_domains/save\"",
        &format!("\"http://127.0.0.2:{port}/elsewhere/save\""),
    );
    tree.write("webinfo/requeryconfig.json", &text);
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        api(ports.api, "POST", "/plugins/requery/trigger", "").await;
        until_state(&ports, "idle").await;
        shutdown.cancel();
        task.await.unwrap();
    });
    server.join().unwrap();
    let calls = seen.lock().unwrap().clone();
    assert_eq!(
        calls,
        [
            "GET /elsewhere/save HTTP/1.1",
            "GET /elsewhere/save HTTP/1.1"
        ]
    );
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/requery")
            .join(name),
    )
    .unwrap()
}

/// The reference tool prints the status line, then the body.
fn golden(name: &str) -> (u16, String) {
    let text = fixture(name);
    let (code, body) = text.split_once('\n').unwrap();
    (code.parse().unwrap(), body.to_owned())
}

#[test]
fn a_file_written_by_the_reference_is_read_and_answered_identically() {
    let tree = Tree::new("interop");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream);
    tree.write("webinfo/requeryconfig.json", &fixture("go_written.json"));
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let (code, body) = golden("go_get_root.txt");
        let root = api(ports.api, "GET", "/plugins/requery/", "").await;
        assert_eq!((root.status, root.body), (code, body));
        let (code, body) = golden("go_get_status.txt");
        let current = api(ports.api, "GET", "/plugins/requery/status", "").await;
        assert_eq!((current.status, current.body), (code, body));
        // The reference's answer to a scheduler update, then to the whole
        // document, on the same input.
        let update = api(
            ports.api,
            "POST",
            "/plugins/requery/scheduler/config",
            r#"{"enabled":false,"start_datetime":"2026-01-01T00:00:00Z","interval_minutes":7,"date_range_days":12}"#,
        )
        .await;
        let text = fixture("go_post_scheduler.txt");
        let mut lines = text.splitn(4, '\n');
        let (first_code, first_body) = (lines.next().unwrap(), lines.next().unwrap());
        let (second_code, second_body) = (lines.next().unwrap(), lines.next().unwrap());
        assert_eq!((update.status.to_string().as_str(), update.body.as_str()), (first_code, format!("{first_body}\n").as_str()));
        let everything = api(ports.api, "GET", "/plugins/requery/", "").await;
        assert_eq!(everything.status.to_string(), second_code);
        assert_eq!(everything.body, second_body);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn an_unparsable_state_file_fails_the_configuration() {
    let tree = Tree::new("badfile");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream);
    tree.write("webinfo/requeryconfig.json", "{not json");
    let error = mosdns_native_host::load_and_compile(&tree.path("config.yaml"))
        .err()
        .expect("must fail")
        .to_string();
    assert!(
        error.contains("failed to parse json from config file"),
        "{error}"
    );
    // `file` is required.
    let yaml = tree
        .text("config.yaml")
        .replace("args: {file: webinfo/requeryconfig.json}", "args: {}");
    tree.write("config.yaml", &yaml);
    let error = mosdns_native_host::load_and_compile(&tree.path("config.yaml"))
        .err()
        .expect("must fail")
        .to_string();
    assert!(
        error.contains("'file' for config json must be specified"),
        "{error}"
    );
}

#[test]
fn stopping_the_host_cancels_a_running_task_and_records_it() {
    let tree = Tree::new("shutdown");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream);
    tree.write(
        "gen/extra.txt",
        "full:a.example\nfull:b.example\nfull:c.example\nfull:d.example\n",
    );
    state(
        &tree,
        &ports,
        2,
        &[("extra", "gen/extra.txt")],
        NO_SCHEDULER,
        IDLE,
    );
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        api(ports.api, "POST", "/plugins/requery/trigger", "").await;
        let deadline = Instant::now() + Duration::from_secs(10);
        while upstream.queries().len() < 2 {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        shutdown.cancel();
        // The host returns only after the task is joined.
        task.await.unwrap();
    });
    let after = upstream.queries().len();
    let saved = json(&tree.text("webinfo/requeryconfig.json"));
    assert_eq!(saved["status"]["task_state"], "cancelled");
    assert_eq!(
        saved["status"]["last_error"],
        "task cancelled by host shutdown"
    );
    assert_ne!(saved["status"]["last_run_end_time"], "0001-01-01T00:00:00Z");
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(
        upstream.queries().len(),
        after,
        "no query after the host stopped"
    );
}

#[test]
fn stopping_the_host_during_a_url_step_is_recorded_as_cancelled() {
    let tree = Tree::new("stalled");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream);
    tree.write("gen/extra.txt", "full:a.example\n");
    state(
        &tree,
        &ports,
        100,
        &[("extra", "gen/extra.txt")],
        NO_SCHEDULER,
        IDLE,
    );
    // A server that accepts and never answers: the save step hangs.
    let listener = std::net::TcpListener::bind("127.0.0.2:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let text = tree.text("webinfo/requeryconfig.json").replace(
        "\"http://127.0.0.1:9099/plugins/top_domains/save\"",
        &format!("\"http://127.0.0.2:{port}/stall\""),
    );
    tree.write("webinfo/requeryconfig.json", &text);
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        api(ports.api, "POST", "/plugins/requery/trigger", "").await;
        let (accepted, _) = tokio::task::spawn_blocking(move || listener.accept().unwrap())
            .await
            .unwrap();
        shutdown.cancel();
        task.await.unwrap();
        drop(accepted);
    });
    let saved = json(&tree.text("webinfo/requeryconfig.json"));
    assert_eq!(saved["status"]["task_state"], "cancelled");
    assert_eq!(
        saved["status"]["last_error"],
        "task cancelled by host shutdown"
    );
}

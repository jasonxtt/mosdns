//! M1-c1: the native `domain_output` plugin — recording, the reference file
//! formats, periodic / early / final writes, in-process and HTTP rule
//! publication, immediate `target_mapper` injection and the management API,
//! through real UDP queries and the real management listener. No test reaches
//! the public network.

#![allow(clippy::too_many_lines)]

use std::fmt::Write as _;
use std::io::{Read, Write as _};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use mosdns_native_host::{BoundHost, HostAssembly, HostOptions};
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// ----- fixtures -------------------------------------------------------------

struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("m1-c1-{name}-{}", std::process::id()));
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

    /// Replaces a file by a directory of the same name, so the next atomic
    /// rename onto it fails.
    fn block(&self, relative: &str) {
        let path = self.path(relative);
        let _ = std::fs::remove_file(&path);
        std::fs::create_dir_all(path).unwrap();
    }

    fn leftovers(&self, directory: &str) -> Vec<String> {
        std::fs::read_dir(self.path(directory))
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .filter(|name| name.contains(".tmp-"))
                    .collect()
            })
            .unwrap_or_default()
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

/// A host whose entry runs the mapper, then the outputs listed in `outputs`
/// (their definitions in `definitions`), then answers by fast mark:
/// 9 -> rcode 9, 4 -> rcode 4, 17 (the mapper default) -> rcode 7.
fn config(tree: &Tree, ports: &Ports, definitions: &str, outputs: &[&str]) {
    let mut execs = String::new();
    for tag in outputs {
        let _ = writeln!(execs, "      - exec: ${tag}");
    }
    let yaml = format!(
        r#"log: {{level: error}}
api: {{http: "127.0.0.1:{api}"}}
plugins:
  - tag: fwd
    type: forward
    args: {{upstreams: [{{addr: "udp://127.0.0.1:9"}}]}}
  - tag: sv4rule
    type: domain_set_light
    args: {{files: ["gen/sv4rule.txt"]}}
  - tag: unified
    type: domain_mapper
    args:
      default_mark: 17
      rules:
        - {{tag: sv4rule, mark: 9}}
{definitions}  - tag: entry
    type: sequence
    args:
      - exec: $unified
{execs}      - matches: fast_mark 9
        exec: reject 9
      - matches: fast_mark 4
        exec: reject 4
      - matches: fast_mark 17
        exec: reject 7
      - exec: reject 0
  - tag: listener
    type: udp_server
    args: {{entry: entry, listen: "127.0.0.1:{dns}"}}
"#,
        api = ports.api,
        dns = ports.dns,
    );
    tree.write("config.yaml", &yaml);
}

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
    headers: Vec<(String, String)>,
    body: String,
}

impl Http {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
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
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((key.trim().to_owned(), value.trim().to_owned()))
        })
        .collect();
    Http {
        status,
        headers,
        body: body.to_owned(),
    }
}

fn wire(name: &str, flags: u8, dnssec: bool) -> Vec<u8> {
    let mut out = vec![0, 7, 1, flags, 0, 1, 0, 0, 0, 0, 0, u8::from(dnssec)];
    for label in name.trim_end_matches('.').split('.') {
        out.push(u8::try_from(label.len()).unwrap());
        out.extend_from_slice(label.as_bytes());
    }
    out.extend([0, 0, 1, 0, 1]);
    if dnssec {
        out.extend([0, 0, 41, 16, 0, 0, 0, 0x80, 0, 0, 0]);
    }
    out
}

async fn ask(port: u16, query: &[u8]) -> u8 {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.send_to(query, ("127.0.0.1", port)).await.unwrap();
    let mut packet = vec![0; 1500];
    let (length, _) = tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut packet))
        .await
        .expect("response in time")
        .unwrap();
    assert!(length >= 12);
    packet[3] & 15
}

async fn rcode(port: u16, name: &str) -> u8 {
    ask(port, &wire(name, 0, false)).await
}

async fn eventually_rcode(port: u16, name: &str, expected: u8) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let got = rcode(port, name).await;
        if got == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{name}: rcode {got}, expected {expected}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn eventually_text(tree: &Tree, relative: &str, predicate: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let text = tree.text(relative);
        if predicate(&text) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "{relative} never matched; now {text:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// `(count, date, key)` of a statistics line.
fn parse_line(line: &str) -> (u64, String, String) {
    let fields: Vec<&str> = line.split(' ').collect();
    assert_eq!(fields.len(), 3, "{line:?}");
    assert_eq!(fields[0].len(), 10, "{line:?}");
    (
        fields[0].parse().unwrap(),
        fields[1].to_owned(),
        fields[2].to_owned(),
    )
}

fn is_date(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    parts.len() == 3
        && parts[0].len() == 4
        && parts[1].len() == 2
        && parts[2].len() == 2
        && text.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

// ----- tests ------------------------------------------------------------------

#[test]
fn a_new_name_takes_effect_at_once_and_is_published_in_process_without_losing_the_hot_entry() {
    let tree = Tree::new("hot");
    let ports = ports();
    config(
        &tree,
        &ports,
        &format!(
            "  - tag: sv4list\n    type: domain_output\n    args:\n      target_mapper: unified\n      target_mark: 4\n      target_tag: special\n      file_stat: gen/sv4list.txt\n      file_rule: gen/sv4rule.txt\n      dump_interval: 1\n      domain_set_url: \"http://127.0.0.1:{}/plugins/sv4rule/post\"\n",
            ports.api
        ),
        &["sv4list"],
    );
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        // First sight: the mapper ran before the output, so the default mark.
        assert_eq!(rcode(ports.dns, "New.Test").await, 7);
        // The very next query already carries the injected mark; no rebuild
        // and no wait are involved.
        assert_eq!(rcode(ports.dns, "new.test").await, 4);
        assert_eq!(rcode(ports.dns, "other.test").await, 7);

        // The periodic write (1 s) produces the reference formats...
        let stat =
            eventually_text(&tree, "gen/sv4list.txt", |text| text.contains("new.test")).await;
        let lines: Vec<(u64, String, String)> = stat.lines().map(parse_line).collect();
        let new = lines.iter().find(|line| line.2 == "new.test").unwrap();
        assert!(new.0 >= 2 && is_date(&new.1), "{stat}");
        let rule = eventually_text(&tree, "gen/sv4rule.txt", |text| {
            text.contains("full:new.test")
        })
        .await;
        assert!(rule.lines().all(|line| line.starts_with("full:")), "{rule}");
        // ...and publishes the list into the domain_set_light in-process, which
        // rebuilds the mapper: the name now matches rule mark 9 as well. The
        // hot entry is kept, so both marks apply (9 answers first).
        eventually_rcode(ports.dns, "new.test", 9).await;
        let shown = api(ports.api, "GET", "/plugins/sv4rule/show", "").await;
        assert!(shown.body.contains("full:new.test"), "{}", shown.body);
        assert_eq!(tree.leftovers("gen"), Vec::<String>::new());

        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn show_save_flush_and_the_other_routes_follow_the_reference() {
    let tree = Tree::new("api");
    let ports = ports();
    tree.write(
        "gen/top.txt",
        "0000000005 2026-10-05 popular.example\n7 plain.example\ngarbage\n\n0000000001 2026-10-04 a.example|AD|CD extra\n",
    );
    config(
        &tree,
        &ports,
        "  - tag: top_domains\n    type: domain_output\n    args:\n      file_stat: gen/top.txt\n      file_rule: gen/toprule.txt\n      gen_rule: gen/gen.txt\n      pattern: \"[/DOMAIN/]10.0.0.1\"\n      appended_string: \"udp://1.1.1.1\"\n      dump_interval: 3600\n",
        &["top_domains"],
    );
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let show = api(ports.api, "GET", "/plugins/top_domains/show", "").await;
        assert_eq!(show.status, 200);
        assert_eq!(
            show.header("content-type"),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(show.header("x-total-count"), Some("3"));
        assert_eq!(
            show.header("access-control-expose-headers"),
            Some("X-Total-Count")
        );
        let lines: Vec<(u64, String, String)> = show.body.lines().map(parse_line).collect();
        let keys: Vec<&str> = lines.iter().map(|line| line.2.as_str()).collect();
        // Two-field line: date = today; three or more: the third is the key.
        assert_eq!(keys, ["plain.example", "popular.example", "a.example|AD|CD"]);
        assert_eq!(lines[0].0, 7);
        assert!(is_date(&lines[0].1));
        assert_eq!(lines[1].1, "2026-10-05");

        let page = api(
            ports.api,
            "GET",
            "/plugins/top_domains/show?q=EXAMPLE&limit=1&offset=1",
            "",
        )
        .await;
        assert_eq!(page.header("x-total-count"), Some("3"));
        assert_eq!(page.body.lines().count(), 1);
        assert!(page.body.contains("popular.example"));
        let filtered = api(ports.api, "GET", "/plugins/top_domains/show?q=%7ccd", "").await;
        assert_eq!(filtered.header("x-total-count"), Some("1"));
        let invalid_limit = api(ports.api, "GET", "/plugins/top_domains/show?limit=x", "").await;
        assert_eq!(invalid_limit.body.lines().count(), 3);

        // Counting continues on top of the loaded numbers.
        assert_eq!(rcode(ports.dns, "popular.example").await, 7);
        let save = api(ports.api, "GET", "/plugins/top_domains/save", "").await;
        assert_eq!((save.status, save.body.as_str()), (200, "domain_output saved"));
        assert_eq!(save.header("content-type"), Some("text/plain; charset=utf-8"));
        let stat = tree.text("gen/top.txt");
        assert_eq!(
            stat.lines().map(parse_line).map(|l| (l.0, l.2)).collect::<Vec<_>>(),
            [
                (7, "plain.example".to_owned()),
                (6, "popular.example".to_owned()),
                (1, "a.example|AD|CD".to_owned())
            ]
        );
        assert_eq!(
            tree.text("gen/toprule.txt"),
            "full:plain.example\nfull:popular.example\nfull:a.example\n"
        );
        assert_eq!(
            tree.text("gen/gen.txt"),
            "udp://1.1.1.1\n[/plain.example/]10.0.0.1\n[/popular.example/]10.0.0.1\n[/a.example/]10.0.0.1\n"
        );

        // Wrong method, unknown action and the unsupported restart.
        assert_eq!(
            api(ports.api, "POST", "/plugins/top_domains/flush", "").await.status,
            405
        );
        assert_eq!(
            api(ports.api, "GET", "/plugins/top_domains/nothing", "").await.status,
            404
        );
        assert_eq!(
            api(ports.api, "GET", "/plugins/top_domains/restartall", "").await.status,
            501
        );

        // Flush clears memory and truncates the files.
        let flush = api(ports.api, "GET", "/plugins/top_domains/flush", "").await;
        assert_eq!((flush.status, flush.body.as_str()), (200, "domain_output flushed"));
        let shown = api(ports.api, "GET", "/plugins/top_domains/show", "").await;
        assert_eq!((shown.body.as_str(), shown.header("x-total-count")), ("", Some("0")));
        assert_eq!(tree.text("gen/top.txt"), "");
        assert_eq!(tree.text("gen/toprule.txt"), "");
        // The appended line is written whenever a pattern is set, as in the reference.
        assert_eq!(tree.text("gen/gen.txt"), "udp://1.1.1.1\n");

        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn closing_the_host_saves_and_a_restart_reads_the_file_back() {
    let tree = Tree::new("restart");
    let ports = ports();
    config(
        &tree,
        &ports,
        "  - tag: top_domains\n    type: domain_output\n    args:\n      file_stat: gen/top.txt\n      file_rule: gen/toprule.txt\n      dump_interval: 3600\n",
        &["top_domains"],
    );
    for round in 1..=2_u64 {
        let host = host(&tree);
        host.block_on(async {
            let (shutdown, task) = serve(&host).await;
            for _ in 0..3 {
                rcode(ports.dns, "again.example").await;
            }
            rcode(ports.dns, "once.example").await;
            shutdown.cancel();
            task.await.unwrap();
        });
        // The final save wrote the file before the host returned.
        let lines: Vec<(u64, String, String)> =
            tree.text("gen/top.txt").lines().map(parse_line).collect();
        assert_eq!(lines[0].0, 3 * round);
        assert_eq!(lines[0].2, "again.example");
        assert_eq!(lines[1].0, round);
        assert_eq!(lines[1].2, "once.example");
        assert_eq!(
            tree.text("gen/toprule.txt"),
            "full:again.example\nfull:once.example\n"
        );
    }
}

#[test]
fn flags_are_part_of_the_key_when_enabled() {
    let tree = Tree::new("flags");
    let ports = ports();
    config(
        &tree,
        &ports,
        "  - tag: top_domains\n    type: domain_output\n    args:\n      enable_flags: true\n      file_stat: gen/top.txt\n      file_rule: gen/toprule.txt\n      dump_interval: 3600\n",
        &["top_domains"],
    );
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        ask(ports.dns, &wire("f.example", 0, false)).await;
        ask(ports.dns, &wire("f.example", 0x20, false)).await;
        // The DO bit never changes the key (the reference tests a query whose
        // OPT was already replaced by the server).
        ask(ports.dns, &wire("f.example", 0x30, true)).await;
        ask(ports.dns, &wire("f.example", 0x20, false)).await;
        let shown = api(ports.api, "GET", "/plugins/top_domains/show", "").await;
        let rows: Vec<(u64, String)> = shown
            .body
            .lines()
            .map(parse_line)
            .map(|l| (l.0, l.2))
            .collect();
        assert_eq!(
            rows,
            [
                (2, "f.example|AD".to_owned()),
                (1, "f.example".to_owned()),
                (1, "f.example|AD|CD".to_owned())
            ]
        );
        api(ports.api, "GET", "/plugins/top_domains/save", "").await;
        // One rule per name, whatever its flags.
        assert_eq!(tree.text("gen/toprule.txt"), "full:f.example\n");
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn max_entries_triggers_an_early_write() {
    let tree = Tree::new("early");
    let ports = ports();
    config(
        &tree,
        &ports,
        "  - tag: top_domains\n    type: domain_output\n    args:\n      max_entries: 3\n      file_stat: gen/top.txt\n      dump_interval: 3600\n",
        &["top_domains"],
    );
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        rcode(ports.dns, "a.example").await;
        rcode(ports.dns, "a.example").await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            tree.text("gen/top.txt"),
            "",
            "two records are below the trigger"
        );
        rcode(ports.dns, "b.example").await;
        let stat = eventually_text(&tree, "gen/top.txt", |text| text.contains("b.example")).await;
        assert_eq!(stat.lines().count(), 2);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn a_failed_write_is_reported_keeps_the_other_files_and_leaves_no_temporary_file() {
    let tree = Tree::new("failure");
    let ports = ports();
    config(
        &tree,
        &ports,
        "  - tag: top_domains\n    type: domain_output\n    args:\n      file_stat: gen/top.txt\n      file_rule: gen/toprule.txt\n      dump_interval: 3600\n",
        &["top_domains"],
    );
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        rcode(ports.dns, "keep.example").await;
        assert_eq!(
            api(ports.api, "GET", "/plugins/top_domains/save", "")
                .await
                .status,
            200
        );
        let before = tree.text("gen/toprule.txt");
        assert_eq!(before, "full:keep.example\n");
        rcode(ports.dns, "more.example").await;
        tree.block("gen/top.txt");
        let failed = api(ports.api, "GET", "/plugins/top_domains/save", "").await;
        assert_eq!(failed.status, 500, "{}", failed.body);
        assert!(failed.body.contains("top.txt"), "{}", failed.body);
        // The other file was still written and nothing was left behind.
        assert_eq!(
            tree.text("gen/toprule.txt"),
            "full:keep.example\nfull:more.example\n"
        );
        assert_eq!(tree.leftovers("gen"), Vec::<String>::new());
        // Memory is untouched by the failure.
        let shown = api(ports.api, "GET", "/plugins/top_domains/show", "").await;
        assert_eq!(shown.header("x-total-count"), Some("2"));
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn a_foreign_domain_set_url_gets_an_http_post_with_the_reference_body() {
    let tree = Tree::new("http");
    let ports = ports();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let push_port = listener.local_addr().unwrap().port();
    let (sender, received) = std::sync::mpsc::channel::<String>();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        while let Ok(length) = stream.read(&mut chunk) {
            if length == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..length]);
            let text = String::from_utf8_lossy(&bytes);
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let expected: usize = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse().ok())
                    })
                    .unwrap_or(0);
                if body.len() >= expected {
                    break;
                }
            }
        }
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        sender
            .send(String::from_utf8_lossy(&bytes).into_owned())
            .unwrap();
    });
    config(
        &tree,
        &ports,
        &format!(
            "  - tag: top_domains\n    type: domain_output\n    args:\n      file_stat: gen/top.txt\n      dump_interval: 1\n      domain_set_url: \"http://127.0.0.1:{push_port}/somewhere/post\"\n"
        ),
        &["top_domains"],
    );
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        rcode(ports.dns, "Push.Test").await;
        let request = tokio::task::spawn_blocking(move || {
            received.recv_timeout(Duration::from_secs(10)).unwrap()
        })
        .await
        .unwrap();
        assert!(
            request.starts_with("POST /somewhere/post HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: application/json"),
            "{request}"
        );
        let body = request.split_once("\r\n\r\n").unwrap().1;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            serde_json::json!({"values": ["full:push.test"]})
        );
        shutdown.cancel();
        task.await.unwrap();
    });
    server.join().unwrap();
}

#[test]
fn bad_arguments_fail_the_configuration() {
    for (arguments, expected) in [
        ("      target_mapper: nosuchmapper\n", "target_mapper"),
        (
            "      target_mapper: unified\n      target_mark: 64\n",
            "target_mark",
        ),
        ("      unknown_field: 1\n", "unknown_field"),
        ("      max_entries: x\n", "max_entries"),
    ] {
        let tree = Tree::new("bad");
        let ports = ports();
        config(
            &tree,
            &ports,
            &format!("  - tag: top_domains\n    type: domain_output\n    args:\n{arguments}"),
            &["top_domains"],
        );
        let error = mosdns_native_host::load_and_compile(&tree.path("config.yaml"))
            .err()
            .unwrap_or_else(|| panic!("{arguments} must fail"))
            .to_string();
        assert!(error.contains(expected), "{arguments}: {error}");
    }
}

/// Files the reference plugin wrote (`tools/m1c1-interop`), and what its own
/// `/show` printed for them.
fn reference_fixture(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/domain_output")
            .join(name),
    )
    .unwrap()
}

/// `(count, date, key)` rows of a `/show` body, ties in key order (the
/// reference orders equal counts arbitrarily).
fn normalized(body: &str) -> Vec<(u64, String, String)> {
    let mut rows: Vec<(u64, String, String)> = body.lines().map(parse_line).collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.cmp(&b.2)));
    rows
}

#[test]
fn files_written_by_the_reference_are_read_and_listed_identically() {
    let tree = Tree::new("interop");
    let ports = ports();
    tree.write("gen/top.txt", &reference_fixture("stat.txt"));
    config(
        &tree,
        &ports,
        "  - tag: top_domains\n    type: domain_output\n    args:\n      file_stat: gen/top.txt\n      file_rule: gen/toprule.txt\n      dump_interval: 3600\n",
        &["top_domains"],
    );
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        // The reference output is `status`, `X-Total-Count: n`, then the body.
        let golden = reference_fixture("show_all.txt");
        let golden_body = golden.splitn(3, '\n').nth(2).unwrap();
        let shown = api(ports.api, "GET", "/plugins/top_domains/show?limit=100", "").await;
        assert_eq!(shown.header("x-total-count"), Some("6"));
        assert_eq!(normalized(&shown.body), normalized(golden_body));

        let page = reference_fixture("show_page.txt");
        let page_body = page.splitn(3, '\n').nth(2).unwrap();
        let shown = api(
            ports.api,
            "GET",
            "/plugins/top_domains/show?q=BETA&limit=1&offset=1",
            "",
        )
        .await;
        assert_eq!(shown.header("x-total-count"), Some("2"));
        assert_eq!(shown.body, page_body);

        // Saving what was loaded reproduces the reference files up to the
        // order of equal counts.
        api(ports.api, "GET", "/plugins/top_domains/save", "").await;
        assert_eq!(
            normalized(&tree.text("gen/top.txt")),
            normalized(&reference_fixture("stat.txt"))
        );
        let mut native: Vec<String> = tree
            .text("gen/toprule.txt")
            .lines()
            .map(str::to_owned)
            .collect();
        let mut reference: Vec<String> = reference_fixture("rule.txt")
            .lines()
            .map(str::to_owned)
            .collect();
        native.sort();
        reference.sort();
        assert_eq!(native, reference);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn flush_forgets_injected_names_and_a_restart_injects_what_was_learned() {
    let tree = Tree::new("learn");
    let ports = ports();
    config(
        &tree,
        &ports,
        "  - tag: sv4list\n    type: domain_output\n    args:\n      target_mapper: unified\n      target_mark: 4\n      target_tag: special\n      file_stat: gen/sv4list.txt\n      dump_interval: 3600\n",
        &["sv4list"],
    );
    let host = host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        assert_eq!(rcode(ports.dns, "learn.test").await, 7);
        assert_eq!(rcode(ports.dns, "learn.test").await, 4);
        // Flushing the statistics also drops what they injected.
        api(ports.api, "GET", "/plugins/sv4list/flush", "").await;
        assert_eq!(rcode(ports.dns, "learn.test").await, 7);
        // It is learned again like any new name.
        assert_eq!(rcode(ports.dns, "learn.test").await, 4);
        shutdown.cancel();
        task.await.unwrap();
    });
    // The final save wrote the statistics; a new host injects the learned
    // names before its first query, with no rule file involved.
    assert!(tree.text("gen/sv4list.txt").contains("learn.test"));
    let host = self::host(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        assert_eq!(rcode(ports.dns, "learn.test").await, 4);
        assert_eq!(rcode(ports.dns, "unknown.test").await, 7);
        shutdown.cancel();
        task.await.unwrap();
    });
}

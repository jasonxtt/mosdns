//! M1-0 acceptance: plugin paths resolve from the configuration root (`-d`),
//! and the `config_all` loading shapes (several listeners, `:port`, optional
//! `enable_audit`, `log.level`/`log.file`) work without `native_management`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use mosdns_native_host::{
    CheckFindingKind, HostAssembly, ListenerKind, check_config, check_config_in, compile_yaml,
    compile_yaml_with_roots, load_and_compile,
};
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SHAPE: &str = "tests/fixtures/m1_0/config_all_shape";

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("m1-0-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn free_port() -> u16 {
    loop {
        let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = tcp.local_addr().unwrap().port();
        if std::net::UdpSocket::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
}

fn query() -> Vec<u8> {
    vec![1, 2, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, b'x', 0, 0, 1, 0, 1]
}

/// `entry` answers NXDOMAIN to everything, so no upstream is involved.
fn reject_config(extra_top: &str, listeners: &str) -> String {
    format!(
        "log: {{ level: error }}\n{extra_top}plugins:\n  - tag: fwd\n    type: forward\n    args: {{ upstreams: [{{ addr: \"udp://127.0.0.1:15453\" }}] }}\n  - tag: entry\n    type: sequence\n    args: [{{ exec: reject 3 }}]\n{listeners}"
    )
}

fn listener(tag: &str, kind: &str, listen: &str, extra: &str) -> String {
    format!(
        "  - tag: {tag}\n    type: {kind}\n    args: {{ entry: entry, listen: \"{listen}\"{extra} }}\n"
    )
}

// --- R1: path root -------------------------------------------------------

#[test]
fn include_file_plugin_paths_resolve_against_the_config_directory_by_default() {
    let config = load_and_compile(&Path::new(SHAPE).join("config.yaml")).expect("loads");

    // `gen/direct.txt` and `rule/direct_ip.txt` come from the root, not from
    // the decoys beside `sub_config/rule_set.yaml`.
    let direct = config.domain_set("direct").expect("direct");
    assert!(direct.matches("root.example"));
    assert!(!direct.matches("decoy.example"));
    let networks = &config.ip_sets[0].prefixes;
    assert!(networks.contains("192.0.2.9".parse().unwrap()));
    assert!(!networks.contains("198.51.100.9".parse().unwrap()));

    let root = std::fs::canonicalize(SHAPE).unwrap();
    let cache = config.caches.first().expect("cache");
    assert_eq!(
        cache.dump_file.as_deref().map(normalize),
        Some(root.join("cache/main.dump"))
    );
    assert_eq!(
        normalize(&config.switches[0].state_file),
        root.join("rule/switch1.txt")
    );
}

/// Resolves a possibly relative path against the working directory and
/// collapses `.`/`..` lexically.
fn normalize(path: &Path) -> PathBuf {
    let absolute = std::env::current_dir().unwrap().join(path);
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// `config` lives in `cfg/`; `-d` names `data/`, which holds the plugin
/// files. `include` still resolves beside the configuration.
fn split_layout(dir: &TempDir) -> (PathBuf, PathBuf) {
    dir.write(
        "cfg/config.yaml",
        &format!(
            "log: {{ level: error }}\ninclude:\n  - inc.yaml\nplugins:\n  - tag: fwd\n    type: forward\n    args: {{ upstreams: [{{ addr: \"udp://127.0.0.1:15453\" }}] }}\n  - tag: entry\n    type: sequence\n    args: [{{ exec: reject 3 }}]\n  - tag: c\n    type: cache\n    args: {{ size: 16, dump_file: cache/c.dump }}\n  - tag: absolute_switch\n    type: switch2\n    args: {{ initial_value: \"{}\" }}\n{}",
            dir.path().join("elsewhere/switch2.txt").display(),
            listener("udp", "udp_server", "127.0.0.1:15390", "")
        ),
    );
    dir.write(
        "cfg/inc.yaml",
        "plugins:\n  - tag: blocked\n    type: domain_set\n    args: { files: [rule/blocked.txt] }\n  - tag: sw\n    type: switch1\n    args: { initial_value: state/sw1.txt }\n",
    );
    dir.write("data/rule/blocked.txt", "full:data.example\n");
    // Decoys where the old per-declaring-file rule and the cwd would look.
    dir.write("cfg/rule/blocked.txt", "full:cfg.example\n");
    (dir.path().join("cfg/config.yaml"), dir.path().join("data"))
}

#[test]
fn dash_d_overrides_the_config_directory_for_plugin_paths_only() {
    let dir = TempDir::new("dash-d");
    let (config, data) = split_layout(&dir);
    let yaml = std::fs::read_to_string(&config).unwrap();
    let compiled = compile_yaml_with_roots(&yaml, config.parent().unwrap(), &data)
        .expect("include resolves beside the config, plugin files from -d");

    let blocked = compiled.domain_set("blocked").unwrap();
    assert!(blocked.matches("data.example"));
    assert!(!blocked.matches("cfg.example"));
    assert_eq!(
        compiled.caches[0].dump_file.as_deref().map(normalize),
        Some(normalize(&data.join("cache/c.dump")))
    );
    let states: BTreeMap<_, _> = compiled
        .switches
        .iter()
        .map(|switch| (switch.tag.as_str(), normalize(&switch.state_file)))
        .collect();
    assert_eq!(states["sw"], normalize(&data.join("state/sw1.txt")));
    // An absolute path is unchanged.
    assert_eq!(
        states["absolute_switch"],
        normalize(&dir.path().join("elsewhere/switch2.txt"))
    );
}

#[test]
fn without_dash_d_the_config_directory_is_the_root() {
    let dir = TempDir::new("no-dash-d");
    let (config, _) = split_layout(&dir);
    let compiled = load_and_compile(&config).expect("compiles");
    let blocked = compiled.domain_set("blocked").unwrap();
    assert!(blocked.matches("cfg.example"));
    assert!(!blocked.matches("data.example"));
    let sw = compiled.switches.iter().find(|s| s.tag == "sw").unwrap();
    assert_eq!(
        normalize(&sw.state_file),
        normalize(&dir.path().join("cfg/state/sw1.txt"))
    );
}

#[test]
fn check_uses_the_same_root_as_start() {
    let dir = TempDir::new("check-root");
    let (config, data) = split_layout(&dir);
    std::fs::remove_file(dir.path().join("cfg/rule/blocked.txt")).unwrap();

    // Root = config directory: the rule file is missing there.
    let report = check_config(&config).expect("check runs");
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.tag.as_deref() == Some("blocked")),
        "{report}"
    );
    // Root = -d: everything compiles.
    let report = check_config_in(&config, Some(&data)).expect("check runs");
    assert!(report.findings.is_empty(), "{report}");
    assert_eq!(report.exit_code(), 0);
}

#[test]
fn the_binary_accepts_dash_d_and_reads_a_relative_config_from_it() {
    let dir = TempDir::new("binary-dash-d");
    let (_, data) = split_layout(&dir);
    // `-c` is relative and read from inside `-d`, like the reference host.
    let data_config = data.join("config.yaml");
    std::fs::copy(dir.path().join("cfg/config.yaml"), &data_config).unwrap();
    std::fs::copy(dir.path().join("cfg/inc.yaml"), data.join("inc.yaml")).unwrap();
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_mosdns"))
            .args(args)
            .current_dir(std::env::temp_dir())
            .output()
            .unwrap();
        (
            output.status.code().unwrap(),
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    };
    let (code, stdout, stderr) = run(&["check", "-d", data.to_str().unwrap(), "-c", "config.yaml"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    let (code, stdout, _) = run(&[
        "check",
        "--dir",
        data.to_str().unwrap(),
        "--config",
        "config.yaml",
    ]);
    assert_eq!(code, 0, "{stdout}");

    let (code, _, stderr) = run(&["check", "-d", "/definitely/not/here", "-c", "config.yaml"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("not a directory"), "{stderr}");
}

// --- R2: listeners ------------------------------------------------------

#[test]
fn listen_accepts_a_bare_port_and_enable_audit_defaults_to_false() {
    let yaml = reject_config(
        "",
        &(listener("udp", "udp_server", ":15353", "")
            + &listener("tcp", "tcp_server", ":15353", "")
            + &listener(
                "alt",
                "udp_server",
                "127.0.0.1:15354",
                ", enable_audit: true",
            )),
    );
    let config = compile_yaml(&yaml).expect("compiles");
    assert_eq!(config.listeners.len(), 3);
    assert_eq!(config.listeners[0].listen, "[::]:15353".parse().unwrap());
    assert_eq!(config.listeners[1].kind, ListenerKind::Tcp);
    // `idle_timeout` is optional too (the reference default is 10 seconds).
    assert_eq!(
        config.listeners[1].idle_timeout,
        Some(Duration::from_secs(10))
    );
    assert!(!config.listeners[0].enable_audit);
    assert!(!config.listeners[1].enable_audit);
    assert!(config.listeners[2].enable_audit);

    let yaml = reject_config(
        "",
        &listener("tcp", "tcp_server", ":15353", ", idle_timeout: 720"),
    );
    assert_eq!(
        compile_yaml(&yaml).unwrap().listeners[0].idle_timeout,
        Some(Duration::from_secs(720))
    );
    for bad in [":0", ":", ":notaport", ":65536", "localhost:53"] {
        let yaml = reject_config("", &listener("udp", "udp_server", bad, ""));
        assert!(compile_yaml(&yaml).is_err(), "{bad} must be rejected");
    }
}

#[test]
fn a_duplicate_protocol_and_address_is_still_rejected() {
    let yaml = reject_config(
        "",
        &(listener("a", "udp_server", ":15353", "") + &listener("b", "udp_server", ":15353", "")),
    );
    let error = compile_yaml(&yaml).err().expect("duplicate rejected");
    assert!(error.reason.contains("duplicate listener"), "{error}");
    // The same address over the other protocol is not a duplicate.
    let yaml = reject_config(
        "",
        &(listener("a", "udp_server", ":15353", "") + &listener("b", "tcp_server", ":15353", "")),
    );
    assert!(compile_yaml(&yaml).is_ok());
}

async fn udp_exchange(target: std::net::SocketAddr) -> Vec<u8> {
    let bind = if target.is_ipv6() {
        "[::1]:0"
    } else {
        "127.0.0.1:0"
    };
    let socket = tokio::net::UdpSocket::bind(bind).await.unwrap();
    socket.send_to(&query(), target).await.unwrap();
    let mut packet = vec![0; 4096];
    let (length, _) = tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut packet))
        .await
        .expect("udp response in time")
        .unwrap();
    packet.truncate(length);
    packet
}

async fn tcp_exchange(target: std::net::SocketAddr) -> Vec<u8> {
    let mut stream = tokio::net::TcpStream::connect(target).await.unwrap();
    let wire = query();
    stream
        .write_all(&u16::try_from(wire.len()).unwrap().to_be_bytes())
        .await
        .unwrap();
    stream.write_all(&wire).await.unwrap();
    let mut prefix = [0; 2];
    tokio::time::timeout(Duration::from_secs(3), stream.read_exact(&mut prefix))
        .await
        .expect("tcp response in time")
        .unwrap();
    let mut response = vec![0; usize::from(u16::from_be_bytes(prefix))];
    stream.read_exact(&mut response).await.unwrap();
    response
}

#[test]
fn several_listeners_serve_and_release_their_sockets_without_native_management() {
    let (a, b) = (free_port(), free_port());
    let yaml = reject_config(
        "",
        &(listener(
            "udp_a",
            "udp_server",
            &format!("127.0.0.1:{a}"),
            ", enable_audit: true",
        ) + &listener("tcp_a", "tcp_server", &format!("127.0.0.1:{a}"), "")
            + &listener("udp_b", "udp_server", &format!("127.0.0.1:{b}"), "")
            + &listener("tcp_b", "tcp_server", &format!("127.0.0.1:{b}"), "")),
    );
    let dir = TempDir::new("multi");
    let config = dir.write("config.yaml", &yaml);
    let host = HostAssembly::from_config_file(&config).expect("assembles");
    host.block_on(async {
        let bound = host.bind_host().await.expect("every listener binds");
        let shutdown = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        for port in [a, b] {
            let target: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
            let udp = udp_exchange(target).await;
            assert_eq!(udp[3] & 15, 3, "UDP :{port} answers NXDOMAIN");
            let tcp = tcp_exchange(target).await;
            assert_eq!(tcp[3] & 15, 3, "TCP :{port} answers NXDOMAIN");
        }
        // Audit is per listener: only the first UDP listener records.
        assert_eq!(host.audit_snapshot().records.len(), 1);
        shutdown.cancel();
        serving.await.unwrap().expect("clean shutdown");
        for port in [a, b] {
            std::net::TcpListener::bind(("127.0.0.1", port)).expect("TCP port released");
            std::net::UdpSocket::bind(("127.0.0.1", port)).expect("UDP port released");
        }
    });
}

#[test]
fn a_failed_bind_releases_the_listeners_that_already_bound() {
    let (a, b) = (free_port(), free_port());
    let occupied = std::net::TcpListener::bind(("127.0.0.1", b)).unwrap();
    let yaml = reject_config(
        "",
        &(listener("udp_a", "udp_server", &format!("127.0.0.1:{a}"), "")
            + &listener("tcp_a", "tcp_server", &format!("127.0.0.1:{a}"), "")
            + &listener("tcp_b", "tcp_server", &format!("127.0.0.1:{b}"), "")),
    );
    let dir = TempDir::new("rollback");
    let host = HostAssembly::from_config_file(&dir.write("config.yaml", &yaml)).unwrap();
    host.block_on(async {
        assert!(
            host.bind_host().await.is_err(),
            "occupied port fails startup"
        );
    });
    std::net::TcpListener::bind(("127.0.0.1", a)).expect("earlier TCP socket released");
    std::net::UdpSocket::bind(("127.0.0.1", a)).expect("earlier UDP socket released");
    drop(occupied);
}

#[test]
fn a_bare_port_listens_dual_stack_and_audits_ipv4_clients_as_ipv4() {
    let port = free_port();
    let yaml = reject_config(
        "",
        &(listener(
            "udp",
            "udp_server",
            &format!(":{port}"),
            ", enable_audit: true",
        ) + &listener(
            "tcp",
            "tcp_server",
            &format!(":{port}"),
            ", enable_audit: true",
        )),
    );
    let dir = TempDir::new("dual-stack");
    let host = HostAssembly::from_config_file(&dir.write("config.yaml", &yaml)).unwrap();
    let ipv6 = std::net::UdpSocket::bind("[::1]:0").is_ok();
    host.block_on(async {
        let bound = host.bind_host().await.expect("wildcard binds");
        let shutdown = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let v4: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
        assert_eq!(udp_exchange(v4).await[3] & 15, 3);
        assert_eq!(tcp_exchange(v4).await[3] & 15, 3);
        if ipv6 {
            let v6: std::net::SocketAddr = "[::1]:0".parse::<std::net::SocketAddr>().unwrap();
            let v6 = std::net::SocketAddr::new(v6.ip(), port);
            assert_eq!(udp_exchange(v6).await[3] & 15, 3);
            assert_eq!(tcp_exchange(v6).await[3] & 15, 3);
        }
        let records = host.audit_snapshot().records;
        assert!(records.len() >= 2);
        for record in records
            .iter()
            .filter(|r| r.client_addr.ip().is_ipv4() || ipv6)
        {
            // A mapped `::ffff:127.0.0.1` must never leak into the audit.
            if let std::net::IpAddr::V6(address) = record.client_addr.ip() {
                assert!(address.to_ipv4_mapped().is_none(), "{record:?}");
            }
        }
        assert!(
            records
                .iter()
                .any(|r| r.client_addr.ip() == std::net::IpAddr::from([127, 0, 0, 1])),
            "IPv4 clients are recorded as plain IPv4: {records:?}"
        );
        shutdown.cancel();
        serving.await.unwrap().expect("clean shutdown");
    });
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, (bool, u64, std::time::SystemTime, Vec<u8>)> {
    let mut entries = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        let content = if metadata.is_file() {
            std::fs::read(&path).unwrap()
        } else {
            Vec::new()
        };
        if metadata.is_dir() {
            for entry in std::fs::read_dir(&path).unwrap() {
                pending.push(entry.unwrap().path());
            }
        }
        entries.insert(
            path,
            (
                metadata.is_dir(),
                metadata.len(),
                metadata.modified().unwrap(),
                content,
            ),
        );
    }
    entries
}

#[test]
fn standard_special_groups_check_is_read_only_and_start_recovers_derived_yaml() {
    let dir = TempDir::new("read-only");
    let port = free_port();
    dir.write(
        "config.yaml",
        &format!(
            "log: {{ level: error }}\ninclude:\n  - sub_config/special_groups.yaml\nplugins:\n  - tag: fwd\n    type: forward\n    args: {{ upstreams: [{{ addr: \"udp://127.0.0.1:15453\" }}] }}\n  - tag: entry\n    type: sequence\n    args: [{{ exec: reject 3 }}]\n{}",
            listener("udp", "udp_server", &format!("127.0.0.1:{port}"), "")
        ),
    );
    dir.write(
        "sub_config/special_groups.yaml",
        "plugins:\n  - tag: special_upstream_matcher\n    type: domain_mapper\n    args: {rules: []}\n  - tag: sequence_special\n    type: sequence\n    args: []\n",
    );
    let before = snapshot(dir.path());
    assert!(
        check_config(&dir.path().join("config.yaml"))
            .unwrap()
            .findings
            .is_empty()
    );
    assert_eq!(
        snapshot(dir.path()),
        before,
        "check never writes derived files"
    );
    let host = HostAssembly::from_config_file(&dir.path().join("config.yaml")).unwrap();
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let shutdown = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        assert_eq!(udp_exchange(([127, 0, 0, 1], port).into()).await[3] & 15, 3);
        shutdown.cancel();
        serving.await.unwrap().unwrap();
    });
    drop(host);
    let generated =
        std::fs::read_to_string(dir.path().join("sub_config/special_groups.yaml")).unwrap();
    assert!(generated.contains("由 special_groups API 自动生成"));
    assert_eq!(
        std::fs::read(dir.path().join("config.yaml")).unwrap(),
        before[&dir.path().join("config.yaml")].3
    );
    assert!(!dir.path().join("webinfo").exists());
}

#[test]
fn the_config_all_shape_has_no_findings_and_check_does_not_touch_it() {
    let report = check_config(&Path::new(SHAPE).join("config.yaml")).expect("check runs");
    assert!(report.findings.is_empty(), "{report}");
    let before = snapshot(Path::new(SHAPE));
    let _ = check_config(&Path::new(SHAPE).join("config.yaml")).unwrap();
    assert_eq!(snapshot(Path::new(SHAPE)), before);
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.kind != CheckFindingKind::Gap)
    );
}

// --- R3: log ------------------------------------------------------------

/// Starts the binary on a configuration whose `ip_set` names a missing rule
/// file (a startup warning), waits for the management API, and returns the
/// log file content.
fn run_with_log(level: &str, file_line: &str) -> (String, std::process::Output) {
    let dir = TempDir::new(&format!("log-{level}"));
    let api = free_port();
    let dns = free_port();
    dir.write(
        "config.yaml",
        &format!(
            "log: {{ level: {level}{file_line} }}\napi:\n  http: \"127.0.0.1:{api}\"\nplugins:\n  - tag: fwd\n    type: forward\n    args: {{ upstreams: [{{ addr: \"udp://127.0.0.1:15453\" }}] }}\n  - tag: ips\n    type: ip_set\n    args: {{ files: [rule/missing_ips.txt] }}\n  - tag: entry\n    type: sequence\n    args: [{{ exec: reject 3 }}]\n{}",
            listener("udp", "udp_server", &format!("127.0.0.1:{dns}"), "")
        ),
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_mosdns"))
        .args(["start", "-d"])
        .arg(dir.path())
        .args(["-c", "config.yaml"])
        .current_dir(std::env::temp_dir())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while std::net::TcpStream::connect(("127.0.0.1", api)).is_err() {
        assert!(Instant::now() < deadline, "host did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    child.kill().unwrap();
    let output = child.wait_with_output().unwrap();
    let log = std::fs::read_to_string(dir.path().join("m.log")).unwrap_or_default();
    (log, output)
}

#[test]
fn log_file_receives_diagnostics_at_or_above_the_level() {
    for (level, expect_warning) in [
        ("debug", true),
        ("info", true),
        ("warn", true),
        ("error", false),
    ] {
        let (log, output) = run_with_log(level, ", file: m.log");
        assert_eq!(
            log.contains("missing IP rule file"),
            expect_warning,
            "level {level}: {log}"
        );
        if expect_warning {
            assert!(log.contains(" WARN "), "{log}");
        }
        // With a file the diagnostic is not duplicated on stderr.
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("missing IP rule file"),
            "level {level}"
        );
    }
}

#[test]
fn without_a_log_file_diagnostics_go_to_stderr_and_an_unknown_level_falls_back_to_info() {
    let (_, output) = run_with_log("verbose", "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("missing IP rule file"), "{stderr}");
    assert!(stderr.contains("unknown log level"), "{stderr}");
}

#[test]
fn a_fatal_startup_error_reaches_both_stderr_and_the_log_file() {
    let dir = TempDir::new("fatal");
    let port = free_port();
    let _occupied = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    dir.write(
        "config.yaml",
        &format!(
            "log: {{ level: error, file: m.log }}\nplugins:\n  - tag: fwd\n    type: forward\n    args: {{ upstreams: [{{ addr: \"udp://127.0.0.1:15453\" }}] }}\n  - tag: entry\n    type: sequence\n    args: [{{ exec: reject 3 }}]\n{}",
            listener("tcp", "tcp_server", &format!("127.0.0.1:{port}"), "")
        ),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_mosdns"))
        .args(["start", "-d"])
        .arg(dir.path())
        .args(["-c", "config.yaml"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("bind failed"), "{stderr}");
    let log = std::fs::read_to_string(dir.path().join("m.log")).unwrap();
    assert!(
        log.contains(" ERROR ") && log.contains("bind failed"),
        "{log}"
    );
}

#[test]
fn log_file_is_appended_not_truncated() {
    let dir = TempDir::new("append");
    dir.write("m.log", "earlier line\n");
    let port = free_port();
    dir.write(
        "config.yaml",
        &format!(
            "log: {{ level: nonsense, file: m.log }}\nplugins:\n  - tag: fwd\n    type: forward\n    args: {{ upstreams: [{{ addr: \"udp://127.0.0.1:15453\" }}] }}\n  - tag: entry\n    type: sequence\n    args: [{{ exec: reject 3 }}]\n{}",
            listener("udp", "udp_server", &format!("127.0.0.1:{port}"), "")
        ),
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_mosdns"))
        .args(["start", "-d"])
        .arg(dir.path())
        .args(["-c", "config.yaml"])
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let log = std::fs::read_to_string(dir.path().join("m.log")).unwrap();
        if log.contains("unknown log level") {
            assert!(log.starts_with("earlier line\n"), "{log}");
            break;
        }
        assert!(Instant::now() < deadline, "no diagnostic written: {log}");
        std::thread::sleep(Duration::from_millis(50));
    }
    child.kill().unwrap();
    child.wait().unwrap();
}

// --- Phase 6b: the experimental `native_management` mode was removed ---------

const MANAGEMENT_REMOVED: &str =
    "native_management was removed; use the standard special_groups configuration";

fn config_with_native_management(value: &str) -> String {
    let port = free_port();
    reject_config(
        &format!("native_management: {value}\n"),
        &listener("udp_a", "udp_server", &format!("127.0.0.1:{port}"), ""),
    )
}

#[test]
fn a_configuration_naming_native_management_is_refused_with_a_clear_message() {
    let dir = TempDir::new("removed-management");
    // Whatever the value was (the opt-in on, off, empty or malformed), the field is gone.
    for value in [
        "{special_groups: true}",
        "{special_groups: false}",
        "{}",
        "true",
    ] {
        let yaml = config_with_native_management(value);
        let config = dir.write("config.yaml", &yaml);
        let error = load_and_compile(&config).err().expect("load is refused");
        assert!(
            error.to_string().contains(MANAGEMENT_REMOVED),
            "{value}: {error}"
        );
        let error = compile_yaml(&yaml).err().expect("compile is refused");
        assert!(
            error.to_string().contains(MANAGEMENT_REMOVED),
            "{value}: {error}"
        );
        let error = HostAssembly::from_config_file(&config)
            .err()
            .expect("assembly is refused");
        assert!(
            error.to_string().contains(MANAGEMENT_REMOVED),
            "{value}: {error}"
        );

        let output = Command::new(env!("CARGO_BIN_EXE_mosdns"))
            .args(["check", "-c"])
            .arg(&config)
            .output()
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(output.status.code(), Some(1), "{value}: {stdout}");
        assert!(stdout.contains(MANAGEMENT_REMOVED), "{value}: {stdout}");
        assert!(stdout.contains("SUMMARY "), "{value}: {stdout}");

        let output = Command::new(env!("CARGO_BIN_EXE_mosdns"))
            .args(["start", "-c"])
            .arg(&config)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{value}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(MANAGEMENT_REMOVED),
            "{value}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn leftover_native_management_state_files_are_neither_read_nor_deleted() {
    let port = free_port();
    let dir = TempDir::new("leftover-management");
    let left = [
        ("webinfo/native_management.lock", "lock"),
        (
            "webinfo/native_management.transaction/journal.json",
            "{not json",
        ),
        ("webinfo/native_management.transaction/commit", "x"),
        (
            "webinfo/native_management.staging/rule/special_50.txt",
            "full:a.example",
        ),
        ("webinfo/native_management.retired/srs/old.json", "[]"),
    ];
    for (name, contents) in left {
        dir.write(name, contents);
    }
    let config = dir.write(
        "config.yaml",
        &reject_config(
            "",
            &listener("udp_a", "udp_server", &format!("127.0.0.1:{port}"), ""),
        ),
    );
    let host = HostAssembly::from_config_file(&config).expect("starts normally");
    host.block_on(async {
        let bound = host.bind_host().await.expect("binds");
        let shutdown = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let reply = udp_exchange(([127, 0, 0, 1], port).into()).await;
        assert_eq!(reply[3] & 15, 3);
        shutdown.cancel();
        serving.await.unwrap().expect("clean shutdown");
    });
    drop(host);
    for (name, contents) in left {
        assert_eq!(
            std::fs::read_to_string(dir.path().join(name)).unwrap(),
            contents,
            "{name} must stay as it was"
        );
    }
}

//! The update guard: the old program, still the service's main process, installs the
//! new files, runs the candidate as its child, and either confirms it or rolls back.
//!
//! The guard is entered by `exec` from the running host (same process id, so a service
//! manager never sees the service stop) as `mosdns update-guard --transaction <file>`.
//! It has no sockets and loads no configuration. See `transaction.rs` for the states.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::package::{self, ConfigState};
use super::transaction::{self, MARKER_ENV, Transaction};

/// Consecutive healthy answers needed.
const REQUIRED_HEALTHY: u32 = 3;
const HEALTH_INTERVAL: Duration = Duration::from_secs(1);
const HEALTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const STOP_GRACE: Duration = Duration::from_secs(10);
const FAILED_STOP_GRACE: Duration = Duration::from_secs(5);
const PREFLIGHT_VERSION_TIMEOUT: Duration = Duration::from_secs(10);
const PREFLIGHT_CHECK_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_HEALTH_BYTES: usize = 256 << 10;
/// The guard's pid, handed to the candidate so it can tell whether the guard still lives.
const GUARD_PID_ENV: &str = "MOSDNS_UPDATE_GUARD_PID";

fn log(message: &str) {
    eprintln!("mosdns update-guard: {message}");
}

/// Replaces this process with `executable` and `args` (`args[0]` is `argv[0]`). The guard
/// marker is removed so the new image is an ordinary service. Returns only on failure.
fn exec_program(executable: &str, args: &[String]) -> std::io::Error {
    let mut command = Command::new(executable);
    if let Some(argv0) = args.first() {
        command.arg0(argv0);
    }
    command
        .args(args.iter().skip(1))
        .env_remove(MARKER_ENV)
        .exec()
}

/// A candidate started by the guard dies with it, instead of staying behind holding the
/// service's ports. Called by `mosdns start` when the guard marker is present.
///
/// The signal is only sent when the parent dies *after* it is set, so the guard's pid is
/// checked as well: a guard that was killed between `spawn` and this call has already left
/// the candidate re-parented, and the candidate must not run on.
pub fn arm_parent_death_signal() {
    if std::env::var_os(MARKER_ENV).is_none() {
        return;
    }
    let _ = rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::TERM));
    let expected = std::env::var(GUARD_PID_ENV)
        .ok()
        .and_then(|value| value.parse::<u32>().ok());
    let parent =
        rustix::process::getppid().and_then(|pid| u32::try_from(pid.as_raw_nonzero().get()).ok());
    if guard_is_gone(expected, parent) {
        eprintln!("mosdns: the update guard is gone, not starting");
        std::process::exit(1);
    }
}

/// Whether the process that started this candidate is no longer its parent.
fn guard_is_gone(expected: Option<u32>, parent: Option<u32>) -> bool {
    expected.is_some_and(|expected| parent != Some(expected))
}

/// Runs the guard. Returns (an error message) only when even the final `exec` failed.
pub fn run(transaction_path: &Path) -> String {
    let mut tx = match transaction::read(transaction_path) {
        Ok(tx) => tx,
        Err(error) => return error,
    };
    if tx.status != "staged" {
        return format!("更新事务状态无效: {}", tx.status);
    }
    if let Err(error) = transaction::validate_paths(&tx) {
        return error;
    }
    match drive(transaction_path, &mut tx) {
        Outcome::Verified => {
            log(&format!("{} 已确认，切换到最终程序", tx.target_version));
            exec_program(&tx.executable_path, &tx.original_args).to_string()
        }
        Outcome::RolledBack(cause) => {
            log(&format!("已回滚: {cause}"));
            exec_program(&tx.executable_path, &tx.original_args).to_string()
        }
        Outcome::Stuck(cause) => {
            // The restore itself failed: the state stays `installing`/`testing` so the next
            // start retries, and the program that is on disk is started as it is.
            log(&format!("回滚未完成，保留事务供下次启动恢复: {cause}"));
            exec_program(&tx.executable_path, &tx.original_args).to_string()
        }
    }
}

enum Outcome {
    Verified,
    RolledBack(String),
    Stuck(String),
}

fn fail(path: &Path, tx: &mut Transaction, cause: String) -> Outcome {
    match transaction::rollback(path, tx, &cause) {
        Ok(()) => Outcome::RolledBack(cause),
        Err(error) => Outcome::Stuck(error),
    }
}

fn drive(path: &Path, tx: &mut Transaction) -> Outcome {
    let root = Path::new(&tx.config_base_dir).to_path_buf();
    // Staged files are re-checked: they were written minutes ago by a program that may
    // have been interrupted, and nothing about them is trusted without their hashes.
    let package = match load_package(tx) {
        Ok(package) => package,
        Err(error) => return fail(path, tx, error),
    };
    if tx.kind == "binary" {
        match super::file_sha256(Path::new(&tx.candidate_path)) {
            Ok(actual) if actual == tx.candidate_sha256 => {}
            Ok(_) => return fail(path, tx, "候选程序的校验值与暂存时不同".to_owned()),
            Err(error) => return fail(path, tx, format!("读取候选程序失败: {error}")),
        }
    }
    tx.status = "installing".to_owned();
    if let Err(error) = transaction::write(path, tx) {
        return fail(path, tx, error);
    }
    if let Some(package) = &package {
        match package::apply(&root, package) {
            Ok(updated) => {
                let state = ConfigState {
                    applied_schema: package.manifest.config_schema,
                    required_schema: package.manifest.config_schema,
                    status: "success".to_owned(),
                    package_id: package.manifest.package_id.clone(),
                    package_sha256: package.sha256.clone(),
                    binary_version: tx.target_version.clone(),
                    files_updated: updated,
                    backup_dir: tx.backup_dir.clone(),
                    message: format!("配置已升级到 schema {}", package.manifest.config_schema),
                    updated_at: super::now_text(),
                    ..ConfigState::default()
                };
                if let Err(error) = package::write_state(&root, &state) {
                    return fail(path, tx, format!("写入配置状态失败: {error}"));
                }
            }
            Err(error) => return fail(path, tx, format!("应用配置包失败: {error}")),
        }
    }
    if tx.kind == "binary" {
        if let Err(error) = super::fsio::durable_copy(
            Path::new(&tx.candidate_path),
            Path::new(&tx.executable_path),
            0o755,
        ) {
            return fail(path, tx, format!("安装候选程序失败: {error}"));
        }
    }
    tx.status = "testing".to_owned();
    if let Err(error) = transaction::write(path, tx) {
        return fail(path, tx, error);
    }
    // From here on any interruption is recovered by `rollback` at the next start.
    let mut child = match spawn_candidate(tx, path) {
        Ok(child) => child,
        Err(error) => return fail(path, tx, format!("启动候选版本失败: {error}")),
    };
    match wait_healthy(tx, &mut child) {
        Ok(()) => {
            // The candidate has proved itself: record that first, then stop it.
            tx.status = "verified".to_owned();
            if let Err(error) = transaction::write(path, tx) {
                stop(&mut child, FAILED_STOP_GRACE);
                return fail(path, tx, error);
            }
            if let Err(error) =
                transaction::write_installed(&root, &tx.target_version, &tx.asset_signature)
            {
                log(&format!("无法记录已安装签名: {error}"));
            }
            stop(&mut child, STOP_GRACE);
            Outcome::Verified
        }
        Err(cause) => {
            stop(&mut child, FAILED_STOP_GRACE);
            fail(path, tx, cause)
        }
    }
}

fn load_package(tx: &Transaction) -> Result<Option<package::Package>, String> {
    if tx.config_package_path.is_empty() {
        return Ok(None);
    }
    let bytes = std::fs::read(&tx.config_package_path)
        .map_err(|error| format!("读取配置包失败: {error}"))?;
    if super::sha256_hex(&bytes) != tx.config_package_sha256 {
        return Err("配置包的校验值与暂存时不同".to_owned());
    }
    package::parse(&bytes, tx.required_config_schema, &tx.config_package_id).map(Some)
}

fn spawn_candidate(tx: &Transaction, path: &Path) -> std::io::Result<Child> {
    let mut command = Command::new(&tx.executable_path);
    if let Some(argv0) = tx.original_args.first() {
        command.arg0(argv0);
    }
    command
        .args(tx.original_args.iter().skip(1))
        .env(MARKER_ENV, path)
        .env(GUARD_PID_ENV, std::process::id().to_string())
        .spawn()
}

fn stop(child: &mut Child, grace: Duration) {
    if let Some(pid) = rustix::process::Pid::from_raw(i32::try_from(child.id()).unwrap_or(0)) {
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
    }
    let end = Instant::now() + grace;
    while Instant::now() < end {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn wait_healthy(tx: &Transaction, child: &mut Child) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(tx.health_timeout_secs.max(1));
    let mut healthy = 0;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Err(format!("候选进程提前退出（{status}）")),
            Ok(None) => {}
            Err(error) => return Err(format!("无法监视候选进程: {error}")),
        }
        if is_healthy(
            &tx.health_url,
            &tx.target_version,
            tx.required_config_schema,
        ) {
            healthy += 1;
            if healthy >= REQUIRED_HEALTHY {
                return Ok(());
            }
        } else {
            healthy = 0;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "候选版本在 {} 秒内未通过健康检查",
                tx.health_timeout_secs.max(1)
            ));
        }
        std::thread::sleep(HEALTH_INTERVAL);
    }
}

/// One health probe: `200`, `ready`, the target version, and the required config schema.
pub(crate) fn is_healthy(url: &str, target_version: &str, required_schema: u32) -> bool {
    let Some((status, body)) = http_get(url) else {
        return false;
    };
    if status != 200 {
        return false;
    }
    let Ok(health) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return false;
    };
    health["ready"] == true
        && health["version"] == target_version
        && (required_schema == 0
            || health["applied_config_schema"]
                .as_u64()
                .is_some_and(|applied| applied >= u64::from(required_schema)))
}

/// A bounded blocking `GET http://host:port/path` (the local management address only).
fn http_get(url: &str) -> Option<(u16, Vec<u8>)> {
    let rest = url.strip_prefix("http://")?;
    let (authority, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
    let address = authority.to_socket_addrs().ok()?.next()?;
    let mut stream = TcpStream::connect_timeout(&address, HEALTH_REQUEST_TIMEOUT).ok()?;
    stream.set_read_timeout(Some(HEALTH_REQUEST_TIMEOUT)).ok()?;
    stream
        .set_write_timeout(Some(HEALTH_REQUEST_TIMEOUT))
        .ok()?;
    write!(
        stream,
        "GET /{path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut response = Vec::new();
    stream
        .take(MAX_HEALTH_BYTES as u64)
        .read_to_end(&mut response)
        .ok()?;
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")?;
    let status = std::str::from_utf8(&response[..split])
        .ok()?
        .split(' ')
        .nth(1)?
        .parse()
        .ok()?;
    Some((status, response[split + 4..].to_vec()))
}

/// Runs `command` for at most `timeout`; its combined output (bounded) and whether it exited 0.
fn run_bounded(mut command: Command, timeout: Duration) -> Result<(bool, String), String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("无法运行候选程序: {error}"))?;
    let take = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = Vec::new();
            if let Some(pipe) = pipe {
                let _ = pipe.take(64 << 10).read_to_end(&mut text);
            }
            text
        })
    };
    let out = take(
        child
            .stdout
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
    );
    let err = take(
        child
            .stderr
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn Read + Send>),
    );
    let end = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("候选程序运行超过 {} 秒", timeout.as_secs()));
            }
            Err(error) => return Err(error.to_string()),
        }
    };
    let mut text = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    text.push_str(&String::from_utf8_lossy(&err.join().unwrap_or_default()));
    Ok((status.success(), text))
}

fn tail(text: &str) -> String {
    let trimmed = text.trim();
    let start = trimmed
        .char_indices()
        .rev()
        .nth(399)
        .map_or(0, |(index, _)| index);
    trimmed[start..].to_owned()
}

/// Before anything is stopped: the candidate runs, reports `expected_version` (when given)
/// and — when a preview root is given — accepts the new configuration (`check` exits 0).
pub(crate) fn preflight(
    candidate: &Path,
    expected_version: Option<&str>,
    preview: Option<(&Path, &str)>,
) -> Result<(), String> {
    if let Some(expected) = expected_version {
        let mut command = Command::new(candidate);
        command.arg("version");
        let (ok, text) = run_bounded(command, PREFLIGHT_VERSION_TIMEOUT)?;
        if !ok {
            return Err(format!("候选程序无法报告版本: {}", tail(&text)));
        }
        if text.trim() != expected {
            return Err(format!(
                "候选程序自报版本 {:?} 与发布版本 {expected:?} 不一致",
                text.trim()
            ));
        }
    }
    if let Some((root, config)) = preview {
        let mut command = Command::new(candidate);
        command
            .arg("check")
            .arg("-d")
            .arg(root)
            .arg("-c")
            .arg(config);
        let (ok, text) = run_bounded(command, PREFLIGHT_CHECK_TIMEOUT)?;
        if !ok {
            return Err(format!("新配置在候选程序下检查未通过: {}", tail(&text)));
        }
    }
    Ok(())
}

/// The management health URL for a listen address (a wildcard becomes loopback).
pub(crate) fn health_url(listen: std::net::SocketAddr) -> String {
    let ip: std::net::IpAddr = if listen.ip().is_unspecified() {
        if listen.is_ipv6() {
            std::net::Ipv6Addr::LOCALHOST.into()
        } else {
            std::net::Ipv4Addr::LOCALHOST.into()
        }
    } else {
        listen.ip()
    };
    format!(
        "http://{}/api/v1/system/health",
        std::net::SocketAddr::new(ip, listen.port())
    )
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn a_candidate_whose_guard_already_died_is_told_so() {
        assert!(!guard_is_gone(Some(100), Some(100)), "the guard lives");
        assert!(guard_is_gone(Some(100), Some(1)), "re-parented to init");
        assert!(guard_is_gone(Some(100), None), "no parent at all");
        assert!(
            !guard_is_gone(None, Some(1)),
            "started without a guard pid: nothing to compare"
        );
    }

    fn temp(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("update-guard-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn serve_once(body: &'static str, status: u16) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/api/v1/system/health",
            listener.local_addr().unwrap()
        );
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap() > 2 {
                line.clear();
            }
            write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        (url, handle)
    }

    #[test]
    fn health_needs_200_ready_the_target_version_and_the_config_schema() {
        let ok = r#"{"ready":true,"version":"rust-v0.2.0","applied_config_schema":1}"#;
        for (body, status, schema, expected) in [
            (ok, 200, 1, true),
            (ok, 200, 0, true),
            (ok, 200, 2, false),
            (ok, 503, 1, false),
            (
                r#"{"ready":false,"version":"rust-v0.2.0","applied_config_schema":1}"#,
                200,
                1,
                false,
            ),
            (
                r#"{"ready":true,"version":"rust-v0.1.0","applied_config_schema":1}"#,
                200,
                1,
                false,
            ),
            (
                r#"{"ready":true,"version":"rust-v0.2.0","applied_config_schema":null}"#,
                200,
                1,
                false,
            ),
            (r#"{"ready":true,"version":"rust-v0.2.0"}"#, 200, 0, true),
            ("not json", 200, 0, false),
        ] {
            let (url, handle) = serve_once(body, status);
            assert_eq!(
                is_healthy(&url, "rust-v0.2.0", schema),
                expected,
                "{body} {status} {schema}"
            );
            handle.join().unwrap();
        }
        assert!(
            !is_healthy("http://127.0.0.1:9/api/v1/system/health", "x", 0),
            "nothing listens"
        );
        assert!(
            !is_healthy("https://example.invalid/", "x", 0),
            "only plain http to the local address"
        );
    }

    #[test]
    fn the_health_url_turns_a_wildcard_into_loopback() {
        assert_eq!(
            health_url("0.0.0.0:9099".parse().unwrap()),
            "http://127.0.0.1:9099/api/v1/system/health"
        );
        assert_eq!(
            health_url("[::]:80".parse().unwrap()),
            "http://[::1]:80/api/v1/system/health"
        );
        assert_eq!(
            health_url("192.0.2.1:8080".parse().unwrap()),
            "http://192.0.2.1:8080/api/v1/system/health"
        );
    }

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn preflight_checks_the_reported_version_and_the_new_configuration() {
        let dir = temp("preflight");
        let good = script(
            &dir,
            "good",
            r#"case "$1" in version) echo rust-v0.2.0;; check) exit 0;; esac"#,
        );
        assert!(preflight(&good, Some("rust-v0.2.0"), None).is_ok());
        assert!(
            preflight(
                &good,
                Some("rust-v0.2.0"),
                Some((&dir, "config_custom.yaml"))
            )
            .is_ok()
        );
        let error = preflight(&good, Some("rust-v0.3.0"), None).unwrap_err();
        assert!(error.contains("不一致"), "{error}");
        let wrong_config = script(
            &dir,
            "badcheck",
            r#"case "$1" in version) echo rust-v0.2.0;; check) echo "SUMMARY gaps=2" >&2; exit 1;; esac"#,
        );
        let error =
            preflight(&wrong_config, Some("rust-v0.2.0"), Some((&dir, "c.yaml"))).unwrap_err();
        assert!(
            error.contains("检查未通过") && error.contains("gaps=2"),
            "{error}"
        );
        let broken = dir.join("not-a-program");
        std::fs::write(&broken, "garbage").unwrap();
        std::fs::set_permissions(&broken, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(preflight(&broken, Some("x"), None).is_err());
        assert!(preflight(&dir.join("absent"), Some("x"), None).is_err());
        // No expectation, no check: nothing to do (a `config` transaction without a package).
        assert!(preflight(&dir.join("absent"), None, None).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_hung_candidate_is_killed_at_the_time_limit() {
        let dir = temp("timeout");
        let hung = script(&dir, "hung", "sleep 30");
        let mut command = Command::new(&hung);
        command.arg("version");
        let started = Instant::now();
        let error = run_bounded(command, Duration::from_millis(300)).unwrap_err();
        assert!(error.contains("超过"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(5));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn tail_keeps_the_end_of_long_output_on_a_character_boundary() {
        let text = "中".repeat(1000);
        let cut = tail(&text);
        assert_eq!(cut.chars().count(), 400);
        assert_eq!(tail("  short \n"), "short");
    }
}

//! Socket ownership checks and bounded retries for child startup bind races.
#![allow(dead_code)] // Each integration binary uses the helpers it needs.

use std::io::{BufRead, BufReader};
use std::process::Child;
use std::process::{Command, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub enum Protocol {
    Udp,
    Tcp,
}

/// Only sockets with an fd in this process count, even if another process has
/// already reused the port. Read failures must not masquerade as release.
#[cfg(target_os = "linux")]
fn owns_port(port: u16, protocol: Protocol) -> bool {
    let tables = match protocol {
        Protocol::Udp => ["/proc/net/udp", "/proc/net/udp6"],
        Protocol::Tcp => ["/proc/net/tcp", "/proc/net/tcp6"],
    };
    let mut inodes = std::collections::HashSet::new();
    for table in tables {
        let contents = std::fs::read_to_string(table).expect("read socket table");
        assert_eq!(contents.split_whitespace().next(), Some("sl"), "{table}");
        for row in contents.lines().skip(1) {
            let columns: Vec<_> = row.split_whitespace().collect();
            assert!(columns.len() >= 10, "malformed socket row: {row}");
            let (_, local_port) = columns[1].rsplit_once(':').expect("local socket address");
            let local_port = u16::from_str_radix(local_port, 16).expect("hex socket port");
            let inode = columns[9].parse::<u64>().expect("socket inode");
            if local_port == port {
                inodes.insert(format!("socket:[{inode}]"));
            }
        }
    }
    for entry in std::fs::read_dir("/proc/self/fd").expect("read own fds") {
        let entry = entry.expect("own fd entry");
        let link = match std::fs::read_link(entry.path()) {
            Ok(link) => link,
            // Other test threads can close fds while we enumerate them.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => panic!("read own fd link: {error}"),
        };
        if inodes.contains(link.to_string_lossy().as_ref()) {
            return true;
        }
    }
    false
}

/// Prove that the detector sees a real held socket before testing rollback.
#[cfg(target_os = "linux")]
pub fn check_release_probe(port: u16, protocol: Protocol) {
    match protocol {
        Protocol::Udp => {
            let _probe = std::net::UdpSocket::bind(("127.0.0.1", port)).expect("UDP probe");
            assert!(owns_port(port, protocol), "held UDP probe not detected");
        }
        Protocol::Tcp => {
            let _probe = std::net::TcpListener::bind(("127.0.0.1", port)).expect("TCP probe");
            assert!(owns_port(port, protocol), "held TCP probe not detected");
        }
    }
}

pub fn assert_released(port: u16, protocol: Protocol) {
    #[cfg(target_os = "linux")]
    assert!(
        !owns_port(port, protocol),
        "this process still owns {protocol:?} port {port} after failed bind"
    );
    #[cfg(not(target_os = "linux"))]
    match protocol {
        Protocol::Udp => {
            std::net::UdpSocket::bind(("127.0.0.1", port)).expect("UDP socket released");
        }
        Protocol::Tcp => {
            std::net::TcpListener::bind(("127.0.0.1", port)).expect("TCP socket released");
        }
    }
}

pub fn retry_addr_in_use<T>(mut start: impl FnMut() -> Result<T, String>) -> T {
    let mut failures = Vec::new();
    for attempt in 1..=5 {
        match start() {
            Ok(value) => return value,
            Err(error) => {
                let occupied =
                    error.contains("Address already in use") || error.contains("AddrInUse");
                failures.push(format!("attempt {attempt}: {error}"));
                assert!(occupied, "startup failed: {}", failures.join("\n"));
                eprintln!("port startup collision: {}", failures.last().unwrap());
            }
        }
    }
    panic!(
        "address still occupied after 5 starts: {}",
        failures.join("\n")
    );
}

pub fn wait_ready(
    child: &mut Child,
    stderr: &Receiver<String>,
    timeout: Duration,
    mut ready: impl FnMut() -> bool,
) -> Result<(), String> {
    let end = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            // The child exited; allow its pipe-reader thread to deliver the final
            // bind diagnostic. A live/hung child never enters this drain.
            let mut lines = Vec::new();
            while let Ok(line) = stderr.recv_timeout(Duration::from_millis(100)) {
                lines.push(line);
            }
            return Err(format!(
                "host exited ({status}); stderr: {}",
                lines.join("\n")
            ));
        }
        if ready()
            && child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_none()
        {
            return Ok(());
        }
        if Instant::now() >= end {
            return Err(format!(
                "host not ready; stderr: {}",
                stderr.try_iter().collect::<Vec<_>>().join("\n")
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Failures reap the child before the next attempt rewrites its configuration.
pub fn spawn_ready(
    command: &mut Command,
    timeout: Duration,
    ready: impl FnMut() -> bool,
) -> Result<(Child, Receiver<String>), String> {
    let mut child = command
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let (sender, stderr) = std::sync::mpsc::channel();
    let pipe = child.stderr.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(pipe).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    if let Err(error) = wait_ready(&mut child, &stderr, timeout, ready) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    Ok((child, stderr))
}

/// Called after `wait`/`wait_with_output`, when the stderr writer has closed.
pub fn collected_stderr(stderr: &Receiver<String>) -> Vec<u8> {
    stderr
        .iter()
        .map(|line| line + "\n")
        .collect::<String>()
        .into_bytes()
}

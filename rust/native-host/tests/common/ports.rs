//! Bounded startup retries, only for the bind(0) -> child bind hand-off race.
#![allow(dead_code)] // Each integration binary uses the helpers it needs.

use std::io::{BufRead, BufReader};
use std::process::Child;
use std::process::{Command, Stdio};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// Keep kernel-selected `bind(0)` ports away from release-after-failure checks.
/// If procfs is unavailable or fewer than two unprivileged ports remain below
/// the ephemeral range, probe both protocols on a kernel-selected port instead.
pub fn release_check_ports() -> (u16, u16) {
    let lower = std::fs::read_to_string("/proc/sys/net/ipv4/ip_local_port_range")
        .ok()
        .and_then(|range| range.split_whitespace().next()?.parse::<u16>().ok())
        .filter(|lower| *lower > 1026);
    let mut sockets = Vec::new();
    for _ in 0..256 {
        let port = if let Some(lower) = lower {
            let mut random = [0; 4];
            getrandom::fill(&mut random).expect("random release-check port");
            1025 + u16::try_from(u32::from_ne_bytes(random) % u32::from(lower - 1025))
                .expect("port offset fits u16")
        } else {
            0
        };
        let tcp = match std::net::TcpListener::bind(("127.0.0.1", port)) {
            Ok(tcp) => tcp,
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(error) => panic!("probe release-check TCP port: {error}"),
        };
        let port = tcp.local_addr().unwrap().port();
        let udp = match std::net::UdpSocket::bind(("127.0.0.1", port)) {
            Ok(udp) => udp,
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(error) => panic!("probe release-check UDP port: {error}"),
        };
        // Hold both protocols until two distinct ports have been selected.
        sockets.push((tcp, udp));
        if sockets.len() == 2 {
            return (
                sockets[0].0.local_addr().unwrap().port(),
                sockets[1].0.local_addr().unwrap().port(),
            );
        }
    }
    panic!("could not probe two release-check ports in 256 attempts");
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

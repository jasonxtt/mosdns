//! Process metrics for `GET /metrics`, read from `/proc/self`.
//!
//! The names are the standard Prometheus process-collector ones that the
//! WebUI already parses (`process_cpu_seconds_total`, `process_start_time_seconds`,
//! `process_resident_memory_bytes`, `process_open_fds`), plus the virtual size,
//! the descriptor limit and the thread count. Go-runtime figures (garbage
//! collections, goroutines, the Go version) have no counterpart in this
//! runtime and are not invented; neither are the system-wide network counters
//! the Go registry adds.

use std::fmt::Write;

/// What `/proc/self` says right now.
#[derive(Debug, PartialEq)]
pub(crate) struct ProcessSample {
    pub(crate) cpu_seconds: f64,
    pub(crate) start_time_seconds: f64,
    pub(crate) resident_bytes: u64,
    pub(crate) virtual_bytes: u64,
    pub(crate) threads: u64,
    pub(crate) open_fds: u64,
    pub(crate) max_fds: Option<u64>,
}

/// The fields of `/proc/<pid>/stat` after the parenthesised command name:
/// `(utime + stime in ticks, threads, start ticks after boot, vsize bytes, rss pages)`.
fn parse_stat(text: &str) -> Option<(u64, u64, u64, u64, u64)> {
    let fields: Vec<&str> = text.rsplit_once(')')?.1.split_whitespace().collect();
    // Field numbers of proc(5) minus 3: the state is the first field after the name.
    let field = |number: usize| fields.get(number - 3)?.parse::<u64>().ok();
    Some((
        field(14)? + field(15)?,
        field(20)?,
        field(22)?,
        field(23)?,
        field(24)?,
    ))
}

/// `btime` (seconds since the epoch at boot) from `/proc/stat`.
fn parse_boot_time(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix("btime "))?
        .trim()
        .parse()
        .ok()
}

/// The soft `Max open files` limit from `/proc/<pid>/limits`; `None` when unlimited or unreadable.
fn parse_max_fds(text: &str) -> Option<u64> {
    let line = text
        .lines()
        .find(|line| line.starts_with("Max open files"))?;
    line.split_whitespace().nth(3)?.parse().ok()
}

#[cfg(target_os = "linux")]
pub(crate) fn sample() -> Option<ProcessSample> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let (cpu_ticks, threads, start_ticks, virtual_bytes, rss_pages) = parse_stat(&stat)?;
    let boot = parse_boot_time(&std::fs::read_to_string("/proc/stat").ok()?)?;
    let ticks = rustix::param::clock_ticks_per_second().max(1) as f64;
    let page = rustix::param::page_size() as u64;
    let open_fds = std::fs::read_dir("/proc/self/fd").ok()?.count() as u64;
    let max_fds = std::fs::read_to_string("/proc/self/limits")
        .ok()
        .and_then(|text| parse_max_fds(&text));
    Some(ProcessSample {
        cpu_seconds: cpu_ticks as f64 / ticks,
        start_time_seconds: boot as f64 + start_ticks as f64 / ticks,
        resident_bytes: rss_pages * page,
        virtual_bytes,
        threads,
        open_fds,
        max_fds,
    })
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn sample() -> Option<ProcessSample> {
    None
}

/// Whether this platform can report process metrics.
pub(crate) const AVAILABLE: bool = cfg!(target_os = "linux");

/// The Prometheus text for one sample.
pub(crate) fn render(sample: &ProcessSample) -> String {
    let mut text = String::new();
    let mut metric = |name: &str, kind: &str, help: &str, value: String| {
        let _ = writeln!(text, "# HELP {name} {help}");
        let _ = writeln!(text, "# TYPE {name} {kind}");
        let _ = writeln!(text, "{name} {value}");
    };
    metric(
        "process_cpu_seconds_total",
        "counter",
        "Total user and system CPU time spent in seconds.",
        format!("{:.2}", sample.cpu_seconds),
    );
    metric(
        "process_start_time_seconds",
        "gauge",
        "Start time of the process since unix epoch in seconds.",
        format!("{:.2}", sample.start_time_seconds),
    );
    metric(
        "process_resident_memory_bytes",
        "gauge",
        "Resident memory size in bytes.",
        sample.resident_bytes.to_string(),
    );
    metric(
        "process_virtual_memory_bytes",
        "gauge",
        "Virtual memory size in bytes.",
        sample.virtual_bytes.to_string(),
    );
    metric(
        "process_open_fds",
        "gauge",
        "Number of open file descriptors.",
        sample.open_fds.to_string(),
    );
    if let Some(limit) = sample.max_fds {
        metric(
            "process_max_fds",
            "gauge",
            "Maximum number of open file descriptors.",
            limit.to_string(),
        );
    }
    metric(
        "mosdns_process_threads",
        "gauge",
        "Number of operating-system threads of the process.",
        sample.threads.to_string(),
    );
    text
}

/// The process section of `/metrics` (empty where the platform cannot report it).
pub(crate) fn metrics_text() -> String {
    sample().map(|sample| render(&sample)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_fields_are_read_after_the_command_name_even_with_spaces_and_parentheses() {
        // pid (name) state ppid pgrp session tty tpgid flags minflt cminflt majflt cmajflt
        // utime stime cutime cstime priority nice threads itrealvalue starttime vsize rss ...
        let text = "42 (a (b) c) S 1 2 3 4 5 6 7 8 9 10 150 50 0 0 20 0 7 0 9000 123456789 321 0";
        assert_eq!(parse_stat(text), Some((200, 7, 9000, 123_456_789, 321)));
        assert_eq!(parse_stat("42 (short) S 1"), None);
        assert_eq!(parse_stat("no parenthesis"), None);
    }

    #[test]
    fn boot_time_and_limits_are_parsed() {
        assert_eq!(
            parse_boot_time("cpu 1 2\nbtime 1700000000\nprocesses 5\n"),
            Some(1_700_000_000)
        );
        assert_eq!(parse_boot_time("cpu 1 2\n"), None);
        let limits = "Limit Soft Limit Hard Limit Units\nMax open files            1024                 524288               files\n";
        assert_eq!(parse_max_fds(limits), Some(1024));
        assert_eq!(
            parse_max_fds("Max open files unlimited unlimited files\n"),
            None
        );
    }

    #[test]
    fn rendering_has_only_process_names_and_never_invents_go_runtime_figures() {
        let text = render(&ProcessSample {
            cpu_seconds: 1.234,
            start_time_seconds: 1_700_000_000.5,
            resident_bytes: 100,
            virtual_bytes: 200,
            threads: 3,
            open_fds: 12,
            max_fds: Some(1024),
        });
        assert!(text.contains("process_cpu_seconds_total 1.23\n"));
        assert!(text.contains("process_start_time_seconds 1700000000.50\n"));
        assert!(text.contains("process_resident_memory_bytes 100\n"));
        assert!(text.contains("process_open_fds 12\n"));
        assert!(text.contains("process_max_fds 1024\n"));
        assert!(text.contains("mosdns_process_threads 3\n"));
        for forbidden in ["go_", "process_network"] {
            assert!(!text.contains(forbidden), "{forbidden} must not be emitted");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_live_sample_is_plausible() {
        // Burn a little CPU so the counter is positive on a coarse tick.
        let mut x = 0_u64;
        for i in 0..50_000_000_u64 {
            x = x.wrapping_add(i ^ (x >> 3));
        }
        assert!(x != 1);
        let sample = sample().expect("linux reports process metrics");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        assert!(sample.cpu_seconds > 0.0);
        assert!(sample.resident_bytes > 1_000_000 && sample.resident_bytes < 64 << 30);
        assert!(sample.virtual_bytes >= sample.resident_bytes);
        assert!(sample.threads >= 1);
        assert!(sample.open_fds >= 3);
        // Started within the last day and not in the future (boot time has 1 s resolution).
        assert!(
            sample.start_time_seconds <= now + 2.0 && sample.start_time_seconds > now - 86_400.0
        );
    }
}

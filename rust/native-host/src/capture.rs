//! In-memory diagnostic capture behind `POST /api/v1/capture/start` and
//! `GET /api/v1/capture/logs`.
//!
//! While a capture runs, debug-level diagnostics are kept in a bounded buffer
//! (and `diag` lets debug lines through to its sink as well); the configured
//! `log.level` itself is never rewritten, so the end of a capture restores the
//! previous level by construction. Expiry is lazy: every user of the buffer
//! compares the deadline with the clock, so there is no timer task to leak.
//!
//! The buffer has its own mutex, separate from the diagnostic sink, and holds
//! already-serialized JSON lines: pushing one never waits for file I/O.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Most lines kept; the oldest is dropped first.
/// (One query's step trace is ~10-20 lines.)
pub(crate) const MAX_LINES: usize = 32768;
/// Most serialized bytes kept across all lines.
pub(crate) const MAX_BYTES: usize = 8 << 20;
/// Longest string value in a line, in characters; longer values are cut.
const MAX_VALUE_CHARS: usize = 256;

/// One field of a captured line.
pub(crate) enum Field<'a> {
    Str(&'a str),
    Uint(u64),
    Bool(bool),
}

#[derive(Default)]
struct State {
    until: Option<Instant>,
    lines: VecDeque<String>,
    bytes: usize,
    dropped: u64,
}

impl State {
    /// Ends the capture when its deadline has passed. Lines already captured
    /// stay readable, as in the reference.
    fn expire(&mut self, now: Instant, flag: &AtomicBool) {
        if self.until.is_some_and(|until| now >= until) {
            self.until = None;
            flag.store(false, Ordering::Relaxed);
        }
    }

    fn start(&mut self, now: Instant, duration: Duration, flag: &AtomicBool) {
        self.lines.clear();
        self.bytes = 0;
        self.dropped = 0;
        self.until = Some(now + duration);
        flag.store(true, Ordering::Relaxed);
    }

    fn push(&mut self, now: Instant, line: String, flag: &AtomicBool) {
        self.expire(now, flag);
        if self.until.is_none() {
            return;
        }
        self.bytes += line.len();
        self.lines.push_back(line);
        while self.lines.len() > MAX_LINES || self.bytes > MAX_BYTES {
            let Some(oldest) = self.lines.pop_front() else {
                break;
            };
            self.bytes -= oldest.len();
            self.dropped += 1;
        }
    }

    fn take(&mut self) -> (VecDeque<String>, u64) {
        self.bytes = 0;
        (
            std::mem::take(&mut self.lines),
            std::mem::take(&mut self.dropped),
        )
    }
}

static STATE: Mutex<State> = Mutex::new(State {
    until: None,
    lines: VecDeque::new(),
    bytes: 0,
    dropped: 0,
});
/// True from `start` until the deadline is noticed; the hot-path hint.
static CAPTURING: AtomicBool = AtomicBool::new(false);

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Begins (or restarts) a capture: the buffer is emptied and the clock reset.
pub(crate) fn start(duration: Duration) {
    state().start(Instant::now(), duration, &CAPTURING);
}

/// Cheap check for the hot path. It may stay true briefly after the deadline;
/// `record` and `active` look at the clock.
pub(crate) fn hint() -> bool {
    CAPTURING.load(Ordering::Relaxed)
}

/// Whether a capture is running now (notices an expired deadline).
pub(crate) fn active() -> bool {
    if !hint() {
        return false;
    }
    let mut state = state();
    state.expire(Instant::now(), &CAPTURING);
    state.until.is_some()
}

/// Adds one debug line to the buffer; does nothing when no capture runs.
pub(crate) fn record(logger: &str, message: &str, fields: &[(&str, Field<'_>)]) {
    if !hint() {
        return;
    }
    let now = std::time::SystemTime::now();
    let line = render(now, "debug", logger, message, fields);
    state().push(Instant::now(), line, &CAPTURING);
}

/// The captured lines as a JSON array (with the final newline of the
/// reference's encoder). Reading empties the buffer.
pub(crate) fn take_json() -> String {
    let (lines, dropped) = state().take();
    let mut out = String::with_capacity(lines.iter().map(|line| line.len() + 1).sum::<usize>() + 2);
    out.push('[');
    let mut first = true;
    if dropped > 0 {
        let notice = render(
            std::time::SystemTime::now(),
            "warn",
            "capture",
            &format!("capture buffer limit reached: {dropped} older lines dropped"),
            &[],
        );
        out.push_str(&notice);
        first = false;
    }
    for line in &lines {
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(line);
    }
    out.push_str("]\n");
    out
}

fn render(
    now: std::time::SystemTime,
    level: &str,
    logger: &str,
    message: &str,
    fields: &[(&str, Field<'_>)],
) -> String {
    use serde_json::{Map, Value};
    let mut map = Map::new();
    for (key, value) in fields {
        let value = match value {
            Field::Str(text) => Value::String(cut(text)),
            Field::Uint(number) => Value::from(*number),
            Field::Bool(flag) => Value::from(*flag),
        };
        map.insert((*key).to_owned(), value);
    }
    // The reference's own keys win over a field of the same name.
    map.insert("level".to_owned(), Value::String(level.to_owned()));
    map.insert(
        "time".to_owned(),
        Value::String(
            time::OffsetDateTime::from(now)
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default(),
        ),
    );
    map.insert("msg".to_owned(), Value::String(cut(message)));
    map.insert("logger_name".to_owned(), Value::String(logger.to_owned()));
    Value::Object(map).to_string()
}

fn cut(text: &str) -> String {
    match text.char_indices().nth(MAX_VALUE_CHARS) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> String {
        format!("{{\"msg\":\"{text}\"}}")
    }

    #[test]
    fn a_capture_keeps_lines_until_its_deadline_and_then_stops_taking_new_ones() {
        let flag = AtomicBool::new(false);
        let mut state = State::default();
        let t0 = Instant::now();
        state.push(t0, line("before"), &flag);
        assert!(state.lines.is_empty(), "nothing is kept without a capture");
        state.start(t0, Duration::from_secs(2), &flag);
        assert!(flag.load(Ordering::Relaxed));
        state.push(t0 + Duration::from_secs(1), line("during"), &flag);
        state.push(t0 + Duration::from_secs(2), line("at deadline"), &flag);
        assert!(
            !flag.load(Ordering::Relaxed),
            "the deadline ends the capture"
        );
        state.push(t0 + Duration::from_secs(3), line("after"), &flag);
        assert_eq!(state.lines, [line("during")]);
    }

    #[test]
    fn a_second_start_empties_the_buffer_and_restarts_the_clock() {
        let flag = AtomicBool::new(false);
        let mut state = State::default();
        let t0 = Instant::now();
        state.start(t0, Duration::from_secs(5), &flag);
        state.push(t0, line("old"), &flag);
        state.start(t0 + Duration::from_secs(4), Duration::from_secs(5), &flag);
        assert!(state.lines.is_empty());
        state.push(t0 + Duration::from_secs(8), line("new"), &flag);
        assert_eq!(state.lines, [line("new")], "the new deadline is 9 s");
        state.push(t0 + Duration::from_secs(9), line("late"), &flag);
        assert_eq!(state.lines.len(), 1);
    }

    #[test]
    fn the_buffer_is_bounded_by_lines_and_bytes_and_drops_the_oldest() {
        let flag = AtomicBool::new(false);
        let mut state = State::default();
        let t0 = Instant::now();
        state.start(t0, Duration::from_secs(60), &flag);
        for index in 0..(MAX_LINES + 10) {
            state.push(t0, line(&index.to_string()), &flag);
        }
        assert_eq!(state.lines.len(), MAX_LINES);
        assert_eq!(state.dropped, 10);
        assert_eq!(state.lines.front().unwrap(), &line("10"));

        let mut state = State::default();
        state.start(t0, Duration::from_secs(60), &flag);
        let big = "x".repeat(MAX_BYTES / 4);
        for _ in 0..8 {
            state.push(t0, big.clone(), &flag);
        }
        assert!(state.bytes <= MAX_BYTES);
        assert_eq!(state.lines.len(), 4);
        assert_eq!(state.dropped, 4);
        assert_eq!(
            state.bytes,
            state.lines.iter().map(String::len).sum::<usize>()
        );
    }

    #[test]
    fn take_returns_everything_once_and_a_dropped_notice_when_lines_were_lost() {
        let flag = AtomicBool::new(false);
        let mut state = State::default();
        let t0 = Instant::now();
        state.start(t0, Duration::from_secs(60), &flag);
        state.push(t0, line("a"), &flag);
        let (lines, dropped) = state.take();
        assert_eq!((lines.len(), dropped), (1, 0));
        let (lines, _) = state.take();
        assert!(lines.is_empty());
    }

    #[test]
    fn rendered_lines_carry_the_reference_keys_and_cut_long_values() {
        let long = "d".repeat(1000);
        let text = render(
            std::time::SystemTime::UNIX_EPOCH,
            "debug",
            "query",
            "dns query completed",
            &[
                ("domain", Field::Str(&long)),
                ("trace_id", Field::Str("t-1")),
                ("msg", Field::Str("shadowed")),
                ("elapsed_ms", Field::Uint(7)),
            ],
        );
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["level"], "debug");
        assert_eq!(value["msg"], "dns query completed");
        assert_eq!(value["logger_name"], "query");
        assert_eq!(value["time"], "1970-01-01T00:00:00Z");
        assert_eq!(value["trace_id"], "t-1");
        assert_eq!(value["elapsed_ms"], 7);
        assert_eq!(value["domain"].as_str().unwrap().chars().count(), 257);
    }
}

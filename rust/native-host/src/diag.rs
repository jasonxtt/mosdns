//! Diagnostic output of the native host, filtered by `log.level` and
//! optionally appended to `log.file` instead of stderr. Not a logging
//! framework: messages are plain text lines and there are no other sinks.

use std::fmt::Arguments;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;

use crate::config::{LogLevel, LogSettings};

struct Sink {
    level: LogLevel,
    file: Option<File>,
}

/// Until `start` installs the configured settings (and for `check` and
/// library users), everything goes to stderr.
static SINK: Mutex<Sink> = Mutex::new(Sink {
    level: LogLevel::Debug,
    file: None,
});

fn sink() -> std::sync::MutexGuard<'static, Sink> {
    SINK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Applies the compiled `log` section: the threshold, and `file` opened for
/// append (created when absent). A level that was not a known name is warned
/// about once the new sink is active.
pub(crate) fn install(settings: &LogSettings) -> std::io::Result<()> {
    let file = match &settings.file {
        Some(path) => Some(OpenOptions::new().append(true).create(true).open(path)?),
        None => None,
    };
    *sink() = Sink {
        level: settings.level,
        file,
    };
    if let Some(level) = &settings.unknown_level {
        warn_line!("unknown log level {level:?}, using info");
    }
    Ok(())
}

pub(crate) fn emit(level: LogLevel, message: Arguments<'_>) {
    // A running capture lowers the threshold to debug without touching the
    // configured level, so the end of the capture restores it by itself.
    let capturing = crate::capture::active();
    sink().write(level, message, capturing);
    if capturing && level == LogLevel::Debug {
        crate::capture::record("host", &message.to_string(), &[]);
    }
}

impl Sink {
    fn write(&mut self, level: LogLevel, message: Arguments<'_>, capturing: bool) {
        if level < self.level && !capturing {
            return;
        }
        match &mut self.file {
            Some(file) => {
                // A failed diagnostic write has nowhere better to go.
                let _ = writeln!(file, "{} {} {message}", timestamp(), name(level));
            }
            None => eprintln!("{message}"),
        }
    }
}

/// Writes a fatal startup error to `log.file` when one is configured. The
/// caller prints it to stderr itself, so it is never lost with the file.
pub fn record_fatal(message: &str) {
    if let Some(file) = &mut sink().file {
        let _ = writeln!(file, "{} {} {message}", timestamp(), name(LogLevel::Error));
    }
}

fn name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Debug => "DEBUG",
        LogLevel::Info => "INFO",
        LogLevel::Warn => "WARN",
        LogLevel::Error => "ERROR",
    }
}

fn timestamp() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

macro_rules! debug_line {
    ($($argument:tt)*) => {
        $crate::diag::emit($crate::config::LogLevel::Debug, format_args!($($argument)*))
    };
}
macro_rules! info_line {
    ($($argument:tt)*) => {
        $crate::diag::emit($crate::config::LogLevel::Info, format_args!($($argument)*))
    };
}
macro_rules! warn_line {
    ($($argument:tt)*) => {
        $crate::diag::emit($crate::config::LogLevel::Warn, format_args!($($argument)*))
    };
}
macro_rules! error_line {
    ($($argument:tt)*) => {
        $crate::diag::emit($crate::config::LogLevel::Error, format_args!($($argument)*))
    };
}
pub(crate) use {debug_line, error_line, info_line, warn_line};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_messages_at_or_above_the_level_reach_the_file() {
        let path = std::env::temp_dir().join(format!("native-diag-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        for (level, expected) in [
            (
                LogLevel::Debug,
                ["DEBUG", "INFO", "WARN", "ERROR"].as_slice(),
            ),
            (LogLevel::Info, ["INFO", "WARN", "ERROR"].as_slice()),
            (LogLevel::Warn, ["WARN", "ERROR"].as_slice()),
            (LogLevel::Error, ["ERROR"].as_slice()),
        ] {
            let _ = std::fs::remove_file(&path);
            let mut sink = Sink {
                level,
                file: Some(
                    OpenOptions::new()
                        .append(true)
                        .create(true)
                        .open(&path)
                        .unwrap(),
                ),
            };
            for emitted in [
                LogLevel::Debug,
                LogLevel::Info,
                LogLevel::Warn,
                LogLevel::Error,
            ] {
                sink.write(emitted, format_args!("message"), false);
            }
            let log = std::fs::read_to_string(&path).unwrap();
            let names: Vec<_> = log
                .lines()
                .map(|line| line.split(' ').nth(1).unwrap())
                .collect();
            assert_eq!(names, expected, "{level:?}");
        }
        std::fs::remove_file(path).unwrap();
    }
}

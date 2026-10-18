//! In-place restart: the same process id, the same executable and arguments.
//!
//! The production service is `Restart=always` with `RestartSec=2min`, so a
//! restart that exits and waits for the service manager would leave DNS down for
//! two minutes. Instead the host drains and saves exactly as it does for
//! `SIGTERM`, and `main` then replaces the process image with `execve` (see
//! `docs/rust/contracts/native-webui-hosting.md`). The process id does not
//! change, so the service manager never notices.

use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// What to execute: the file the process was started from and its original `argv`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestartTarget {
    exe: PathBuf,
    /// `argv` as received, `argv[0]` included.
    args: Vec<OsString>,
    /// Set when this target is the update guard of a staged transaction.
    transaction: Option<PathBuf>,
}

static TARGET: OnceLock<RestartTarget> = OnceLock::new();
/// The target the next restart runs instead of the captured one (an update guard).
static PENDING: Mutex<Option<RestartTarget>> = Mutex::new(None);

/// Makes the next restart `exec` `target` instead of this program again.
pub(crate) fn set_pending(target: RestartTarget) {
    *PENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(target);
}

/// Forgets a pending target (the restart it was for could not be scheduled).
pub(crate) fn clear_pending() {
    *PENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// Takes the pending target, if any (for `main`, once a restart was requested).
#[must_use]
pub fn take_pending() -> Option<RestartTarget> {
    PENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

/// Linux appends this to `/proc/self/exe` when the file was replaced or removed
/// after the process started. The path still names the file a new binary was
/// installed over, which is exactly what a restart after an upgrade must run.
const DELETED_SUFFIX: &str = " (deleted)";

fn without_deleted_suffix(path: PathBuf) -> PathBuf {
    match path
        .to_str()
        .and_then(|text| text.strip_suffix(DELETED_SUFFIX))
    {
        Some(stripped) => PathBuf::from(stripped),
        None => path,
    }
}

impl RestartTarget {
    pub(crate) fn new(exe: PathBuf, args: Vec<OsString>) -> Self {
        Self {
            exe,
            args,
            transaction: None,
        }
    }

    /// The update guard for a staged transaction: `exe` is the old program's copy and the
    /// arguments are `update-guard --transaction <file>`.
    pub(crate) fn guard(exe: PathBuf, transaction: PathBuf) -> Self {
        let args = vec![
            exe.clone().into_os_string(),
            OsString::from("update-guard"),
            OsString::from("--transaction"),
            transaction.clone().into_os_string(),
        ];
        Self {
            exe,
            args,
            transaction: Some(transaction),
        }
    }

    /// The staged update transaction this target hands over to, if it is a guard.
    #[must_use]
    pub fn transaction(&self) -> Option<&Path> {
        self.transaction.as_deref()
    }

    /// The program the process was started from.
    pub(crate) fn executable(&self) -> &Path {
        &self.exe
    }

    /// `argv` as the process was started, as text (`None` when an argument is not UTF-8).
    pub(crate) fn arguments(&self) -> Option<Vec<String>> {
        self.args
            .iter()
            .map(|arg| arg.to_str().map(str::to_owned))
            .collect()
    }

    /// Checks, before anything is stopped, that the executable can plausibly be run.
    pub(crate) fn preflight(&self) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(&self.exe)
            .map_err(|error| format!("cannot restart: {}: {error}", self.exe.display()))?;
        if !metadata.is_file() {
            return Err(format!(
                "cannot restart: {} is not a file",
                self.exe.display()
            ));
        }
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(format!(
                "cannot restart: {} is not executable",
                self.exe.display()
            ));
        }
        Ok(())
    }

    /// Replaces the process image. Returns only when that failed.
    pub fn exec(&self) -> std::io::Error {
        let mut command = std::process::Command::new(&self.exe);
        if let Some(argv0) = self.args.first() {
            command.arg0(argv0);
        }
        command.args(self.args.iter().skip(1)).exec()
    }
}

/// Records the restart target once, at process start (before the executable can be
/// replaced). Without it, `POST /api/v1/system/restart` is reported unsupported.
pub fn capture() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = TARGET.set(RestartTarget::new(
            without_deleted_suffix(exe),
            std::env::args_os().collect(),
        ));
    }
}

/// The captured target, if any.
pub(crate) fn target() -> Option<&'static RestartTarget> {
    TARGET.get()
}

/// The captured target (for `main`).
#[must_use]
pub fn captured() -> Option<&'static RestartTarget> {
    target()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("restart-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn the_deleted_marker_of_a_replaced_binary_is_removed() {
        assert_eq!(
            without_deleted_suffix(PathBuf::from("/usr/local/bin/mosdns (deleted)")),
            PathBuf::from("/usr/local/bin/mosdns")
        );
        assert_eq!(
            without_deleted_suffix(PathBuf::from("/usr/local/bin/mosdns")),
            PathBuf::from("/usr/local/bin/mosdns")
        );
    }

    #[test]
    fn preflight_accepts_only_an_executable_regular_file() {
        let dir = temp("preflight");
        let file = dir.join("run");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        let target = RestartTarget::new(file.clone(), vec!["run".into()]);
        assert_eq!(target.preflight(), Ok(()));
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(target.preflight().unwrap_err().contains("not executable"));
        assert!(
            RestartTarget::new(dir.clone(), vec![])
                .preflight()
                .unwrap_err()
                .contains("not a file")
        );
        let missing = RestartTarget::new(dir.join("absent"), vec![]);
        assert!(missing.preflight().unwrap_err().contains("cannot restart"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_exec_returns_the_error_and_leaves_the_process_alone() {
        let dir = temp("exec-fails");
        let file = dir.join("not-executable");
        std::fs::write(&file, "x").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let error = RestartTarget::new(file, vec!["x".into()]).exec();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        let error = RestartTarget::new(dir.join("absent"), vec![]).exec();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

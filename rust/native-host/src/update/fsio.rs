//! Durable file replacement: temporary file in the same directory, `fsync` the file,
//! `rename` over the target, `fsync` the directory. A crash leaves either the old or the
//! new content, never a torn file, and a completed replacement survives power loss.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_name(target: &Path) -> io::Result<PathBuf> {
    let parent = target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    Ok(parent.join(format!(
        ".{}.update-{}-{}",
        name.to_string_lossy(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )))
}

/// Flushes a directory's entries to disk.
pub(crate) fn sync_dir(dir: &Path) -> io::Result<()> {
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    File::open(dir)?.sync_all()
}

/// Proves that a file can be created (and removed) in `dir`, the way a durable replacement
/// of a file in it would start.
pub(crate) fn check_writable_dir(dir: &Path) -> io::Result<()> {
    let probe = temp_name(&dir.join("probe"))?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&probe)?;
    std::fs::remove_file(&probe)
}

/// Writes `data` as `path` (creating missing parent directories) durably and atomically.
pub(crate) fn durable_write(path: &Path, data: &[u8], mode: u32) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = temp_name(path)?;
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temporary)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        // `mode` is subject to the umask at creation; the final mode is the requested one.
        std::fs::set_permissions(
            &temporary,
            std::os::unix::fs::PermissionsExt::from_mode(mode),
        )?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
        return result;
    }
    sync_dir(path.parent().unwrap_or_else(|| Path::new(".")))
}

/// Copies `source` to `destination` the same way (streaming, for programs).
pub(crate) fn durable_copy(source: &Path, destination: &Path, mode: u32) -> io::Result<()> {
    if let Some(parent) = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = temp_name(destination)?;
    let result = (|| {
        let mut input = File::open(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temporary)?;
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        std::fs::set_permissions(
            &temporary,
            std::os::unix::fs::PermissionsExt::from_mode(mode),
        )?;
        std::fs::rename(&temporary, destination)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
        return result;
    }
    sync_dir(destination.parent().unwrap_or_else(|| Path::new(".")))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("fsio-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn a_durable_write_replaces_atomically_creates_parents_and_leaves_no_temporary() {
        let root = dir("write");
        let target = root.join("a/b/file.yaml");
        durable_write(&target, b"one", 0o644).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"one");
        durable_write(&target, b"two", 0o600).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"two");
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let names: Vec<_> = std::fs::read_dir(target.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["file.yaml"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn writability_is_proved_by_creating_a_file_and_leaves_nothing_behind() {
        let root = dir("writable");
        check_writable_dir(&root).unwrap();
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        assert!(check_writable_dir(&root.join("absent")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_durable_copy_keeps_the_requested_mode_and_a_failed_write_leaves_the_old_file() {
        let root = dir("copy");
        let source = root.join("src");
        std::fs::write(&source, b"payload").unwrap();
        let target = root.join("bin");
        std::fs::write(&target, b"old").unwrap();
        durable_copy(&source, &target, 0o755).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"payload");
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o755
        );
        // A missing source fails and does not touch the target or leave a temporary.
        assert!(durable_copy(&root.join("absent"), &target, 0o755).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"payload");
        assert!(std::fs::read_dir(&root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".update-")
        }));
        std::fs::remove_dir_all(root).unwrap();
    }
}

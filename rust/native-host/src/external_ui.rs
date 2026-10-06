//! Startup-pinned external UI registry and request-owned blocking file work.
use mosdns_upstream_core::TransportCancellation;
#[cfg(target_os = "linux")]
use std::collections::BTreeMap;
use std::{io, path::Path, sync::Arc};
use tokio::{io::AsyncWrite, sync::Semaphore};
#[cfg(target_os = "linux")]
use tokio::{io::AsyncWriteExt, sync::mpsc};

#[derive(Default)]
pub(crate) struct Registry {
    #[cfg(all(test, target_os = "linux"))]
    io_gate: Option<Arc<IoGate>>,
    #[cfg(target_os = "linux")]
    mounts: BTreeMap<String, Arc<std::os::fd::OwnedFd>>,
}

impl Registry {
    pub(crate) fn scan(base: Option<&Path>) -> Self {
        #[cfg(target_os = "linux")]
        {
            use rustix::fs::{Dir, Mode, OFlags, ResolveFlags, open, openat2};
            let mut registry = Self::default();
            let Some(base) = base else {
                return registry;
            };
            let scan = || -> Result<_, rustix::io::Errno> {
                let base = open(
                    base,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                    Mode::empty(),
                )?;
                let ui = openat2(
                    &base,
                    "ui",
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                    Mode::empty(),
                    ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS,
                )?;
                Ok(ui)
            };
            let ui = match scan() {
                Ok(ui) => ui,
                Err(rustix::io::Errno::NOENT) => return registry,
                Err(error) => {
                    eprintln!("native UI registry unavailable: {error}");
                    return registry;
                }
            };
            let mut skipped = 0;
            let result = (|| -> Result<(), rustix::io::Errno> {
                let mut dir = Dir::read_from(&ui)?;
                for entry in &mut dir {
                    let entry = entry?;
                    let Ok(name) = entry.file_name().to_str() else {
                        skipped += 1;
                        continue;
                    };
                    if matches!(name, "." | "..") {
                        continue;
                    }
                    if name.is_empty()
                        || name.contains(['/', '\\'])
                        || name.chars().any(char::is_control)
                        || matches!(
                            name,
                            "root"
                                | "log"
                                | "log1"
                                | "legacy"
                                | "blog"
                                | "assets"
                                | "debug"
                                | "metrics"
                                | "plugins"
                                | "api"
                        )
                    {
                        skipped += 1;
                        continue;
                    }
                    match openat2(
                        &ui,
                        name,
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                        Mode::empty(),
                        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS,
                    ) {
                        Ok(fd) => {
                            registry.mounts.insert(name.to_owned(), Arc::new(fd));
                        }
                        Err(_) => skipped += 1,
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                eprintln!("native UI registry enumeration failed: {error}");
                return Self::default();
            }
            if skipped > 0 {
                eprintln!(
                    "native UI registry skipped {skipped} reserved, invalid, symlink or non-directory entries"
                );
            }
            registry
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = base;
            Self::default()
        }
    }

    pub(crate) async fn serve<W: AsyncWrite + Unpin>(
        &self,
        writer: &mut W,
        target: &str,
        method: &str,
        slots: Arc<Semaphore>,
        shutdown: &TransportCancellation,
    ) -> io::Result<bool> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (writer, target, method, slots, shutdown);
            Ok(false)
        }
        #[cfg(target_os = "linux")]
        {
            let raw = target.split('?').next().unwrap_or(target);
            let decoded = crate::static_ui::decoded_path(target);
            let route = decoded.as_deref().unwrap_or(raw);
            let name = route
                .strip_prefix('/')
                .unwrap_or(route)
                .split('/')
                .next()
                .unwrap_or("");
            let Some(root) = self.mounts.get(name) else {
                return Ok(false);
            };
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
            let response = async {
                if !matches!(method, "GET" | "HEAD") {
                    return quick(header(writer, 405, 0, "Allow: GET, HEAD\r\n"), shutdown).await;
                }
                let Some(path) = decoded.as_deref() else {
                    return quick(header(writer, 404, 0, ""), shutdown).await;
                };
                if path == format!("/{name}") {
                    return quick(redirect(writer, &format!("/{name}/"), target), shutdown).await;
                }
                let Some(permit) = slots.try_acquire_owned().ok() else {
                    return quick(header(writer, 503, 0, ""), shutdown).await;
                };
                let permit = Arc::new(permit);
                let root = root.clone();
                let relative = path[name.len() + 2..].to_owned();
                let index = path.ends_with('/');
                let head = method == "HEAD";
                let (sender, mut receiver) = mpsc::channel(1);
                let worker_permit = permit.clone();
                #[cfg(test)]
                let gate = self.io_gate.clone();
                let worker = tokio::task::spawn_blocking(move || {
                    #[cfg(test)]
                    if let Some(gate) = gate {
                        gate.wait();
                    }
                    let _permit = worker_permit;
                    read_file(root, &relative, index, head, sender);
                });
                // Cancellation/timeouts close the channel first, then join the
                // started operation. No dropped await can orphan an FD/permit.
                let transfer = async {
                    match receiver.recv().await {
                        Some(Event::Status(status)) => header(writer, status, 0, "").await,
                        Some(Event::Directory) => {
                            redirect(writer, &format!("{path}/"), target).await
                        }
                        Some(Event::Ready(length)) => {
                            let kind =
                                crate::static_ui::mime(if index { "index.html" } else { path });
                            header(writer, 200, length, &format!("Content-Type: {kind}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n")).await?;
                            while let Some(event) = receiver.recv().await {
                                match event {
                                    Event::Chunk(bytes) => writer.write_all(&bytes).await?,
                                    Event::Error(error) => return Err(error),
                                    _ => {
                                        return Err(io::Error::other(
                                            "external UI worker protocol",
                                        ));
                                    }
                                }
                            }
                            writer.flush().await
                        }
                        _ => Err(io::Error::other("external UI worker failed")),
                    }
                };
                let result = tokio::select! {
                    biased;
                    () = shutdown.cancelled() => Ok(()),
                    result = tokio::time::timeout_at(deadline, transfer) => result.unwrap_or_else(|_| Err(io::Error::new(io::ErrorKind::TimedOut, "external UI response deadline"))),
                };
                receiver.close();
                drop(receiver);
                let joined = worker.await.map_err(io::Error::other);
                drop(permit);
                joined?;
                result
            };
            response.await?;
            Ok(true)
        }
    }
}

#[cfg(target_os = "linux")]
async fn redirect<W: AsyncWrite + Unpin>(
    writer: &mut W,
    path: &str,
    target: &str,
) -> io::Result<()> {
    // Encode decoded filesystem segments back into a URL, preserving only path separators.
    let mut encoded = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    match crate::static_ui::redirect_target(&encoded, target) {
        Some(location) => header(writer, 301, 0, &format!("Location: {location}\r\n")).await,
        None => header(writer, 404, 0, "").await,
    }
}
#[cfg(target_os = "linux")]
async fn header<W: AsyncWrite + Unpin>(
    writer: &mut W,
    status: u16,
    length: u64,
    extra: &str,
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        301 => "Moved Permanently",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    writer.write_all(format!("HTTP/1.1 {status} {reason}\r\n{extra}Content-Length: {length}\r\nConnection: close\r\n\r\n").as_bytes()).await
}
#[cfg(target_os = "linux")]
enum Event {
    Status(u16),
    Directory,
    Ready(u64),
    Chunk(Vec<u8>),
    Error(io::Error),
}
#[cfg(target_os = "linux")]
fn read_file(
    root: Arc<std::os::fd::OwnedFd>,
    relative: &str,
    index: bool,
    head: bool,
    sender: mpsc::Sender<Event>,
) {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::io::Read;
    let open = |path: &str| {
        openat2(
            &*root,
            path,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS,
        )
    };
    let path = if relative.is_empty() { "." } else { relative };
    let opened = open(path).map(std::fs::File::from);
    let mut file = match opened {
        Ok(file) => file,
        Err(error) => {
            let _ = sender.blocking_send(Event::Status(open_status(error)));
            return;
        }
    };
    let mut metadata = match file.metadata() {
        Ok(meta) => meta,
        Err(_) => {
            let _ = sender.blocking_send(Event::Status(500));
            return;
        }
    };
    if metadata.is_dir() {
        if !index {
            let _ = sender.blocking_send(Event::Directory);
            return;
        }
        file = match open(&format!("{path}/index.html")).map(std::fs::File::from) {
            Ok(file) => file,
            Err(error) => {
                let _ = sender.blocking_send(Event::Status(open_status(error)));
                return;
            }
        };
        metadata = match file.metadata() {
            Ok(meta) => meta,
            Err(_) => {
                let _ = sender.blocking_send(Event::Status(500));
                return;
            }
        };
    }
    if !metadata.is_file() {
        let _ = sender.blocking_send(Event::Status(404));
        return;
    }
    if metadata.len() > 16 * 1024 * 1024 {
        let _ = sender.blocking_send(Event::Status(413));
        return;
    }
    if sender.blocking_send(Event::Ready(metadata.len())).is_err() || head {
        return;
    }
    let mut remaining = metadata.len();
    while remaining > 0 {
        let mut chunk = vec![0; usize::try_from(remaining.min(64 * 1024)).unwrap_or(64 * 1024)];
        if let Err(error) = file.read_exact(&mut chunk) {
            let _ = sender.blocking_send(Event::Error(error));
            return;
        }
        remaining -= chunk.len() as u64;
        if sender.blocking_send(Event::Chunk(chunk)).is_err() {
            return;
        }
    }
}
#[cfg(target_os = "linux")]
fn open_status(error: rustix::io::Errno) -> u16 {
    match error {
        rustix::io::Errno::NXIO
        | rustix::io::Errno::NODEV
        | rustix::io::Errno::NOENT
        | rustix::io::Errno::NOTDIR
        | rustix::io::Errno::LOOP
        | rustix::io::Errno::XDEV
        | rustix::io::Errno::AGAIN => 404,
        _ => 500,
    }
}

#[cfg(target_os = "linux")]
async fn quick<F: std::future::Future<Output = io::Result<()>>>(
    response: F,
    shutdown: &TransportCancellation,
) -> io::Result<()> {
    tokio::select! {
        biased;
        () = shutdown.cancelled() => Ok(()),
        result = tokio::time::timeout(std::time::Duration::from_secs(10), response) => result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "external UI response deadline"))?,
    }
}

#[cfg(all(test, target_os = "linux"))]
#[derive(Default)]
struct IoGate {
    open: std::sync::Mutex<bool>,
    changed: std::sync::Condvar,
    entered: std::sync::atomic::AtomicUsize,
}
#[cfg(all(test, target_os = "linux"))]
impl IoGate {
    fn wait(&self) {
        self.entered
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.changed.wait(open).unwrap();
        }
    }
    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    #[test]
    fn blocking_owner_retains_permits_and_shutdown_joins_started_io() {
        let root = std::env::temp_dir().join(format!("mosdns-ui-owned-{}", std::process::id()));
        std::fs::create_dir_all(root.join("ui/demo")).unwrap();
        std::fs::write(root.join("ui/demo/index.html"), "proof").unwrap();
        let gate = Arc::new(IoGate::default());
        let mut registry = Registry::scan(Some(&root));
        registry.io_gate = Some(gate.clone());
        let registry = Arc::new(registry);
        crate::HostRuntime::new().unwrap().block_on(async {
            let slots = Arc::new(Semaphore::new(4));
            let shutdown = TransportCancellation::new();
            let mut tasks = Vec::new();
            let mut readers = Vec::new();
            for _ in 0..4 {
                let (mut writer, reader) = tokio::io::duplex(1024);
                readers.push(reader);
                let registry = registry.clone();
                let slots = slots.clone();
                let shutdown = shutdown.clone();
                tasks.push(tokio::task::spawn_local(async move {
                    registry
                        .serve(&mut writer, "/demo/", "HEAD", slots, &shutdown)
                        .await
                }));
            }
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while gate.entered.load(std::sync::atomic::Ordering::SeqCst) < 4 {
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
            })
            .await
            .unwrap();
            // The LocalSet keeps running while all filesystem owners stall.
            assert_eq!(slots.available_permits(), 0);
            let (mut writer, mut reader) = tokio::io::duplex(1024);
            registry
                .serve(&mut writer, "/demo/", "HEAD", slots.clone(), &shutdown)
                .await
                .unwrap();
            drop(writer);
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).await.unwrap();
            assert!(bytes.starts_with(b"HTTP/1.1 503"));
            shutdown.cancel();
            tokio::task::yield_now().await;
            assert!(tasks.iter().all(|task| !task.is_finished()));
            assert_eq!(slots.available_permits(), 0);
            gate.release();
            for task in tasks {
                assert!(task.await.unwrap().unwrap());
            }
            assert_eq!(slots.available_permits(), 4);
            drop(readers);
        });
        std::fs::remove_dir_all(root).unwrap();
    }
}

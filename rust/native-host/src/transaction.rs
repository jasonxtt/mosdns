//! The bounded blocking-I/O worker: filesystem work for the compiler and the apply path runs
//! on one dedicated thread with a one-slot queue, off the host runtime.
//! (The durable `native_management` transaction store that shared this file was removed.)
use std::sync::OnceLock;
use std::sync::mpsc::{SyncSender, sync_channel};

type IoJob = Box<dyn FnOnce() + Send + 'static>;

struct BlockingIoWorker {
    sender: SyncSender<IoJob>,
}

static BLOCKING_IO_WORKER: OnceLock<Result<BlockingIoWorker, String>> = OnceLock::new();

std::thread_local! {
    static ON_BLOCKING_IO_WORKER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run filesystem work on one dedicated worker with one queued operation.
/// Compiler results stay on the current thread because the native graph is Rc-owned.
pub(crate) fn blocking_io<T, F>(operation: F) -> std::io::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    if ON_BLOCKING_IO_WORKER.with(std::cell::Cell::get) {
        return std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation))
            .map_err(|_| std::io::Error::other("blocking I/O worker operation panicked"));
    }
    let worker = BLOCKING_IO_WORKER.get_or_init(|| {
        let (sender, receiver) = sync_channel::<IoJob>(1);
        std::thread::Builder::new()
            .name("mosdns-blocking-io".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    ON_BLOCKING_IO_WORKER.with(|inside| inside.set(true));
                    job();
                    ON_BLOCKING_IO_WORKER.with(|inside| inside.set(false));
                }
            })
            .map(|_| BlockingIoWorker { sender })
            .map_err(|error| error.to_string())
    });
    let worker = worker
        .as_ref()
        .map_err(|reason| std::io::Error::other(reason.clone()))?;
    let (send, receive) = sync_channel(0);
    worker
        .sender
        .send(Box::new(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation))
                .map_err(|_| std::io::Error::other("blocking I/O worker operation panicked"));
            let _ = send.send(result);
        }))
        .map_err(|_| std::io::Error::other("blocking I/O worker stopped"))?;
    receive
        .recv()
        .map_err(|_| std::io::Error::other("blocking I/O worker stopped"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_io_worker_serializes_filesystem_jobs_on_one_named_thread() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let active = std::sync::Arc::new(AtomicUsize::new(0));
        let peak = std::sync::Arc::new(AtomicUsize::new(0));
        let mut jobs = Vec::new();
        for _ in 0..8 {
            let active = active.clone();
            let peak = peak.clone();
            jobs.push(std::thread::spawn(move || {
                blocking_io(move || {
                    assert_eq!(std::thread::current().name(), Some("mosdns-blocking-io"));
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    active.fetch_sub(1, Ordering::SeqCst);
                })
                .unwrap();
            }));
        }
        for job in jobs {
            job.join().unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }
}

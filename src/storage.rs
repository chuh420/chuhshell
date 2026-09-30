use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub fn read_limited(path: &Path, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(std::io::Error::other("File exceeds the size limit"));
    }
    Ok(bytes)
}

pub fn read_text(path: &Path, limit: usize) -> std::io::Result<String> {
    String::from_utf8(read_limited(path, limit)?)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

#[derive(Debug)]
enum Commit {
    Durable,
    Unconfirmed(std::io::Error),
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Commit::Unconfirmed(error) = atomic_write_with_sync(path, bytes, |parent| {
        std::fs::File::open(parent)?.sync_all()
    })? {
        eprintln!(
            "chuhshell: saved {} but durability could not be confirmed: {error}",
            path.display()
        );
    }
    Ok(())
}

fn atomic_write_with_sync(
    path: &Path,
    bytes: &[u8],
    sync: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::io::Result<Commit> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("Missing parent directory"))?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".chuhshell-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        if let Ok(metadata) = std::fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        Ok(match sync(parent) {
            Ok(()) => Commit::Durable,
            Err(error) => Commit::Unconfirmed(error),
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

type Job = Box<dyn FnOnce() + Send>;

struct Worker {
    sender: async_channel::Sender<Job>,
    thread: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Worker {
    fn new() -> Self {
        let (sender, receiver) = async_channel::bounded::<Job>(64);
        let thread = std::thread::spawn(move || {
            while let Ok(job) = receiver.recv_blocking() {
                job();
            }
        });
        Self {
            sender,
            thread: std::sync::Mutex::new(Some(thread)),
        }
    }

    fn submit<T: Send + 'static, F: FnOnce() -> Result<T, String> + Send + 'static>(
        &self,
        operation: F,
    ) -> impl std::future::Future<Output = Result<T, String>> + use<T, F> {
        let (sender, receiver) = async_channel::bounded(1);
        let queued = self
            .sender
            .try_send(Box::new(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation))
                    .unwrap_or_else(|_| Err("Settings worker interrupted".into()));
                let _ = sender.try_send(result);
            }))
            .map_err(|_| "Settings queue is full or stopped".to_owned());
        async move {
            queued?;
            receiver
                .recv()
                .await
                .map_err(|_| "Settings worker stopped".to_owned())?
        }
    }

    fn stop(&self) {
        self.sender.close();
        if let Some(thread) = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = thread.join();
        }
    }
}

static WORKER: std::sync::OnceLock<Worker> = std::sync::OnceLock::new();

pub fn run<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> impl std::future::Future<Output = Result<T, String>> {
    WORKER.get_or_init(Worker::new).submit(operation)
}

pub fn shutdown() {
    if let Some(worker) = WORKER.get() {
        worker.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn committed_write_and_durability_failure_are_distinct() {
        let path = std::env::temp_dir().join(format!("chuhshell-commit-{}", std::process::id()));
        let result = atomic_write_with_sync(&path, b"committed", |_| {
            Err(std::io::Error::other("fsync failed"))
        })
        .unwrap();
        assert!(matches!(result, Commit::Unconfirmed(_)));
        assert_eq!(std::fs::read(&path).unwrap(), b"committed");
        assert!(read_limited(&path, 8).is_err());
        assert_eq!(read_limited(&path, 9).unwrap(), b"committed");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn worker_bounds_the_queue_and_drains_in_submission_order() {
        let worker = Worker::new();
        let (release, wait) = std::sync::mpsc::channel();
        let (started, ready) = std::sync::mpsc::channel();
        let first = worker.submit(move || {
            started.send(()).unwrap();
            wait.recv().unwrap();
            Ok(())
        });
        ready.recv().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut jobs = Vec::new();
        for i in 0..64 {
            let seen = seen.clone();
            jobs.push(worker.submit(move || {
                seen.lock().unwrap().push(i);
                Ok(())
            }));
        }
        let context = glib::MainContext::new();
        assert!(context.block_on(worker.submit(|| Ok(()))).is_err());
        release.send(()).unwrap();
        worker.stop();
        context.block_on(first).unwrap();
        for job in jobs {
            context.block_on(job).unwrap();
        }
        assert_eq!(*seen.lock().unwrap(), (0..64).collect::<Vec<_>>());
        assert!(context.block_on(worker.submit(|| Ok(()))).is_err());
    }

    #[test]
    fn replacement_preserves_permissions_and_complete_content() {
        let path = std::env::temp_dir().join(format!("chuhshell-storage-{}", std::process::id()));
        atomic_write(&path, b"first").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        atomic_write(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        std::fs::remove_file(path).unwrap();
    }
}

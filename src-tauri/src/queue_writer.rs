//! Saves the download queue on one background thread, in order.
//!
//! Callers never write the file themselves: they bump the queue revision and ask for it to
//! be saved. Ordinary changes (progress, status) are coalesced for up to 250 ms; a flush
//! barrier asks for the save at once and waits until that revision, or a newer snapshot,
//! is on disk. The snapshot is taken when the writer owns the store lock, so a stale copy
//! of the queue is never scheduled or written over a newer one.
//!
//! On disk the queue is versioned: `{ schemaVersion, revision, jobs }`. The bare job list
//! that 1.0.4 wrote is still read (and backed up once before it is replaced); a file from
//! a newer version is set aside with a notice rather than replaced by an empty queue.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::downloader::Job;

pub const QUEUE_SCHEMA: u16 = 2;
const COALESCE: Duration = Duration::from_millis(250);
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(2);

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    pub schema_version: u16,
    pub revision: u64,
    pub jobs: Vec<Job>,
}

#[derive(Default)]
struct Progress {
    /// Highest revision anyone asked to have saved.
    requested: u64,
    /// Highest revision known to be on disk.
    durable: u64,
    /// Someone is waiting: skip the coalescing delay.
    urgent: bool,
    /// The last attempt failed; reported once until a save succeeds again.
    failing: bool,
    stopped: bool,
}

struct Shared {
    progress: Mutex<Progress>,
    changed: Condvar,
}

/// Saves a snapshot and returns the revision it contains.
pub type SaveFn = Box<dyn FnMut() -> Result<u64, String> + Send>;
pub type ReportFn = Box<dyn Fn(String) + Send>;

pub struct QueueWriter {
    shared: Arc<Shared>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl QueueWriter {
    pub fn start(mut save: SaveFn, report: ReportFn) -> Self {
        let shared = Arc::new(Shared {
            progress: Mutex::new(Progress::default()),
            changed: Condvar::new(),
        });
        let worker = shared.clone();
        let thread = std::thread::Builder::new()
            .name("claws-queue-writer".into())
            .spawn(move || loop {
                {
                    let mut p = worker.progress.lock().unwrap();
                    while p.requested <= p.durable && !p.stopped {
                        p = worker.changed.wait(p).unwrap();
                    }
                    if p.requested <= p.durable {
                        return; // stopped with nothing left to save
                    }
                    // Coalesce ordinary updates; a barrier or a stop cuts the wait short.
                    let deadline = Instant::now() + COALESCE;
                    while !p.urgent && !p.stopped {
                        let left = deadline.saturating_duration_since(Instant::now());
                        if left.is_zero() {
                            break;
                        }
                        p = worker.changed.wait_timeout(p, left).unwrap().0;
                    }
                    p.urgent = false;
                }
                let result = save();
                let mut p = worker.progress.lock().unwrap();
                match result {
                    Ok(revision) => {
                        p.durable = p.durable.max(revision);
                        p.failing = false;
                        worker.changed.notify_all();
                    }
                    Err(error) => {
                        let first = !p.failing;
                        p.failing = true;
                        // Waiters give up at their own timeout; the change stays dirty.
                        worker.changed.notify_all();
                        drop(p);
                        if first {
                            report(error);
                        }
                        let p = worker.progress.lock().unwrap();
                        if p.stopped {
                            return;
                        }
                        let _ = worker.changed.wait_timeout(p, RETRY_AFTER_FAILURE);
                    }
                }
            })
            .expect("cannot start the queue writer");
        Self {
            shared,
            thread: Mutex::new(Some(thread)),
        }
    }

    /// Asks for `revision` (or newer) to be saved, within the coalescing delay unless urgent.
    pub fn request(&self, revision: u64, urgent: bool) {
        let mut p = self.shared.progress.lock().unwrap();
        p.requested = p.requested.max(revision);
        p.urgent |= urgent;
        self.shared.changed.notify_all();
    }

    /// Saves now and waits until `revision` or a newer snapshot is durable. Returns false
    /// if that did not happen within `timeout` (a failing disk); the change stays pending.
    /// Blocks: call it from a blocking context, never while holding a state lock.
    pub fn flush(&self, revision: u64, timeout: Duration) -> bool {
        self.request(revision, true);
        let deadline = Instant::now() + timeout;
        let mut p = self.shared.progress.lock().unwrap();
        while p.durable < revision {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || p.stopped {
                return false;
            }
            p = self.shared.changed.wait_timeout(p, left).unwrap().0;
        }
        true
    }

    /// Saves anything pending, then ends the thread.
    pub fn stop(&self) {
        {
            let mut p = self.shared.progress.lock().unwrap();
            p.stopped = true;
            p.urgent = true;
            self.shared.changed.notify_all();
        }
        if let Some(thread) = self.thread.lock().unwrap().take() {
            let _ = thread.join();
        }
    }

    #[cfg(test)]
    fn durable(&self) -> u64 {
        self.shared.progress.lock().unwrap().durable
    }
}

/// Reads the queue, accepting the 1.0.4 format. Damaged files are moved aside (as before);
/// a file from a newer version is kept aside untouched, with a notice.
pub fn load(path: &Path, notices: &mut Vec<String>) -> Vec<Job> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            notices.push(format!("Claws could not read your download queue ({e}), so it started with an empty one."));
            return Vec::new();
        }
    };
    let set_aside = |label: &str| -> PathBuf {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let aside = path.with_file_name(format!("queue.{label}-{stamp}.json"));
        let _ = std::fs::rename(path, &aside);
        aside
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => {
            let aside = set_aside("corrupt");
            notices.push(format!("Your download queue was damaged, so Claws started with an empty one. The old file was kept as {}.", file_name(&aside)));
            return Vec::new();
        }
    };
    if value.is_array() {
        // Written by 1.0.4 or earlier. Kept once, untouched, before the new format replaces it.
        let backup = path.with_file_name("queue.pre-1.0.5.json");
        if !backup.exists() {
            let _ = std::fs::copy(path, &backup);
        }
        return match serde_json::from_value(value) {
            Ok(jobs) => jobs,
            Err(_) => {
                notices.push("Your download queue from an earlier version could not be read, so Claws started with an empty one. A copy was kept as queue.pre-1.0.5.json.".into());
                Vec::new()
            }
        };
    }
    let version = value.get("schemaVersion").and_then(|v| v.as_u64()).unwrap_or(0);
    if version > QUEUE_SCHEMA as u64 {
        let aside = set_aside("newer-version");
        notices.push(format!(
            "Your download queue was saved by a newer version of Claws and was not changed. It was kept as {}; open that version again to continue those downloads.",
            file_name(&aside)
        ));
        return Vec::new();
    }
    match serde_json::from_value::<QueueSnapshot>(value) {
        Ok(snapshot) => snapshot.jobs,
        Err(_) => {
            let aside = set_aside("corrupt");
            notices.push(format!("Your download queue was damaged, so Claws started with an empty one. The old file was kept as {}.", file_name(&aside)));
            Vec::new()
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    fn writer_with(save: impl FnMut() -> Result<u64, String> + Send + 'static) -> (QueueWriter, Arc<Mutex<Vec<String>>>) {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let sink = reports.clone();
        (QueueWriter::start(Box::new(save), Box::new(move |e| sink.lock().unwrap().push(e))), reports)
    }

    #[test]
    fn ordinary_updates_are_coalesced() {
        let revision = Arc::new(AtomicU64::new(0));
        let saves = Arc::new(AtomicUsize::new(0));
        let (r, s) = (revision.clone(), saves.clone());
        let (writer, _) = writer_with(move || {
            s.fetch_add(1, Ordering::SeqCst);
            Ok(r.load(Ordering::SeqCst))
        });
        for _ in 0..50 {
            let rev = revision.fetch_add(1, Ordering::SeqCst) + 1;
            writer.request(rev, false);
        }
        assert!(writer.flush(50, Duration::from_secs(5)));
        assert!(saves.load(Ordering::SeqCst) <= 3, "{} saves for 50 updates", saves.load(Ordering::SeqCst));
        writer.stop();
    }

    #[test]
    fn a_barrier_does_not_wait_for_the_coalescing_delay() {
        let (writer, _) = writer_with(|| Ok(1));
        let started = Instant::now();
        assert!(writer.flush(1, Duration::from_secs(5)));
        assert!(started.elapsed() < Duration::from_millis(200), "{:?}", started.elapsed());
        writer.stop();
    }

    #[test]
    fn durable_revisions_never_go_backwards() {
        let revision = Arc::new(AtomicU64::new(0));
        let r = revision.clone();
        let (writer, _) = writer_with(move || Ok(r.load(Ordering::SeqCst)));
        revision.store(5, Ordering::SeqCst);
        writer.request(5, true);
        writer.request(3, true); // a late, older notification
        assert!(writer.flush(5, Duration::from_secs(5)));
        assert_eq!(writer.durable(), 5);
        writer.request(4, true);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(writer.durable(), 5);
        writer.stop();
    }

    #[test]
    fn a_slow_disk_still_ends_with_the_newest_state() {
        let revision = Arc::new(AtomicU64::new(0));
        let r = revision.clone();
        let (writer, _) = writer_with(move || {
            let captured = r.load(Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(120));
            Ok(captured)
        });
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let revision = revision.clone();
                std::thread::spawn(move || {
                    for _ in 0..10 {
                        revision.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(5));
                    }
                })
            })
            .collect();
        for _ in 0..20 {
            writer.request(revision.load(Ordering::SeqCst), false);
            std::thread::sleep(Duration::from_millis(10));
        }
        threads.into_iter().for_each(|t| t.join().unwrap());
        let last = revision.load(Ordering::SeqCst);
        assert_eq!(last, 80);
        assert!(writer.flush(last, Duration::from_secs(5)));
        assert_eq!(writer.durable(), 80);
        writer.stop();
    }

    #[test]
    fn a_failure_is_reported_once_and_recovers() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let a = attempts.clone();
        let (writer, reports) = writer_with(move || {
            if a.fetch_add(1, Ordering::SeqCst) < 2 {
                Err("disk full".into())
            } else {
                Ok(1)
            }
        });
        assert!(!writer.flush(1, Duration::from_millis(300)), "the first attempt fails");
        // It keeps retrying in the background and succeeds on the third attempt.
        assert!(writer.flush(1, Duration::from_secs(10)));
        assert_eq!(reports.lock().unwrap().as_slice(), ["disk full"]);
        writer.stop();
    }

    #[test]
    fn stop_saves_what_is_pending() {
        let saved = Arc::new(AtomicU64::new(0));
        let s = saved.clone();
        let (writer, _) = writer_with(move || {
            s.store(9, Ordering::SeqCst);
            Ok(9)
        });
        writer.request(9, false);
        writer.stop();
        assert_eq!(saved.load(Ordering::SeqCst), 9);
    }

    fn temp(tag: &str) -> PathBuf {
        crate::download_tests::temp_dir(tag)
    }

    #[test]
    fn the_1_0_4_queue_loads_and_is_backed_up() {
        let dir = temp("queue-legacy");
        let path = dir.join("queue.json");
        std::fs::write(&path, "[]").unwrap();
        let mut notices = Vec::new();
        assert!(load(&path, &mut notices).is_empty());
        assert!(notices.is_empty());
        assert_eq!(std::fs::read_to_string(dir.join("queue.pre-1.0.5.json")).unwrap(), "[]");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_newer_queue_is_set_aside_not_emptied() {
        let dir = temp("queue-newer");
        let path = dir.join("queue.json");
        std::fs::write(&path, r#"{"schemaVersion":99,"revision":1,"jobs":[]}"#).unwrap();
        let mut notices = Vec::new();
        assert!(load(&path, &mut notices).is_empty());
        assert_eq!(notices.len(), 1);
        assert!(!path.exists());
        let kept: Vec<String> = crate::download_tests::names(&dir);
        assert!(kept.iter().any(|n| n.starts_with("queue.newer-version-")), "{kept:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_current_format_round_trips() {
        let dir = temp("queue-current");
        let path = dir.join("queue.json");
        let snapshot = QueueSnapshot { schema_version: QUEUE_SCHEMA, revision: 7, jobs: Vec::new() };
        std::fs::write(&path, serde_json::to_string(&snapshot).unwrap()).unwrap();
        let mut notices = Vec::new();
        assert!(load(&path, &mut notices).is_empty());
        assert!(notices.is_empty());
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}

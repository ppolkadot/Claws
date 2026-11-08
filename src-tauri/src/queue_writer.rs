//! Saves the download queue on one background thread, in order.
//!
//! Ordinary changes are coalesced for up to 250 ms; a flush barrier saves at once and
//! waits until that revision or a newer one is on disk. The snapshot is taken while the
//! writer holds the store lock, so a stale copy is never written over a newer one.
//!
//! On disk: `{ schemaVersion, revision, jobs }`. The bare job list 1.0.4 wrote is still
//! read (and backed up once); a file from a newer version is set aside, not emptied.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::downloader::Job;

/// 3: jobs carry `videoPreset` (and later option fields). An older Claws would read such a
/// job but drop the field on its next save, silently turning an Original download back
/// into a Compatible one, so it sets a schema 3 file aside instead. Schema 2 files load
/// unchanged; their jobs read as Compatible.
pub const QUEUE_SCHEMA: u16 = 3;
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
    /// Save attempts begun so far, and the number and error of the latest failed one. A
    /// barrier fails as soon as an attempt that began after it was requested fails.
    attempts: u64,
    failed_attempt: u64,
    last_error: String,
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
                    p.attempts += 1;
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
                        p.failed_attempt = p.attempts;
                        p.last_error = error.clone();
                        // Barriers waiting on this attempt fail now; the change stays dirty
                        // and is retried.
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

    /// Saves now and waits until `revision` or newer is durable. Fails if a save begun after
    /// this request fails, on timeout, or once the writer stopped; the change stays pending
    /// and is retried. Blocks: never call it while holding a state lock.
    pub fn flush(&self, revision: u64, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        let mut p = self.shared.progress.lock().unwrap();
        // An attempt already running may have taken its snapshot before this revision;
        // only attempts numbered after this one are sure to contain it.
        let begun = p.attempts;
        p.requested = p.requested.max(revision);
        p.urgent = true;
        self.shared.changed.notify_all();
        while p.durable < revision {
            if p.failed_attempt > begun {
                return Err(p.last_error.clone());
            }
            if p.stopped {
                return Err("the queue writer has stopped".into());
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!("saving took longer than {} seconds", timeout.as_secs()));
            }
            p = self.shared.changed.wait_timeout(p, left).unwrap().0;
        }
        Ok(())
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

/// Reads the queue, accepting the 1.0.4 format. Damaged files and files from a newer
/// version are moved aside untouched, with a notice.
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
        // 1.0.4 or earlier: backed up once before the new format replaces it.
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
        assert!(writer.flush(50, Duration::from_secs(5)).is_ok());
        assert!(saves.load(Ordering::SeqCst) <= 3, "{} saves for 50 updates", saves.load(Ordering::SeqCst));
        writer.stop();
    }

    #[test]
    fn a_barrier_does_not_wait_for_the_coalescing_delay() {
        let (writer, _) = writer_with(|| Ok(1));
        let started = Instant::now();
        assert!(writer.flush(1, Duration::from_secs(5)).is_ok());
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
        assert!(writer.flush(5, Duration::from_secs(5)).is_ok());
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
        assert!(writer.flush(last, Duration::from_secs(5)).is_ok());
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
        let started = Instant::now();
        assert_eq!(writer.flush(1, Duration::from_secs(10)), Err("disk full".to_string()), "the first attempt fails");
        assert!(started.elapsed() < Duration::from_secs(1), "a failed save is reported at once, not at the timeout");
        // A new barrier asks for a new attempt; the third one succeeds.
        assert!(writer.flush(1, Duration::from_secs(10)).is_err());
        assert!(writer.flush(1, Duration::from_secs(10)).is_ok());
        assert_eq!(reports.lock().unwrap().as_slice(), ["disk full"]);
        writer.stop();
    }

    #[test]
    fn a_slow_save_times_the_barrier_out_and_still_lands() {
        let (writer, _) = writer_with(|| {
            std::thread::sleep(Duration::from_millis(400));
            Ok(1)
        });
        let err = writer.flush(1, Duration::from_millis(100)).unwrap_err();
        assert!(err.contains("longer than"), "{err}");
        assert!(writer.flush(1, Duration::from_secs(5)).is_ok());
        writer.stop();
    }

    #[test]
    fn a_barrier_after_stop_fails() {
        let (writer, _) = writer_with(|| Ok(1));
        writer.stop();
        assert!(writer.flush(2, Duration::from_secs(1)).is_err());
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

    /// One saved job as 1.0.5 (queue schema 2) wrote it: no `videoPreset`. `execution` is
    /// the frozen settings of a job that had started, or null for one that had not.
    fn schema_2_job(id: &str, status: &str, execution: &str) -> String {
        format!(
            r#"{{"id":"{id}","url":"https://example.com/{id}","playlistIndex":null,"title":"{id}","thumbnail":null,"uploader":null,"duration":60.0,
               "options":{{"mode":"video","maxHeight":2160,"audioFormat":"mp3","subtitleLangs":[],"autoSubs":false,"embedSubs":false,"fastSave":false}},
               "status":"{status}","progress":0.25,"progressKnown":true,"progressEstimated":false,"downloadedBytes":100,"totalBytes":400,
               "speed":null,"eta":null,"filePath":null,"error":null,"createdAt":1,"partialFiles":[],"execution":{execution},
               "workspace":null,"cancelRequested":false,"revision":4,"attempt":0}}"#
        )
    }

    #[test]
    fn a_schema_2_queue_loads_with_every_job_compatible() {
        use crate::download_policy::VideoPreset;
        let dir = temp("queue-schema-2");
        let path = dir.join("queue.json");
        let execution = r#"{"schemaVersion":1,"outputDir":"D:/Out","filenameTemplate":"%(title)s.%(ext)s","embedMetadata":true,"audioSelection":"preferMatching","fragmentConcurrency":8,"transport":"native"}"#;
        let text = format!(r#"{{"schemaVersion":2,"revision":9,"jobs":[{},{}]}}"#, schema_2_job("started", "paused", execution), schema_2_job("waiting", "queued", "null"));
        std::fs::write(&path, text).unwrap();
        let mut notices = Vec::new();
        let jobs = load(&path, &mut notices);
        assert!(notices.is_empty(), "{notices:?}");
        assert!(path.exists(), "an older queue is read in place, not set aside");
        assert_eq!(jobs.len(), 2);
        assert!(jobs.iter().all(|j| j.options.video_preset == VideoPreset::Compatible));
        assert!(jobs[0].execution.as_ref().is_some_and(|e| e.output_dir == Path::new("D:/Out")), "the frozen settings survive");
        assert!(jobs[1].execution.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_schema_3_queue_round_trips_its_presets() {
        use crate::download_policy::VideoPreset;
        assert_eq!(QUEUE_SCHEMA, 3, "presets are a field an older Claws would drop: it must set the file aside");
        let dir = temp("queue-schema-3");
        let path = dir.join("queue.json");
        let mut job: crate::downloader::Job = serde_json::from_str(&schema_2_job("eight-k", "paused", "null")).unwrap();
        job.options.video_preset = VideoPreset::Original;
        job.options.max_height = Some(4320);
        let snapshot = QueueSnapshot { schema_version: QUEUE_SCHEMA, revision: 3, jobs: vec![job] };
        let text = serde_json::to_string(&snapshot).unwrap();
        assert!(text.contains(r#""schemaVersion":3"#) && text.contains(r#""videoPreset":"original""#), "{text}");
        std::fs::write(&path, text).unwrap();
        let mut notices = Vec::new();
        let jobs = load(&path, &mut notices);
        assert!(notices.is_empty(), "{notices:?}");
        assert_eq!(jobs[0].options.video_preset, VideoPreset::Original);
        assert_eq!(jobs[0].options.max_height, Some(4320));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_next_schema_is_set_aside() {
        let dir = temp("queue-schema-next");
        let path = dir.join("queue.json");
        std::fs::write(&path, format!(r#"{{"schemaVersion":{},"revision":1,"jobs":[]}}"#, QUEUE_SCHEMA + 1)).unwrap();
        let mut notices = Vec::new();
        assert!(load(&path, &mut notices).is_empty());
        assert_eq!(notices.len(), 1);
        assert!(crate::download_tests::names(&dir).iter().any(|n| n.starts_with("queue.newer-version-")));
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

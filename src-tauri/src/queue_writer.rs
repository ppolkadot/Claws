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

const RECOVERY_JOURNAL: &str = "queue.recovery.json";
const RECOVERY_JOURNAL_SCHEMA: u16 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ConsumedBackup {
    file_name: String,
    sha256: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct PendingRecovery {
    backup: ConsumedBackup,
    restored_jobs: bool,
    snapshot: QueueSnapshot,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct RecoveryJournal {
    schema_version: u16,
    #[serde(default)]
    consumed: Vec<ConsumedBackup>,
    #[serde(default)]
    pending: Option<PendingRecovery>,
}

impl Default for RecoveryJournal {
    fn default() -> Self {
        Self {
            schema_version: RECOVERY_JOURNAL_SCHEMA,
            consumed: Vec::new(),
            pending: None,
        }
    }
}

fn recovery_journal_path(path: &Path) -> PathBuf {
    path.with_file_name(RECOVERY_JOURNAL)
}

fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(bytes);
    digest.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(sha256_bytes(&bytes))
}

fn remove_retrying(path: &Path) -> std::io::Result<()> {
    let mut attempt = 0;
    loop {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e)
                if (e.kind() == std::io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(5 | 32)))
                    && attempt < 6 =>
            {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            result => return result,
        }
    }
}

/// Marks a recovered backup as consumed. An earlier consumed backup of the same name is
/// never replaced: the new one takes a unique name instead.
fn consume_backup(backup: &Path) -> std::io::Result<()> {
    let mut target = backup.with_extension("recovered");
    if target.exists() {
        target = backup.with_extension(format!("{}.recovered", uuid::Uuid::new_v4()));
    }
    crate::store::rename_retrying(backup, &target)
}

fn finalize_recovery_journal(path: &Path, notices: &mut Vec<String>) -> crate::error::Result<()> {
    let journal_path = recovery_journal_path(path);
    let bytes = match std::fs::read(&journal_path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(crate::error::AppError::msg(format!(
                "Claws could not read its queue recovery journal ({e}). Startup stopped to preserve your queue."
            )));
        }
    };
    let mut journal: RecoveryJournal = match serde_json::from_slice::<RecoveryJournal>(&bytes) {
        Ok(journal) if journal.schema_version == RECOVERY_JOURNAL_SCHEMA => journal,
        _ => {
            return Err(crate::error::AppError::msg(
                "Claws could not verify its queue recovery journal. Startup stopped to preserve your queue.",
            ));
        }
    };
    if let Some(pending) = journal.pending.take() {
        // What is at the queue path decides whether the recovery was already published.
        // Only a schema 3 queue holding every recovered job counts as that; anything else
        // gets the missing jobs added rather than being treated as the recovery itself.
        enum OnDisk {
            Missing,
            /// An older-format queue: replaced by the recovery after its bytes are kept.
            Older { backup_name: &'static str, bytes: Vec<u8> },
            Current(QueueSnapshot),
            /// Written by a newer Claws: never rewritten here.
            Newer,
            Damaged,
        }
        let on_disk = match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Ok(value) if value.is_array() => match serde_json::from_value::<Vec<Job>>(value) {
                    Ok(_) => OnDisk::Older { backup_name: "queue.pre-1.0.5.json", bytes },
                    Err(_) => OnDisk::Damaged,
                },
                Ok(value) => match value.get("schemaVersion").and_then(|v| v.as_u64()) {
                    Some(version) if version > QUEUE_SCHEMA as u64 => OnDisk::Newer,
                    _ => match serde_json::from_value::<QueueSnapshot>(value) {
                        Ok(snapshot) if snapshot.schema_version < QUEUE_SCHEMA => {
                            OnDisk::Older { backup_name: "queue.pre-1.1.0.json", bytes }
                        }
                        Ok(snapshot) => OnDisk::Current(snapshot),
                        Err(_) => OnDisk::Damaged,
                    },
                },
                Err(_) => OnDisk::Damaged,
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => OnDisk::Missing,
            Err(e) => {
                return Err(crate::error::AppError::msg(format!(
                    "Claws could not read your queue while finishing recovery ({e}). Startup stopped to preserve it."
                )));
            }
        };
        let publish = match on_disk {
            OnDisk::Missing => Some(pending.snapshot.clone()),
            OnDisk::Older { backup_name, bytes } => {
                back_up_older_queue(path, backup_name, &bytes).map_err(|error| {
                    crate::error::AppError::msg(format!("Claws could not back up the older queue ({error}); startup stopped to preserve it."))
                })?;
                Some(pending.snapshot.clone())
            }
            OnDisk::Damaged => {
                // The same treatment load() gives a damaged queue: keep the bytes, then recover.
                let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
                let aside = path.with_file_name(format!("queue.corrupt-{stamp}-{}.json", uuid::Uuid::new_v4()));
                crate::store::rename_retrying(path, &aside)?;
                notices.push(format!("Your download queue was damaged. The old file was kept as {}.", file_name(&aside)));
                Some(pending.snapshot.clone())
            }
            OnDisk::Newer => {
                // load() sets this queue aside next; the recovery stays pending until then.
                journal.pending = Some(pending);
                return Ok(());
            }
            OnDisk::Current(mut on_disk) => {
                let present: std::collections::HashSet<_> = on_disk.jobs.iter().map(|job| job.id.clone()).collect();
                let mut missing: Vec<Job> = pending.snapshot.jobs.iter().filter(|job| !present.contains(&job.id)).cloned().collect();
                if missing.is_empty() {
                    None
                } else {
                    missing.append(&mut on_disk.jobs);
                    on_disk.jobs = missing;
                    on_disk.revision = on_disk.revision.max(pending.snapshot.revision).saturating_add(1);
                    Some(on_disk)
                }
            }
        };
        if let Some(snapshot) = publish {
            crate::store::save_snapshot(path, || snapshot.clone())?;
            if pending.restored_jobs {
                notices.push("Restored the download queue preserved by an older Claws, including jobs added while using that version.".into());
            }
        }
        if !journal.consumed.contains(&pending.backup) {
            journal.consumed.push(pending.backup);
        }
        crate::store::save_snapshot(&journal_path, || journal.clone())?;
    }
    if let Some(parent) = path.parent() {
        let mut remaining = Vec::new();
        for entry in journal.consumed.drain(..) {
            let backup_path = parent.join(&entry.file_name);
            match sha256_file(&backup_path) {
                Ok(hash) if hash == entry.sha256 => {
                    if let Err(error) = consume_backup(&backup_path) {
                        notices.push(format!("The queue was restored, but its backup could not be marked recovered ({error})."));
                        remaining.push(entry);
                    }
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    notices.push(format!("The queue was restored, but its backup could not be marked recovered ({error})."));
                    remaining.push(entry);
                }
            }
        }
        journal.consumed = remaining;
    }
    if journal.consumed.is_empty() && journal.pending.is_none() {
        remove_retrying(&journal_path).map_err(|e| {
            crate::error::AppError::msg(format!(
                "Claws could not finalize its queue recovery journal ({e}). Startup stopped to preserve your queue."
            ))
        })?;
    } else {
        crate::store::save_snapshot(&journal_path, || journal.clone())?;
    }
    Ok(())
}

/// Keeps the exact bytes of an older-format queue before the new format replaces them. A
/// backup already there with other content is neither overwritten nor mistaken for this
/// copy: the bytes go to a uniquely named file beside it instead.
fn back_up_older_queue(path: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let backup = path.with_file_name(name);
    let target = match std::fs::read(&backup) {
        Ok(existing) if existing == bytes => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => backup,
        // Other content, or content that cannot be read: keep these bytes under a unique name.
        Ok(_) | Err(_) => {
            let stem = name.strip_suffix(".json").unwrap_or(name);
            // An earlier start may already have kept these very bytes.
            let prefix = format!("{stem}-");
            let kept = path.parent().and_then(|dir| std::fs::read_dir(dir).ok()).is_some_and(|entries| {
                entries.flatten().any(|entry| {
                    let file = entry.file_name();
                    let file = file.to_string_lossy();
                    file.starts_with(&prefix) && file.ends_with(".json") && std::fs::read(entry.path()).is_ok_and(|b| b == bytes)
                })
            });
            if kept {
                return Ok(());
            }
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
            path.with_file_name(format!("{stem}-{stamp}-{}.json", uuid::Uuid::new_v4()))
        }
    };
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&target)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Reads the queue, accepting the 1.0.4 format. Damaged files and files from a newer
/// version are moved aside untouched, with a notice.
pub fn load(path: &Path, notices: &mut Vec<String>) -> crate::error::Result<Vec<Job>> {
    finalize_recovery_journal(path, notices)?;
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(recover_downgrade(path, Vec::new(), notices)),
        Err(e) => {
            return Err(crate::error::AppError::msg(format!("Claws could not read your queue ({e}). Startup stopped to preserve it.")));
        }
    };
    let set_aside = |label: &str| -> std::io::Result<PathBuf> {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let aside = path.with_file_name(format!("queue.{label}-{stamp}-{}.json", uuid::Uuid::new_v4()));
        crate::store::rename_retrying(path, &aside)?;
        Ok(aside)
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => {
            let aside = set_aside("corrupt")?;
            notices.push(format!("Your download queue was damaged, so Claws started with an empty one. The old file was kept as {}.", file_name(&aside)));
            return Ok(recover_downgrade(path, Vec::new(), notices));
        }
    };
    if value.is_array() {
        return match serde_json::from_value::<Vec<Job>>(value) {
            Ok(jobs) => {
                // 1.0.4 or earlier: backed up once before the new format replaces it.
                if let Err(error) = back_up_older_queue(path, "queue.pre-1.0.5.json", text.as_bytes()) {
                    return Err(crate::error::AppError::msg(format!(
                        "Claws could not back up the older queue ({error}); startup stopped to preserve it."
                    )));
                }
                Ok(recover_downgrade(path, jobs, notices))
            }
            Err(_) => {
                let aside = set_aside("corrupt")?;
                notices.push(format!(
                    "Your download queue from an earlier version could not be read, so Claws started with an empty one. The old file was kept as {}.",
                    file_name(&aside)
                ));
                Ok(recover_downgrade(path, Vec::new(), notices))
            }
        };
    }
    let version = value.get("schemaVersion").and_then(|v| v.as_u64()).unwrap_or(0);
    if version > QUEUE_SCHEMA as u64 {
        let aside = set_aside("newer-version")?;
        notices.push(format!(
            "Your download queue was saved by a newer version of Claws and was not changed. It was kept as {}; open that version again to continue those downloads.",
            file_name(&aside)
        ));
        return Ok(Vec::new());
    }
    Ok(match serde_json::from_value::<QueueSnapshot>(value) {
        Ok(snapshot) => {
            if snapshot.schema_version < QUEUE_SCHEMA {
                // Keep the exact old representation before a new version writes it.
                if let Err(error) = back_up_older_queue(path, "queue.pre-1.1.0.json", text.as_bytes()) {
                    return Err(crate::error::AppError::msg(format!("Claws could not back up the older queue ({error}); startup stopped to preserve it.")));
                }
                recover_downgrade(path, snapshot.jobs, notices)
            } else { snapshot.jobs }
        }
        Err(_) => {
            let aside = set_aside("corrupt")?;
            notices.push(format!("Your download queue was damaged, so Claws started with an empty one. The old file was kept as {}.", file_name(&aside)));
            recover_downgrade(path, Vec::new(), notices)
        }
    })
}

/// Older Claws sets a newer queue aside. Restore it when returning to this version,
/// retaining jobs created during the downgrade. Never read an unsupported schema.
fn recover_downgrade(path: &Path, current: Vec<Job>, notices: &mut Vec<String>) -> Vec<Job> {
    let Some(parent) = path.parent() else { return current };
    let journal_path = recovery_journal_path(path);
    let mut journal = match std::fs::read(&journal_path) {
        Ok(bytes) => match serde_json::from_slice::<RecoveryJournal>(&bytes) {
            Ok(j) if j.schema_version == RECOVERY_JOURNAL_SCHEMA && j.pending.is_none() => j,
            _ => {
                notices.push("The saved newer-version queue could not be restored because its recovery journal is damaged; backups were kept.".into());
                return current;
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => RecoveryJournal::default(),
        Err(error) => {
            notices.push(format!("The saved newer-version queue could not be restored ({error}); its backup was kept."));
            return current;
        }
    };
    let Ok(entries) = std::fs::read_dir(parent) else { return current };
    let mut matching = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("queue.newer-version-") || !name.ends_with(".json") {
            continue;
        }
        let candidate_path = entry.path();
        if journal.consumed.iter().any(|c| {
            c.file_name == name && sha256_file(&candidate_path).is_ok_and(|hash| hash == c.sha256)
        }) {
            continue;
        }
        matching.push(candidate_path);
    }
    matching.sort();
    if matching.is_empty() {
        return current;
    }
    if matching.len() != 1 {
        notices.push("Multiple saved newer-version queues were found. They were preserved; restore the intended one manually to avoid reviving obsolete downloads.".into());
        return current;
    }
    let backup = matching.into_iter().next().unwrap();
    let bytes = match std::fs::read(&backup) {
        Ok(bytes) => bytes,
        Err(error) => {
            notices.push(format!("The saved newer-version queue could not be read ({error}); its backup was kept."));
            return current;
        }
    };
    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => {
            notices.push("The saved newer-version queue was damaged and could not be restored; its backup was kept.".into());
            return current;
        }
    };
    if value.get("schemaVersion").and_then(|v| v.as_u64()) != Some(QUEUE_SCHEMA as u64) {
        notices.push("The saved newer-version queue uses an unsupported format and was kept untouched.".into());
        return current;
    }
    let mut snapshot: QueueSnapshot = match serde_json::from_value::<QueueSnapshot>(value) {
        Ok(snapshot) if snapshot.schema_version == QUEUE_SCHEMA => snapshot,
        _ => {
            notices.push("The saved newer-version queue was damaged and could not be restored; its backup was kept.".into());
            return current;
        }
    };
    let had_restored_jobs = !snapshot.jobs.is_empty();
    let backup_record = ConsumedBackup {
        file_name: file_name(&backup),
        sha256: sha256_bytes(&bytes),
    };
    let ids: std::collections::HashSet<_> = snapshot.jobs.iter().map(|job| job.id.clone()).collect();
    snapshot.jobs.extend(current.iter().filter(|job| !ids.contains(&job.id)).cloned());
    snapshot.revision = snapshot.revision.saturating_add(1);

    journal.pending = Some(PendingRecovery {
        backup: backup_record.clone(),
        restored_jobs: had_restored_jobs,
        snapshot: snapshot.clone(),
    });
    if let Err(error) = crate::store::save_snapshot(&journal_path, || journal.clone()) {
        notices.push(format!("The saved newer-version queue could not be restored ({error}); its backup was kept."));
        return current;
    }
    if let Err(error) = crate::store::save_snapshot(path, || snapshot.clone()) {
        journal.pending = None;
        if journal.consumed.is_empty() {
            let _ = remove_retrying(&journal_path);
        } else {
            let _ = crate::store::save_snapshot(&journal_path, || journal.clone());
        }
        notices.push(format!("The saved newer-version queue could not be restored ({error}); its backup was kept."));
        return current;
    }
    journal.pending = None;
    if !journal.consumed.contains(&backup_record) {
        journal.consumed.push(backup_record.clone());
    }
    let _ = crate::store::save_snapshot(&journal_path, || journal.clone());

    // Consume only after the restored queue is durable, preventing resurrection later.
    match consume_backup(&backup) {
        Ok(()) => {
            journal.consumed.retain(|c| c != &backup_record);
            if journal.consumed.is_empty() {
                let _ = remove_retrying(&journal_path);
            } else {
                let _ = crate::store::save_snapshot(&journal_path, || journal.clone());
            }
        }
        Err(error) => {
            notices.push(format!("The queue was restored, but its backup could not be marked recovered ({error})."));
        }
    }
    if had_restored_jobs {
        notices.push("Restored the download queue preserved by an older Claws, including jobs added while using that version.".into());
    }
    snapshot.jobs
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn load(path: &Path, notices: &mut Vec<String>) -> Vec<Job> {
        super::load(path, notices).expect("queue load")
    }

    #[test]
    fn an_unreadable_queue_stops_instead_of_becoming_an_empty_writable_queue() {
        let dir = temp("queue-unreadable");
        let path = dir.join("queue.json");
        std::fs::create_dir(&path).unwrap();
        assert!(super::load(&path, &mut Vec::new()).is_err());
        assert!(path.is_dir());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_failed_set_aside_stops_without_overwriting_the_original() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = temp("queue-locked");
        let path = dir.join("queue.json");
        std::fs::write(&path, b"damaged but irreplaceable").unwrap();
        // Allow reads, but withhold FILE_SHARE_DELETE so the move cannot succeed.
        let held = std::fs::OpenOptions::new().read(true).share_mode(1).open(&path).unwrap();
        assert!(super::load(&path, &mut Vec::new()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"damaged but irreplaceable");
        drop(held);
        let _ = std::fs::remove_dir_all(dir);
    }

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
    fn returning_after_a_downgrade_restores_jobs_once_and_keeps_older_version_additions() {
        let dir = temp("downgrade-recovery");
        let path = dir.join("queue.json");
        let old_job = schema_2_job("old", "paused", "null");
        let new_job = schema_2_job("new", "paused", "null");
        std::fs::write(&path, format!(r#"{{"schemaVersion":2,"revision":2,"jobs":[{new_job}]}}"#)).unwrap();
        let backup = dir.join("queue.newer-version-20261108-120000.json");
        std::fs::write(&backup, format!(r#"{{"schemaVersion":3,"revision":4,"jobs":[{old_job}]}}"#)).unwrap();
        let mut notices = Vec::new();
        let restored = load(&path, &mut notices);
        assert_eq!(restored.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), ["old", "new"]);
        assert!(dir.join("queue.pre-1.1.0.json").exists());
        assert!(!backup.exists());
        // Clearing the current queue must not resurrect a consumed backup.
        std::fs::write(&path, r#"{"schemaVersion":3,"revision":9,"jobs":[]}"#).unwrap();
        assert!(load(&path, &mut notices).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn recovery_without_a_current_queue_preserves_original_options() {
        let dir = temp("downgrade-missing");
        let path = dir.join("queue.json");
        let mut job: Job = serde_json::from_str(&schema_2_job("original", "paused", "null")).unwrap();
        job.options.video_preset = crate::download_policy::VideoPreset::Original;
        let backup = dir.join("queue.newer-version-saved.json");
        let snapshot = QueueSnapshot { schema_version: QUEUE_SCHEMA, revision: 4, jobs: vec![job] };
        std::fs::write(&backup, serde_json::to_vec(&snapshot).unwrap()).unwrap();
        let restored = load(&path, &mut Vec::new());
        assert_eq!(restored[0].options.video_preset, crate::download_policy::VideoPreset::Original);
        assert!(path.exists());
        assert!(!backup.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ambiguous_or_unsupported_recovery_backups_are_never_consumed() {
        let dir = temp("downgrade-ambiguous");
        let path = dir.join("queue.json");
        let job = schema_2_job("saved", "paused", "null");
        let a = dir.join("queue.newer-version-a.json");
        let b = dir.join("queue.newer-version-b.json");
        std::fs::write(&a, format!(r#"{{"schemaVersion":3,"revision":1,"jobs":[{job}]}}"#)).unwrap();
        std::fs::write(&b, std::fs::read(&a).unwrap()).unwrap();
        let mut notices = Vec::new();
        assert!(load(&path, &mut notices).is_empty());
        assert!(notices.iter().any(|n| n.contains("Multiple")));
        assert!(a.exists() && b.exists());
        std::fs::remove_file(&b).unwrap();
        std::fs::write(&a, format!(r#"{{"schemaVersion":99,"revision":1,"jobs":[{job}]}}"#)).unwrap();
        assert!(load(&path, &mut Vec::new()).is_empty());
        assert!(a.exists());
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_recovery_save_keeps_both_queues() {
        let dir = temp("downgrade-save-failed");
        let path = dir.join("queue.json");
        std::fs::create_dir(&path).unwrap(); // an unwritable publication target
        let backup = dir.join("queue.newer-version-saved.json");
        let job = schema_2_job("preserved", "paused", "null");
        let text = format!(r#"{{"schemaVersion":3,"revision":1,"jobs":[{job}]}}"#);
        std::fs::write(&backup, &text).unwrap();
        let current = serde_json::from_str(&schema_2_job("current", "paused", "null")).unwrap();
        let mut notices = Vec::new();
        let kept = recover_downgrade(&path, vec![current], &mut notices);
        assert_eq!(kept[0].id, "current");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), text);
        assert!(notices.iter().any(|n| n.contains("could not be restored")));
        let _ = std::fs::remove_dir_all(dir);
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

    #[test]
    fn corrupt_legacy_array_is_set_aside_even_when_pre_1_0_5_backup_exists() {
        let dir = temp("legacy-array-corrupt");
        let path = dir.join("queue.json");
        let pre = dir.join("queue.pre-1.0.5.json");
        std::fs::write(&pre, b"earlier valid backup").unwrap();
        std::fs::write(&path, br#"[{"not_a_valid_job": true}]"#).unwrap();
        let mut notices = Vec::new();
        assert!(load(&path, &mut notices).is_empty());
        assert!(!path.exists(), "corrupt array queue must be moved aside");
        assert_eq!(std::fs::read(&pre).unwrap(), b"earlier valid backup");
        let corrupt: Vec<_> = crate::download_tests::names(&dir)
            .into_iter()
            .filter(|n| n.starts_with("queue.corrupt-"))
            .collect();
        assert_eq!(corrupt.len(), 1);
        assert_eq!(std::fs::read(dir.join(&corrupt[0])).unwrap(), br#"[{"not_a_valid_job": true}]"#);
        assert!(notices.iter().any(|n| n.contains(&corrupt[0])), "{notices:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn legacy_array_queue_also_recovers_preserved_newer_version_backup() {
        let dir = temp("legacy-array-downgrade");
        let path = dir.join("queue.json");
        let old_job = schema_2_job("from-schema-3", "paused", "null");
        let array_job = schema_2_job("from-1-0-4", "paused", "null");
        let raw_array = format!("[{array_job}]");
        std::fs::write(&path, &raw_array).unwrap();
        let backup = dir.join("queue.newer-version-20261108-120000.json");
        std::fs::write(&backup, format!(r#"{{"schemaVersion":3,"revision":4,"jobs":[{old_job}]}}"#)).unwrap();
        let mut notices = Vec::new();
        let restored = load(&path, &mut notices);
        assert_eq!(
            restored.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(),
            ["from-schema-3", "from-1-0-4"]
        );
        assert_eq!(std::fs::read_to_string(dir.join("queue.pre-1.0.5.json")).unwrap(), raw_array);
        assert!(!backup.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn empty_newer_backup_does_not_make_an_old_nonempty_queue_authoritative() {
        let dir = temp("recovery-empty-candidate");
        let path = dir.join("queue.json");
        std::fs::write(&path, r#"{"schemaVersion":2,"revision":1,"jobs":[]}"#).unwrap();
        let old = dir.join("queue.newer-version-20260101.json");
        let empty = dir.join("queue.newer-version-20261108.json");
        let job = schema_2_job("obsolete", "paused", "null");
        std::fs::write(&old, format!(r#"{{"schemaVersion":3,"revision":1,"jobs":[{job}]}}"#)).unwrap();
        std::fs::write(&empty, r#"{"schemaVersion":3,"revision":2,"jobs":[]}"#).unwrap();
        let mut notices = Vec::new();
        assert!(load(&path, &mut notices).is_empty());
        assert!(old.exists() && empty.exists());
        assert!(!notices.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn single_empty_newer_backup_is_consumed_while_keeping_downgrade_additions() {
        let dir = temp("recovery-single-empty");
        let path = dir.join("queue.json");
        let added = schema_2_job("added-while-downgraded", "paused", "null");
        std::fs::write(&path, format!(r#"{{"schemaVersion":2,"revision":1,"jobs":[{added}]}}"#)).unwrap();
        let empty = dir.join("queue.newer-version-20261108.json");
        std::fs::write(&empty, r#"{"schemaVersion":3,"revision":5,"jobs":[]}"#).unwrap();
        let mut notices = Vec::new();
        let jobs = load(&path, &mut notices);
        assert_eq!(jobs.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), ["added-while-downgraded"]);
        assert!(!empty.exists(), "empty backup should be marked recovered");
        assert!(empty.with_extension("recovered").exists());
        assert!(notices.is_empty(), "{notices:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unreadable_or_corrupt_newer_backup_is_never_silently_ignored() {
        let dir = temp("recovery-corrupt-candidate");
        let path = dir.join("queue.json");
        std::fs::write(&path, r#"{"schemaVersion":2,"revision":1,"jobs":[]}"#).unwrap();
        let bad = dir.join("queue.newer-version-20261108.json");
        std::fs::write(&bad, "{ malformed json").unwrap();
        let mut notices = Vec::new();
        assert!(load(&path, &mut notices).is_empty());
        assert!(bad.exists(), "corrupt backup must be preserved");
        assert!(!notices.is_empty(), "corrupt backup must surface a notice");

        // Adding a second valid backup alongside the corrupt one must not make the valid one authoritative.
        let old = dir.join("queue.newer-version-20260101.json");
        let job = schema_2_job("obsolete", "paused", "null");
        std::fs::write(&old, format!(r#"{{"schemaVersion":3,"revision":1,"jobs":[{job}]}}"#)).unwrap();
        let mut notices = Vec::new();
        assert!(load(&path, &mut notices).is_empty());
        assert!(bad.exists() && old.exists());
        assert!(notices.iter().any(|n| n.contains("Multiple")), "{notices:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn transient_and_persistent_locks_on_newer_backup_preserve_once_only_recovery() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = temp("recovery-locked-backup");
        let path = dir.join("queue.json");
        let old_job = schema_2_job("old", "paused", "null");
        std::fs::write(&path, r#"{"schemaVersion":2,"revision":1,"jobs":[]}"#).unwrap();
        let backup = dir.join("queue.newer-version-20261108-120000.json");
        std::fs::write(&backup, format!(r#"{{"schemaVersion":3,"revision":4,"jobs":[{old_job}]}}"#)).unwrap();

        // 1. Transient sharing lock on the backup clears within the retry window.
        let transient = std::fs::OpenOptions::new().read(true).share_mode(1).open(&backup).unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            drop(transient);
        });
        let mut notices = Vec::new();
        let restored = load(&path, &mut notices);
        releaser.join().unwrap();
        assert_eq!(restored.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), ["old"]);
        assert!(!backup.exists(), "transient lock should still consume the backup");
        assert!(!notices.iter().any(|n| n.contains("could not be marked recovered")), "{notices:?}");

        // 2. Persistent sharing lock on a backup prevents immediate rename, but durable
        // consumption evidence prevents resurrection after clear + another downgrade.
        std::fs::write(&path, r#"{"schemaVersion":2,"revision":2,"jobs":[]}"#).unwrap();
        let backup2 = dir.join("queue.newer-version-20261108-130000.json");
        std::fs::write(&backup2, format!(r#"{{"schemaVersion":3,"revision":5,"jobs":[{old_job}]}}"#)).unwrap();
        let persistent = std::fs::OpenOptions::new().read(true).share_mode(1).open(&backup2).unwrap();
        let mut notices = Vec::new();
        let restored = load(&path, &mut notices);
        assert_eq!(restored.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), ["old"]);
        assert!(backup2.exists(), "locked backup cannot be renamed yet");
        assert!(notices.iter().any(|n| n.contains("could not be marked recovered")), "{notices:?}");

        // User clears the queue and downgrades again while the first backup was never physically renamed.
        let backup3 = dir.join("queue.newer-version-20261108-140000.json");
        std::fs::write(&backup3, r#"{"schemaVersion":3,"revision":6,"jobs":[]}"#).unwrap();
        std::fs::write(&path, r#"{"schemaVersion":2,"revision":3,"jobs":[]}"#).unwrap();
        drop(persistent);

        let mut notices = Vec::new();
        let after_second_downgrade = load(&path, &mut notices);
        assert!(after_second_downgrade.is_empty(), "cleared job must not be resurrected: {after_second_downgrade:?}");
        assert!(!backup2.exists(), "previously consumed backup is finalized once unlocked");
        assert!(!backup3.exists(), "second downgrade backup is consumed");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn crash_after_save_before_consume_finalizes_on_next_start_and_prevents_resurrection() {
        let dir = temp("recovery-crash-journal");
        let path = dir.join("queue.json");
        let journal_path = recovery_journal_path(&path);
        let backup = dir.join("queue.newer-version-20261108-120000.json");
        let old_job: Job = serde_json::from_str(&schema_2_job("old", "paused", "null")).unwrap();
        let snapshot = QueueSnapshot {
            schema_version: QUEUE_SCHEMA,
            revision: 5,
            jobs: vec![old_job],
        };
        let backup_bytes = serde_json::to_vec(&snapshot).unwrap();
        std::fs::write(&backup, &backup_bytes).unwrap();

        // Simulate a crash after queue.json was published at schema 3, before backup was renamed.
        std::fs::write(&path, &backup_bytes).unwrap();
        let journal = RecoveryJournal {
            schema_version: RECOVERY_JOURNAL_SCHEMA,
            consumed: Vec::new(),
            pending: Some(PendingRecovery {
                backup: ConsumedBackup {
                    file_name: file_name(&backup),
                    sha256: sha256_bytes(&backup_bytes),
                },
                restored_jobs: true,
                snapshot: snapshot.clone(),
            }),
        };
        std::fs::write(&journal_path, serde_json::to_vec(&journal).unwrap()).unwrap();

        // Next startup finalizes the consume even though queue.json is already schema 3.
        let mut notices = Vec::new();
        let loaded = load(&path, &mut notices);
        assert_eq!(loaded.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), ["old"]);
        assert!(!backup.exists(), "backup must be consumed on startup");
        assert!(backup.with_extension("recovered").exists());
        assert!(!journal_path.exists(), "journal is removed once finalized");

        // Clearing and downgrading again must not resurrect the consumed backup.
        std::fs::write(&path, r#"{"schemaVersion":2,"revision":9,"jobs":[]}"#).unwrap();
        assert!(load(&path, &mut notices).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A pending journal for `old` whose backup is on disk, plus the queue file contents.
    fn pending_journal_for(dir: &Path, old_id: &str, queue_text: Option<&str>) -> (PathBuf, PathBuf) {
        let path = dir.join("queue.json");
        let backup = dir.join("queue.newer-version-20261108-120000.json");
        let old_job: Job = serde_json::from_str(&schema_2_job(old_id, "paused", "null")).unwrap();
        let snapshot = QueueSnapshot { schema_version: QUEUE_SCHEMA, revision: 5, jobs: vec![old_job] };
        let backup_bytes = serde_json::to_vec(&snapshot).unwrap();
        std::fs::write(&backup, &backup_bytes).unwrap();
        if let Some(text) = queue_text {
            std::fs::write(&path, text).unwrap();
        }
        let journal = RecoveryJournal {
            schema_version: RECOVERY_JOURNAL_SCHEMA,
            consumed: Vec::new(),
            pending: Some(PendingRecovery {
                backup: ConsumedBackup { file_name: file_name(&backup), sha256: sha256_bytes(&backup_bytes) },
                restored_jobs: true,
                snapshot,
            }),
        };
        std::fs::write(recovery_journal_path(&path), serde_json::to_vec(&journal).unwrap()).unwrap();
        (path, backup)
    }

    #[test]
    fn a_pending_recovery_is_not_treated_as_published_by_an_unrelated_schema_3_queue() {
        // The recovery's own publication failed, the journal could not be cleared, the app ran
        // on and saved its in-memory queue at schema 3. The backup's jobs must still survive.
        let dir = temp("recovery-pending-stale-publication");
        let current = schema_2_job("current", "paused", "null");
        let text = format!(r#"{{"schemaVersion":3,"revision":2,"jobs":[{current}]}}"#);
        let (path, backup) = pending_journal_for(&dir, "old", Some(&text));
        let jobs = load(&path, &mut Vec::new());
        let mut ids: Vec<_> = jobs.iter().map(|j| j.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, ["current", "old"]);
        assert!(!backup.exists() && backup.with_extension("recovered").exists());
        assert!(!recovery_journal_path(&path).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_pending_recovery_is_published_when_the_crash_came_before_the_queue_was_replaced() {
        let dir = temp("recovery-pending-unpublished");
        let current = schema_2_job("current", "paused", "null");
        let text = format!(r#"{{"schemaVersion":2,"revision":1,"jobs":[{current}]}}"#);
        let (path, backup) = pending_journal_for(&dir, "old", Some(&text));
        let jobs = load(&path, &mut Vec::new());
        assert_eq!(jobs.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), ["old"]);
        assert!(!backup.exists() && backup.with_extension("recovered").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_pending_recovery_beside_a_legacy_array_queue_is_published() {
        // load() backs the array up, then journals the recovery, then replaces the queue.
        // A crash before the replacement leaves exactly this state.
        let dir = temp("recovery-pending-array-queue");
        let current = schema_2_job("current", "paused", "null");
        let text = format!("[{current}]");
        let (path, backup) = pending_journal_for(&dir, "old", Some(&text));
        let jobs = super::load(&path, &mut Vec::new()).expect("a valid legacy queue must not stop startup");
        assert_eq!(jobs.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), ["old"]);
        assert_eq!(copies_of(&dir, text.as_bytes()).len(), 1, "the array bytes are kept");
        assert!(!backup.exists() && backup.with_extension("recovered").exists());
        assert!(!recovery_journal_path(&path).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_pending_recovery_beside_a_damaged_queue_sets_it_aside_and_publishes() {
        let dir = temp("recovery-pending-damaged-queue");
        let (path, backup) = pending_journal_for(&dir, "old", Some("{ not a queue"));
        let jobs = super::load(&path, &mut Vec::new()).expect("a damaged queue must not stop startup for good");
        assert_eq!(jobs.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), ["old"]);
        assert_eq!(copies_of(&dir, b"{ not a queue").len(), 1, "the damaged bytes are kept aside");
        assert!(!backup.exists() && backup.with_extension("recovered").exists());
        assert!(!recovery_journal_path(&path).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_pending_recovery_does_not_rewrite_a_queue_from_a_newer_schema() {
        let dir = temp("recovery-pending-newer-queue");
        let text = r#"{"schemaVersion":4,"revision":9,"jobs":[],"futureField":true}"#;
        let (path, backup) = pending_journal_for(&dir, "old", Some(text));
        let jobs = super::load(&path, &mut Vec::new()).unwrap();
        assert!(jobs.is_empty());
        assert_eq!(copies_of(&dir, text.as_bytes()).len(), 1, "the newer queue is kept byte for byte");
        assert!(backup.exists(), "the recovery is still pending");
        assert!(recovery_journal_path(&path).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_pending_recovery_keeps_the_older_queue_it_replaces() {
        let dir = temp("recovery-pending-older-queue");
        let current = schema_2_job("current", "paused", "null");
        let text = format!(r#"{{"schemaVersion":2,"revision":1,"jobs":[{current}]}}"#);
        let (path, _backup) = pending_journal_for(&dir, "old", Some(&text));
        let jobs = super::load(&path, &mut Vec::new()).unwrap();
        assert_eq!(jobs.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), ["old"]);
        assert_eq!(copies_of(&dir, text.as_bytes()).len(), 1, "the replaced schema 2 bytes are kept");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn consuming_a_backup_never_overwrites_an_earlier_recovered_file() {
        let dir = temp("recovery-consumed-name-taken");
        let path = dir.join("queue.json");
        let backup = dir.join("queue.newer-version-20261108-120000.json");
        let old_job: Job = serde_json::from_str(&schema_2_job("old", "paused", "null")).unwrap();
        let snapshot = QueueSnapshot { schema_version: QUEUE_SCHEMA, revision: 5, jobs: vec![old_job] };
        let backup_bytes = serde_json::to_vec(&snapshot).unwrap();
        std::fs::write(&backup, &backup_bytes).unwrap();
        std::fs::write(backup.with_extension("recovered"), b"an earlier consumed backup").unwrap();

        let jobs = load(&path, &mut Vec::new());
        assert_eq!(jobs.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), ["old"]);
        assert!(!backup.exists());
        assert_eq!(std::fs::read(backup.with_extension("recovered")).unwrap(), b"an earlier consumed backup");
        assert_eq!(copies_of(&dir, &backup_bytes).len(), 1, "the consumed backup is still on disk");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unreadable_older_migration_backup_does_not_stop_startup() {
        let dir = temp("migration-backup-unreadable");
        let path = dir.join("queue.json");
        let job = schema_2_job("a", "paused", "null");
        let schema_2 = format!(r#"{{"schemaVersion":2,"revision":1,"jobs":[{job}]}}"#);
        // A directory at the backup name cannot be read as a file.
        std::fs::create_dir(dir.join("queue.pre-1.1.0.json")).unwrap();
        std::fs::write(&path, &schema_2).unwrap();
        let jobs = super::load(&path, &mut Vec::new()).expect("an odd backup name must not stop startup");
        assert_eq!(jobs.len(), 1);
        assert_eq!(copies_of(&dir, schema_2.as_bytes()).len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Every file in `dir` other than `queue.json` whose content equals `bytes`.
    fn copies_of(dir: &Path, bytes: &[u8]) -> Vec<String> {
        crate::download_tests::names(dir)
            .into_iter()
            .filter(|n| n != "queue.json" && std::fs::read(dir.join(n)).is_ok_and(|b| b == bytes))
            .collect()
    }

    #[test]
    fn an_older_migration_backup_with_other_content_does_not_stand_in_for_the_current_bytes() {
        let dir = temp("migration-backup-differs");
        let path = dir.join("queue.json");
        let job = schema_2_job("a", "paused", "null");
        let schema_2 = format!(r#"{{"schemaVersion":2,"revision":1,"jobs":[{job}]}}"#);
        let array = format!("[{job}]");
        std::fs::write(dir.join("queue.pre-1.1.0.json"), b"an earlier, different state").unwrap();
        std::fs::write(dir.join("queue.pre-1.0.5.json"), b"another earlier state").unwrap();

        std::fs::write(&path, &schema_2).unwrap();
        assert_eq!(load(&path, &mut Vec::new()).len(), 1);
        assert_eq!(copies_of(&dir, schema_2.as_bytes()).len(), 1, "the exact schema 2 bytes are kept");
        assert_eq!(std::fs::read(dir.join("queue.pre-1.1.0.json")).unwrap(), b"an earlier, different state");

        std::fs::write(&path, &array).unwrap();
        assert_eq!(load(&path, &mut Vec::new()).len(), 1);
        assert_eq!(copies_of(&dir, array.as_bytes()).len(), 1, "the exact array bytes are kept");
        assert_eq!(std::fs::read(dir.join("queue.pre-1.0.5.json")).unwrap(), b"another earlier state");

        // Loading the same bytes again adds no further copies.
        assert_eq!(load(&path, &mut Vec::new()).len(), 1);
        assert_eq!(copies_of(&dir, array.as_bytes()).len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn damaged_recovery_journal_stops_startup_without_dropping_files() {
        let dir = temp("recovery-damaged-journal");
        let path = dir.join("queue.json");
        let journal_path = recovery_journal_path(&path);
        let backup = dir.join("queue.newer-version-20261108-120000.json");
        std::fs::write(&path, r#"{"schemaVersion":2,"revision":1,"jobs":[]}"#).unwrap();
        std::fs::write(&backup, r#"{"schemaVersion":3,"revision":2,"jobs":[]}"#).unwrap();
        std::fs::write(&journal_path, b"{ corrupted journal").unwrap();

        assert!(super::load(&path, &mut Vec::new()).is_err());
        assert!(journal_path.exists());
        assert!(backup.exists());
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}

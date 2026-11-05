use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{de::DeserializeOwned, Serialize};

use crate::error::Result;

/// Serialises every JSON write, so concurrent writers cannot race on a file or rename
/// an older snapshot over a newer one. Taken before any state mutex, never inside one.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

fn write_lock() -> MutexGuard<'static, ()> {
    WRITE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Missing file: the default. Unreadable or damaged: moved aside, reported in `notices`,
/// then the default.
pub fn load_or_recover<T: DeserializeOwned + Default>(path: &Path, what: &str, notices: &mut Vec<String>) -> T {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return T::default(),
        Err(e) => {
            notices.push(format!("Claws could not read your {what} ({e}), so it started with an empty one."));
            return T::default();
        }
    };
    match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => {
            let backup = backup_path(path);
            let kept = fs::rename(path, &backup).is_ok();
            notices.push(if kept {
                format!(
                    "Your {what} was damaged, so Claws started with an empty one. The old file was kept as {}.",
                    backup.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                )
            } else {
                format!("Your {what} was damaged, so Claws started with an empty one.")
            });
            T::default()
        }
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!("{stem}.corrupt-{stamp}.json"))
}

/// Temp file, flushed to the disk, then an atomic rename: a crash leaves either the old
/// file or the new one, never a half-written one. (A storage device that loses power can
/// still lose what it acknowledged; no test here can prove otherwise.) Callers must hold
/// the write lock.
fn write_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", uuid::Uuid::new_v4().simple()));
    let bytes = serde_json::to_vec_pretty(value)?;
    let result = fs::File::create(&tmp)
        .and_then(|mut file| {
            file.write_all(&bytes)?;
            file.sync_all()
        })
        .and_then(|_| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    Ok(result?)
}

/// Snapshots under the write lock, so a writer that snapshots later also writes later.
pub fn save_snapshot<T: Serialize>(path: &Path, snapshot: impl FnOnce() -> T) -> Result<()> {
    let _guard = write_lock();
    let value = snapshot();
    write_atomic(path, &value)
}

/// Writes `value`, then runs `commit` (usually updating memory) under the same lock.
pub fn save_then<T: Serialize>(path: &Path, value: &T, commit: impl FnOnce()) -> Result<()> {
    let _guard = write_lock();
    write_atomic(path, value)?;
    commit();
    Ok(())
}

/// Removes temp files left behind by a crash mid-write.
pub fn sweep_temp_files(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".tmp") && name.contains(".json") {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("claws-store-{tag}-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_is_silent_default() {
        let dir = temp_dir("missing");
        let mut notices = Vec::new();
        let value: Vec<u32> = load_or_recover(&dir.join("nope.json"), "library", &mut notices);
        assert!(value.is_empty());
        assert!(notices.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_file_is_kept_and_reported() {
        let dir = temp_dir("corrupt");
        let path = dir.join("history.json");
        fs::write(&path, "{ not json").unwrap();
        let mut notices = Vec::new();
        let value: Vec<u32> = load_or_recover(&path, "library", &mut notices);
        assert!(value.is_empty());
        assert_eq!(notices.len(), 1);
        assert!(!path.exists(), "the damaged file should be moved aside");
        let backups: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("history.corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn concurrent_saves_leave_valid_json() {
        let dir = temp_dir("concurrent");
        let path = dir.join("queue.json");
        let threads: Vec<_> = (0..16)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for j in 0..20 {
                        save_snapshot(&path, || vec![i * 100 + j; 50]).unwrap();
                    }
                })
            })
            .collect();
        threads.into_iter().for_each(|t| t.join().unwrap());
        let text = fs::read_to_string(&path).unwrap();
        let value: Vec<u32> = serde_json::from_str(&text).unwrap();
        assert_eq!(value.len(), 50);
        let leftovers = fs::read_dir(&dir).unwrap().flatten().count();
        assert_eq!(leftovers, 1, "no temp files should remain");
        let _ = fs::remove_dir_all(dir);
    }
}

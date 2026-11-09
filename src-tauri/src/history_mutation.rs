//! Every change to the Library goes through here, so memory and `history.json` cannot
//! disagree and a download that finishes while the Library is being edited is never lost.
//!
//! A change is built as a proposed list, saved, and only then committed to memory (and
//! announced); a failed save changes nothing, and artwork is deleted only after the save
//! worked. Mutations are serialized by one lock taken before anything else: it is never
//! taken while the history mutex is held, and the history mutex is only taken, briefly,
//! inside the store's write lock (store lock before history lock) or to copy the list.
//!
//! "Delete File Too" journals what it is about to delete before touching anything: its own
//! file per operation under `history-journal/`, with a schema, an operation id, a phase and
//! the identities that make each file Claws' to delete. The file is then deleted through a
//! verified handle (see `download_files::remove_verified`), the entry removed and saved, and
//! the journal retired. A crash anywhere in between is finished at the next start by
//! `reconcile`, which repeats the same verified, idempotent steps.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::download_files::{self, FileIdentity, MoveState, Removal, RemoveError};
use crate::error::{notify, AppError, Result};
use crate::history::{self, HistoryItem};
use crate::state::AppState;
use crate::store;
use crate::AppHandle;

/// Serializes Library mutations (and with them the completion that records a download).
static MUTATIONS: Mutex<()> = Mutex::new(());

/// The held mutation lock; passing `&Held` is the proof that it is.
pub(crate) struct Held {
    _guard: MutexGuard<'static, ()>,
}

#[cfg(test)]
thread_local! {
    /// How many `Held` this thread has right now, so a test can tell its own lock from another
    /// test's (the lock itself is shared by the whole test process).
    static HELD_HERE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
impl Drop for Held {
    fn drop(&mut self) {
        HELD_HERE.with(|n| n.set(n.get() - 1));
    }
}

fn mutations() -> Held {
    let guard = MUTATIONS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    #[cfg(test)]
    HELD_HERE.with(|n| n.set(n.get() + 1));
    Held { _guard: guard }
}

pub const JOURNAL_SCHEMA: u16 = 1;

pub const NO_EVIDENCE: &str = "Claws cannot confirm that this file is the one it downloaded, so it was not deleted from your disk. Use “Show in folder” to delete it yourself, or remove the entry from the Library only.";

pub fn journal_dir(state: &AppState) -> PathBuf {
    state.data_dir.join("history-journal")
}

/// Builds the proposed Library with `change`, saves it, then commits it to memory and
/// announces it. `change` returns its result and whether it changed anything (an unchanged
/// list is not saved). A failed save leaves memory exactly as it was.
pub(crate) fn mutate<R>(app: &AppHandle, change: impl FnOnce(&mut Vec<HistoryItem>) -> (R, bool)) -> Result<R> {
    let held = mutations();
    mutate_locked(app, &held, change)
}

/// `mutate` for a caller that already holds the mutation lock (`held` is the proof), so it
/// can check the Library and act on what it found without another change landing between.
fn mutate_locked<R>(app: &AppHandle, _held: &Held, change: impl FnOnce(&mut Vec<HistoryItem>) -> (R, bool)) -> Result<R> {
    let state = app.state::<AppState>();
    let mut proposed = state.history.lock().unwrap().clone();
    let (result, changed) = change(&mut proposed);
    if !changed {
        return Ok(result);
    }
    store::save_then(&state.history_path(), &proposed, || {
        *state.history.lock().unwrap() = proposed.clone();
    })?;
    let _ = app.emit("history-changed", ());
    Ok(result)
}

/// Adds (or replaces, by id) a Library entry.
pub fn add(app: &AppHandle, item: HistoryItem) -> Result<()> {
    mutate(app, |history| {
        history.retain(|h| h.id != item.id);
        history.insert(0, item);
        ((), true)
    })
}

/// Removes one entry from the Library; its file is not touched. A missing entry is fine.
pub fn remove_entry(app: &AppHandle, id: &str) -> Result<()> {
    let removed = {
        let held = mutations();
        withdraw(app, &held, id)?
    };
    finish_removal(app, id, removed);
    Ok(())
}

/// Takes the entry out of the Library (saved, then committed). The mutation lock is held.
fn withdraw(app: &AppHandle, held: &Held, id: &str) -> Result<Option<HistoryItem>> {
    mutate_locked(app, held, |history| {
        let found = history.iter().find(|h| h.id == id).cloned();
        history.retain(|h| h.id != id);
        let changed = found.is_some();
        (found, changed)
    })
}

/// Only once the removal is saved. A pending "Delete File Too" for the entry goes with it:
/// the file is kept, so it must not be deleted at the next start.
fn finish_removal(app: &AppHandle, id: &str, removed: Option<HistoryItem>) {
    retire(&journal_dir(&app.state::<AppState>()), &[id]);
    removed.iter().for_each(history::remove_thumbnail);
}

pub fn clear(app: &AppHandle) -> Result<()> {
    let removed = mutate(app, |history| {
        let all = std::mem::take(history);
        let changed = !all.is_empty();
        (all, changed)
    })?;
    let ids: Vec<&str> = removed.iter().map(|h| h.id.as_str()).collect();
    retire(&journal_dir(&app.state::<AppState>()), &ids);
    removed.iter().for_each(history::remove_thumbnail);
    Ok(())
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum Phase {
    /// Written before anything is deleted: the deletion may or may not have happened.
    Intent,
    /// The files are gone; only the entry's removal is left.
    FilesDeleted,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct Target {
    path: PathBuf,
    identity: FileIdentity,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct Journal {
    schema_version: u16,
    op_id: String,
    phase: Phase,
    entry_id: String,
    file: Target,
    #[serde(default)]
    sidecars: Vec<Target>,
}

fn journal_file(dir: &Path, op_id: &str) -> PathBuf {
    dir.join(format!("op-{op_id}.json"))
}

fn write_journal(path: &Path, journal: &Journal) -> Result<()> {
    store::save_snapshot(path, || journal)
}

/// Writes a journal under the mutation lock, which recovery also takes to sweep the
/// temporary files a crash left: the file being written is never taken for one of them.
fn write_journal_locked(path: &Path, journal: &Journal) -> Result<()> {
    let _held = mutations();
    write_journal(path, journal)
}

/// Deletes the media file, then the recorded side files, each only if it is still the
/// object whose identity was recorded. Blocking: it may hash large files.
///
/// Returns the side files that stayed behind because they could not be deleted (in use, no
/// permission), so the user can be told. One that is missing or was replaced is not
/// reported: it is gone, or it is the user's.
fn delete_targets(journal: &Journal) -> std::result::Result<Vec<PathBuf>, RemoveError> {
    remove_one(&journal.file)?;
    let mut left = Vec::new();
    for side in &journal.sidecars {
        if let Err(RemoveError::Io(_) | RemoveError::Unsupported | RemoveError::Unverifiable(_)) = remove_one(side) {
            left.push(side.path.clone());
        }
    }
    Ok(left)
}

fn left_behind(left: &[PathBuf]) -> Option<String> {
    let names: Vec<String> = left.iter().map(|p| p.display().to_string()).collect();
    (!names.is_empty()).then(|| format!("These side files Claws saved could not be deleted and were left in place: {}.", names.join(", ")))
}

/// Removes a journal file, trying again briefly: a virus scanner or indexer can hold a
/// freshly written file for a moment. Whether it is gone.
fn discard(file: &Path) -> bool {
    discard_pausing(file, || std::thread::sleep(std::time::Duration::from_millis(30)))
}

const DISCARD_ATTEMPTS: u32 = 10;

/// `discard` with the wait between attempts supplied, so a test can end a lock exactly
/// there instead of racing the clock.
fn discard_pausing(file: &Path, mut pause: impl FnMut()) -> bool {
    for attempt in 0..DISCARD_ATTEMPTS {
        match fs::remove_file(file) {
            Ok(()) => return true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) if attempt + 1 < DISCARD_ATTEMPTS => pause(),
            Err(_) => {}
        }
    }
    false
}

/// Notices raised before the window listens would be lost; they are held until it reports
/// ready and shown straight away after that.
#[derive(Default)]
struct Notes {
    ready: bool,
    held: Vec<String>,
}

impl Notes {
    /// The message to show now, or `None` if it was held for later.
    fn submit(&mut self, message: String) -> Option<String> {
        if self.ready {
            return Some(message);
        }
        self.held.push(message);
        None
    }

    fn release(&mut self) -> Vec<String> {
        self.ready = true;
        std::mem::take(&mut self.held)
    }
}

static NOTES: Mutex<Notes> = Mutex::new(Notes { ready: false, held: Vec::new() });

/// Reports a startup recovery problem: a toast once the window listens, always stderr.
fn report(app: &AppHandle, message: String) {
    let now = NOTES.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).submit(message.clone());
    match now {
        Some(message) => notify(app, message),
        None => eprintln!("{message}"),
    }
}

/// The window is listening: show what recovery found while it was starting.
pub fn release_notices(app: &AppHandle) {
    let held = NOTES.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).release();
    for message in held {
        notify(app, message);
    }
}

fn remove_one(target: &Target) -> std::result::Result<Removal, RemoveError> {
    download_files::remove_verified(&target.path, &target.identity)
}

fn refusal(error: RemoveError) -> AppError {
    match error {
        RemoveError::Replaced => AppError::msg("The file at that location is no longer the one Claws saved (it was moved or replaced), so it was not deleted and the entry is still in your library. Use “Show in folder”, or remove the entry from the Library only."),
        RemoveError::Unsupported => AppError::msg(format!("This system cannot confirm which file would be deleted, so nothing was deleted. {NO_EVIDENCE}")),
        RemoveError::Io(e) => AppError::msg(format!("The file could not be deleted ({e}), so it is still in your library. Close any app using it and try again.")),
        // Not "in use": the file opened, but Claws could not read what it needs to know it is the right one.
        RemoveError::Unverifiable(e) => AppError::msg(format!("Claws could not confirm that this file is the one it saved ({e}), so nothing was deleted and the entry is still in your library. Try again, or use “Show in folder” to delete it yourself.")),
    }
}

/// Removes an entry and, with `delete_file`, the file Claws saved for it. Blocking.
///
/// Automatic deletion needs proof: an entry without a recorded file identity (saved by an
/// older version) is refused with an explanation, and nothing on disk is touched. The
/// intent and the identities are journalled durably first; if that fails, nothing is
/// deleted. If the file cannot be deleted the entry stays. If the file goes but the Library
/// cannot be saved, the journal finishes the removal later.
pub fn delete_entry(app: &AppHandle, id: &str, delete_file: bool) -> Result<()> {
    if !delete_file {
        return remove_entry(app, id);
    }
    let state = app.state::<AppState>();
    let Some(item) = state.history.lock().unwrap().iter().find(|h| h.id == id).cloned() else {
        return Ok(()); // already removed
    };
    let Some(path) = item.file_path.clone() else {
        return remove_entry(app, id); // nothing was saved to delete
    };
    let Some(identity) = item.file_identity.clone() else {
        return Err(AppError::msg(NO_EVIDENCE));
    };
    let mut journal = Journal {
        schema_version: JOURNAL_SCHEMA,
        op_id: uuid::Uuid::new_v4().simple().to_string(),
        phase: Phase::Intent,
        entry_id: item.id.clone(),
        file: Target { path: PathBuf::from(path), identity },
        sidecars: item
            .published_sidecars
            .iter()
            .filter(|m| m.state == MoveState::Done)
            .map(|m| Target { path: m.destination.clone(), identity: m.identity.clone() })
            .collect(),
    };
    let dir = journal_dir(&state);
    let file = journal_file(&dir, &journal.op_id);
    write_journal_locked(&file, &journal).map_err(|e| AppError::msg(format!("Claws could not record the deletion ({e}), so nothing was deleted.")))?;
    let left = match delete_targets(&journal) {
        Ok(left) => left,
        Err(error) => {
            // The media file is untouched (side files are only deleted after it): nothing to
            // finish. If the record cannot be removed, say so: it would delete at the next start.
            if !discard(&file) {
                return Err(AppError::msg(format!("{} Claws also could not clear its record of the attempt ({}); remove the entry from the Library only if you want to keep the file.", refusal(error), file.display())));
            }
            return Err(refusal(error));
        }
    };
    journal.phase = Phase::FilesDeleted;
    let _ = write_journal_locked(&file, &journal); // best effort: the intent record already covers it
    match remove_entry(app, id) {
        Ok(()) => {
            if let Some(message) = left_behind(&left) {
                notify(app, message);
            }
            Ok(())
        }
        Err(e) => Err(AppError::msg(format!(
            "The file was deleted, but Claws could not update your library ({e}). The entry will go the next time Claws starts, or when you remove it again."
        ))),
    }
}

/// Removes the journals of entries that left the Library. Unreadable ones are left for
/// `reconcile`.
fn retire(dir: &Path, entry_ids: &[&str]) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let ours = fs::read(&path).ok().and_then(|bytes| serde_json::from_slice::<Journal>(&bytes).ok()).is_some_and(|j| entry_ids.contains(&j.entry_id.as_str()));
        if ours {
            discard(&path);
        }
    }
}

/// Finishes every unresolved deletion left by an earlier run, off the startup path. Each
/// step is the verified, idempotent one `delete_entry` takes: files already gone are
/// fine, a file that is no longer the recorded object is left alone, and a record this
/// version cannot read is kept untouched. Completion recording during startup cannot be
/// lost to it: both go through `mutate`.
pub fn reconcile(app: &AppHandle) -> std::thread::JoinHandle<()> {
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let dir = journal_dir(&state);
        let Ok(entries) = fs::read_dir(&dir) else { return };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        files.sort();
        for path in files {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if name.ends_with(".tmp") {
                // A crash while writing; the real record is intact. Under the lock a deletion
                // records its journal with, so none is being written now.
                let _held = mutations();
                let _ = fs::remove_file(&path);
                continue;
            }
            if !name.ends_with(".json") {
                continue;
            }
            let journal = match read_journal(&path) {
                Read::Vanished => continue,
                Read::Unreadable => {
                    report(&app, format!("Claws found an unfinished library deletion it cannot read ({name}). It was left untouched in {}.", dir.display()));
                    continue;
                }
                Read::Journal(journal) => journal,
            };
            resolve(&app, &path, &journal, &mut || {});
        }
    })
}

#[derive(Debug)]
enum Read {
    /// The file is gone: a deletion that finished (or was retired) after the folder was listed.
    Vanished,
    /// Not a journal this version can act on (damaged, or written by another version).
    Unreadable,
    Journal(Journal),
}

fn read_journal(path: &Path) -> Read {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Read::Vanished,
        Err(_) => return Read::Unreadable,
    };
    let journal = serde_json::from_slice::<serde_json::Value>(&bytes).ok().and_then(|value| {
        (value.get("schemaVersion")?.as_u64()? == u64::from(JOURNAL_SCHEMA)).then(|| serde_json::from_value::<Journal>(value).ok()).flatten()
    });
    journal.map_or(Read::Unreadable, Read::Journal)
}

/// The mutation lock is held from the check that the entry is still in the Library to its
/// removal, so a Library-only removal of that entry (keep the file) cannot land between the
/// check and the deletion. (A crash's leftover is all that is ever finished here, so waiting
/// on a large file's identity check costs only startup recovery.)
///
/// `between` runs after the entry is checked and before anything is deleted; tests look at
/// the locks there.
fn resolve(app: &AppHandle, file: &Path, journal: &Journal, between: &mut dyn FnMut()) {
    let held = mutations();
    if journal.phase == Phase::Intent {
        // An entry the user already removed is not a deletion to finish: the files were
        // either deleted then or kept on purpose, and either way this must delete nothing.
        if !app.state::<AppState>().history.lock().unwrap().iter().any(|h| h.id == journal.entry_id) {
            discard(file);
            return;
        }
        between();
        match delete_targets(journal) {
            Ok(left) => {
                if let Some(message) = left_behind(&left) {
                    report(app, message);
                }
            }
            Err(RemoveError::Replaced) => {
                // Another file is at the path: not ours to delete, and the entry stays.
                report(app, format!("Claws did not finish deleting {} because it is no longer the file it saved. It was left alone.", journal.file.path.display()));
                discard(file);
                return;
            }
            Err(e) => {
                report(app, format!("Claws could not finish deleting {} ({e}). It will try again the next time it starts.", journal.file.path.display()));
                return;
            }
        }
    }
    match withdraw(app, &held, &journal.entry_id) {
        Ok(removed) => {
            drop(held);
            finish_removal(app, &journal.entry_id, removed);
            discard(file);
        }
        Err(e) => report(app, format!("Claws could not finish removing a deleted download from your library ({e}). It will try again the next time it starts.")),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::*;
    use crate::download_files::{identify, FileIdentity, Move, MoveState};
    use crate::download_tests::temp_dir;
    use crate::lifecycle_tests::TestApp;

    fn app(tag: &str) -> TestApp {
        let dir = temp_dir(tag);
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        TestApp::with_data(dir, out, Vec::new())
    }

    fn item(id: &str, path: Option<&Path>, identity: Option<FileIdentity>) -> HistoryItem {
        let mut item: HistoryItem = serde_json::from_value(serde_json::json!({
            "id": id, "title": format!("Title {id}"), "url": "https://fixture.invalid/test",
            "uploader": null, "duration": null, "thumbnail": null, "thumbnailLocal": null,
            "filePath": path.map(|p| p.to_string_lossy().into_owned()), "fileSize": 5, "mode": "video",
            "quality": "Original", "finishedAt": 1
        }))
        .unwrap();
        item.file_identity = identity;
        item
    }

    /// An entry whose real file exists, with the identity a verified download would record.
    fn owned(t: &TestApp, id: &str, bytes: &[u8]) -> (HistoryItem, PathBuf) {
        let path = t.out.join(format!("{id}.mp4"));
        fs::write(&path, bytes).unwrap();
        (item(id, Some(&path), Some(identify(&path).unwrap())), path)
    }

    /// Puts entries in memory and on disk, as a previous session would have left them.
    fn seed(t: &TestApp, items: Vec<HistoryItem>) {
        crate::store::save_snapshot(&t.dir.join("history.json"), || items.clone()).unwrap();
        *t.state().history.lock().unwrap() = items;
    }

    fn ids(items: &[HistoryItem]) -> Vec<String> {
        items.iter().map(|h| h.id.clone()).collect()
    }

    fn memory(t: &TestApp) -> Vec<String> {
        ids(&t.state().history.lock().unwrap())
    }

    fn disk(t: &TestApp) -> Vec<String> {
        let text = fs::read_to_string(t.dir.join("history.json")).unwrap_or_default();
        ids(&serde_json::from_str::<Vec<HistoryItem>>(&text).unwrap_or_default())
    }

    fn journals(t: &TestApp) -> usize {
        fs::read_dir(journal_dir(&t.state())).map(|d| d.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".json")).count()).unwrap_or(0)
    }

    fn job_for(path: &Path, workspace: Option<crate::download_files::WorkspaceRef>) -> crate::downloader::Job {
        let mut value = serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(), "url": "https://fixture.invalid/test", "title": "Recorded",
            "thumbnail": null, "uploader": null, "duration": null,
            "options": serde_json::to_value(crate::downloader::DownloadOptions::default()).unwrap(),
            "status": "completed", "progress": 1.0, "speed": null, "eta": null,
            "filePath": path.to_string_lossy(), "error": null, "createdAt": 1
        });
        if let Some(workspace) = workspace {
            value["workspace"] = serde_json::to_value(workspace).unwrap();
        }
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn entries_saved_before_ownership_evidence_still_load_and_new_ones_round_trip() {
        let saved = r#"[{"id":"h1","title":"T","url":"https://example.com/v","uploader":null,"duration":1.0,"thumbnail":null,"thumbnailLocal":null,"filePath":"D:/Out/T.mp4","fileSize":10,"mode":"video","quality":"4K","finishedAt":1}]"#;
        let items: Vec<HistoryItem> = serde_json::from_str(saved).unwrap();
        assert!(items[0].file_identity.is_none() && items[0].published_sidecars.is_empty());
        let mut with = items[0].clone();
        with.file_identity = Some(FileIdentity::Content { size: 3, sha256: "ab".into() });
        with.published_sidecars = vec![Move {
            relative: "T.en.srt".into(),
            destination: PathBuf::from("D:/Out/T.en.srt"),
            identity: FileIdentity::FileId { volume: 1, id: "00ff".into() },
            state: MoveState::Done,
        }];
        let text = serde_json::to_string(&with).unwrap();
        assert!(text.contains("fileIdentity") && text.contains("publishedSidecars"), "{text}");
        let back: HistoryItem = serde_json::from_str(&text).unwrap();
        assert_eq!(back.file_identity, with.file_identity);
        assert_eq!(back.published_sidecars, with.published_sidecars);
    }

    #[test]
    fn a_library_that_cannot_be_saved_is_not_changed_and_loses_no_artwork() {
        let t = app("hm-save-fails");
        let thumbs = t.state().thumbs_dir();
        fs::create_dir_all(&thumbs).unwrap();
        let mut a = item("a", None, None);
        let mut b = item("b", None, None);
        for entry in [&mut a, &mut b] {
            let art = thumbs.join(format!("{}.jpg", entry.id));
            fs::write(&art, b"art").unwrap();
            entry.thumbnail_local = Some(art.to_string_lossy().into_owned());
        }
        seed(&t, vec![a, b]);
        t.block("history.json");

        assert!(remove_entry(t.handle(), "a").is_err());
        assert!(clear(t.handle()).is_err());
        let job = job_for(&t.out.join("new.mp4"), None);
        assert!(history::record(t.handle(), &job).is_err());
        assert!(delete_entry(t.handle(), "a", false).is_err());

        assert_eq!(memory(&t), ["a", "b"], "memory changed although nothing was saved");
        assert!(thumbs.join("a.jpg").is_file() && thumbs.join("b.jpg").is_file(), "artwork went before the save succeeded");
        t.unblock("history.json");
        assert!(remove_entry(t.handle(), "a").is_ok());
        assert_eq!(memory(&t), ["b"]);
        assert_eq!(disk(&t), ["b"]);
        assert!(!thumbs.join("a.jpg").exists() && thumbs.join("b.jpg").is_file());
        assert!(clear(t.handle()).is_ok());
        assert!(memory(&t).is_empty() && disk(&t).is_empty());
        assert!(!thumbs.join("b.jpg").exists());
    }

    #[test]
    fn concurrent_completions_and_clears_keep_memory_and_disk_in_step() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let t = app("hm-concurrent");
        let handle = t.handle();
        let cleared = AtomicBool::new(false);
        let expected = Mutex::new(Vec::<String>::new());
        std::thread::scope(|scope| {
            for worker in 0..6 {
                let (cleared, expected) = (&cleared, &expected);
                scope.spawn(move || {
                    for n in 0..8 {
                        let id = format!("w{worker}-{n}");
                        // Observed before the add starts: the clear had already returned.
                        let after_clear = cleared.load(Ordering::SeqCst);
                        add(handle, item(&id, None, None)).unwrap();
                        if after_clear {
                            expected.lock().unwrap().push(id);
                        }
                        if worker == 0 && n == 4 {
                            clear(handle).unwrap();
                            cleared.store(true, Ordering::SeqCst);
                        }
                    }
                });
            }
        });
        assert_eq!(memory(&t), disk(&t), "the saved Library disagrees with the one in memory");
        // The clear could only have removed what was there before it ran: every completion
        // recorded after it returned must have survived, in memory and on disk.
        let (in_memory, on_disk) = (memory(&t), disk(&t));
        let expected = expected.into_inner().unwrap();
        assert!(expected.len() >= 3, "worker 0 alone adds three entries after its clear");
        for id in &expected {
            assert!(in_memory.contains(id) && on_disk.contains(id), "{id} was recorded after the clear and then lost");
        }
        add(handle, item("after", None, None)).unwrap();
        assert_eq!(memory(&t)[0], "after");
        assert_eq!(disk(&t)[0], "after");
    }

    #[test]
    fn a_replacement_with_the_same_name_and_size_is_not_deleted_as_the_download() {
        let t = app("hm-replaced");
        let (entry, path) = owned(&t, "a", b"media");
        seed(&t, vec![entry]);
        fs::rename(&path, t.out.join("moved.mp4")).unwrap();
        fs::write(&path, b"MEDIA").unwrap();
        let error = delete_entry(t.handle(), "a", true).unwrap_err().to_string();
        assert!(error.contains("no longer"), "{error}");
        assert_eq!(fs::read(&path).unwrap(), b"MEDIA");
        assert_eq!(memory(&t), ["a"]);
        assert_eq!(disk(&t), ["a"]);
        assert_eq!(journals(&t), 0, "nothing was deleted, so nothing stays journalled");
    }

    #[test]
    fn an_entry_without_ownership_evidence_is_removed_from_the_library_only() {
        let t = app("hm-legacy");
        let path = t.out.join("old.mp4");
        fs::write(&path, b"someone's file").unwrap();
        seed(&t, vec![item("old", Some(&path), None)]);
        let error = delete_entry(t.handle(), "old", true).unwrap_err().to_string();
        assert!(error.contains("Show in folder") || error.contains("Library only"), "{error}");
        assert!(path.is_file(), "a file Claws cannot prove it saved was deleted");
        assert_eq!(memory(&t), ["old"]);
        // The Library-only removal still works and leaves the file.
        delete_entry(t.handle(), "old", false).unwrap();
        assert!(memory(&t).is_empty() && disk(&t).is_empty());
        assert!(path.is_file());
    }

    #[test]
    fn deleting_the_file_and_entry_removes_both_and_clears_the_journal() {
        let t = app("hm-delete");
        let (entry, path) = owned(&t, "a", b"media");
        let (other, other_path) = owned(&t, "b", b"other");
        seed(&t, vec![entry, other]);
        delete_entry(t.handle(), "a", true).unwrap();
        assert!(!path.exists());
        assert!(other_path.is_file());
        assert_eq!((memory(&t), disk(&t)), (vec!["b".to_string()], vec!["b".to_string()]));
        assert_eq!(journals(&t), 0);
        // Idempotent: the entry is already gone.
        delete_entry(t.handle(), "a", true).unwrap();
    }

    #[test]
    fn a_missing_file_does_not_keep_the_entry() {
        let t = app("hm-missing");
        let (entry, path) = owned(&t, "a", b"media");
        seed(&t, vec![entry]);
        fs::remove_file(&path).unwrap();
        delete_entry(t.handle(), "a", true).unwrap();
        assert!(memory(&t).is_empty() && disk(&t).is_empty());
        assert_eq!(journals(&t), 0);
    }

    #[cfg(windows)]
    #[test]
    fn a_locked_file_keeps_both_the_file_and_the_entry() {
        use std::os::windows::fs::OpenOptionsExt;
        let t = app("hm-locked");
        let (entry, path) = owned(&t, "a", b"media");
        seed(&t, vec![entry]);
        let held = fs::OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
        let error = delete_entry(t.handle(), "a", true).unwrap_err().to_string();
        assert!(error.contains("could not be deleted"), "{error}");
        drop(held);
        assert!(path.is_file());
        assert_eq!(memory(&t), ["a"]);
        assert_eq!(disk(&t), ["a"]);
        assert_eq!(journals(&t), 0);
    }

    #[test]
    fn a_library_only_removal_retires_a_stale_deletion_so_a_kept_file_is_never_deleted_later() {
        for clear_all in [false, true] {
            let t = app(if clear_all { "hm-stale-clear" } else { "hm-stale-remove" });
            let (entry, path) = owned(&t, "a", b"media");
            seed(&t, vec![entry.clone()]);
            // An intent that survived an earlier refusal (the journal could not be removed then).
            let stale = leave_journal(&t, &entry, Phase::Intent);
            if clear_all {
                clear(t.handle()).unwrap();
            } else {
                remove_entry(t.handle(), "a").unwrap();
            }
            assert!(!stale.exists(), "the user chose to keep the file; its deletion intent must go with the entry");
            reconcile(t.handle()).join().unwrap();
            assert_eq!(fs::read(&path).unwrap(), b"media", "a file the user kept was deleted at the next start");
        }
    }

    #[test]
    fn a_deletion_intent_for_an_entry_that_is_gone_deletes_nothing() {
        let t = app("hm-intent-no-entry");
        let (entry, path) = owned(&t, "a", b"media");
        seed(&t, Vec::new());
        let stale = leave_journal(&t, &entry, Phase::Intent);
        reconcile(t.handle()).join().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"media", "a file was deleted for an entry that is no longer in the Library");
        assert!(!stale.exists(), "nothing is left to finish, so the record goes");
    }

    #[cfg(windows)]
    #[test]
    fn a_side_file_that_could_not_be_deleted_is_reported_but_a_replaced_one_is_not() {
        use std::os::windows::fs::OpenOptionsExt;
        let t = app("hm-left-sidecars");
        let (entry, path) = owned(&t, "a", b"media");
        let side = |name: &str, bytes: &[u8]| {
            let file = t.out.join(name);
            fs::write(&file, bytes).unwrap();
            Target { path: file.clone(), identity: identify(&file).unwrap() }
        };
        let locked = side("a.en.srt", b"english");
        let replaced = side("a.es.srt", b"spanish");
        let gone = side("a.fr.srt", b"french");
        fs::remove_file(&gone.path).unwrap();
        fs::rename(&replaced.path, t.out.join("elsewhere.srt")).unwrap();
        fs::write(&replaced.path, b"SPANISH").unwrap();
        let journal = Journal {
            schema_version: JOURNAL_SCHEMA,
            op_id: "left".into(),
            phase: Phase::Intent,
            entry_id: entry.id.clone(),
            file: Target { path: path.clone(), identity: entry.file_identity.clone().unwrap() },
            sidecars: vec![locked.clone(), replaced.clone(), gone],
        };
        let held = fs::OpenOptions::new().read(true).share_mode(0).open(&locked.path).unwrap();
        let left = delete_targets(&journal).unwrap();
        drop(held);
        assert!(!path.exists());
        assert_eq!(left, vec![locked.path.clone()], "only a side file of Claws' own that stayed behind is worth a notice");
        assert!(locked.path.is_file() && replaced.path.is_file());
    }

    #[cfg(windows)]
    #[test]
    fn a_journal_that_is_briefly_locked_is_still_discarded() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = temp_dir("hm-discard");
        let file = dir.join("op-x.json");
        fs::write(&file, b"{}").unwrap();
        // The lock ends during the first wait: no clock, no second thread, no race.
        let held = std::cell::RefCell::new(Some(fs::OpenOptions::new().read(true).share_mode(0).open(&file).unwrap()));
        let pauses = std::cell::Cell::new(0);
        assert!(
            discard_pausing(&file, || {
                pauses.set(pauses.get() + 1);
                held.borrow_mut().take();
            }),
            "a momentary lock kept a cancelled deletion alive"
        );
        assert_eq!(pauses.get(), 1, "the first attempt met the lock and the second found it gone");
        assert!(!file.exists());
        assert!(discard(&file), "an already removed journal is fine");
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_journal_that_stays_locked_is_given_up_after_a_bounded_number_of_attempts() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = temp_dir("hm-discard-stuck");
        let file = dir.join("op-y.json");
        fs::write(&file, b"{}").unwrap();
        let held = fs::OpenOptions::new().read(true).share_mode(0).open(&file).unwrap();
        let mut pauses = 0;
        assert!(!discard_pausing(&file, || pauses += 1), "a journal that cannot be removed was reported as gone");
        assert_eq!(pauses, DISCARD_ATTEMPTS - 1);
        drop(held);
        assert!(file.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_identity_that_could_not_be_read_is_not_described_as_a_file_in_use() {
        let unreadable = refusal(RemoveError::Unverifiable(std::io::Error::other("the file id could not be read"))).to_string();
        assert!(unreadable.contains("could not confirm") && unreadable.contains("nothing was deleted"), "{unreadable}");
        assert!(!unreadable.contains("Close any app"), "a failed identity check was reported as a file in use: {unreadable}");
        let in_use = refusal(RemoveError::Io(std::io::Error::other("used by another process"))).to_string();
        assert!(in_use.contains("Close any app"), "{in_use}");
    }

    #[test]
    fn notices_raised_before_the_window_listens_are_held_and_then_shown() {
        let mut notes = Notes::default();
        assert_eq!(notes.submit("first".into()), None);
        assert_eq!(notes.submit("second".into()), None);
        assert_eq!(notes.release(), ["first", "second"]);
        assert_eq!(notes.submit("later".into()), Some("later".to_string()));
        assert!(notes.release().is_empty());
    }

    #[test]
    fn a_journal_that_cannot_be_written_deletes_nothing() {
        let t = app("hm-journal-fails");
        let (entry, path) = owned(&t, "a", b"media");
        seed(&t, vec![entry]);
        // A file where the journal folder belongs makes every journal write fail.
        fs::write(journal_dir(&t.state()), b"in the way").unwrap();
        let error = delete_entry(t.handle(), "a", true).unwrap_err().to_string();
        assert!(error.contains("nothing was deleted"), "{error}");
        assert_eq!(fs::read(&path).unwrap(), b"media");
        assert_eq!(memory(&t), ["a"]);
    }

    #[test]
    fn a_library_save_failing_after_the_file_went_is_reconciled_by_the_journal() {
        let t = app("hm-after-delete");
        let (entry, path) = owned(&t, "a", b"media");
        seed(&t, vec![entry]);
        t.block("history.json");
        let error = delete_entry(t.handle(), "a", true).unwrap_err().to_string();
        assert!(error.contains("was deleted") && error.contains("library"), "{error}");
        assert!(!path.exists());
        assert_eq!(memory(&t), ["a"], "the entry must stay until its removal is saved");
        assert_eq!(journals(&t), 1, "the unfinished deletion must stay journalled");

        t.unblock("history.json");
        reconcile(t.handle()).join().unwrap();
        assert!(memory(&t).is_empty() && disk(&t).is_empty());
        assert_eq!(journals(&t), 0);
        reconcile(t.handle()).join().unwrap(); // idempotent
        assert!(memory(&t).is_empty());
    }

    #[test]
    fn a_restart_after_the_file_went_but_before_the_library_was_saved_finishes_the_removal() {
        let t = app("hm-restart");
        let (entry, path) = owned(&t, "a", b"media");
        let (keep, keep_path) = owned(&t, "b", b"other");
        seed(&t, vec![entry, keep]);
        t.block("history.json");
        assert!(delete_entry(t.handle(), "a", true).is_err());
        t.unblock("history.json");
        // The Library file as the crashed run left it: still listing both entries.
        let listed = t.state().history.lock().unwrap().clone();
        assert_eq!(ids(&listed), ["a", "b"]);
        crate::store::save_snapshot(&t.dir.join("history.json"), || listed.clone()).unwrap();
        let t = t.restart();
        assert!(!path.exists() && keep_path.is_file());
        assert_eq!(memory(&t), ["b"]);
        assert_eq!(disk(&t), ["b"]);
        assert_eq!(journals(&t), 0);
    }

    #[test]
    fn only_recorded_and_identity_verified_side_files_are_deleted() {
        let t = app("hm-sidecars");
        let (mut entry, path) = owned(&t, "a", b"media");
        let recorded = |name: &str, bytes: &[u8]| {
            let side = t.out.join(name);
            fs::write(&side, bytes).unwrap();
            Move { relative: name.into(), destination: side.clone(), identity: identify(&side).unwrap(), state: MoveState::Done }
        };
        let ours = recorded("a.en.srt", b"english");
        let replaced = recorded("a.es.srt", b"spanish");
        entry.published_sidecars = vec![ours.clone(), replaced.clone()];
        // Look-alikes that were never recorded: same stem, and an unrelated neighbour.
        fs::write(t.out.join("a.fr.srt"), b"the user's own French").unwrap();
        fs::write(t.out.join("a.mp4.bak"), b"backup").unwrap();
        // One recorded side file was swapped for the user's own.
        fs::rename(&replaced.destination, t.out.join("elsewhere.srt")).unwrap();
        fs::write(&replaced.destination, b"SPANISH").unwrap();
        seed(&t, vec![entry]);

        delete_entry(t.handle(), "a", true).unwrap();
        assert!(!path.exists() && !ours.destination.exists());
        assert_eq!(fs::read(&replaced.destination).unwrap(), b"SPANISH", "a replaced side file was deleted");
        assert_eq!(fs::read(t.out.join("a.fr.srt")).unwrap(), b"the user's own French", "side files are never found by name");
        assert!(t.out.join("a.mp4.bak").is_file() && t.out.join("elsewhere.srt").is_file());
        assert!(memory(&t).is_empty());
    }

    #[test]
    fn a_journal_from_a_newer_version_is_left_alone_with_its_entry() {
        let t = app("hm-journal-schema");
        let (entry, path) = owned(&t, "a", b"media");
        seed(&t, vec![entry]);
        let dir = journal_dir(&t.state());
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("op-future.json");
        fs::write(&file, r#"{"schemaVersion":99,"opId":"future","phase":"quantum","entryId":"a"}"#).unwrap();
        reconcile(t.handle()).join().unwrap();
        assert!(file.is_file(), "an unreadable destructive record was discarded");
        assert!(path.is_file());
        assert_eq!(memory(&t), ["a"]);
    }

    /// A journal as a crashed `delete_entry` leaves it, for the entry and file of `entry`.
    fn leave_journal(t: &TestApp, entry: &HistoryItem, phase: Phase) -> PathBuf {
        let journal = Journal {
            schema_version: JOURNAL_SCHEMA,
            op_id: "crashed".into(),
            phase,
            entry_id: entry.id.clone(),
            file: Target { path: PathBuf::from(entry.file_path.clone().unwrap()), identity: entry.file_identity.clone().unwrap() },
            sidecars: Vec::new(),
        };
        let file = journal_file(&journal_dir(&t.state()), &journal.op_id);
        write_journal(&file, &journal).unwrap();
        file
    }

    #[cfg(windows)]
    #[test]
    fn a_download_whose_marker_cannot_be_read_is_not_saved_without_its_ownership_evidence() {
        use std::os::windows::fs::OpenOptionsExt;
        let t = app("hm-record-locked-marker");
        let job_id = uuid::Uuid::new_v4().to_string();
        let (reference, mut manifest) = download_files::open_workspace(&t.out, &job_id, 0).unwrap();
        fs::write(reference.path.join("clip.mp4"), b"media").unwrap();
        manifest.outputs = vec!["clip.mp4".into()];
        manifest.state = download_files::Publication::Ready;
        download_files::write_manifest(&manifest).unwrap();
        let published = download_files::publish(&mut manifest, &mut |_| true).unwrap();
        let mut job = job_for(&published.media, Some(reference.clone()));
        job.id = job_id.clone();

        // Another program holds the marker for longer than recording waits for it.
        let held = fs::OpenOptions::new().read(true).share_mode(0).open(reference.path.join(download_files::MARKER)).unwrap();
        let error = history::record(t.handle(), &job).unwrap_err().to_string();
        assert!(error.contains("could not be read right now"), "{error}");
        assert!(memory(&t).is_empty() && disk(&t).is_empty(), "an entry without evidence was saved, and it could never be deleted from the Library");
        // Readable again, the same download is recorded with the proof it was saved with.
        drop(held);
        history::record(t.handle(), &job).unwrap();
        let entry = t.state().history.lock().unwrap().iter().find(|h| h.id == job_id).cloned().expect("recorded");
        assert_eq!(entry.file_identity, Some(identify(&published.media).unwrap()));
        let _ = fs::remove_file(&published.media);
        // And an old download with no marker is still recorded, as before: without evidence.
        let legacy_id = uuid::Uuid::new_v4().to_string();
        let mut legacy = job_for(&t.out.join("legacy.mp4"), None);
        legacy.id = legacy_id.clone();
        history::record(t.handle(), &legacy).unwrap();
        assert!(t.state().history.lock().unwrap().iter().find(|h| h.id == legacy_id).is_some_and(|h| h.file_identity.is_none()));
    }

    fn mutation_lock_is_held() -> bool {
        HELD_HERE.with(|n| n.get() > 0)
    }

    #[test]
    fn the_lock_check_sees_only_a_guard_held_by_this_thread() {
        // Another test (another thread) holding the lock must not make this thread's check true.
        let (taken_tx, taken_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let other = std::thread::spawn(move || {
            let _held = mutations();
            taken_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });
        taken_rx.recv().unwrap();
        assert!(!mutation_lock_is_held(), "a lock held by another thread was taken for this thread's");
        release_tx.send(()).unwrap();
        other.join().unwrap();
        assert!(!mutation_lock_is_held());
        let held = mutations();
        assert!(mutation_lock_is_held());
        drop(held);
        assert!(!mutation_lock_is_held(), "the check still says held after the guard was dropped");
    }

    #[test]
    fn a_deletion_is_finished_with_the_entry_checked_and_the_files_deleted_in_one_locked_step() {
        let t = app("hm-resolve-locked");
        let (entry, path) = owned(&t, "a", b"media");
        seed(&t, vec![entry.clone()]);
        let file = leave_journal(&t, &entry, Phase::Intent);
        let Read::Journal(journal) = read_journal(&file) else { panic!("the journal could not be read") };
        let mut locked_when_deleting = None;
        resolve(t.handle(), &file, &journal, &mut || locked_when_deleting = Some(mutation_lock_is_held()));
        // Between the check that the entry is still in the Library and the deletion, a
        // Library-only removal of that entry (keep the file) must not be able to slip in.
        assert_eq!(locked_when_deleting, Some(true), "the entry was checked and the file deleted without the mutation lock");
        assert!(!path.exists() && memory(&t).is_empty() && disk(&t).is_empty());
        assert!(!file.exists());
        // An entry that is already gone still deletes nothing, and never reaches the deletion.
        let (kept, kept_path) = owned(&t, "b", b"kept");
        let stale = leave_journal(&t, &kept, Phase::Intent);
        let Read::Journal(stale_journal) = read_journal(&stale) else { panic!("the journal could not be read") };
        resolve(t.handle(), &stale, &stale_journal, &mut || panic!("deleted for an entry that is not in the Library"));
        assert!(kept_path.is_file() && !stale.exists());
    }

    #[test]
    fn a_journal_that_vanished_after_listing_is_not_reported_as_unreadable() {
        let dir = temp_dir("hm-read-journal");
        assert!(matches!(read_journal(&dir.join("op-gone.json")), Read::Vanished), "a finished deletion looked like a damaged record");
        let damaged = dir.join("op-damaged.json");
        fs::write(&damaged, b"{ not json").unwrap();
        assert!(matches!(read_journal(&damaged), Read::Unreadable));
        let future = dir.join("op-future.json");
        fs::write(&future, r#"{"schemaVersion":99,"opId":"f","phase":"intent","entryId":"a"}"#).unwrap();
        assert!(matches!(read_journal(&future), Read::Unreadable));
        // A directory where a file belongs is not a vanished journal either.
        let folder = dir.join("op-folder.json");
        fs::create_dir(&folder).unwrap();
        assert!(matches!(read_journal(&folder), Read::Unreadable));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recovery_does_not_sweep_a_temporary_journal_while_a_deletion_is_being_recorded() {
        let t = app("hm-tmp-sweep");
        let dir = journal_dir(&t.state());
        fs::create_dir_all(&dir).unwrap();
        let stale = dir.join("op-crashed.json.0123456789abcdef.tmp");
        fs::write(&stale, b"half a journal").unwrap();
        // While a deletion is recording its journal (it holds the mutation lock), the
        // temporary file it is writing must not be taken for a crash's leftover.
        let held = mutations();
        let sweeper = reconcile(t.handle());
        std::thread::sleep(Duration::from_millis(300));
        assert!(stale.exists(), "a temporary journal was swept while the mutation lock was held");
        drop(held);
        sweeper.join().unwrap();
        assert!(!stale.exists(), "a crash's leftover is still swept");
    }

    #[test]
    fn a_deletion_records_its_journal_under_the_mutation_lock() {
        let t = app("hm-journal-locked");
        let (entry, path) = owned(&t, "a", b"media");
        seed(&t, vec![entry]);
        let held = mutations();
        let handle = t.handle().clone();
        let deleting = std::thread::spawn(move || delete_entry(&handle, "a", true));
        std::thread::sleep(Duration::from_millis(300));
        // Writing the journal (which recovery's sweep of temporary files must not meet) waits
        // its turn, so nothing has been written or deleted yet.
        assert!(path.is_file(), "the file was deleted before the journal was recorded under the lock");
        assert_eq!(journals(&t), 0);
        drop(held);
        deleting.join().unwrap().unwrap();
        assert!(!path.exists() && memory(&t).is_empty());
    }

    #[test]
    fn a_restart_after_the_intent_was_journalled_but_before_the_file_went_finishes_the_deletion() {
        let t = app("hm-intent-only");
        let (entry, path) = owned(&t, "a", b"media");
        seed(&t, vec![entry.clone()]);
        let file = leave_journal(&t, &entry, Phase::Intent);
        reconcile(t.handle()).join().unwrap();
        assert!(!path.exists(), "the journalled deletion was not carried out");
        assert!(memory(&t).is_empty() && disk(&t).is_empty());
        assert!(!file.exists());
    }

    #[test]
    fn a_restart_never_deletes_a_file_that_is_no_longer_the_journalled_one() {
        let t = app("hm-intent-replaced");
        let (entry, path) = owned(&t, "a", b"media");
        seed(&t, vec![entry.clone()]);
        let file = leave_journal(&t, &entry, Phase::Intent);
        fs::rename(&path, t.out.join("moved.mp4")).unwrap();
        fs::write(&path, b"MEDIA").unwrap();
        reconcile(t.handle()).join().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"MEDIA");
        assert_eq!(memory(&t), ["a"], "the entry of a file that was not deleted must stay");
        assert!(!file.exists(), "the transaction is resolved: nothing is deleted");
    }

    #[test]
    fn a_journal_whose_files_are_gone_only_removes_the_entry_and_a_missing_entry_is_fine() {
        let t = app("hm-files-deleted");
        let (entry, path) = owned(&t, "a", b"media");
        seed(&t, vec![entry.clone()]);
        let file = leave_journal(&t, &entry, Phase::FilesDeleted);
        fs::remove_file(&path).unwrap();
        reconcile(t.handle()).join().unwrap();
        assert!(memory(&t).is_empty() && !file.exists());
        // The same journal again, with the entry long gone.
        let again = leave_journal(&t, &entry, Phase::FilesDeleted);
        reconcile(t.handle()).join().unwrap();
        assert!(!again.exists());
    }
}

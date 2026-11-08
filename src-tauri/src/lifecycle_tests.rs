//! Lifecycle tests: the real downloader, queue writer and stores, driven through a mock
//! Tauri app with its own disposable data folder, and the fixture tool standing in for
//! yt-dlp (`cargo build --features test-tools --bin claws-fixture-tool`; without it these
//! tests skip). The owner's real queue, library and downloads are never touched.
//!
//! Faults are real file-system failures: a folder put where a JSON file has to be renamed
//! makes that save fail exactly as a denied write or a full disk would.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::Manager;

use crate::download_tests::{fixture_tool, temp_dir};
use crate::downloader::{self, DownloadOptions, EnqueueItem, Job, JobStatus};
use crate::settings::Settings;
use crate::state::AppState;
use crate::AppHandle;

pub(crate) struct TestApp {
    app: tauri::App<tauri::test::MockRuntime>,
    pub dir: PathBuf,
    pub out: PathBuf,
    /// Free bytes the app reads, in order; the last value repeats. Plenty by default.
    pub free: Arc<Mutex<VecDeque<u64>>>,
    pub low_space_delay: Arc<std::sync::atomic::AtomicU64>,
}

impl TestApp {
    /// A fresh app with the fixture tool installed as its yt-dlp, or `None` (skip) when the
    /// tool has not been built.
    pub fn new(tag: &str) -> Option<Self> {
        let Some(tool) = fixture_tool() else {
            eprintln!("skipped: build claws-fixture-tool with --features test-tools");
            return None;
        };
        let dir = temp_dir(tag);
        let out = dir.join("out");
        let bin = dir.join("bin").join("yt-dlp");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::copy(&tool, bin.join("yt-dlp.exe")).unwrap();
        Some(Self::with_data(dir, out, Vec::new()))
    }

    /// An app started on an existing data folder (a restart), restoring `queue`.
    pub fn with_data(dir: PathBuf, out: PathBuf, queue: Vec<Job>) -> Self {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        let settings = Settings {
            output_dir: out.to_string_lossy().into_owned(),
            embed_metadata: false,
            ..Settings::default()
        };
        let mut notices = Vec::new();
        let history = crate::store::load_or_recover(&dir.join("history.json"), "library", &mut notices);
        let cleanup = crate::store::load_or_recover(&dir.join("cleanup.json"), "cleanup list", &mut notices);
        let writer = downloader::start_queue_writer(app.handle());
        let free: Arc<Mutex<VecDeque<u64>>> = Arc::new(Mutex::new(VecDeque::from([u64::MAX / 4])));
        let mut state = AppState::new(dir.clone(), settings, history, cleanup, writer, false);
        let readings = free.clone();
        let low_space_delay = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let delay = low_space_delay.clone();
        state.free_space = Box::new(move |_| {
            let mut readings = readings.lock().unwrap();
            let value = *readings.front().unwrap();
            if readings.len() > 1 {
                readings.pop_front();
            }
            drop(readings);
            if value < 512 * 1024 * 1024 {
                std::thread::sleep(Duration::from_millis(delay.load(std::sync::atomic::Ordering::SeqCst)));
            }
            Ok(value)
        });
        app.manage(state);
        downloader::restore_queue(app.handle(), queue).join().expect("startup recovery");
        Self { app, dir, out, free, low_space_delay }
    }

    pub fn set_free(&self, readings: &[u64]) {
        *self.free.lock().unwrap() = readings.iter().copied().collect();
    }

    /// Closes this app the way Claws exits and starts a new one on the same data.
    pub fn restart(self) -> Self {
        self.restart_after(|_| {})
    }

    /// Like `restart`, with `change` applied to the data folder while Claws is closed.
    pub fn restart_after(self, change: impl FnOnce(&Path)) -> Self {
        let (dir, out) = (self.dir.clone(), self.out.clone());
        downloader::shutdown(self.handle());
        std::mem::forget(self); // its folder is reused, so Drop must not remove it
        change(&dir);
        let mut notices = Vec::new();
        let queue = crate::queue_writer::load(&dir.join("queue.json"), &mut notices);
        Self::with_data(dir, out, queue)
    }

    pub fn handle(&self) -> &AppHandle {
        self.app.handle()
    }

    pub fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }

    pub fn enqueue(&self, link: &str) -> String {
        self.enqueue_with(link, DownloadOptions::default())
    }

    pub fn enqueue_with(&self, link: &str, options: DownloadOptions) -> String {
        let item = EnqueueItem { url: link.into(), playlist_index: None, title: link.into(), thumbnail: None, uploader: None, duration: None };
        let added = downloader::enqueue(self.handle(), vec![item], options).expect("enqueue");
        added.jobs[0].job_id.clone()
    }

    pub fn job(&self, id: &str) -> Option<Job> {
        downloader::list(self.handle()).into_iter().find(|j| j.id == id)
    }

    pub fn wait_job(&self, id: &str, done: impl Fn(&Job) -> bool, what: &str) -> Job {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(job) = self.job(id).filter(|j| done(j)) {
                return job;
            }
            if Instant::now() > deadline {
                panic!("timed out waiting for {what}: {:?}", self.job(id).map(|j| (j.status, j.error)));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Lines the fixture tool logged, one per start.
    pub fn starts(&self) -> Vec<String> {
        std::fs::read_to_string(self.out.join("fixture-starts.log"))
            .map(|t| t.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// Makes saves of `name` in the data folder fail.
    pub fn block(&self, name: &str) {
        let path = self.dir.join(name);
        let _ = std::fs::remove_file(&path);
        std::fs::create_dir_all(&path).unwrap();
    }

    pub fn unblock(&self, name: &str) {
        std::fs::remove_dir_all(self.dir.join(name)).unwrap();
    }

    pub fn saved_queue(&self) -> Vec<Job> {
        let text = std::fs::read_to_string(self.dir.join("queue.json")).unwrap_or_default();
        serde_json::from_str::<crate::queue_writer::QueueSnapshot>(&text).map(|s| s.jobs).unwrap_or_default()
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        downloader::shutdown(self.handle());
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub(crate) fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn running(job: &Job) -> bool {
    job.status == JobStatus::Downloading && job.downloaded_bytes.is_some()
}

fn part_file(workspace: &Path) -> Vec<u8> {
    std::fs::read_dir(workspace)
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().ends_with(".part"))
        .map(|e| std::fs::read(e.path()).unwrap())
        .unwrap_or_default()
}

#[test]
fn a_start_that_cannot_be_saved_starts_nothing_and_retries_in_place() {
    let Some(t) = TestApp::new("b4-start") else { return };
    t.block("queue.json");
    let id = t.enqueue("https://fixture.invalid/hang/alpha");
    let job = t.wait_job(&id, |j| j.status == JobStatus::Failed, "the start to fail");
    let error = job.error.clone().unwrap_or_default();
    assert!(error.contains("could not save its download queue") && error.contains("not started"), "{error}");
    assert!(t.starts().is_empty(), "yt-dlp started without a saved checkpoint");
    let workspace = job.workspace.clone().expect("the work folder is kept");
    assert!(workspace.path.is_dir());

    t.unblock("queue.json");
    downloader::resume(t.handle(), &id);
    let job = t.wait_job(&id, running, "the retry to run");
    assert_eq!(job.workspace.as_ref(), Some(&workspace));
    assert_eq!(t.starts().len(), 1);
    let saved = t.saved_queue();
    let on_disk = saved.iter().find(|j| j.id == id).expect("the started job is on disk");
    assert_eq!(on_disk.workspace.as_ref(), Some(&workspace));
    assert!(on_disk.execution.is_some());
}

#[test]
fn a_cancel_that_cannot_be_saved_deletes_nothing() {
    let Some(t) = TestApp::new("b4-cancel") else { return };
    let id = t.enqueue("https://fixture.invalid/hang/bravo");
    let job = t.wait_job(&id, running, "the download to run");
    let workspace = job.workspace.unwrap().path;
    t.block("queue.json");
    downloader::cancel(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Cancelled, "the cancel");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(part_file(&workspace), b"partial", "files went before the cancel was saved");

    t.unblock("queue.json");
    downloader::remove(t.handle(), &id).unwrap();
    wait_until("the work folder to go", || !workspace.exists());
}

#[test]
fn nothing_is_deleted_or_forgotten_without_a_saved_cleanup_record() {
    let Some(t) = TestApp::new("b4-record") else { return };
    let id = t.enqueue("https://fixture.invalid/hang/charlie");
    t.wait_job(&id, running, "the download to run");
    downloader::pause(t.handle(), &id);
    let job = t.wait_job(&id, |j| j.status == JobStatus::Paused, "the pause");
    let workspace = job.workspace.unwrap().path;

    t.block("cleanup.json");
    let err = downloader::remove(t.handle(), &id).unwrap_err().to_string();
    assert!(err.contains("cleanup list") && err.contains("not removed"), "{err}");
    assert!(t.job(&id).is_some(), "the job must stay findable");
    downloader::cancel(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Cancelled, "the cancel");
    std::thread::sleep(Duration::from_millis(1000));
    assert!(workspace.join(crate::download_files::MARKER).is_file(), "deleted without a record");

    t.unblock("cleanup.json");
    downloader::remove(t.handle(), &id).unwrap();
    wait_until("the work folder to go", || !workspace.exists());
    assert!(t.job(&id).is_none());
}

fn library_entries(t: &TestApp, id: &str) -> usize {
    let text = std::fs::read_to_string(t.dir.join("history.json")).unwrap_or_default();
    let items: Vec<crate::history::HistoryItem> = serde_json::from_str(&text).unwrap_or_default();
    items.iter().filter(|h| h.id == id).count()
}

/// A finished download whose Library entry could not be saved: published, failed, and
/// still in the queue with its work folder.
fn published_but_not_recorded(t: &TestApp, name: &str) -> (String, PathBuf) {
    t.block("history.json");
    let id = t.enqueue(&format!("https://fixture.invalid/ok/{name}"));
    let job = t.wait_job(&id, |j| j.status == JobStatus::Failed, "the Library save to fail");
    let error = job.error.clone().unwrap_or_default();
    assert!(error.contains("could not add it to your Library") && error.contains("Try Again"), "{error}");
    let media = t.out.join(format!("{name}.mp4"));
    assert_eq!(std::fs::read(&media).unwrap(), b"merged");
    let workspace = job.workspace.unwrap().path;
    let manifest = crate::download_files::read_manifest(&workspace).expect("the manifest is kept");
    assert_eq!(manifest.state, crate::download_files::Publication::Published);
    assert!(t.saved_queue().iter().any(|j| j.id == id), "the queue must keep the job");
    (id, workspace)
}

#[test]
fn a_library_save_failure_keeps_the_completion_and_try_again_finishes_it() {
    let Some(t) = TestApp::new("b2-retry") else { return };
    let (id, workspace) = published_but_not_recorded(&t, "kilo");
    t.unblock("history.json");
    downloader::resume(t.handle(), &id);
    let job = t.wait_job(&id, |j| j.status == JobStatus::Completed, "the completion");
    assert_eq!(job.file_path.as_deref(), Some(t.out.join("kilo.mp4").to_string_lossy().as_ref()));
    assert_eq!(library_entries(&t, &id), 1);
    assert_eq!(t.starts().len(), 1, "Try Again downloaded again");
    wait_until("the work folder to go", || !workspace.exists());
    wait_until("the queue to retire the job", || t.saved_queue().iter().all(|j| j.id != id));
}

#[test]
fn an_unrecorded_completion_is_finished_at_the_next_start_exactly_once() {
    let Some(t) = TestApp::new("b2-restart") else { return };
    let (id, workspace) = published_but_not_recorded(&t, "lima");
    t.unblock("history.json");
    let t = t.restart();
    let job = t.job(&id).expect("recovered");
    assert_eq!(job.status, JobStatus::Completed);
    assert_eq!(library_entries(&t, &id), 1);
    wait_until("the work folder to go", || !workspace.exists());
    wait_until("the queue to retire the job", || t.saved_queue().iter().all(|j| j.id != id));
    let t = t.restart();
    assert!(t.job(&id).is_none());
    assert_eq!(library_entries(&t, &id), 1);
    assert_eq!(t.starts().len(), 1);
}

#[test]
fn a_restart_while_the_library_still_cannot_be_saved_keeps_everything() {
    let Some(t) = TestApp::new("b2-still") else { return };
    let (id, workspace) = published_but_not_recorded(&t, "mike");
    let t = t.restart(); // history.json is still blocked
    let job = t.job(&id).expect("still in the queue");
    assert_eq!(job.status, JobStatus::Failed);
    std::thread::sleep(Duration::from_millis(500));
    assert!(workspace.exists(), "the recovery record went before the Library had the download");
    t.unblock("history.json");
    downloader::resume(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Completed, "the completion");
    assert_eq!(library_entries(&t, &id), 1);
}

#[test]
fn a_move_whose_record_was_lost_is_finished_without_downloading_again() {
    use crate::download_files::{read_manifest, write_manifest, MoveState, Publication};
    let Some(t) = TestApp::new("b3-app") else { return };
    let (id, workspace) = published_but_not_recorded(&t, "november");
    t.unblock("history.json");
    // Rewind the journal to the instant between the move and its record.
    let mut manifest = read_manifest(&workspace).unwrap();
    manifest.state = Publication::Publishing;
    manifest.moves.iter_mut().for_each(|m| m.state = MoveState::Moving);
    write_manifest(&manifest).unwrap();
    let t = t.restart();
    assert_eq!(t.job(&id).map(|j| j.status), Some(JobStatus::Failed));
    downloader::resume(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Completed, "the completion");
    assert_eq!(std::fs::read(t.out.join("november.mp4")).unwrap(), b"merged");
    assert_eq!(library_entries(&t, &id), 1);
    assert_eq!(t.starts().len(), 1, "the download ran again");
}

#[test]
fn a_taken_name_never_overwrites_and_try_again_publishes_after_it_is_moved() {
    let Some(t) = TestApp::new("b3-conflict") else { return };
    std::fs::create_dir_all(&t.out).unwrap();
    std::fs::write(t.out.join("oscar.mp4"), b"the user's own").unwrap();
    let id = t.enqueue("https://fixture.invalid/ok/oscar");
    let job = t.wait_job(&id, |j| j.status == JobStatus::Failed, "the conflict");
    assert!(job.error.unwrap_or_default().contains("already exists"));
    assert_eq!(std::fs::read(t.out.join("oscar.mp4")).unwrap(), b"the user's own");
    std::fs::rename(t.out.join("oscar.mp4"), t.out.join("mine.mp4")).unwrap();
    downloader::resume(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Completed, "the publication");
    assert_eq!(std::fs::read(t.out.join("oscar.mp4")).unwrap(), b"merged");
    assert_eq!(std::fs::read(t.out.join("mine.mp4")).unwrap(), b"the user's own");
    assert_eq!(t.starts().len(), 1);
}

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

#[test]
fn a_second_download_is_admitted_only_beside_what_the_first_still_needs() {
    let Some(t) = TestApp::new("b5-admit") else { return };
    t.set_free(&[2 * GIB + GIB / 2]);
    // The first plans 1 GiB: with its margin it holds about 1.5 of the 2.5 GiB.
    let first = t.enqueue(&format!("https://fixture.invalid/hang/alpha/{GIB}"));
    let workspace = t.wait_job(&first, running, "the first download").workspace.unwrap().path;
    let held = || t.state().disk_ledger.lock().unwrap().held(&first);
    let admitted = held().expect("the first is reserved");
    std::fs::write(workspace.join("alpha.f18.mp4.part-Frag1"), vec![0u8; 64 * MIB as usize]).unwrap();
    // At least 64 MiB: the tool's own few bytes may land after the first sample too.
    wait_until("the reservation to shrink as bytes land", || held().is_some_and(|h| h <= admitted - 64 * MIB));

    let second = t.enqueue(&format!("https://fixture.invalid/hang/bravo/{GIB}"));
    let job = t.wait_job(&second, |j| j.status == JobStatus::Failed, "the second to be refused");
    assert!(job.error.unwrap_or_default().contains("Not enough free space"));

    downloader::cancel(t.handle(), &first);
    t.wait_job(&first, |j| j.status == JobStatus::Cancelled, "the cancel");
    wait_until("the first's claim to go", || held().is_none());
    downloader::resume(t.handle(), &second);
    t.wait_job(&second, running, "the second download");
}

#[test]
fn low_space_during_processing_keeps_retryable_files_or_a_valid_result() {
    let Some(t) = TestApp::new("b5-merge") else { return };
    t.set_free(&[2 * GIB, 100 * MIB]);
    let id = t.enqueue("https://fixture.invalid/ok/papa");
    let job = t.wait_job(&id, |j| matches!(j.status, JobStatus::Failed | JobStatus::Completed), "processing to settle");
    if job.status == JobStatus::Completed {
        assert_eq!(std::fs::read(t.out.join("papa.mp4")).unwrap(), b"merged");
        assert_eq!(library_entries(&t, &id), 1);
        return;
    }
    let error = job.error.unwrap_or_default();
    assert!(error.contains("to finish processing") && error.contains("files are kept"), "{error}");
    assert!(!t.out.join("papa.mp4").exists());
    let workspace = job.workspace.unwrap().path;
    let kept = crate::download_tests::names(&workspace);
    let streams = workspace.join("papa.f137.mp4").is_file() && workspace.join("papa.f140.m4a").is_file();
    assert!(streams || workspace.join("papa.mp4").is_file(), "{kept:?}");
    assert!(t.state().disk_ledger.lock().unwrap().held(&id).is_none());
    t.set_free(&[2 * GIB]);
    downloader::resume(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Completed, "the retry");
}

#[test]
fn a_retried_cancel_survives_a_restart_with_its_new_bytes() {
    let Some(t) = TestApp::new("b1-restart") else { return };
    let id = t.enqueue("https://fixture.invalid/hang/echo");
    let first = t.wait_job(&id, running, "the first attempt").workspace.unwrap().path;
    downloader::cancel(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Cancelled, "the cancel");
    wait_until("the cancelled attempt's files to go", || !first.exists());

    downloader::resume(t.handle(), &id);
    let job = t.wait_job(&id, running, "the retry");
    assert!(!job.cancel_requested, "the retry still carries the old cancel");
    let second = job.workspace.unwrap().path;
    assert_eq!(part_file(&second), b"partial");

    let t = t.restart();
    let job = t.job(&id).expect("the retry is still in the queue");
    assert_eq!(job.status, JobStatus::Paused);
    std::thread::sleep(Duration::from_millis(800)); // startup cleanup has run
    assert_eq!(part_file(&second), b"partial", "startup deleted the retry's bytes");
    downloader::resume(t.handle(), &id);
    t.wait_job(&id, |j| running(j) && j.downloaded_bytes == Some(14), "the resumed retry");
    assert_eq!(part_file(&second), b"partialpartial", "the retry was restarted instead of resumed");
}

#[cfg(windows)]
#[test]
fn a_retry_while_old_files_are_locked_keeps_both_safe() {
    use std::os::windows::fs::OpenOptionsExt;
    let Some(t) = TestApp::new("b1-locked") else { return };
    let id = t.enqueue("https://fixture.invalid/hang/foxtrot");
    let first = t.wait_job(&id, running, "the first attempt").workspace.unwrap().path;
    // Something else (a preview, a scanner) holds a file of the attempt being cancelled.
    let lock = std::fs::OpenOptions::new().read(true).share_mode(0).open(first.join("foxtrot.f18.mp4.part")).unwrap();
    downloader::cancel(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Cancelled, "the cancel");

    downloader::resume(t.handle(), &id);
    let second = t.wait_job(&id, running, "the retry").workspace.unwrap().path;
    assert_ne!(first, second);
    std::thread::sleep(Duration::from_millis(2500)); // the old cleanup gives up for now
    assert!(first.exists(), "the locked folder cannot have gone");
    assert!(t.state().pending_cleanup.lock().unwrap().iter().any(|p| p.workspace.as_ref().is_some_and(|w| w.path == first)));

    drop(lock);
    downloader::retry_pending_cleanup(t.handle());
    assert!(!first.exists());
    assert_eq!(part_file(&second), b"partial");
    assert!(t.state().pending_cleanup.lock().unwrap().is_empty());
}

#[test]
fn repeated_cancel_and_retry_start_one_new_attempt() {
    let Some(t) = TestApp::new("b1-repeat") else { return };
    let id = t.enqueue("https://fixture.invalid/hang/golf");
    t.wait_job(&id, running, "the first attempt");
    downloader::cancel(t.handle(), &id);
    downloader::cancel(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Cancelled, "the cancel");
    downloader::resume(t.handle(), &id);
    downloader::resume(t.handle(), &id);
    let job = t.wait_job(&id, running, "the retry");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(job.attempt, 1);
    assert_eq!(t.starts().len(), 2, "{:?}", t.starts());
    let folders = crate::download_tests::names(&t.out.join(crate::download_files::WORK_DIR));
    assert_eq!(folders.len(), 1, "only the retry's folder remains: {folders:?}");
}

#[test]
fn a_job_removed_during_its_retry_stays_gone_and_leaves_nothing() {
    let Some(t) = TestApp::new("b1-remove") else { return };
    let id = t.enqueue("https://fixture.invalid/hang/hotel");
    t.wait_job(&id, running, "the first attempt");
    downloader::cancel(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Cancelled, "the cancel");
    downloader::resume(t.handle(), &id);
    downloader::remove(t.handle(), &id).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert!(t.job(&id).is_none(), "a removed job came back");
    let work = t.out.join(crate::download_files::WORK_DIR);
    wait_until("every attempt's folder to go", || !work.exists() || crate::download_tests::names(&work).is_empty());
    assert!(t.saved_queue().iter().all(|j| j.id != id));
}

#[test]
fn shutdown_right_after_a_retry_keeps_it_resumable() {
    let Some(t) = TestApp::new("b1-shutdown") else { return };
    let id = t.enqueue("https://fixture.invalid/hang/india");
    t.wait_job(&id, running, "the first attempt");
    downloader::cancel(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Cancelled, "the cancel");
    downloader::resume(t.handle(), &id);
    let t = t.restart();
    // Either the retry had started (and comes back paused) or it had not been saved yet
    // (and the job is simply gone with its cancel); never a cancel that deletes a retry.
    if let Some(job) = t.job(&id) {
        assert_eq!(job.status, JobStatus::Paused);
        assert!(!job.cancel_requested);
        downloader::resume(t.handle(), &id);
        t.wait_job(&id, running, "the retry after restart");
    }
}

#[test]
fn shutdown_that_cannot_save_keeps_cancelled_files() {
    let Some(t) = TestApp::new("b4-shutdown") else { return };
    let id = t.enqueue("https://fixture.invalid/hang/delta");
    let job = t.wait_job(&id, running, "the download to run");
    let workspace = job.workspace.unwrap().path;
    t.block("queue.json");
    downloader::remove(t.handle(), &id).unwrap();
    downloader::shutdown(t.handle());
    assert!(workspace.join(crate::download_files::MARKER).is_file(), "deleted while the queue could not be saved");
}

#[test]
fn a_published_completion_outlives_a_late_cancel_and_library_failure() {
    let Some(t) = TestApp::new("published-cancel") else { return };
    let (id, workspace) = published_but_not_recorded(&t, "latecancel");
    // The persisted state from Cancel during publication plus a failed Library save.
    t.state().jobs.lock().unwrap().iter_mut().find(|j| j.id == id).unwrap().cancel_requested = true;
    downloader::flush_queue(t.handle()).unwrap();
    t.unblock("history.json");
    let t = t.restart();
    assert_eq!(t.job(&id).unwrap().status, JobStatus::Completed);
    assert_eq!(library_entries(&t, &id), 1);
    assert!(!t.job(&id).unwrap().cancel_requested);
    wait_until("the completed workspace cleanup", || !workspace.exists());
}

#[test]
fn a_late_low_space_notification_does_not_fail_a_completed_transfer() {
    let Some(t) = TestApp::new("late-budget") else { return };
    t.low_space_delay.store(300, std::sync::atomic::Ordering::SeqCst);
    // The delayed low-space sample resolves after the fast fixture has already exited.
    t.set_free(&[2 * GIB, 100 * MIB]);
    let id = t.enqueue("https://fixture.invalid/ok-fast/fastbudget");
    let job = t.wait_job(&id, |j| matches!(j.status, JobStatus::Completed | JobStatus::Failed), "the fast transfer");
    if job.status == JobStatus::Completed {
        assert_eq!(std::fs::read(t.out.join("fastbudget.mp4")).unwrap(), b"merged");
    } else {
        assert!(job.workspace.unwrap().path.join("fastbudget.mp4").is_file());
        panic!("valid completed output was rejected: {:?}", job.error);
    }
}

#[test]
fn a_missing_published_output_is_not_restored_as_completed() {
    let Some(t) = TestApp::new("missing-output") else { return };
    let (id, workspace) = published_but_not_recorded(&t, "missingoutput");
    std::fs::remove_file(t.out.join("missingoutput.mp4")).unwrap();
    t.unblock("history.json");
    let t = t.restart();
    assert_eq!(t.job(&id).unwrap().status, JobStatus::Failed);
    assert_eq!(library_entries(&t, &id), 0);
    assert!(workspace.exists());
    downloader::resume(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Failed, "verification failure");
    assert_eq!(t.starts().len(), 1);
}

#[test]
fn cancel_after_publication_does_not_delete_unrecorded_completion() {
    let Some(t) = TestApp::new("cancel-published") else { return };
    let (id, workspace) = published_but_not_recorded(&t, "savedcancel");
    downloader::cancel(t.handle(), &id);
    assert!(!t.job(&id).unwrap().cancel_requested);
    assert_eq!(t.job(&id).unwrap().status, JobStatus::Failed);
    assert!(workspace.exists());
    t.unblock("history.json");
    let t = t.restart();
    assert_eq!(t.job(&id).unwrap().status, JobStatus::Completed);
    assert_eq!(library_entries(&t, &id), 1);
}

#[test]
fn startup_recovery_returns_before_library_work_and_prevents_duplicate_starts() {
    let Some(t) = TestApp::new("background-recovery") else { return };
    let (id, _) = published_but_not_recorded(&t, "background");
    t.unblock("history.json");
    let queue = t.saved_queue();
    let state = t.state();
    wait_until("the original worker to settle", || state.workers.load(std::sync::atomic::Ordering::SeqCst) == 0);
    // Hold a leaf lock that the recovery worker needs. Startup must return anyway.
    let mut history = state.history.lock().unwrap();
    // A real restart reloads the disk Library, losing the failed in-memory insert.
    history.clear();
    let recovery = downloader::restore_queue(t.handle(), queue);
    assert_eq!(t.job(&id).unwrap().status, JobStatus::Processing);
    downloader::resume(t.handle(), &id);
    assert_eq!(t.starts().len(), 1, "recovery must not start another download");
    drop(history);
    recovery.join().unwrap();
    assert_eq!(t.job(&id).unwrap().status, JobStatus::Completed);
    assert_eq!(library_entries(&t, &id), 1);
    assert_eq!(t.starts().len(), 1);
}

// ---- Video presets ------------------------------------------------------------------------

fn original(max_height: Option<u32>) -> DownloadOptions {
    DownloadOptions { max_height, video_preset: crate::download_policy::VideoPreset::Original, ..DownloadOptions::default() }
}

#[test]
fn a_paused_original_download_resumes_with_the_same_selection_after_settings_change_and_restart() {
    let Some(t) = TestApp::new("preset-frozen") else { return };
    let id = t.enqueue_with("https://fixture.invalid/hang/hotel", original(Some(4320)));
    t.wait_job(&id, running, "the first start");
    downloader::pause(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Paused, "the pause");
    {
        // Every setting that shapes a download, changed while it is paused.
        let state = t.state();
        let mut settings = state.settings.lock().unwrap();
        settings.audio_selection = crate::download_policy::AudioSelection::Best;
        settings.embed_metadata = true;
        settings.filename_template = "%(id)s.%(ext)s".into();
    }
    let t = t.restart();
    let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(t.dir.join("queue.json")).unwrap()).unwrap();
    assert_eq!(saved["schemaVersion"], 3);
    assert_eq!(saved["jobs"][0]["options"]["videoPreset"], "original");
    let job = t.job(&id).expect("still queued");
    assert_eq!(job.options.video_preset, crate::download_policy::VideoPreset::Original);
    assert_eq!(job.options.max_height, Some(4320));

    downloader::resume(t.handle(), &id);
    wait_until("the resumed start", || t.starts().len() == 2);
    let starts = t.starts();
    assert_eq!(starts.len(), 2, "{starts:?}");
    for start in &starts {
        assert!(start.ends_with("sort=res:4320 merge=mp4/mkv"), "every start uses the preset it was queued with: {start}");
    }
}

#[test]
fn an_original_download_saved_as_mkv_reaches_the_folder_and_library() {
    let Some(t) = TestApp::new("preset-mkv") else { return };
    let id = t.enqueue_with("https://fixture.invalid/ok-fast/india/4096/mkv", original(None));
    let job = t.wait_job(&id, |j| j.status == JobStatus::Completed, "the download");
    let saved = PathBuf::from(job.file_path.expect("a saved file"));
    assert_eq!(saved, t.out.join("india.mkv"));
    assert_eq!(std::fs::read(&saved).unwrap(), b"merged");
    let history = { t.state().history.lock().unwrap().clone() };
    let item = history.iter().find(|h| h.id == id).expect("in the Library");
    assert_eq!(item.quality, "Best · Original");
    assert_eq!(item.mode, "video");
    assert_eq!(item.file_path.as_deref(), Some(saved.to_string_lossy().as_ref()));
}

#[test]
fn a_saved_job_with_a_preset_this_version_cannot_make_fails_without_starting() {
    let Some(t) = TestApp::new("preset-unavailable") else { return };
    let id = t.enqueue("https://fixture.invalid/hang/juliet");
    t.wait_job(&id, running, "the first start");
    downloader::pause(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Paused, "the pause");
    // As a later version that can make it would have saved it.
    let t = t.restart_after(|dir| {
        let path = dir.join("queue.json");
        let mut saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        saved["jobs"][0]["options"]["videoPreset"] = "editingProRes".into();
        std::fs::write(path, saved.to_string()).unwrap();
    });
    let job = t.job(&id).expect("the saved job still loads");
    assert_eq!(job.options.video_preset, crate::download_policy::VideoPreset::EditingProRes);
    downloader::resume(t.handle(), &id);
    let job = t.wait_job(&id, |j| j.status == JobStatus::Failed, "the refusal");
    assert!(job.error.unwrap_or_default().contains("editing preset"));
    assert_eq!(t.starts().len(), 1, "yt-dlp was never started for it");
}

// ---- Transcripts ----------------------------------------------------------------------------

fn transcript(code: &str, auto: bool, format: crate::transcript::TranscriptFormat) -> DownloadOptions {
    DownloadOptions { mode: "transcript".into(), subtitle_langs: vec![code.into()], auto_subs: auto, transcript_format: format, ..DownloadOptions::default() }
}

/// Files in the output folder, apart from the fixture's log and Claws' work folders.
fn published(t: &TestApp) -> Vec<String> {
    if !t.out.is_dir() {
        return Vec::new();
    }
    crate::download_tests::names(&t.out)
        .into_iter()
        .filter(|n| n != "fixture-starts.log" && n != crate::download_files::WORK_DIR)
        .collect()
}

#[test]
fn a_transcript_is_saved_to_the_folder_and_library_without_any_media() {
    use crate::transcript::TranscriptFormat::Txt;
    let Some(t) = TestApp::new("transcript-ok") else { return };
    let id = t.enqueue_with("https://fixture.invalid/captions/papa", transcript("en", false, Txt));
    let job = t.wait_job(&id, |j| j.status == JobStatus::Completed, "the transcript");
    let saved = t.out.join("papa [en].txt");
    assert_eq!(job.file_path.as_deref(), Some(saved.to_string_lossy().as_ref()));
    let text = std::fs::read_to_string(&saved).unwrap();
    assert_eq!(text, "Ana: Hello & welcome\r\nこれは字幕です 🎬\r\n");
    assert_eq!(published(&t), ["papa [en].txt"], "only the transcript is published, not the raw captions");
    // No media bytes were ever reported.
    assert_eq!((job.downloaded_bytes, job.total_bytes), (None, None));
    let starts = t.starts();
    assert_eq!(starts.len(), 1);
    assert!(starts[0].contains("skip=yes subs=en kind=human sort=- merge=-"), "{starts:?}");
    let history = { t.state().history.lock().unwrap().clone() };
    let item = history.iter().find(|h| h.id == id).expect("in the Library");
    assert_eq!(item.mode, "transcript");
    assert_eq!(item.quality, "Transcript · English · TXT");
    assert_eq!(item.file_size, Some(text.len() as u64));
    let workspace = job.workspace.unwrap().path;
    wait_until("the work folder to go", || !workspace.exists());
}

#[test]
fn automatic_captions_are_named_as_automatic_and_lose_their_rolling_repeats() {
    use crate::transcript::TranscriptFormat::TxtTimestamps;
    let Some(t) = TestApp::new("transcript-auto") else { return };
    let id = t.enqueue_with("https://fixture.invalid/captions/quebec", transcript("en", true, TxtTimestamps));
    t.wait_job(&id, |j| j.status == JobStatus::Completed, "the transcript");
    let text = std::fs::read_to_string(t.out.join("quebec [en auto, timestamps].txt")).unwrap();
    assert_eq!(text, "[00:00:00] hello everyone\r\n[00:00:02] and welcome back\r\n");
    assert!(t.starts()[0].contains("subs=en kind=auto"), "{:?}", t.starts());
    let history = { t.state().history.lock().unwrap().clone() };
    assert_eq!(history.iter().find(|h| h.id == id).unwrap().quality, "Transcript · English (auto-generated) · TXT with timestamps");
}

#[test]
fn a_taken_transcript_name_is_never_overwritten_and_try_again_publishes_it() {
    use crate::transcript::TranscriptFormat::Srt;
    let Some(t) = TestApp::new("transcript-conflict") else { return };
    std::fs::create_dir_all(&t.out).unwrap();
    std::fs::write(t.out.join("romeo [en].srt"), b"the user's own").unwrap();
    let id = t.enqueue_with("https://fixture.invalid/captions/romeo", transcript("en", false, Srt));
    let job = t.wait_job(&id, |j| j.status == JobStatus::Failed, "the conflict");
    assert!(job.error.unwrap_or_default().contains("already exists"));
    assert_eq!(std::fs::read(t.out.join("romeo [en].srt")).unwrap(), b"the user's own");
    std::fs::rename(t.out.join("romeo [en].srt"), t.out.join("mine.srt")).unwrap();
    downloader::resume(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Completed, "the publication");
    let srt = std::fs::read_to_string(t.out.join("romeo [en].srt")).unwrap();
    assert!(srt.starts_with("1\r\n00:00:01,000 --> 00:00:02,000\r\nAna: Hello & welcome\r\n"), "{srt}");
    assert_eq!(std::fs::read(t.out.join("mine.srt")).unwrap(), b"the user's own");
    assert_eq!(t.starts().len(), 1, "Try Again only published");
}

#[test]
fn a_cancelled_transcript_removes_its_work_folder_and_publishes_nothing() {
    use crate::transcript::TranscriptFormat::Txt;
    let Some(t) = TestApp::new("transcript-cancel") else { return };
    let id = t.enqueue_with("https://fixture.invalid/captions-hang/sierra", transcript("en", false, Txt));
    let job = t.wait_job(
        &id,
        |j| j.status == JobStatus::Processing && j.workspace.as_ref().is_some_and(|w| w.path.join("sierra.en.vtt").is_file()),
        "the captions to arrive",
    );
    assert!(!job.progress_known && job.downloaded_bytes.is_none(), "no made-up byte progress");
    let workspace = job.workspace.unwrap().path;
    downloader::cancel(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Cancelled, "the cancel");
    wait_until("the work folder to go", || !workspace.exists());
    assert!(published(&t).is_empty(), "{:?}", published(&t));
}

#[test]
fn a_transcript_whose_library_save_fails_is_finished_by_try_again() {
    use crate::transcript::TranscriptFormat::Vtt;
    let Some(t) = TestApp::new("transcript-library") else { return };
    t.block("history.json");
    let id = t.enqueue_with("https://fixture.invalid/captions/tango", transcript("de", false, Vtt));
    let job = t.wait_job(&id, |j| j.status == JobStatus::Failed, "the Library save to fail");
    let error = job.error.clone().unwrap_or_default();
    assert!(error.contains("could not add it to your Library") && error.contains("Try Again"), "{error}");
    let saved = t.out.join("tango [de].vtt");
    assert!(std::fs::read_to_string(&saved).unwrap().starts_with("WEBVTT\r\n\r\n00:00:01.000 --> 00:00:02.000\r\nAna: Hello &amp; welcome\r\n"));
    let workspace = job.workspace.unwrap().path;
    let manifest = crate::download_files::read_manifest(&workspace).expect("the manifest is kept");
    assert_eq!(manifest.state, crate::download_files::Publication::Published);
    assert_eq!(manifest.outputs, ["tango [de].vtt"], "the raw captions are not an output");
    t.unblock("history.json");
    downloader::resume(t.handle(), &id);
    let job = t.wait_job(&id, |j| j.status == JobStatus::Completed, "the completion");
    assert_eq!(job.file_path.as_deref(), Some(saved.to_string_lossy().as_ref()));
    assert_eq!(library_entries(&t, &id), 1);
    assert_eq!(t.starts().len(), 1, "Try Again fetched the captions again");
    wait_until("the work folder to go", || !workspace.exists());
}

#[test]
fn empty_or_missing_captions_fail_clearly_and_publish_nothing() {
    use crate::transcript::TranscriptFormat::Txt;
    let Some(t) = TestApp::new("transcript-empty") else { return };
    let empty = t.enqueue_with("https://fixture.invalid/captions-empty/uniform", transcript("en", false, Txt));
    let job = t.wait_job(&empty, |j| j.status == JobStatus::Failed, "the empty captions");
    assert_eq!(job.error.as_deref(), Some(crate::transcript::EMPTY));
    let none = t.enqueue_with("https://fixture.invalid/captions-none/victor", transcript("fr", true, Txt));
    let job = t.wait_job(&none, |j| j.status == JobStatus::Failed, "the missing captions");
    let error = job.error.unwrap_or_default();
    assert!(error.contains("automatic captions") && error.contains("fr"), "{error}");
    assert!(published(&t).is_empty(), "{:?}", published(&t));
}

#[test]
fn a_transcript_job_resumes_with_its_choices_after_a_restart() {
    use crate::transcript::TranscriptFormat::Srt;
    let Some(t) = TestApp::new("transcript-restart") else { return };
    let id = t.enqueue_with("https://fixture.invalid/captions-hang/whiskey", transcript("en", true, Srt));
    t.wait_job(&id, |j| j.status == JobStatus::Processing, "the first start");
    let t = t.restart();
    let job = t.job(&id).expect("still queued");
    assert_eq!(job.status, JobStatus::Paused);
    assert_eq!(job.options.transcript_format, Srt);
    assert!(job.options.auto_subs);
    let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(t.dir.join("queue.json")).unwrap()).unwrap();
    assert_eq!(saved["jobs"][0]["options"]["mode"], "transcript");
    assert_eq!(saved["jobs"][0]["options"]["transcriptFormat"], "srt");
}

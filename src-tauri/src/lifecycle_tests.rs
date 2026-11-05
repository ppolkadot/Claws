//! Lifecycle tests: the real downloader, queue writer and stores, driven through a mock
//! Tauri app with its own disposable data folder, and the fixture tool standing in for
//! yt-dlp (`cargo build --features test-tools --bin claws-fixture-tool`; without it these
//! tests skip). The owner's real queue, library and downloads are never touched.
//!
//! Faults are real file-system failures: a folder put where a JSON file has to be renamed
//! makes that save fail exactly as a denied write or a full disk would.

use std::path::{Path, PathBuf};
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
        app.manage(AppState::new(dir.clone(), settings, history, cleanup, writer, false));
        downloader::restore_queue(app.handle(), queue);
        Self { app, dir, out }
    }

    /// Closes this app the way Claws exits and starts a new one on the same data.
    pub fn restart(self) -> Self {
        let (dir, out) = (self.dir.clone(), self.out.clone());
        downloader::shutdown(self.handle());
        std::mem::forget(self); // its folder is reused, so Drop must not remove it
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
        let item = EnqueueItem { url: link.into(), playlist_index: None, title: link.into(), thumbnail: None, uploader: None, duration: None };
        let added = downloader::enqueue(self.handle(), vec![item], DownloadOptions::default()).expect("enqueue");
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

    /// The queue as it is on disk.
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

// --- B4: required saves ----------------------------------------------------------------

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

    // Once saving works, Try Again starts in the same frozen folder.
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

    // Removing it once saving works again deletes the files.
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

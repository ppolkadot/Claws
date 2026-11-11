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

fn spotify_origin_value() -> serde_json::Value {
    serde_json::json!({"kind":"spotifyYouTube","spotifyUrl":"https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT","spotifyTitle":"Believe","spotifyArtists":["Cher"]})
}

/// Opt-in integration: real tools and a deliberately chosen YouTube recording,
/// isolated from the owner's queue, cookies, settings and download folder.
#[test]
#[ignore = "live audio save; set CLAWS_SPOTIFY_LIVE_TOOLS to an installed Claws bin folder"]
fn spotify_live_audio_save_keeps_selected_source() {
    fn copy_tools(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let destination = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tools(&entry.path(), &destination);
            } else {
                std::fs::copy(entry.path(), destination).unwrap();
            }
        }
    }
    let tools =
        PathBuf::from(std::env::var_os("CLAWS_SPOTIFY_LIVE_TOOLS").expect("provide tools folder"));
    let dir = temp_dir("spotify-live-audio");
    copy_tools(&tools, &dir.join("bin"));
    let app = TestApp::with_data(dir.clone(), dir.join("out"), Vec::new());
    // Explicit test selection from the live search; no product auto-match behavior.
    let source = "https://www.youtube.com/watch?v=n7wvAEDOxAs";
    let preview =
        tauri::async_runtime::block_on(crate::probe::probe(app.handle(), source)).unwrap();
    assert_eq!(preview.url, source);
    assert_eq!(preview.audio_available, Some(true));
    for format in ["mp3", "m4a", "opus", "flac", "wav", "best"] {
        for fast_save in [false, true] {
            {
                let state = app.state();
                let mut settings = state.settings.lock().unwrap();
                settings.embed_metadata = !fast_save;
                settings.output_dir = app
                    .out
                    .join(format!("{format}-fast-{fast_save}"))
                    .to_string_lossy()
                    .into_owned();
            }
            let item: EnqueueItem = serde_json::from_value(serde_json::json!({
                "url":source,"title":preview.title,"uploader":preview.uploader,"duration":preview.duration,
                "matchOrigin":spotify_origin_value()
            })).unwrap();
            let jobs = downloader::enqueue(
                app.handle(),
                vec![item],
                DownloadOptions {
                    mode: "audio".into(),
                    audio_format: format.into(),
                    fast_save,
                    ..Default::default()
                },
            )
            .unwrap();
            let id = &jobs.jobs[0].job_id;
            let deadline = Instant::now() + Duration::from_secs(180);
            loop {
                let job = app.job(id).unwrap();
                if matches!(job.status, JobStatus::Completed | JobStatus::Failed) {
                    assert_eq!(
                        job.status,
                        JobStatus::Completed,
                        "live {format} fast_save={fast_save} save failed: {:?}",
                        job.error
                    );
                    assert_eq!(job.url, source);
                    let entry = library_entry(&app, id);
                    assert_eq!(entry.url, source);
                    assert_eq!(
                        serde_json::to_value(&entry).unwrap()["matchOrigin"],
                        spotify_origin_value()
                    );
                    let path = PathBuf::from(entry.file_path.unwrap());
                    assert!(std::fs::metadata(&path).unwrap().len() > 0);
                    let output = std::process::Command::new(app.dir.join("bin/ffprobe.exe"))
                        .args([
                            "-v",
                            "error",
                            "-select_streams",
                            "a:0",
                            "-show_entries",
                            "stream=codec_name",
                            "-of",
                            "default=nw=1:nk=1",
                        ])
                        .arg(path)
                        .output()
                        .unwrap();
                    assert!(output.status.success());
                    let codec = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    match format {
                        "mp3" | "opus" | "flac" => assert_eq!(codec, format),
                        "m4a" => assert_eq!(codec, "aac"),
                        "wav" => assert!(codec.starts_with("pcm_")),
                        _ => assert!(!codec.is_empty()),
                    }
                    eprintln!("Live {format} fast_save={fast_save} saved and ffprobed ({codec}); chosen URL and provenance retained.");
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "live save exceeded three minutes"
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    let app = app.restart();
    let state = app.state();
    let entries = state.history.lock().unwrap();
    assert_eq!(entries.len(), 12);
    assert!(entries
        .iter()
        .all(|entry| entry.url == source && entry.match_origin.is_some()));
    eprintln!(
        "All 12 live format/Fast Save variants retain their source and provenance after restart."
    );
}

#[test]
fn spotify_origin_queue_restart_history_and_updates_keep_chosen_source() {
    let dir = temp_dir("spotify-provenance");
    let out = dir.join("out");
    let t = TestApp::with_data(dir, out, Vec::new());
    // Occupy the single worker slot with a paused-independent active placeholder so
    // enqueue is tested without spawning yt-dlp or contacting a live service.
    let blocker: Job = serde_json::from_value(serde_json::json!({"id":"blocker","url":"https://fixture.invalid/blocker","title":"Blocker","options":{},"status":"downloading","progress":0,"createdAt":0})).unwrap();
    t.state().jobs.lock().unwrap().push(blocker);
    let item = || {
        serde_json::from_value::<EnqueueItem>(serde_json::json!({"url":"https://www.youtube.com/watch?v=abcdefghijk","title":"Chosen YouTube title","matchOrigin":spotify_origin_value()})).unwrap()
    };
    let audio = DownloadOptions {
        mode: "audio".into(),
        ..Default::default()
    };
    let added = downloader::enqueue(t.handle(), vec![item()], audio.clone()).unwrap();
    let id = added.jobs[0].job_id.clone();
    assert_eq!(
        serde_json::to_value(&added.jobs[0].job).unwrap()["matchOrigin"],
        spotify_origin_value()
    );
    let ordinary: EnqueueItem = serde_json::from_value(serde_json::json!({"url":"https://www.youtube.com/watch?v=abcdefghijk","title":"Ordinary duplicate"})).unwrap();
    assert!(downloader::enqueue(t.handle(), vec![ordinary], audio.clone()).is_err());
    assert_eq!(t.job(&id).unwrap().title, "Chosen YouTube title");
    let t = t.restart();
    let job = t.job(&id).unwrap();
    assert_eq!(job.url, "https://www.youtube.com/watch?v=abcdefghijk");
    assert_eq!(
        serde_json::to_value(&job).unwrap()["matchOrigin"],
        spotify_origin_value()
    );
    crate::history::record(t.handle(), &job).unwrap();
    crate::history_mutation::mutate(t.handle(), |items| {
        items[0].thumbnail_local = Some("local.jpg".into());
        ((), true)
    })
    .unwrap();
    let saved: Vec<crate::history::HistoryItem> =
        crate::store::load_or_recover(&t.dir.join("history.json"), "history", &mut Vec::new());
    assert_eq!(saved[0].url, job.url);
    assert_eq!(
        serde_json::to_value(&saved[0]).unwrap()["matchOrigin"],
        spotify_origin_value()
    );
    assert_eq!(saved[0].thumbnail_local.as_deref(), Some("local.jpg"));
    // A failed attempt remains tied to the exact accepted source on retry. Occupy
    // the slot again so this state transition cannot start a real media process.
    {
        let state = t.state();
        let mut jobs = state.jobs.lock().unwrap();
        jobs.iter_mut().find(|j| j.id == "blocker").unwrap().status = JobStatus::Downloading;
        jobs.iter_mut().find(|j| j.id == id).unwrap().status = JobStatus::Failed;
    }
    downloader::resume(t.handle(), &id);
    assert_eq!(t.job(&id).unwrap().url, job.url);
    assert_eq!(
        serde_json::to_value(t.job(&id).unwrap()).unwrap()["matchOrigin"],
        spotify_origin_value()
    );
}

#[test]
fn spotify_origin_matched_duplicate_preserves_ordinary_first_acceptance() {
    let dir = temp_dir("spotify-duplicate");
    let out = dir.join("out");
    let t = TestApp::with_data(dir, out, Vec::new());
    let blocker: Job = serde_json::from_value(serde_json::json!({"id":"blocker","url":"https://fixture.invalid/blocker","title":"Blocker","options":{},"status":"downloading","progress":0,"createdAt":0})).unwrap();
    t.state().jobs.lock().unwrap().push(blocker);
    let ordinary: EnqueueItem = serde_json::from_value(
        serde_json::json!({"url":"https://www.youtube.com/watch?v=abcdefghijk","title":"Ordinary"}),
    )
    .unwrap();
    let matched: EnqueueItem = serde_json::from_value(serde_json::json!({"url":"https://www.youtube.com/watch?v=abcdefghijk","title":"Matched","matchOrigin":spotify_origin_value()})).unwrap();
    let added = downloader::enqueue(
        t.handle(),
        vec![ordinary, matched],
        DownloadOptions {
            mode: "audio".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(added.jobs.len(), 1);
    assert_eq!(added.jobs[0].job.title, "Ordinary");
    assert!(serde_json::to_value(&added.jobs[0].job).unwrap()["matchOrigin"].is_null());
    let old: crate::history::HistoryItem = serde_json::from_value(serde_json::json!({"id":"old","title":"Old","url":"https://example.test/old","mode":"video","quality":"Best","finishedAt":0})).unwrap();
    assert!(serde_json::to_value(old).unwrap()["matchOrigin"].is_null());
}

#[test]
fn spotify_origin_published_file_ownership_survives_library_restart_and_repair() {
    let Some(t) = TestApp::new("spotify-published") else {
        return;
    };
    let item: EnqueueItem=serde_json::from_value(serde_json::json!({"url":"https://www.youtube.com/watch?v=abcdefghijk","title":"Chosen recording","matchOrigin":spotify_origin_value()})).unwrap();
    let added = downloader::enqueue(
        t.handle(),
        vec![item],
        DownloadOptions {
            mode: "audio".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let id = added.jobs[0].job_id.clone();
    t.wait_job(
        &id,
        |j| j.status == JobStatus::Completed,
        "matched fixture publication",
    );
    let entry = library_entry(&t, &id);
    assert!(entry.file_identity.is_some());
    let identity = entry.file_identity.clone();
    let path = entry.file_path.clone().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    downloader::clear_finished(t.handle());
    let t = t.restart();
    crate::history::repair_kept_files(t.handle());
    let restored = library_entry(&t, &id);
    assert_eq!(restored.url, "https://www.youtube.com/watch?v=abcdefghijk");
    assert_eq!(restored.file_identity, identity);
    assert_eq!(
        serde_json::to_value(&restored).unwrap()["matchOrigin"],
        spotify_origin_value()
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

pub(crate) struct TestApp {
    app: tauri::App<tauri::test::MockRuntime>,
    pub dir: PathBuf,
    pub out: PathBuf,
    /// Free bytes the app reads, in order; the last value repeats. Plenty by default.
    pub free: Arc<Mutex<VecDeque<u64>>>,
    pub low_space_delay: Arc<std::sync::atomic::AtomicU64>,
    /// Holds free-space samples on a barrier and records where they ran.
    pub gate: Arc<SampleGate>,
}

/// A barrier for free-space samples, standing in for a slow (SMB-like) volume. Samples
/// are numbered from 0 in the order they start; those from `hold_from` on wait until the
/// gate is released. It also records samples that ran on a known async worker thread (which
/// a blocking disk call must never occupy) and can make the volume unreadable.
pub(crate) struct SampleGate {
    started: std::sync::atomic::AtomicUsize,
    on_worker: std::sync::atomic::AtomicUsize,
    workers: Mutex<std::collections::HashSet<std::thread::ThreadId>>,
    hold_from: std::sync::atomic::AtomicUsize,
    unreadable: std::sync::atomic::AtomicBool,
    panic_at: std::sync::atomic::AtomicUsize,
    released: Mutex<bool>,
    changed: std::sync::Condvar,
}

impl SampleGate {
    fn new() -> Self {
        Self {
            started: Default::default(),
            on_worker: Default::default(),
            workers: Default::default(),
            hold_from: std::sync::atomic::AtomicUsize::new(usize::MAX),
            unreadable: Default::default(),
            panic_at: std::sync::atomic::AtomicUsize::new(usize::MAX),
            released: Mutex::new(true),
            changed: Default::default(),
        }
    }

    fn enter(&self) -> std::io::Result<()> {
        use std::sync::atomic::Ordering::SeqCst;
        let index = self.started.fetch_add(1, SeqCst);
        if index == self.panic_at.load(SeqCst) {
            panic!("the free-space sample panicked");
        }
        if self.workers.lock().unwrap().contains(&std::thread::current().id()) {
            self.on_worker.fetch_add(1, SeqCst);
        }
        let mut released = self.released.lock().unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while index >= self.hold_from.load(SeqCst) && !*released && Instant::now() < deadline {
            released = self.changed.wait_timeout(released, Duration::from_millis(50)).unwrap().0;
        }
        drop(released);
        if self.unreadable.load(SeqCst) {
            return Err(std::io::Error::other("the volume cannot be read"));
        }
        Ok(())
    }

    pub fn hold_from(&self, index: usize) {
        *self.released.lock().unwrap() = false;
        self.hold_from.store(index, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn panic_at(&self, index: usize) {
        self.panic_at.store(index, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }

    pub fn started(&self) -> usize {
        self.started.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn on_worker(&self) -> usize {
        self.on_worker.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Learns the runtime's async worker threads by polling many sleeping tasks (a thread of
    /// the blocking pool never runs one), so a sample on one of them can be told apart.
    /// Workers missed here only make the check less sensitive, never wrongly fail it.
    pub fn learn_workers(&self) -> usize {
        for _ in 0..40 {
            let (tx, rx) = std::sync::mpsc::channel();
            for _ in 0..64 {
                let tx = tx.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(2)).await;
                    let _ = tx.send(std::thread::current().id());
                });
            }
            drop(tx);
            self.workers.lock().unwrap().extend(rx.iter());
        }
        self.workers.lock().unwrap().len()
    }
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
        let gate = Arc::new(SampleGate::new());
        let sampler = gate.clone();
        state.free_space = Box::new(move |_| {
            sampler.enter()?;
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
        crate::history_mutation::reconcile(app.handle()).join().expect("history journal recovery");
        crate::history::start_kept_repair(app.handle()).join().expect("kept-files repair");
        Self { app, dir, out, free, low_space_delay, gate }
    }

    pub fn set_free(&self, readings: &[u64]) {
        *self.free.lock().unwrap() = readings.iter().copied().collect();
    }

    pub fn restart(self) -> Self {
        self.restart_after(|_| {})
    }

    pub fn restart_after(self, change: impl FnOnce(&Path)) -> Self {
        let (dir, out) = (self.dir.clone(), self.out.clone());
        downloader::shutdown(self.handle());
        std::mem::forget(self); // its folder is reused, so Drop must not remove it
        change(&dir);
        let mut notices = Vec::new();
        let queue = crate::queue_writer::load(&dir.join("queue.json"), &mut notices).expect("queue load");
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
        let item = EnqueueItem { url: link.into(), match_origin: None, playlist_index: None, title: link.into(), thumbnail: None, uploader: None, duration: None };
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

    pub fn starts(&self) -> Vec<String> {
        std::fs::read_to_string(self.out.join("fixture-starts.log"))
            .map(|t| t.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

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
        self.gate.release();
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
    use crate::download_files::{verify, write_manifest, MoveState, Publication, WorkspaceRef};
    let Some(t) = TestApp::new("b3-app") else { return };
    let (id, workspace) = published_but_not_recorded(&t, "november");
    t.unblock("history.json");
    // Rewind the journal to the instant between the move and its record.
    let mut manifest = verify(&WorkspaceRef { schema_version: 1, path: workspace.clone(), attempt: 0 }, &id).unwrap();
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

/// Counts ticks of an unrelated async task, to show the runtime keeps running.
fn heartbeat() -> Arc<std::sync::atomic::AtomicUsize> {
    let beats = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let ticks = beats.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(10)).await;
            ticks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    });
    beats
}

fn held(t: &TestApp, id: &str) -> Option<u64> {
    t.state().disk_ledger.lock().unwrap().held(id)
}

fn ledger_is_locked(t: &TestApp) -> bool {
    matches!(t.state().disk_ledger.try_lock(), Err(std::sync::TryLockError::WouldBlock))
}

#[test]
fn space_is_never_sampled_on_an_async_worker() {
    let Some(t) = TestApp::new("w6-off-worker") else { return };
    let workers = t.gate.learn_workers();
    if workers < 2 {
        // A one-core runner has a single async worker: the check cannot tell it from the pool.
        eprintln!("skipped: could not tell the async workers apart ({workers} found)");
        return;
    }
    // Admission samples once; the merge's recheck samples again.
    let id = t.enqueue("https://fixture.invalid/ok-fast/golf");
    t.wait_job(&id, |j| j.status == JobStatus::Completed, "the download");
    assert!(t.gate.started() >= 2, "expected an admission and a processing sample, saw {}", t.gate.started());
    assert_eq!(t.gate.on_worker(), 0, "a free-space sample ran on an async worker");
}

#[test]
fn space_a_blocked_admission_sample_leaves_async_work_and_ledger_free_commands_running() {
    let Some(t) = TestApp::new("w6-blocked") else { return };
    t.gate.hold_from(0);
    let id = t.enqueue(&format!("https://fixture.invalid/hang/hotel/{GIB}"));
    wait_until("the admission sample to start", || t.gate.started() >= 1);
    let beats = heartbeat();
    let before = beats.load(std::sync::atomic::Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(500));
    let ticks = beats.load(std::sync::atomic::Ordering::SeqCst) - before;
    assert!(ticks >= 20, "the runtime stalled behind the sample: {ticks} ticks in 500 ms");
    // Commands that never touch the ledger answer while the sample is blocked.
    let started = Instant::now();
    assert!(downloader::list(t.handle()).iter().any(|j| j.id == id));
    assert!(started.elapsed() < Duration::from_millis(500));
    // The reading and the decision stay together under the ledger's lock. Anything that
    // needs the ledger (not asked here) waits for the sample: the remaining, documented bound.
    assert!(ledger_is_locked(&t), "the sample was taken outside the ledger's lock");

    t.gate.release();
    t.wait_job(&id, running, "the download after the sample");
    assert!(held(&t, &id).is_some(), "the admitted job is reserved");
    downloader::cancel(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Cancelled, "the cancel");
}

#[test]
fn space_two_admissions_cannot_reserve_the_same_bytes() {
    let Some(t) = TestApp::new("w6-same-bytes") else { return };
    // Each plans 1 GiB and needs about 1.5 GiB with its margin: only one fits in 2.5 GiB.
    t.set_free(&[2 * GIB + GIB / 2]);
    t.gate.hold_from(0);
    let first = t.enqueue(&format!("https://fixture.invalid/hang/india/{GIB}"));
    let second = t.enqueue(&format!("https://fixture.invalid/hang/juliet/{GIB}"));
    wait_until("a sample to start", || t.gate.started() >= 1);
    // One sample is out under the ledger's lock; the other admission waits for the lock.
    std::thread::sleep(Duration::from_millis(200));
    assert!(ledger_is_locked(&t), "the sample was taken outside the ledger's lock");
    assert_eq!(t.gate.started(), 1, "a second sample started beside the first");
    t.gate.release();
    wait_until("exactly one job to be refused", || {
        [&first, &second].iter().filter(|id| t.job(id).is_some_and(|j| j.status == JobStatus::Failed)).count() == 1
    });
    let (kept, refused) = if t.job(&first).unwrap().status == JobStatus::Failed { (&second, &first) } else { (&first, &second) };
    let error = t.job(refused).unwrap().error.unwrap_or_default();
    assert!(error.contains("Not enough free space"), "{error}");
    assert!(held(&t, kept).is_some(), "the admitted job holds its reservation");
    assert_eq!(held(&t, refused), None);
    downloader::cancel(t.handle(), kept);
    t.wait_job(kept, |j| j.status == JobStatus::Cancelled, "the cancel");
}

#[test]
fn space_an_unreadable_volume_is_watched_not_refused() {
    let Some(t) = TestApp::new("w6-unreadable") else { return };
    t.gate.unreadable.store(true, std::sync::atomic::Ordering::SeqCst);
    let id = t.enqueue(&format!("https://fixture.invalid/hang/kilo/{GIB}"));
    t.wait_job(&id, running, "the download to run without a reading");
    assert_eq!(held(&t, &id), None, "an unreadable volume reserves nothing");
    downloader::cancel(t.handle(), &id);
    t.wait_job(&id, |j| j.status == JobStatus::Cancelled, "the cancel");
}

#[test]
fn space_a_panicking_sample_neither_admits_silently_nor_keeps_another_jobs_claim() {
    let Some(t) = TestApp::new("w6-panic") else { return };
    // Sample 0 admits the first job; sample 1 panics under the ledger's lock (poisoning it).
    t.gate.panic_at(1);
    let first = t.enqueue(&format!("https://fixture.invalid/hang/nova/{GIB}"));
    t.wait_job(&first, running, "the first download");
    assert!(held(&t, &first).is_some(), "the first is reserved");
    let second = t.enqueue(&format!("https://fixture.invalid/hang/oscar2/{GIB}"));
    wait_until("the panicking sample", || t.gate.started() >= 2);
    // The panic is no reason to fail the job, and no reservation is made for it.
    t.wait_job(&second, running, "the second download to run on the disk watch");
    let claim = |id: &str| t.state().disk_ledger.lock().unwrap_or_else(std::sync::PoisonError::into_inner).held(id);
    assert_eq!(claim(&second), None);
    // The ledger is poisoned but sound: a later download is admitted and reserved as usual,
    // not sent to the disk watch because every admission would panic on the poisoned lock.
    assert!(t.state().disk_ledger.is_poisoned(), "the panicking sample was meant to poison the ledger");
    let third = t.enqueue(&format!("https://fixture.invalid/hang/papa3/{GIB}"));
    t.wait_job(&third, running, "the third download");
    wait_until("the third's reservation", || claim(&third).is_some());
    // The poisoned ledger still lets a finished job's claim go.
    downloader::cancel(t.handle(), &first);
    t.wait_job(&first, |j| j.status == JobStatus::Cancelled, "the cancel");
    wait_until("the first's claim to go", || claim(&first).is_none());
    downloader::cancel(t.handle(), &second);
    t.wait_job(&second, |j| j.status == JobStatus::Cancelled, "the second cancel");
    downloader::cancel(t.handle(), &third);
    t.wait_job(&third, |j| j.status == JobStatus::Cancelled, "the third cancel");
    wait_until("the third's claim to go", || claim(&third).is_none());
}

#[test]
fn space_a_cancel_while_the_sample_blocks_stays_cancelled_and_holds_nothing() {
    let Some(t) = TestApp::new("w6-cancel") else { return };
    // The reading would refuse the job, but the user has already cancelled it.
    t.set_free(&[100 * MIB]);
    t.gate.hold_from(0);
    let id = t.enqueue(&format!("https://fixture.invalid/hang/lima/{GIB}"));
    wait_until("the admission sample to start", || t.gate.started() >= 1);
    downloader::cancel(t.handle(), &id);
    // The process is stopped at once; the job settles once the sample in flight returns.
    t.gate.release();
    t.wait_job(&id, |j| j.status == JobStatus::Cancelled, "the cancel");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(t.job(&id).map(|j| j.status), Some(JobStatus::Cancelled), "the late sample changed a cancelled job");
    wait_until("no reservation to outlive the job", || held(&t, &id).is_none());
}

#[test]
fn space_a_job_that_finishes_before_its_processing_sample_returns_completes() {
    let Some(t) = TestApp::new("w6-finished-first") else { return };
    // Sample 0 is the admission; sample 1 is the merge's recheck, held, and then short.
    t.set_free(&[2 * GIB, 100 * MIB]);
    t.gate.hold_from(1);
    let id = t.enqueue("https://fixture.invalid/ok-fast/mike");
    wait_until("the processing sample to start", || t.gate.started() >= 2);
    // The tool finishes while the sample is out; its process is reaped and forgotten, so
    // a late "stop" has no process to kill (and must not touch a reused id).
    wait_until("the process to be reaped", || t.state().running.lock().unwrap().get(&id).is_some_and(|r| r.pid.is_none()));
    assert_ne!(t.job(&id).unwrap().status, JobStatus::Failed);
    t.gate.release();
    t.wait_job(&id, |j| j.status == JobStatus::Completed, "completion despite the late short reading");
    assert_eq!(std::fs::read(t.out.join("mike.mp4")).unwrap(), b"merged");
    assert_eq!(library_entries(&t, &id), 1);
    wait_until("the claim to go", || held(&t, &id).is_none());
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

/// Queue rows offer a drag with the job's id; it resolves through the Library entry the
/// job recorded (same id), never through a path from the page.
#[test]
fn a_completed_queue_job_drags_only_its_recorded_library_file() {
    let Some(t) = TestApp::new("drag-completed") else { return };
    let id = t.enqueue("https://fixture.invalid/ok-fast/drag");
    let job = t.wait_job(&id, |j| j.status == JobStatus::Completed, "the download");
    let saved = PathBuf::from(job.file_path.expect("a saved file"));
    let history = { t.state().history.lock().unwrap().clone() };
    assert_eq!(crate::file_drag::recorded_file(&history, &id).unwrap(), dunce::canonicalize(&saved).unwrap());
    assert_eq!(std::fs::read(&saved).unwrap(), b"merged", "resolving never touches the file");

    crate::history_mutation::delete_entry(t.handle(), &id, false).unwrap();
    let history = { t.state().history.lock().unwrap().clone() };
    let error = crate::file_drag::recorded_file(&history, &id).unwrap_err().to_string();
    assert!(error.contains("no longer in the Library"), "{error}");
    assert!(saved.is_file(), "removing the entry keeps the file");
}

/// The Library entry of a finished download carries what proves the file is the one this
/// download saved, so Delete File Too can later tell it from any file put in its place.
#[test]
fn a_completed_download_records_the_identity_that_proves_it_is_ours() {
    let Some(t) = TestApp::new("hm-evidence") else { return };
    let id = t.enqueue("https://fixture.invalid/ok-fast/evidence");
    let job = t.wait_job(&id, |j| j.status == JobStatus::Completed, "the download");
    let saved = PathBuf::from(job.file_path.expect("a saved file"));
    let recorded = |t: &TestApp| {
        let history = { t.state().history.lock().unwrap().clone() };
        history.into_iter().find(|h| h.id == id).expect("in the Library")
    };
    assert_eq!(recorded(&t).file_identity, Some(crate::download_files::identify(&saved).unwrap()));

    let t = t.restart();
    assert_eq!(recorded(&t).file_identity, Some(crate::download_files::identify(&saved).unwrap()));

    // A replacement at the same path is not the download; the real file is.
    std::fs::rename(&saved, t.out.join("moved-away.mp4")).unwrap();
    std::fs::write(&saved, b"merged").unwrap();
    assert!(crate::history_mutation::delete_entry(t.handle(), &id, true).is_err());
    assert_eq!(std::fs::read(&saved).unwrap(), b"merged");
    std::fs::remove_file(&saved).unwrap();
    std::fs::rename(t.out.join("moved-away.mp4"), &saved).unwrap();
    crate::history_mutation::delete_entry(t.handle(), &id, true).unwrap();
    assert!(!saved.exists());
    assert_eq!(library_entries(&t, &id), 0);
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


fn transcript(code: &str, auto: bool, format: crate::transcript::TranscriptFormat) -> DownloadOptions {
    DownloadOptions { mode: "transcript".into(), subtitle_langs: vec![code.into()], auto_subs: auto, transcript_format: format, ..DownloadOptions::default() }
}

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
    let events = record_job_events(&t);
    let id = t.enqueue_with("https://fixture.invalid/captions/papa", transcript("en", false, Txt));
    let job = t.wait_job(&id, |j| j.status == JobStatus::Completed, "the transcript");
    // No media is fetched, so the run shows as processing once it starts and never goes
    // back (the caption conversion is a pre-download postprocessor, but no transfer follows).
    let seen = events_of(&events, &id);
    let first = seen.iter().position(|j| j.status == JobStatus::Processing).expect("a processing event");
    assert!(seen[first..].iter().all(|j| matches!(j.status, JobStatus::Processing | JobStatus::Completed)), "{:?}", seen.iter().map(|j| j.status).collect::<Vec<_>>());
    let saved = t.out.join("papa [en].txt");
    assert_eq!(job.file_path.as_deref(), Some(saved.to_string_lossy().as_ref()));
    let text = std::fs::read_to_string(&saved).unwrap();
    assert_eq!(text, "Ana: Hello & welcome\r\nこれは字幕です 🎬\r\n");
    assert_eq!(published(&t), ["papa [en].txt"], "only the transcript is published, not the raw captions");
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

fn record_job_events(t: &TestApp) -> Arc<Mutex<Vec<Job>>> {
    use tauri::Listener;
    let events: Arc<Mutex<Vec<Job>>> = Default::default();
    let sink = events.clone();
    t.handle().listen("job-update", move |event| {
        let payload: serde_json::Value = serde_json::from_str(event.payload()).expect("an event payload");
        let job = serde_json::from_value(payload["job"].clone()).expect("a job in the event");
        sink.lock().unwrap().push(job);
    });
    events
}

/// One job's events in revision order: the order the interface applies them in, whichever
/// thread happened to emit first.
fn events_of(events: &Arc<Mutex<Vec<Job>>>, id: &str) -> Vec<Job> {
    let mut seen: Vec<Job> = events.lock().unwrap().iter().filter(|j| j.id == id).cloned().collect();
    seen.sort_by_key(|j| j.revision);
    seen
}

#[test]
fn final_processing_stays_final_when_transfer_lines_arrive_late() {
    let Some(t) = TestApp::new("w5-late") else { return };
    let events = record_job_events(&t);
    let id = t.enqueue("https://fixture.invalid/late-progress/latepipe");
    // The fixture announces the merge on stderr, then sends the last stream's progress on
    // stdout, with the process alive for a while after.
    wait_until("the late transfer lines", || t.out.join("fixture-late.log").is_file());
    std::thread::sleep(Duration::from_millis(300)); // Claws has read them
    let job = t.job(&id).unwrap();
    assert_eq!(job.status, JobStatus::Processing, "a late transfer line restored the transfer state");
    assert_eq!((job.speed, job.eta), (None, None), "late telemetry came back during processing");
    t.wait_job(&id, |j| j.status == JobStatus::Completed, "the completion");

    let seen = events_of(&events, &id);
    let first = seen.iter().position(|j| j.status == JobStatus::Processing).expect("a processing event");
    assert!(seen[..first].iter().all(|j| j.status == JobStatus::Downloading), "{:?}", seen[..first].iter().map(|j| j.status).collect::<Vec<_>>());
    let stage_bytes = seen[first].downloaded_bytes;
    for j in &seen[first..] {
        assert!(matches!(j.status, JobStatus::Processing | JobStatus::Completed), "state regressed to {:?}", j.status);
        if j.status == JobStatus::Processing {
            assert_eq!((j.speed, j.eta), (None, None), "speed or ETA reappeared during processing");
            assert_eq!(j.downloaded_bytes, stage_bytes, "late transfer figures overwrote the processing display");
        }
    }
}

#[test]
fn a_postprocessor_that_runs_before_the_download_does_not_end_the_transfer_stage() {
    let Some(t) = TestApp::new("w5-preconvert") else { return };
    let events = record_job_events(&t);
    let id = t.enqueue("https://fixture.invalid/ok-preconvert/preconvert");
    t.wait_job(&id, |j| j.status == JobStatus::Completed, "the completion");
    assert_eq!(std::fs::read(t.out.join("preconvert.mp4")).unwrap(), b"merged");

    let seen = events_of(&events, &id);
    assert_eq!(seen[0].status, JobStatus::Downloading, "a started job is downloading");
    let first = seen.iter().position(|j| j.status == JobStatus::Processing).expect("the merge shows as processing");
    let transfer = &seen[..first];
    assert!(transfer.iter().all(|j| j.status == JobStatus::Downloading), "the subtitle conversion showed as processing");
    assert!(
        transfer.iter().any(|j| j.downloaded_bytes.is_some() && j.speed.is_some()),
        "Pause and the transfer figures must be available while the streams download"
    );
    assert!(seen[first..].iter().all(|j| matches!(j.status, JobStatus::Processing | JobStatus::Completed)));
}

#[test]
fn space_is_not_rechecked_for_a_postprocessor_that_runs_before_the_transfer() {
    let Some(t) = TestApp::new("cleanup-preconvert-space") else { return };
    let id = t.enqueue("https://fixture.invalid/ok-preconvert/prespace");
    t.wait_job(&id, |j| j.status == JobStatus::Completed, "the completion");
    // One sample admits the download and one rechecks before the merge. The subtitle
    // conversion that precedes the transfer announces itself the same way, but the space
    // for the processing that follows the last byte is not in question yet, and a shortfall
    // found then would stop the job with a message about processing it has not reached.
    assert_eq!(t.gate.started(), 2, "the pre-download postprocessor triggered a processing-space check");
}

#[test]
fn a_network_failure_after_a_pre_download_postprocessor_is_not_reported_as_processing() {
    let Some(t) = TestApp::new("w5-prefail") else { return };
    let id = t.enqueue("https://fixture.invalid/fail-preconvert/prefail");
    let job = t.wait_job(&id, |j| j.status == JobStatus::Failed, "the transfer to fail");
    let error = job.error.clone().unwrap_or_default();
    assert!(error.contains("timed out"), "{error}");
    assert!(!error.contains("processing the file failed"), "a transfer failure was reported as a processing failure: {error}");
}


fn with_captions() -> DownloadOptions {
    DownloadOptions { subtitle_langs: vec!["en".into(), "es".into()], embed_subs: false, ..DownloadOptions::default() }
}

fn library_entry(t: &TestApp, id: &str) -> crate::history::HistoryItem {
    let history = { t.state().history.lock().unwrap().clone() };
    history.into_iter().find(|h| h.id == id).expect("in the Library")
}

fn library_entry_count(t: &TestApp, id: &str) -> usize {
    t.state().history.lock().unwrap().iter().filter(|h| h.id == id).count()
}

fn saved_library(t: &TestApp) -> Vec<crate::history::HistoryItem> {
    serde_json::from_str(&std::fs::read_to_string(t.dir.join("history.json")).unwrap()).unwrap()
}

/// Finishes a download whose `.en.srt` name is taken by the user's own file.
fn withheld_download(t: &TestApp, name: &str) -> (String, PathBuf) {
    std::fs::create_dir_all(&t.out).unwrap();
    std::fs::write(t.out.join(format!("{name}.en.srt")), b"the user's own").unwrap();
    let id = t.enqueue_with(&format!("https://fixture.invalid/ok-subs/{name}"), with_captions());
    let job = t.wait_job(&id, |j| j.status == JobStatus::Completed, "the download");
    (id, job.workspace.expect("the work folder is kept").path)
}

fn kept_text(item: &crate::history::HistoryItem) -> String {
    let (reveal, file) = crate::history::reveal_plan(item).expect("a kept-files record");
    assert_eq!(reveal.status, crate::history::KeptStatus::Shown, "{reveal:?}");
    std::fs::read_to_string(file.expect("the file to show")).unwrap()
}

#[test]
fn a_withheld_sidecar_survives_restart_clear_finished_and_a_library_reload_and_stays_reachable() {
    let Some(t) = TestApp::new("w4-withheld") else { return };
    let (id, workspace) = withheld_download(&t, "pine");
    // The media and the free side file were published; the user's file was not touched.
    assert_eq!(std::fs::read(t.out.join("pine.mp4")).unwrap(), b"merged");
    assert!(std::fs::read_to_string(t.out.join("pine.es.srt")).unwrap().contains("fixture es"));
    assert_eq!(std::fs::read(t.out.join("pine.en.srt")).unwrap(), b"the user's own");

    let entry = library_entry(&t, &id);
    let kept = entry.kept_files.clone().expect("the Library entry records the kept file");
    assert_eq!(kept.sources, ["pine.en.srt"], "the retained source, not the occupied destination");
    assert_eq!(kept.workspace.path, workspace);
    assert!(kept_text(&entry).contains("fixture en"));
    assert_eq!(saved_library(&t)[0].kept_files, Some(kept.clone()), "saved with the entry before anything was retired");
    assert!(workspace.join("pine.en.srt").is_file());

    // Clear Finished removes the Queue row and nothing else.
    downloader::clear_finished(t.handle());
    assert!(t.job(&id).is_none());
    assert_eq!(library_entry(&t, &id).kept_files, Some(kept.clone()));
    assert!(kept_text(&library_entry(&t, &id)).contains("fixture en"));

    // Restart: the Queue has nothing about the job, the Library still points at the file.
    let t = t.restart();
    assert!(t.job(&id).is_none());
    assert_eq!(library_entry(&t, &id).kept_files, Some(kept.clone()), "after restart");
    assert_eq!(saved_library(&t)[0].kept_files, Some(kept), "after reload");
    assert!(kept_text(&library_entry(&t, &id)).contains("fixture en"));
    downloader::retry_pending_cleanup(t.handle());
    assert!(workspace.join("pine.en.srt").is_file(), "a cleanup retry must not take the retained file");
    assert_eq!(std::fs::read(t.out.join("pine.en.srt")).unwrap(), b"the user's own");
    assert_eq!(std::fs::read(t.out.join("pine.mp4")).unwrap(), b"merged");
}

#[test]
fn a_download_whose_sidecars_were_all_published_keeps_nothing_and_cleans_up() {
    let Some(t) = TestApp::new("w4-no-conflict") else { return };
    let id = t.enqueue_with("https://fixture.invalid/ok-subs/quiet", with_captions());
    let job = t.wait_job(&id, |j| j.status == JobStatus::Completed, "the download");
    assert!(library_entry(&t, &id).kept_files.is_none());
    let workspace = job.workspace.unwrap().path;
    wait_until("the work folder to go", || !workspace.exists());
    assert!(t.out.join("quiet.en.srt").is_file() && t.out.join("quiet.es.srt").is_file());
}

#[test]
fn removal_never_takes_the_retained_file_and_its_status_says_what_went_wrong() {
    use crate::history::KeptStatus;
    let Some(t) = TestApp::new("w4-removal") else { return };
    let (id, workspace) = withheld_download(&t, "oak");
    let retained = workspace.join("oak.en.srt");

    // Delete File Too deletes the media and the side file Claws published, not the kept one.
    crate::history_mutation::delete_entry(t.handle(), &id, true).unwrap();
    assert!(!t.out.join("oak.mp4").exists() && !t.out.join("oak.es.srt").exists());
    assert_eq!(std::fs::read(t.out.join("oak.en.srt")).unwrap(), b"the user's own");
    assert!(retained.is_file(), "the retained file went with the media");
    assert_eq!(library_entry_count(&t, &id), 0);

    // Library-only removal leaves it too.
    let (second, second_workspace) = withheld_download(&t, "elm");
    crate::history_mutation::delete_entry(t.handle(), &second, false).unwrap();
    assert!(second_workspace.join("elm.en.srt").is_file());
    assert!(t.out.join("elm.mp4").is_file());

    // Offline and missing are told apart from a successful show.
    let (third, third_workspace) = withheld_download(&t, "ash");
    let entry = library_entry(&t, &third);
    assert_eq!(crate::history::reveal_plan(&entry).unwrap().0.status, KeptStatus::Shown);
    std::fs::remove_file(third_workspace.join("ash.en.srt")).unwrap();
    assert_eq!(crate::history::reveal_plan(&entry).unwrap().0.status, KeptStatus::Missing);
    let mut offline = entry.clone();
    let gone = t.dir.join("unplugged");
    offline.kept_files.as_mut().unwrap().workspace.path = gone.join(crate::download_files::WORK_DIR).join(third_workspace.file_name().unwrap());
    assert_eq!(crate::history::reveal_plan(&offline).unwrap().0.status, KeptStatus::Offline);
    let mut none = entry;
    none.kept_files = None;
    assert!(crate::history::reveal_plan(&none).is_err());
}

#[test]
fn a_kept_file_of_an_entry_saved_before_kept_records_is_found_again_at_the_next_start() {
    let Some(t) = TestApp::new("w4-repair-start") else { return };
    let (id, workspace) = withheld_download(&t, "fir");
    // As an older version left it: the entry has no kept-files record.
    let t = t.restart_after(|dir| {
        let path = dir.join("history.json");
        let mut library: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        library[0].as_object_mut().unwrap().remove("keptFiles");
        std::fs::write(&path, serde_json::to_vec(&library).unwrap()).unwrap();
    });
    let entry = library_entry(&t, &id);
    assert!(entry.kept_files.is_some(), "the startup look attaches it");
    assert!(kept_text(&entry).contains("fixture en"));
    assert_eq!(saved_library(&t)[0].kept_files, entry.kept_files, "and saves it");
    assert!(workspace.join("fir.en.srt").is_file());
}

/// A finished download with a taken side-file name whose Library entry could not be saved:
/// published, failed, still in the queue, its work folder holding the retained file.
fn withheld_but_not_recorded(t: &TestApp, name: &str) -> (String, PathBuf) {
    std::fs::create_dir_all(&t.out).unwrap();
    std::fs::write(t.out.join(format!("{name}.en.srt")), b"the user's own").unwrap();
    t.block("history.json");
    let id = t.enqueue_with(&format!("https://fixture.invalid/ok-subs/{name}"), with_captions());
    let job = t.wait_job(&id, |j| j.status == JobStatus::Failed, "the Library save to fail");
    assert!(job.error.unwrap_or_default().contains("could not add it to your Library"));
    (id, job.workspace.expect("the work folder is kept").path)
}

#[test]
fn start_recovery_of_an_unrecorded_completion_records_its_kept_file_before_anything_is_retired() {
    let Some(t) = TestApp::new("w4-recover") else { return };
    let (id, workspace) = withheld_but_not_recorded(&t, "yew");
    t.unblock("history.json");
    let t = t.restart();
    assert_eq!(t.job(&id).expect("recovered").status, JobStatus::Completed);
    let entry = library_entry(&t, &id);
    assert!(kept_text(&entry).contains("fixture en"));
    assert_eq!(saved_library(&t)[0].kept_files, entry.kept_files);
    std::thread::sleep(Duration::from_millis(500)); // a cleanup would have run by now
    assert!(workspace.join("yew.en.srt").is_file(), "recovery took the retained file with the work folder");
    wait_until("the queue to retire the job", || t.saved_queue().iter().all(|j| j.id != id));
    let t = t.restart();
    assert!(t.job(&id).is_none());
    assert!(kept_text(&library_entry(&t, &id)).contains("fixture en"));
}

#[test]
fn start_recovery_gives_an_entry_without_a_kept_record_one_before_the_job_is_retired() {
    let Some(t) = TestApp::new("w4-recover-known") else { return };
    let (id, workspace) = withheld_but_not_recorded(&t, "ash2");
    t.unblock("history.json");
    // The entry was saved by an older version: it knows the media, not the kept file.
    let mut older: crate::history::HistoryItem = serde_json::from_value(serde_json::json!({
        "id": id, "title": "older", "url": "https://fixture.invalid/ok-subs/ash2", "uploader": null, "duration": null,
        "thumbnail": null, "thumbnailLocal": null, "filePath": t.out.join("ash2.mp4").to_string_lossy(),
        "fileSize": 6, "mode": "video", "quality": "Best", "finishedAt": 1
    }))
    .unwrap();
    older.kept_files = None;
    crate::store::save_snapshot(&t.dir.join("history.json"), || vec![older.clone()]).unwrap();
    let t = t.restart();
    assert_eq!(t.job(&id).expect("recovered").status, JobStatus::Completed);
    assert!(kept_text(&library_entry(&t, &id)).contains("fixture en"));
    std::thread::sleep(Duration::from_millis(500));
    assert!(workspace.join("ash2.en.srt").is_file());
}

#[test]
fn start_recovery_keeps_a_known_entry_retryable_when_its_kept_record_cannot_be_saved() {
    let Some(t) = TestApp::new("w4-recover-known-save-failed") else { return };
    let (id, workspace) = withheld_but_not_recorded(&t, "cedar");
    wait_until("the failed download worker to settle", || t.state().workers.load(std::sync::atomic::Ordering::SeqCst) == 0);
    t.unblock("history.json");
    let older: crate::history::HistoryItem = serde_json::from_value(serde_json::json!({
        "id": id, "title": "older", "url": "https://fixture.invalid/ok-subs/cedar", "uploader": null, "duration": null,
        "thumbnail": null, "thumbnailLocal": null, "filePath": t.out.join("cedar.mp4").to_string_lossy(),
        "fileSize": 6, "mode": "video", "quality": "Best", "finishedAt": 1
    })).unwrap();
    crate::history_mutation::add(t.handle(), older).unwrap();
    t.block("history.json");
    let starts = t.starts().len();
    downloader::restore_queue(t.handle(), vec![t.job(&id).unwrap()]).join().unwrap();
    let recovered = t.job(&id).unwrap();
    assert_eq!(recovered.status, JobStatus::Failed, "a failed kept-file save must not retire recovery");
    assert!(recovered.error.unwrap_or_default().contains("Press Try Again"));
    assert!(library_entry(&t, &id).kept_files.is_none());
    downloader::flush_queue(t.handle()).unwrap();
    assert!(t.saved_queue().iter().any(|job| job.id == id), "the retry must remain durable");
    assert!(workspace.join("cedar.en.srt").is_file());
    t.unblock("history.json");
    downloader::resume(t.handle(), &id);
    t.wait_job(&id, |job| job.status == JobStatus::Completed, "the kept-file retry to complete");
    assert!(kept_text(&library_entry(&t, &id)).contains("fixture en"));
    assert!(saved_library(&t)[0].kept_files.is_some());
    assert_eq!(t.starts().len(), starts, "retry must not download the media again");
    assert_eq!(std::fs::read(t.out.join("cedar.en.srt")).unwrap(), b"the user's own");
}

#[test]
fn removing_a_finished_download_that_never_reached_the_library_keeps_its_retained_file() {
    let Some(t) = TestApp::new("w4-remove-unrecorded") else { return };
    let (id, workspace) = withheld_but_not_recorded(&t, "elm");
    t.unblock("history.json");
    downloader::remove(t.handle(), &id).unwrap();
    assert!(t.job(&id).is_none());
    wait_until("the removal to settle", || t.state().pending_cleanup.lock().unwrap().is_empty());
    std::thread::sleep(Duration::from_millis(500)); // a cleanup would have run by now
    let kept = std::fs::read_to_string(workspace.join("elm.en.srt")).expect("the only copy was deleted");
    assert!(kept.contains("fixture en"));
    assert_eq!(std::fs::read(t.out.join("elm.en.srt")).unwrap(), b"the user's own");
}

#[test]
fn cancelling_a_finished_download_that_never_reached_the_library_keeps_its_retained_file() {
    let Some(t) = TestApp::new("w4-cancel-unrecorded") else { return };
    let (id, workspace) = withheld_but_not_recorded(&t, "oak");
    downloader::cancel(t.handle(), &id);
    std::thread::sleep(Duration::from_millis(500));
    assert!(workspace.join("oak.en.srt").is_file());
    assert!(t.job(&id).is_some(), "a committed publication is not cancelled");
}

fn release_merges(t: &TestApp) {
    std::fs::write(t.out.join("fixture-merge-release"), b"go").unwrap();
}

#[test]
fn a_queued_job_starts_transferring_while_three_others_are_processing() {
    let Some(t) = TestApp::new("slots-free") else { return };
    // The defaults: three transfers, and two processing jobs that hold no transfer slot.
    let merging: Vec<String> = ["one", "two", "three"].iter().map(|name| t.enqueue(&format!("https://fixture.invalid/merge-hold/{name}"))).collect();
    let fourth = t.enqueue("https://fixture.invalid/hang/four");
    for id in &merging {
        t.wait_job(id, |j| j.status == JobStatus::Processing, "the merge to start");
    }
    // Three merges overflow the processing limit by one, which holds one of three slots.
    t.wait_job(&fourth, running, "the fourth to transfer beside three merges");
    for id in &merging {
        assert_eq!(t.job(id).unwrap().status, JobStatus::Processing, "a merge ended before it was released");
    }

    release_merges(&t);
    for id in &merging {
        t.wait_job(id, |j| j.status == JobStatus::Completed, "the completion");
        wait_until("the finished job's disk claim to go", || t.state().disk_ledger.lock().unwrap().held(id).is_none());
    }
    assert!(running(&t.job(&fourth).unwrap()), "the transfer was disturbed by the completions");
}

#[test]
fn processing_beyond_its_limit_holds_a_transfer_slot() {
    let Some(t) = TestApp::new("slots-full") else { return };
    {
        let state = t.state();
        let mut settings = state.settings.lock().unwrap();
        settings.max_concurrent = 1;
        settings.max_processing = 1;
    }
    let first = t.enqueue("https://fixture.invalid/merge-hold/first");
    let second = t.enqueue("https://fixture.invalid/merge-hold/second");
    let third = t.enqueue("https://fixture.invalid/hang/third");
    // The first merge frees the only transfer slot for the second; the second merge is
    // one more than the limit and keeps the slot, so the third waits.
    t.wait_job(&first, |j| j.status == JobStatus::Processing, "the first merge");
    t.wait_job(&second, |j| j.status == JobStatus::Processing, "the second merge");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(t.job(&third).unwrap().status, JobStatus::Queued, "more ran at once than the two limits allow");
    assert_eq!(t.starts().len(), 2);

    release_merges(&t);
    t.wait_job(&third, running, "the third once the merges are done");
    for id in [&first, &second] {
        t.wait_job(id, |j| j.status == JobStatus::Completed, "the completion");
        wait_until("the finished job's disk claim to go", || t.state().disk_ledger.lock().unwrap().held(id).is_none());
    }
}

fn one_at_a_time(t: &TestApp) {
    t.state().settings.lock().unwrap().max_concurrent = 1;
}

#[test]
fn bulk_pause_resume_and_retry_act_on_every_matching_job_in_one_call() {
    let Some(t) = TestApp::new("bulk") else { return };
    one_at_a_time(&t);
    let first = t.enqueue("https://fixture.invalid/hang/bulk-a");
    let second = t.enqueue("https://fixture.invalid/hang/bulk-b");
    let third = t.enqueue("https://fixture.invalid/hang/bulk-c");
    t.wait_job(&first, running, "the first download");

    assert_eq!(downloader::pause_all(t.handle()), 3);
    for id in [&first, &second, &third] {
        t.wait_job(id, |j| j.status == JobStatus::Paused, "the pause");
    }
    assert_eq!(downloader::pause_all(t.handle()), 0, "nothing is left to pause");

    assert_eq!(downloader::resume_all(t.handle()), 3);
    // One starts (whichever is free first: a paused job's old worker may still be leaving)
    // and the other two wait in line.
    let all = [&first, &second, &third];
    wait_until("one of them to run again", || all.iter().any(|id| running(&t.job(id).unwrap())));
    let waiting: Vec<&String> = all.iter().copied().filter(|id| t.job(id).unwrap().status == JobStatus::Queued).collect();
    assert_eq!(waiting.len(), 2, "one slot, so two still wait");

    // A failed job is tried again; the paused ones are left alone.
    for id in &waiting {
        downloader::pause(t.handle(), id);
    }
    t.state().settings.lock().unwrap().max_concurrent = 2;
    let failing = t.enqueue("https://fixture.invalid/fail/bulk-fail");
    t.wait_job(&failing, |j| j.status == JobStatus::Failed, "the failure");
    let starts = |name: &str| t.starts().iter().filter(|line| line.contains(name)).count();
    assert_eq!(starts("bulk-fail"), 1);
    assert_eq!(downloader::retry_failed(t.handle()), 1);
    wait_until("the failed job to start again", || starts("bulk-fail") == 2);
    for id in &waiting {
        assert_eq!(t.job(id).unwrap().status, JobStatus::Paused, "Retry Failed must not resume a paused job");
    }
}

#[test]
fn a_job_moved_to_the_top_starts_next_and_keeps_its_place_after_a_restart() {
    let Some(t) = TestApp::new("prioritize") else { return };
    one_at_a_time(&t);
    let first = t.enqueue("https://fixture.invalid/hang/top-a");
    let second = t.enqueue("https://fixture.invalid/hang/top-b");
    let third = t.enqueue("https://fixture.invalid/hang/top-c");
    t.wait_job(&first, running, "the first download");

    downloader::prioritize(t.handle(), &third);
    let moved = t.job(&third).unwrap().priority;
    assert!(moved > 0);
    downloader::prioritize(t.handle(), &third);
    assert_eq!(t.job(&third).unwrap().priority, moved, "a job already on top is not moved again");
    downloader::prioritize(t.handle(), &first);
    assert_eq!(t.job(&first).unwrap().priority, 0, "a running job is not moved");

    downloader::cancel(t.handle(), &first);
    t.wait_job(&third, running, "the moved job to start before the older one");
    assert_eq!(t.job(&second).unwrap().status, JobStatus::Queued);

    wait_until("the order to be saved", || t.saved_queue().iter().any(|j| j.id == third && j.priority == moved));
    let t = t.restart();
    assert_eq!(t.job(&third).unwrap().priority, moved);
    assert_eq!(t.job(&second).unwrap().priority, 0);
}

#[test]
fn the_processing_stage_is_named_while_it_runs_and_is_not_saved() {
    let Some(t) = TestApp::new("stage") else { return };
    let id = t.enqueue("https://fixture.invalid/merge-hold/staged");
    let job = t.wait_job(&id, |j| j.status == JobStatus::Processing, "the merge");
    assert_eq!(job.stage.as_deref(), Some("Merging video and audio"));
    downloader::flush_queue(t.handle()).expect("the queue is saved");
    let text = std::fs::read_to_string(t.dir.join("queue.json")).unwrap();
    assert!(text.contains(&id) && text.contains("\"processing\""), "the processing job is in the saved queue");
    assert!(!text.contains("\"stage\""), "the stage was written to the queue file");
    release_merges(&t);
    t.wait_job(&id, |j| j.status == JobStatus::Completed, "the completion");
}

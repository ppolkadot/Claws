use std::collections::HashSet;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::disk_budget;
use crate::download_files::{self, Publication, WorkspaceRef};
use crate::download_policy::{self, FailureKind, JobExecution, Source};
use crate::error::{notify, AppError, Result};
use crate::history;
use crate::queue_writer::{QueueSnapshot, QUEUE_SCHEMA};
use crate::transfer_progress::TransferProgress;
use crate::probe::{self, error_text, validate_url};
use crate::settings::Settings;
use crate::state::AppState;
use crate::store;
use crate::tools;

const MAX_BATCH: usize = 500;
const AUDIO_FORMATS: &[&str] = &["mp3", "m4a", "opus", "flac", "wav", "best"];

// Marker prefixes for the lines yt-dlp is asked to print, so they can never be
// confused with ordinary log output.
const TAG_PROGRESS: &str = "CLAWS|";
const TAG_POST: &str = "CLAWSPP|";
const TAG_FORMATS: &str = "CLAWSFMT|";
const TAG_DONE: &str = "CLAWSDONE|";

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Downloading,
    Processing,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl JobStatus {
    fn is_active(self) -> bool {
        matches!(self, JobStatus::Downloading | JobStatus::Processing)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct DownloadOptions {
    /// "video" or "audio".
    pub mode: String,
    /// Tallest video to fetch; `None` means the best available.
    pub max_height: Option<u32>,
    pub audio_format: String,
    pub subtitle_langs: Vec<String>,
    pub auto_subs: bool,
    pub embed_subs: bool,
    /// Fast Save: no metadata pass and subtitles kept as separate files, so the file is not
    /// rewritten after it downloads. The chosen video and audio streams are unchanged.
    pub fast_save: bool,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            mode: "video".into(),
            max_height: None,
            audio_format: "mp3".into(),
            subtitle_langs: Vec::new(),
            auto_subs: false,
            embed_subs: false,
            fast_save: false,
        }
    }
}

impl DownloadOptions {
    /// Whether subtitles go into the video (a whole-file pass) rather than next to it.
    pub fn embeds_subtitles(&self) -> bool {
        self.embed_subs && !self.fast_save
    }

    fn validate(&mut self) -> Result<()> {
        if self.mode != "video" && self.mode != "audio" {
            return Err(AppError::msg("Unknown download type."));
        }
        if !AUDIO_FORMATS.contains(&self.audio_format.as_str()) {
            return Err(AppError::msg("Unsupported audio format."));
        }
        self.max_height = self.max_height.map(|h| h.clamp(144, 4320));
        self.subtitle_langs.truncate(12);
        let valid_lang = |l: &String| {
            !l.is_empty()
                && l.len() <= 24
                && l.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        };
        if !self.subtitle_langs.iter().all(valid_lang) {
            return Err(AppError::msg("Invalid subtitle language."));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub url: String,
    /// Set when the playlist entry had no link of its own: the job then downloads item
    /// `playlist_index` of the playlist at `url`.
    #[serde(default)]
    pub playlist_index: Option<u32>,
    pub title: String,
    pub thumbnail: Option<String>,
    pub uploader: Option<String>,
    pub duration: Option<f64>,
    pub options: DownloadOptions,
    pub status: JobStatus,
    /// Overall progress, 0.0 to 1.0, weighted by bytes. Meaningful only when
    /// `progress_known`; 1.0 only once the file is published.
    pub progress: f64,
    /// False while some stream's size is unknown: the interface then shows bytes and speed
    /// with an indeterminate bar instead of a made-up percentage.
    #[serde(default)]
    pub progress_known: bool,
    /// Some expected size is an estimate, so the percentage and time left are approximate.
    #[serde(default)]
    pub progress_estimated: bool,
    #[serde(default)]
    pub downloaded_bytes: Option<u64>,
    #[serde(default)]
    pub total_bytes: Option<u64>,
    pub speed: Option<f64>,
    pub eta: Option<u64>,
    pub file_path: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    /// Only for downloads started by 1.0.4 or earlier, which wrote straight into the
    /// output folder: the file names yt-dlp reported, so their exact `.part`/`.ytdl`
    /// leftovers can be removed on cancel. New jobs keep everything in `workspace`.
    #[serde(default)]
    pub partial_files: Vec<String>,
    /// Settings bound when the job first started; resumes and retries reuse them.
    #[serde(default)]
    pub execution: Option<JobExecution>,
    /// The job's own work folder (see `download_files`).
    #[serde(default)]
    pub workspace: Option<WorkspaceRef>,
    /// Set durably when the user cancels or removes the job, so a crash before the work
    /// folder is deleted still ends in a cancelled job and a retried cleanup.
    #[serde(default)]
    pub cancel_requested: bool,
    /// Bumped (under the jobs lock) on every observable change, so the interface can drop
    /// events that arrive late or twice.
    #[serde(default)]
    pub revision: u64,
}

pub const EVENT_SCHEMA: u16 = 1;

/// One job's state at a revision, as sent to the interface.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct JobEvent {
    pub schema_version: u16,
    pub job_id: String,
    pub revision: u64,
    pub job: Job,
}

impl JobEvent {
    pub fn of(job: &Job) -> Self {
        Self { schema_version: EVENT_SCHEMA, job_id: job.id.clone(), revision: job.revision, job: job.clone() }
    }
}

/// Jobs added together (one enqueue), sent as one event and returned by `enqueue`.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct JobsAdded {
    pub schema_version: u16,
    pub jobs: Vec<JobEvent>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct JobRemoved<'a> {
    schema_version: u16,
    job_id: &'a str,
    revision: u64,
}

/// The next job revision. Call with the jobs lock held, so revisions follow the order in
/// which changes were made.
fn next_revision(state: &AppState) -> u64 {
    state.job_revision.fetch_add(1, Ordering::SeqCst) + 1
}

fn emit_removed(app: &AppHandle, id: &str) {
    let revision = next_revision(&app.state::<AppState>());
    let _ = app.emit("job-removed", JobRemoved { schema_version: EVENT_SCHEMA, job_id: id, revision });
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueItem {
    pub url: String,
    #[serde(default)]
    pub playlist_index: Option<u32>,
    pub title: String,
    pub thumbnail: Option<String>,
    pub uploader: Option<String>,
    pub duration: Option<f64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Intent {
    None,
    Pause,
    Cancel,
}

/// A job whose yt-dlp process exists. The entry stays until `finish` has settled the
/// job, so pause, cancel and resume never see a half-finished state.
pub struct Running {
    /// `None` once the process has exited, so a late kill never hits a reused pid.
    pid: Option<u32>,
    intent: Intent,
    /// Legacy leftovers from earlier runs, handed over when a running job is removed.
    cleanup: Vec<String>,
    /// The work folder of a running job that was removed from the list meanwhile.
    workspace: Option<WorkspaceRef>,
}

impl Running {
    /// Cancel wins: a pause arriving after a cancel must not keep the partial files.
    fn request(&mut self, intent: Intent) {
        if self.intent != Intent::Cancel {
            self.intent = intent;
        }
    }
}

fn emit_job(app: &AppHandle, job: &Job) {
    let _ = app.emit("job-update", JobEvent::of(job));
}

/// The queue file's contents: everything not finished. Called by the queue writer with
/// the store lock held, so the revision read here is covered by the jobs it saves.
pub fn queue_snapshot(state: &AppState) -> QueueSnapshot {
    let revision = state.queue_revision.load(Ordering::SeqCst);
    let jobs = state
        .jobs
        .lock()
        .unwrap()
        .iter()
        .filter(|j| j.status != JobStatus::Completed && j.status != JobStatus::Cancelled)
        .cloned()
        .collect();
    QueueSnapshot { schema_version: QUEUE_SCHEMA, revision, jobs }
}

/// Marks the queue changed; the writer saves it shortly, off this thread.
fn persist_queue(app: &AppHandle) {
    let state = app.state::<AppState>();
    let revision = state.queue_revision.fetch_add(1, Ordering::SeqCst) + 1;
    state.queue_writer.request(revision, false);
}

/// Saves the queue now and waits until it is on disk (bounded). For transitions that
/// must survive a crash: a job's work folder being committed, a cancel before its files
/// are deleted, a completion before the library mentions it, and shutdown. Blocking.
pub fn flush_queue(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    let revision = state.queue_revision.fetch_add(1, Ordering::SeqCst) + 1;
    state.queue_writer.flush(revision, Duration::from_secs(5))
}

async fn flush_queue_async(app: &AppHandle) {
    let app = app.clone();
    let _ = tokio::task::spawn_blocking(move || flush_queue(&app)).await;
}

fn update_job(app: &AppHandle, id: &str, change: impl FnOnce(&mut Job)) -> Option<Job> {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut jobs = state.jobs.lock().unwrap();
        let job = jobs.iter_mut().find(|j| j.id == id)?;
        change(job);
        job.revision = next_revision(&state);
        job.clone()
    };
    emit_job(app, &snapshot);
    Some(snapshot)
}

fn job_status(app: &AppHandle, id: &str) -> Option<JobStatus> {
    app.state::<AppState>()
        .jobs
        .lock()
        .unwrap()
        .iter()
        .find(|j| j.id == id)
        .map(|j| j.status)
}

pub fn restore_queue(app: &AppHandle, saved: Vec<Job>) {
    let mut saved = saved;
    let mut cleanup = Vec::new();
    let mut recovered = Vec::new();
    for job in &mut saved {
        // A cancel whose cleanup did not finish before the app closed.
        if job.cancel_requested && job.status != JobStatus::Completed {
            job.status = JobStatus::Cancelled;
            cleanup.push(PendingCleanup::of(job));
            job.partial_files.clear();
            continue;
        }
        // Published before the app closed, but the completion was not saved yet.
        if job.status != JobStatus::Completed {
            let manifest = job.workspace.as_ref().and_then(|w| download_files::verify(w, &job.id).ok());
            if let Some(manifest) = manifest.filter(|m| m.state == Publication::Published) {
                if let Some(media) = manifest.published.first().filter(|p| p.is_file()) {
                    job.status = JobStatus::Completed;
                    job.progress = 1.0;
                    job.speed = None;
                    job.eta = None;
                    job.error = None;
                    job.file_path = Some(media.to_string_lossy().into_owned());
                    recovered.push(job.clone());
                    cleanup.push(PendingCleanup::of(job));
                    continue;
                }
            }
        }
        // Nothing survives a restart mid-download; everything resumes on demand.
        if matches!(
            job.status,
            JobStatus::Queued | JobStatus::Downloading | JobStatus::Processing
        ) {
            job.status = JobStatus::Paused;
            job.speed = None;
            job.eta = None;
        }
    }
    let state = app.state::<AppState>();
    // Revisions continue above anything saved, so the interface never sees one go back.
    let highest = saved.iter().map(|j| j.revision).max().unwrap_or(0);
    state.job_revision.store(highest, Ordering::SeqCst);
    *state.jobs.lock().unwrap() = saved;
    for job in &recovered {
        let known = state.history.lock().unwrap().iter().any(|h| h.id == job.id);
        if !known {
            history::record(app, job);
        }
    }
    // Off the startup path: deleting a large work folder can take a moment.
    let app = app.clone();
    std::thread::spawn(move || {
        for item in cleanup {
            cleanup_now(&app, item);
        }
        retry_pending_cleanup(&app);
    });
}

pub fn list(app: &AppHandle) -> Vec<Job> {
    app.state::<AppState>().jobs.lock().unwrap().clone()
}

pub fn enqueue(app: &AppHandle, items: Vec<EnqueueItem>, mut options: DownloadOptions) -> Result<JobsAdded> {
    if app.state::<AppState>().shutting_down.load(Ordering::SeqCst) {
        return Err(AppError::msg("Claws is closing."));
    }
    if items.is_empty() {
        return Err(AppError::msg("Nothing to download."));
    }
    if items.len() > MAX_BATCH {
        return Err(AppError::msg(format!("Too many items at once (limit {MAX_BATCH}).")));
    }
    options.validate()?;

    // The same link with the same choices, already waiting or running, would only make a
    // second copy race the first for the same file name.
    let state = app.state::<AppState>();
    let pending: HashSet<(String, Option<u32>, String)> = {
        let jobs = state.jobs.lock().unwrap();
        jobs.iter()
            .filter(|j| !matches!(j.status, JobStatus::Completed | JobStatus::Cancelled | JobStatus::Failed))
            .map(|j| (j.url.clone(), j.playlist_index, options_key(&j.options)))
            .collect()
    };
    let mut seen = HashSet::new();
    let mut created = Vec::with_capacity(items.len());
    for item in items {
        let url = validate_url(&item.url)?;
        let key = (url.clone(), item.playlist_index.filter(|i| *i > 0), options_key(&options));
        if pending.contains(&key) || !seen.insert(key) {
            continue;
        }
        created.push(Job {
            id: uuid::Uuid::new_v4().to_string(),
            url,
            playlist_index: item.playlist_index.filter(|i| *i > 0),
            title: if item.title.trim().is_empty() {
                "Untitled".into()
            } else {
                item.title
            },
            thumbnail: item.thumbnail.filter(|t| t.starts_with("https://")),
            uploader: item.uploader,
            duration: item.duration,
            options: options.clone(),
            status: JobStatus::Queued,
            progress: 0.0,
            progress_known: false,
            progress_estimated: false,
            downloaded_bytes: None,
            total_bytes: None,
            speed: None,
            eta: None,
            file_path: None,
            error: None,
            created_at: chrono::Utc::now().timestamp_millis(),
            partial_files: Vec::new(),
            execution: None,
            workspace: None,
            cancel_requested: false,
            revision: 0,
        });
    }
    if created.is_empty() {
        return Err(AppError::msg("That is already in the queue with the same options."));
    }

    let added = {
        let mut jobs = state.jobs.lock().unwrap();
        for job in &mut created {
            job.revision = next_revision(&state);
        }
        jobs.extend(created.iter().cloned());
        JobsAdded { schema_version: EVENT_SCHEMA, jobs: created.iter().map(JobEvent::of).collect() }
    };
    // One event for the whole batch: a 500-item playlist is one structural change.
    let _ = app.emit("jobs-added", &added);
    persist_queue(app);
    pump(app);
    Ok(added)
}

fn options_key(options: &DownloadOptions) -> String {
    serde_json::to_string(options).unwrap_or_default()
}

/// Keeps `AppState::workers` and `live_jobs` accurate however `run_job` ends.
struct WorkerSlot {
    app: AppHandle,
    id: String,
}

impl Drop for WorkerSlot {
    fn drop(&mut self) {
        let state = self.app.state::<AppState>();
        state.live_jobs.lock().unwrap().remove(&self.id);
        state.workers.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn pump(app: &AppHandle) {
    let state = app.state::<AppState>();
    let limit = state.settings.lock().unwrap().max_concurrent.max(1) as usize;
    loop {
        // Exiting workers call pump too; none of them may start another download.
        if state.shutting_down.load(Ordering::SeqCst) {
            break;
        }
        let claimed = {
            let mut jobs = state.jobs.lock().unwrap();
            let active = jobs.iter().filter(|j| j.status.is_active()).count();
            if active >= limit {
                None
            } else {
                let mut live = state.live_jobs.lock().unwrap();
                // A job whose earlier worker is still alive waits for it to exit; that
                // worker pumps again on its way out.
                jobs.iter_mut()
                    .find(|j| j.status == JobStatus::Queued && !live.contains(&j.id))
                    .map(|job| {
                        job.status = JobStatus::Downloading;
                        job.error = None;
                        job.revision = next_revision(&state);
                        live.insert(job.id.clone());
                        job.clone()
                    })
            }
        };
        let Some(job) = claimed else { break };
        emit_job(app, &job);
        state.workers.fetch_add(1, Ordering::SeqCst);
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _slot = WorkerSlot {
                app: app.clone(),
                id: job.id.clone(),
            };
            run_job(app, job.id).await
        });
    }
}

pub(crate) fn kill_tree(pid: u32, wait: bool) {
    // yt-dlp.exe is a launcher that starts the real interpreter (and ffmpeg) as
    // children, so the whole tree has to go.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("taskkill");
        cmd.args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if wait {
            let _ = cmd.status();
        } else {
            let _ = cmd.spawn();
        }
    }
    #[cfg(not(windows))]
    let _ = (pid, wait);
}

// Pause, cancel and resume run under the `running` lock, as do process registration
// (`execute`) and settling (`finish`), so they never interleave.

pub fn pause(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    {
        let mut running = state.running.lock().unwrap();
        if let Some(entry) = running.get_mut(id) {
            entry.request(Intent::Pause);
            if let Some(pid) = entry.pid {
                kill_tree(pid, false);
            }
        } else {
            // Queued, or claimed but not spawned yet: run_job notices the status and stops.
            update_job(app, id, |j| {
                if j.status == JobStatus::Queued || j.status.is_active() {
                    j.status = JobStatus::Paused;
                    j.speed = None;
                    j.eta = None;
                }
            });
        }
    }
    persist_queue(app);
}

pub fn resume(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    {
        let running = state.running.lock().unwrap();
        // A job whose process is still winding down is settled by `finish` first.
        if !running.contains_key(id) {
            update_job(app, id, |j| {
                if matches!(
                    j.status,
                    JobStatus::Paused | JobStatus::Failed | JobStatus::Cancelled
                ) {
                    j.status = JobStatus::Queued;
                    j.error = None;
                }
            });
        }
    }
    persist_queue(app);
    pump(app);
}

pub fn cancel(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    let leftovers = {
        let mut running = state.running.lock().unwrap();
        if let Some(entry) = running.get_mut(id) {
            entry.request(Intent::Cancel);
            if let Some(pid) = entry.pid {
                kill_tree(pid, false);
            }
            update_job(app, id, |j| j.cancel_requested = true);
            None
        } else {
            let mut files = None;
            update_job(app, id, |j| {
                if matches!(
                    j.status,
                    JobStatus::Queued
                        | JobStatus::Paused
                        | JobStatus::Failed
                        | JobStatus::Downloading
                        | JobStatus::Processing
                ) {
                    j.status = JobStatus::Cancelled;
                    j.speed = None;
                    j.eta = None;
                    j.cancel_requested = true;
                    files = Some(PendingCleanup::of(j));
                    j.partial_files.clear();
                }
            });
            files
        }
    };
    // Saved before anything is deleted, so a crash mid-cleanup still ends cancelled.
    persist_queue(app);
    // No process holds these files, so they can go right away.
    if let Some(item) = leftovers {
        spawn_cleanup(app, item);
    }
}

fn find_job(app: &AppHandle, id: &str) -> Option<Job> {
    app.state::<AppState>()
        .jobs
        .lock()
        .unwrap()
        .iter()
        .find(|j| j.id == id)
        .cloned()
}

/// Files a cancelled or removed job leaves behind: its work folder, and for downloads
/// started before work folders existed, the exact partial files it reported.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PendingCleanup {
    pub job_id: String,
    pub workspace: Option<WorkspaceRef>,
    #[serde(default)]
    pub legacy: Vec<String>,
}

impl PendingCleanup {
    fn of(job: &Job) -> Self {
        Self {
            job_id: job.id.clone(),
            workspace: job.workspace.clone(),
            legacy: job.partial_files.clone(),
        }
    }

    fn is_empty(&self) -> bool {
        self.workspace.is_none() && self.legacy.is_empty()
    }
}

/// Records a cleanup in cleanup.json before anything is deleted, so a crash midway is
/// finished at the next start.
fn register_cleanup(app: &AppHandle, item: &PendingCleanup) {
    let state = app.state::<AppState>();
    {
        let mut pending = state.pending_cleanup.lock().unwrap();
        pending.retain(|p| p.job_id != item.job_id);
        pending.push(item.clone());
    }
    save_pending_cleanup(app);
}

fn spawn_cleanup(app: &AppHandle, item: PendingCleanup) {
    if item.is_empty() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = tokio::task::spawn_blocking(move || {
            // The cancelled state is on disk before its files go.
            flush_queue(&app);
            register_cleanup(&app, &item);
            cleanup_now(&app, item)
        })
        .await;
    });
}

/// Deletes a job's leftovers now. If Windows still holds a file, the cleanup is saved
/// and retried at the next start rather than reported as done.
fn cleanup_now(app: &AppHandle, item: PendingCleanup) -> bool {
    let state = app.state::<AppState>();
    remove_partials_now(&item.legacy);
    let removed = match &item.workspace {
        Some(workspace) => match download_files::remove_workspace(workspace, &item.job_id) {
            Ok(()) => true,
            Err(_) if !workspace.path.exists() => true,
            Err(e) => {
                notify(app, format!("Claws could not remove the unfinished files of a cancelled download yet ({e}). It will try again the next time it starts."));
                false
            }
        },
        None => true,
    };
    let changed = {
        let mut pending = state.pending_cleanup.lock().unwrap();
        let before = pending.clone();
        pending.retain(|p| p.job_id != item.job_id);
        if !removed {
            pending.push(item);
        }
        *pending != before
    };
    if changed {
        save_pending_cleanup(app);
    }
    removed
}

fn save_pending_cleanup(app: &AppHandle) {
    let state = app.state::<AppState>();
    let result = store::save_snapshot(&state.cleanup_path(), || state.pending_cleanup.lock().unwrap().clone());
    if let Err(e) = result {
        notify(app, format!("Claws could not save its cleanup list: {e}"));
    }
}

/// Retries cleanups that could not finish earlier because a file was still in use.
pub fn retry_pending_cleanup(app: &AppHandle) {
    let items = app.state::<AppState>().pending_cleanup.lock().unwrap().clone();
    for item in items {
        cleanup_now(app, item);
    }
}

pub fn remove(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    let Some(job) = find_job(app, id) else { return };
    let leftovers = PendingCleanup::of(&job);
    let handed_off = {
        let mut running = state.running.lock().unwrap();
        match running.get_mut(id) {
            Some(entry) => {
                // `finish` cleans up once the process is really gone.
                entry.request(Intent::Cancel);
                entry.cleanup.extend(leftovers.legacy.iter().cloned());
                entry.workspace = leftovers.workspace.clone();
                if let Some(pid) = entry.pid {
                    kill_tree(pid, false);
                }
                true
            }
            None => false,
        }
    };
    // Finished downloads keep their file; anything unfinished loses its leftovers.
    let clean = !handed_off && job.status != JobStatus::Completed && !leftovers.is_empty();
    if clean {
        // Recorded before the job disappears, so an interrupted cleanup is retried.
        register_cleanup(app, &leftovers);
    }
    state.jobs.lock().unwrap().retain(|j| j.id != id);
    emit_removed(app, id);
    persist_queue(app);
    if clean {
        spawn_cleanup(app, leftovers);
    }
}

pub fn clear_finished(app: &AppHandle) {
    let state = app.state::<AppState>();
    let removed: Vec<String> = {
        let mut jobs = state.jobs.lock().unwrap();
        let ids = jobs
            .iter()
            .filter(|j| matches!(j.status, JobStatus::Completed | JobStatus::Cancelled))
            .map(|j| j.id.clone())
            .collect();
        jobs.retain(|j| !matches!(j.status, JobStatus::Completed | JobStatus::Cancelled));
        ids
    };
    for id in removed {
        emit_removed(app, &id);
    }
    persist_queue(app);
}

/// Called once as the app exits. Raises `shutting_down` (nothing new can start), kills
/// every download and lookup tree, waits for the workers to settle, then fixes up what
/// they did not and saves the queue, so jobs come back paused.
pub fn shutdown(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.shutting_down.store(true, Ordering::SeqCst);

    type Stopping = (String, Option<u32>, Intent, Vec<String>, Option<WorkspaceRef>);
    let stopping: Vec<Stopping> = {
        let mut running = state.running.lock().unwrap();
        running
            .iter_mut()
            .map(|(id, entry)| {
                entry.request(Intent::Pause);
                (id.clone(), entry.pid, entry.intent, entry.cleanup.clone(), entry.workspace.clone())
            })
            .collect()
    };
    let lookups: Vec<u32> = state.probes.lock().unwrap().values().copied().collect();
    for pid in stopping.iter().filter_map(|s| s.1).chain(lookups) {
        kill_tree(pid, true);
    }

    // Bounded: a worker stuck behind a tool install cannot finish; the fix-up covers it.
    let deadline = Instant::now() + Duration::from_secs(5);
    while state.workers.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }

    let mut leftovers = Vec::new();
    {
        let mut jobs = state.jobs.lock().unwrap();
        for job in jobs.iter_mut() {
            let intent = stopping.iter().find(|(id, ..)| *id == job.id).map(|s| s.2);
            if intent == Some(Intent::Cancel) && job.status != JobStatus::Completed {
                job.status = JobStatus::Cancelled;
                job.cancel_requested = true;
                leftovers.push(PendingCleanup::of(job));
                job.partial_files.clear();
            } else if job.status.is_active() || job.status == JobStatus::Queued {
                job.status = JobStatus::Paused;
                job.speed = None;
                job.eta = None;
            }
        }
    }
    for (id, _, intent, cleanup, workspace) in stopping {
        // Removed from the list while running: only the handed-over files are left.
        if intent == Intent::Cancel && !leftovers.iter().any(|l| l.job_id == id) {
            leftovers.push(PendingCleanup { job_id: id, workspace, legacy: cleanup });
        }
    }
    // The cancellations are saved before anything is deleted, so no deleted file can
    // belong to a job that comes back paused.
    flush_queue(app);
    let leftovers: Vec<PendingCleanup> = leftovers.into_iter().filter(|l| !l.is_empty()).collect();
    for item in &leftovers {
        register_cleanup(app, item);
    }
    if !leftovers.is_empty() {
        // Give Windows a moment to release the killed processes' handles.
        std::thread::sleep(Duration::from_millis(300));
        for item in leftovers {
            cleanup_now(app, item);
        }
    }
    flush_queue(app);
    state.queue_writer.stop();
}

fn build_args(
    app: &AppHandle,
    job: &Job,
    settings: &Settings,
    execution: &JobExecution,
    workspace: &Path,
    info_json: Option<&Path>,
) -> Vec<String> {
    // Sign-in and tool locations stay current; everything shaping the files is frozen.
    let source = match (info_json, job.playlist_index) {
        (Some(path), _) => Source::InfoJson(path),
        (None, Some(index)) => Source::PlaylistItem { url: &job.url, index },
        (None, None) => Source::Url(&job.url),
    };
    download_policy::download_args(tools::common_args(app, settings), &job.options, execution, workspace, source)
}

struct Outcome {
    success: bool,
    error: Option<String>,
    final_path: Option<String>,
    used_cache: bool,
    /// What kind of failure, when yt-dlp itself failed.
    kind: Option<FailureKind>,
}

impl Outcome {
    fn failed(message: impl Into<String>) -> Self {
        Self {
            success: false,
            error: Some(message.into()),
            final_path: None,
            used_cache: false,
            kind: None,
        }
    }
}

async fn execute(app: &AppHandle, id: &str, allow_cache: bool) -> Outcome {
    let state = app.state::<AppState>();
    let Some(job) = list(app).into_iter().find(|j| j.id == id) else {
        return Outcome::failed("Job no longer exists.");
    };
    if job.status != JobStatus::Downloading {
        // Paused or cancelled between being claimed and starting.
        return Outcome::failed("");
    }
    let Some(ytdlp) = tools::find_ytdlp(app) else {
        return Outcome::failed("yt-dlp is not installed yet. Open Settings to install it.");
    };
    let settings = state.settings.lock().unwrap().clone();
    let (execution, workspace, manifest) = match prepare(app, &job, &settings).await {
        Ok(prepared) => prepared,
        Err(message) => return Outcome::failed(message),
    };
    // Downloaded and verified before; only the publication is left to do. Registered as
    // running (without a process) so a cancel meanwhile is settled by `finish`.
    if matches!(manifest.state, Publication::Ready | Publication::Publishing) {
        let mut running = state.running.lock().unwrap();
        if state.shutting_down.load(Ordering::SeqCst) {
            return Outcome::failed("");
        }
        let entry = running.entry(id.to_string()).or_insert(Running {
            pid: None,
            intent: Intent::None,
            cleanup: Vec::new(),
            workspace: None,
        });
        match job_status(app, id) {
            Some(JobStatus::Paused) => entry.request(Intent::Pause),
            Some(JobStatus::Cancelled) | None => entry.request(Intent::Cancel),
            _ => {}
        }
        if entry.intent != Intent::None {
            return Outcome::failed("");
        }
        return Outcome {
            success: true,
            error: None,
            final_path: None,
            used_cache: false,
            kind: None,
        };
    }

    // Playlist items picked by index are never cached: the cache holds single videos.
    let info_json = (allow_cache && job.playlist_index.is_none())
        .then(|| probe::cached_info(app, &job.url))
        .flatten();
    let args = build_args(app, &job, &settings, &execution, &workspace.path, info_json.as_deref());
    let mut command = tools::command(&ytdlp);
    command
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    // Spawned and registered under `running` after checking the shutdown flag under the
    // same lock, so shutdown either sees this process or it never starts.
    let mut child = {
        let mut running = state.running.lock().unwrap();
        if state.shutting_down.load(Ordering::SeqCst) {
            update_job(app, id, |j| {
                if j.status.is_active() {
                    j.status = JobStatus::Paused;
                    j.speed = None;
                    j.eta = None;
                }
            });
            return Outcome::failed("");
        }
        let child = match command.spawn() {
            Ok(child) => child,
            Err(e) => return Outcome::failed(format!("Could not start yt-dlp: {e}")),
        };
        if let Some(pid) = child.id() {
            // A retry reuses the entry, keeping any pause or cancel that arrived between
            // the two attempts.
            let entry = running.entry(id.to_string()).or_insert(Running {
                pid: Some(pid),
                intent: Intent::None,
                cleanup: Vec::new(),
                workspace: None,
            });
            entry.pid = Some(pid);
            // A pause or cancel may have landed (as a status) before the entry existed.
            match job_status(app, id) {
                Some(JobStatus::Paused) => entry.request(Intent::Pause),
                Some(JobStatus::Cancelled) | None => entry.request(Intent::Cancel),
                _ => {}
            }
            if entry.intent != Intent::None {
                kill_tree(pid, false);
            }
        }
        child
    };

    // yt-dlp prints download progress on stdout but post-processing progress and its
    // errors on stderr, so both streams are read as they arrive.
    let stderr = child.stderr.take();
    let stderr_app = app.clone();
    let stderr_id = id.to_string();
    let processing = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stderr_processing = processing.clone();
    let stderr_task = tauri::async_runtime::spawn(async move {
        let mut text = String::new();
        let Some(stream) = stderr else { return text };
        let mut reader = BufReader::new(stream);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    let line = line.trim_end();
                    if let Some(rest) = line.strip_prefix(TAG_POST) {
                        if rest.starts_with("started") {
                            stderr_processing.store(true, Ordering::SeqCst);
                            mark_processing(&stderr_app, &stderr_id);
                        }
                    } else if text.len() < 64 * 1024 {
                        text.push_str(line);
                        text.push('\n');
                    }
                }
            }
        }
        text
    });

    let mut outcome = Outcome {
        success: false,
        error: None,
        final_path: None,
        used_cache: info_json.is_some(),
        kind: None,
    };
    let mut progress = TransferProgress::new(&job.options.mode);
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let mut budget = Budget::new(&job, &execution, &workspace.path);
    let pid = child.id();
    let mut stopped: Option<String> = None;

    if let Some(stdout) = child.stdout.take() {
        let mut reader = BufReader::new(stdout);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    let line = line.trim_end();
                    handle_line(app, id, line, &mut progress, &mut last_emit, &mut outcome);
                    if stopped.is_none() {
                        if let Some(reason) = budget.after_line(app, id, line, &progress).await {
                            // Not enough space: stop before the transfer (or before the
                            // disk fills), keeping what is already downloaded.
                            if let Some(pid) = pid {
                                kill_tree(pid, false);
                            }
                            stopped = Some(reason);
                        }
                    }
                }
            }
        }
    }

    let status = child.wait().await;
    // Forget the pid before `child` drops and Windows can reuse it.
    if let Some(entry) = state.running.lock().unwrap().get_mut(id) {
        entry.pid = None;
    }
    let stderr_text = stderr_task.await.unwrap_or_default();
    match status {
        Ok(s) if s.success() => outcome.success = true,
        Ok(s) => {
            let kind = download_policy::classify(&stderr_text, processing.load(Ordering::SeqCst));
            let message = if stderr_text.trim().is_empty() {
                format!("yt-dlp stopped with exit code {}.", s.code().unwrap_or(-1))
            } else {
                error_text(&stderr_text)
            };
            outcome.kind = Some(kind);
            outcome.error = Some(match kind.summary() {
                Some(summary) => format!("{summary} {message}"),
                None => message,
            });
        }
        Err(e) => outcome.error = Some(format!("yt-dlp stopped unexpectedly: {e}")),
    }
    if let Some(reason) = stopped {
        outcome.success = false;
        outcome.kind = Some(FailureKind::LocalIo);
        outcome.error = Some(reason);
    }
    outcome
}

/// Disk-space checks for one run: once the selection (and so the sizes) is known, the
/// estimated peak is checked and reserved; when a size is unknown, free space is watched.
struct Budget {
    job_id: String,
    workspace: std::path::PathBuf,
    volume: String,
    mode: String,
    audio_format: String,
    duration: Option<f64>,
    rewrite: bool,
    checked: bool,
    watching: bool,
    last_watch: Instant,
}

impl Budget {
    fn new(job: &Job, execution: &JobExecution, workspace: &Path) -> Self {
        Self {
            job_id: job.id.clone(),
            workspace: workspace.to_path_buf(),
            volume: disk_budget::volume_key(workspace),
            mode: job.options.mode.clone(),
            audio_format: job.options.audio_format.clone(),
            duration: job.duration,
            rewrite: execution.embed_metadata || job.options.embeds_subtitles(),
            checked: false,
            watching: false,
            last_watch: Instant::now(),
        }
    }

    /// A reason to stop, if the volume cannot hold this download.
    async fn after_line(&mut self, app: &AppHandle, id: &str, line: &str, progress: &TransferProgress) -> Option<String> {
        if !self.checked && line.starts_with(TAG_FORMATS) {
            self.checked = true;
            let Some(sizes) = progress.planned_sizes() else {
                self.watching = true;
                return None;
            };
            let total: Option<u64> = sizes.iter().copied().sum();
            let steps = disk_budget::Steps {
                merge: sizes.len() > 1,
                rewrite: self.rewrite,
                audio_output: (self.mode == "audio" && self.audio_format != "best")
                    .then(|| disk_budget::audio_output(&self.audio_format, total, self.duration))
                    .flatten(),
            };
            let unconvertible_audio = self.mode == "audio" && self.audio_format != "best" && steps.audio_output.is_none();
            let Some(peak) = disk_budget::peak_bytes(&sizes, steps).filter(|_| !unconvertible_audio) else {
                self.watching = true;
                return None;
            };
            let workspace = self.workspace.clone();
            let (on_disk, free) = tokio::task::spawn_blocking(move || (disk_budget::bytes_in(&workspace), disk_budget::free_bytes(&workspace)))
                .await
                .ok()?;
            // An unreadable volume is not evidence of a shortfall: watch instead.
            let Ok(free) = free else {
                self.watching = true;
                return None;
            };
            let additional = peak.saturating_sub(on_disk);
            let state = app.state::<AppState>();
            let mut reservations = state.disk_reservations.lock().unwrap();
            let others: u64 = reservations
                .iter()
                .filter(|(job, (volume, _))| **job != self.job_id && *volume == self.volume)
                .map(|(_, (_, bytes))| *bytes)
                .sum();
            return match disk_budget::check(additional, free, others) {
                Ok(()) => {
                    reservations.insert(id.to_string(), (self.volume.clone(), additional + disk_budget::margin(additional)));
                    None
                }
                Err(short) => Some(format!(
                    "Not enough free space on {} for this download. It needs about {} more (estimated, including a safety margin) and about {} is available. Free up space, then press Try Again; what was downloaded is kept.",
                    self.volume.to_uppercase(),
                    human_bytes(short.required),
                    human_bytes(short.available),
                )),
            };
        }
        if self.watching && line.starts_with(TAG_PROGRESS) && self.last_watch.elapsed() >= Duration::from_secs(5) {
            self.last_watch = Instant::now();
            let workspace = self.workspace.clone();
            let free = tokio::task::spawn_blocking(move || disk_budget::free_bytes(&workspace)).await.ok()?.ok()?;
            if free < disk_budget::WATCH_FLOOR {
                return Some(format!(
                    "{} is almost full ({} free), so the download was stopped before the disk filled up. Free up space, then press Try Again; what was downloaded is kept.",
                    self.volume.to_uppercase(),
                    human_bytes(free),
                ));
            }
        }
        None
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 || value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn mark_processing(app: &AppHandle, id: &str) {
    update_job(app, id, |j| {
        if j.status.is_active() {
            j.status = JobStatus::Processing;
        }
        j.speed = None;
        j.eta = None;
    });
}

fn handle_line(
    app: &AppHandle,
    id: &str,
    line: &str,
    progress: &mut TransferProgress,
    last_emit: &mut Instant,
    outcome: &mut Outcome,
) {
    if let Some(rest) = line.strip_prefix(TAG_FORMATS) {
        progress.set_plan(rest);
    } else if let Some(rest) = line.strip_prefix(TAG_DONE) {
        if !rest.trim().is_empty() {
            outcome.final_path = Some(rest.trim().to_string());
        }
    } else if let Some(rest) = line.strip_prefix(TAG_POST) {
        if rest.starts_with("started") {
            mark_processing(app, id);
        }
    } else if let Some(rest) = line.strip_prefix(TAG_PROGRESS) {
        // status|downloaded|total|estimate|speed|eta|fragment|fragments|format_id|filename
        // (the file name goes last and may itself contain '|').
        let fields: Vec<&str> = rest.splitn(10, '|').collect();
        // Every file this run writes is inside the job's work folder, which is what
        // cleanup removes, so no per-file bookkeeping (or save) is needed here.
        let Some(snapshot) = progress.update(&fields) else { return };

        let finished = fields[0] == "finished";
        if finished || last_emit.elapsed() >= Duration::from_millis(150) {
            *last_emit = Instant::now();
            update_job(app, id, |j| {
                if j.status.is_active() {
                    j.status = JobStatus::Downloading;
                }
                j.progress_known = snapshot.fraction.is_some();
                j.progress = snapshot.fraction.unwrap_or(j.progress);
                j.progress_estimated = snapshot.estimated;
                j.downloaded_bytes = Some(snapshot.downloaded);
                j.total_bytes = snapshot.total;
                j.speed = snapshot.speed;
                j.eta = snapshot.eta;
            });
        }
    }
}

/// Binds the job's settings and opens its work folder the first time it starts; later
/// starts reuse both, whatever the settings say now. Saved before yt-dlp starts, so every
/// file it writes already has a recorded owner.
async fn prepare(
    app: &AppHandle,
    job: &Job,
    settings: &Settings,
) -> std::result::Result<(JobExecution, WorkspaceRef, download_files::Manifest), String> {
    let first_start = job.execution.is_none();
    let execution = job
        .execution
        .clone()
        .unwrap_or_else(|| JobExecution::freeze(settings, job.options.fast_save));
    let output = execution.output_dir.clone();
    let id = job.id.clone();
    let (workspace, manifest) = tokio::task::spawn_blocking(move || download_files::open_workspace(&output, &id))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Cannot use the output folder: {e}"))?;
    if first_start || job.workspace.as_ref() != Some(&workspace) {
        if first_start && !job.partial_files.is_empty() {
            notify(
                app,
                format!(
                    "“{}” was started by an earlier version of Claws. It now downloads in a private work folder, so it starts again from the beginning. Its earlier partial files were left where they were.",
                    job.title
                ),
            );
        }
        update_job(app, &job.id, |j| {
            j.execution = Some(execution.clone());
            j.workspace = Some(workspace.clone());
        });
        flush_queue_async(app).await;
    }
    Ok((execution, workspace, manifest))
}

/// Turns a successful yt-dlp run into published files: records what to publish, then
/// moves it to the output folder without replacing anything. On failure the files stay
/// in the work folder, so Try Again only has to publish them.
async fn publish(app: &AppHandle, id: &str, outcome: &mut Outcome) {
    let Some(job) = find_job(app, id) else {
        // Removed while finishing: the running entry carries its cleanup.
        outcome.success = false;
        outcome.error = None;
        return;
    };
    let Some(workspace) = job.workspace.clone() else {
        outcome.success = false;
        outcome.error = Some("This download has no work folder.".into());
        return;
    };
    let reported = outcome.final_path.clone();
    let wants_sidecars = !job.options.subtitle_langs.is_empty() && !job.options.embeds_subtitles();
    let app_for_reservations = app.clone();
    let id_owned = id.to_string();
    let result = tokio::task::spawn_blocking(move || -> std::result::Result<download_files::Published, String> {
        let mut manifest = download_files::verify(&workspace, &id_owned).map_err(|e| e.to_string())?;
        if manifest.state == Publication::Active {
            let reported = reported.ok_or("yt-dlp did not report the finished file.")?;
            let media = download_files::owned_relative(&workspace.path, Path::new(&reported))
                .ok_or("yt-dlp reported a finished file outside the download's work folder.")?;
            let mut outputs = vec![media.to_string_lossy().into_owned()];
            if wants_sidecars {
                outputs.extend(download_files::sidecars(&workspace.path, &media).iter().map(|p| p.to_string_lossy().into_owned()));
            }
            manifest.outputs = outputs;
            manifest.state = Publication::Ready;
            download_files::write_manifest(&manifest).map_err(|e| e.to_string())?;
        }
        let state = app_for_reservations.state::<AppState>();
        let mut held = Vec::new();
        let result = download_files::publish(&mut manifest, &mut |target| {
            let key = target.to_string_lossy().to_lowercase();
            let free = state.destinations.lock().unwrap().insert(key.clone());
            if free {
                held.push(key);
            }
            free
        });
        {
            let mut destinations = state.destinations.lock().unwrap();
            for key in held {
                destinations.remove(&key);
            }
        }
        result.map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                let name = manifest.outputs.first().map(|o| o.as_str()).unwrap_or("The file");
                let folder = manifest.destination_root.display();
                format!("“{name}” already exists in {folder}. Move or rename that file, then press Try Again; the download is kept until then.")
            } else {
                format!("The finished file could not be saved to the output folder: {e}")
            }
        })
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));

    match result {
        Ok(published) => {
            outcome.final_path = Some(published.media.to_string_lossy().into_owned());
            if published.skipped.is_empty() {
                // Only the marker (and nothing of value) is left in the work folder.
                let leftover = PendingCleanup { job_id: id.to_string(), workspace: job.workspace.clone(), legacy: Vec::new() };
                spawn_cleanup(app, leftover);
            } else {
                let names: Vec<String> = published.skipped.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
                notify(app, format!("Saved “{}”, but {} already existed, so the new copy was kept in the download's work folder.", job.title, names.join(", ")));
            }
        }
        Err(message) => {
            outcome.success = false;
            outcome.error = Some(message);
        }
    }
}

async fn run_job(app: AppHandle, id: String) {
    let state = app.state::<AppState>();
    // Waits while a tool is being replaced.
    let _engine = state.engine.read().await;

    let mut outcome = execute(&app, &id, true).await;

    // Cached media links can expire. If a cached run failed on its own (not paused or
    // cancelled) in a way fresh links can fix, read the link again and retry once. A full
    // disk or a failed merge would only repeat the whole transfer.
    let interrupted = state
        .running
        .lock()
        .unwrap()
        .get(&id)
        .is_some_and(|r| r.intent != Intent::None);
    let refreshable = !outcome.success && outcome.kind.is_some_and(FailureKind::refresh_helps);
    let closing = state.shutting_down.load(Ordering::SeqCst);
    if outcome.used_cache && refreshable && !interrupted && !closing {
        let retry = {
            let mut jobs = state.jobs.lock().unwrap();
            jobs.iter_mut()
                .find(|j| j.id == id && j.status.is_active())
                .map(|j| {
                    j.status = JobStatus::Downloading;
                    j.revision = next_revision(&state);
                    j.clone()
                })
        };
        if let Some(job) = retry {
            emit_job(&app, &job);
            probe::forget(&app, &job.url);
            outcome = execute(&app, &id, false).await;
        }
    }

    // Success wins over a pause or cancel that arrives now: the file is published.
    if outcome.success {
        update_job(&app, &id, |j| {
            if j.status.is_active() {
                j.status = JobStatus::Processing;
            }
            j.speed = None;
            j.eta = None;
        });
        publish(&app, &id, &mut outcome).await;
    }

    finish(&app, &id, outcome).await;
    state.disk_reservations.lock().unwrap().remove(&id);
    // Free the job before pumping, so a resume that arrived meanwhile can start it.
    state.live_jobs.lock().unwrap().remove(&id);
    pump(&app);
}

/// Final status for a job whose process exited. Success always wins, so a pause or
/// cancel that arrived as it completed cannot hide a finished file.
fn settle(app: &AppHandle, id: &str, intent: Intent, outcome: &Outcome) -> Option<Job> {
    if outcome.success {
        return update_job(app, id, |j| {
            j.status = JobStatus::Completed;
            j.progress = 1.0;
            j.progress_known = true;
            j.speed = None;
            j.eta = None;
            j.error = None;
            j.file_path = outcome.final_path.clone();
            j.partial_files.clear();
            j.cancel_requested = false;
        });
    }
    match intent {
        Intent::Pause => {
            update_job(app, id, |j| {
                j.status = JobStatus::Paused;
                j.speed = None;
                j.eta = None;
            });
        }
        Intent::Cancel => {
            update_job(app, id, |j| {
                j.status = JobStatus::Cancelled;
                j.speed = None;
                j.eta = None;
                j.partial_files.clear();
            });
        }
        Intent::None => {
            // An empty message means the job was paused or cancelled before it started,
            // and its status already says so.
            let message = outcome.error.clone().unwrap_or_default();
            update_job(app, id, |j| {
                if j.status.is_active() {
                    j.status = JobStatus::Failed;
                    j.error = Some(message);
                }
                j.speed = None;
                j.eta = None;
            });
        }
    }
    None
}

async fn finish(app: &AppHandle, id: &str, outcome: Outcome) {
    let state = app.state::<AppState>();
    let mut completed = None;
    // Remove the entry and settle the job in one critical section. A cancel is sticky
    // (nothing downgrades it), so the files it leaves are collected right there.
    let cancelled = {
        let mut running = state.running.lock().unwrap();
        let entry = running.remove(id);
        let intent = entry.as_ref().map(|r| r.intent).unwrap_or(Intent::None);
        if intent == Intent::Cancel && !outcome.success {
            let job = find_job(app, id);
            let mut item = match &job {
                Some(job) => PendingCleanup::of(job),
                None => PendingCleanup { job_id: id.to_string(), workspace: None, legacy: Vec::new() },
            };
            if let Some(entry) = entry {
                item.legacy.extend(entry.cleanup);
                item.workspace = item.workspace.or(entry.workspace);
            }
            item.legacy.sort();
            item.legacy.dedup();
            settle(app, id, intent, &outcome);
            Some(item)
        } else {
            completed = settle(app, id, intent, &outcome);
            None
        }
    };
    // The final state is durable before any file is deleted or the library mentions it.
    flush_queue_async(app).await;
    if let Some(item) = cancelled.filter(|i| !i.is_empty()) {
        // The process tree is gone, but Windows may release its handles a moment later;
        // `remove_workspace` retries, and a folder still in use is retried at next start.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let app = app.clone();
        let _ = tokio::task::spawn_blocking(move || {
            register_cleanup(&app, &item);
            cleanup_now(&app, item)
        })
        .await;
    }
    if let Some(job) = completed {
        history::record(app, &job);
    }
}

/// For downloads started before work folders existed: removes exactly `<name>.part`,
/// `<name>.ytdl` and `<name>.part-Frag<n>` next to each reported file, never anything
/// else that merely starts with the same name.
pub(crate) fn remove_partials_now(files: &[String]) {
    for file in files {
        let path = Path::new(file);
        let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let entry_name = entry.file_name();
            let Some(entry_name) = entry_name.to_str() else {
                continue;
            };
            let is_file = entry.file_type().is_ok_and(|t| t.is_file());
            if is_file && download_files::is_legacy_partial(name, entry_name) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_saved_by_1_0_4_still_load() {
        // queue.json as 1.0.4 wrote it: none of the fields added since.
        let saved = r#"[
          {"id":"0b7f6a43-5d0c-4f43-9a61-3f8fb4d3a1a0","url":"https://example.com/v","title":"Paused one",
           "thumbnail":null,"uploader":null,"duration":12.0,
           "options":{"mode":"video","maxHeight":1080,"audioFormat":"mp3","subtitleLangs":[],"autoSubs":false,"embedSubs":false},
           "status":"paused","progress":0.4,"speed":null,"eta":null,"filePath":null,"error":null,"createdAt":1,
           "partialFiles":["D:/Downloads/Claws/v.f137.mp4"]},
          {"id":"1c8e7b54-6e1d-4054-8b72-4a9fc5e4b2b1","url":"https://example.com/w","title":"Failed one",
           "thumbnail":null,"uploader":null,"duration":null,
           "options":{"mode":"audio","maxHeight":null,"audioFormat":"m4a","subtitleLangs":[],"autoSubs":false,"embedSubs":false},
           "status":"failed","progress":0.0,"speed":null,"eta":null,"filePath":null,"error":"HTTP Error 403","createdAt":2}
        ]"#;
        let jobs: Vec<Job> = serde_json::from_str(saved).unwrap();
        assert_eq!(jobs.len(), 2);
        assert!(jobs.iter().all(|j| j.execution.is_none() && j.workspace.is_none() && !j.cancel_requested));
        assert_eq!(jobs[0].status, JobStatus::Paused);
        assert_eq!(jobs[0].partial_files.len(), 1);
        assert_eq!(jobs[1].status, JobStatus::Failed);
    }

    #[test]
    fn execution_is_frozen_from_the_settings_at_first_start() {
        let mut settings = Settings { output_dir: "D:/First".into(), ..Settings::default() };
        let frozen = JobExecution::freeze(&settings, false);
        settings.output_dir = "E:/Second".into();
        settings.filename_template = "%(id)s.%(ext)s".into();
        settings.embed_metadata = false;
        assert_eq!(frozen.output_dir, std::path::PathBuf::from("D:/First"));
        assert_eq!(frozen.filename_template, Settings::default().filename_template);
        assert!(frozen.embed_metadata);
        // Survives a save and reload unchanged.
        let round: JobExecution = serde_json::from_str(&serde_json::to_string(&frozen).unwrap()).unwrap();
        assert_eq!(round, frozen);
    }

    #[test]
    fn cancel_is_not_downgraded_by_pause() {
        let mut entry = Running {
            pid: Some(1),
            intent: Intent::None,
            cleanup: Vec::new(),
            workspace: None,
        };
        entry.request(Intent::Cancel);
        entry.request(Intent::Pause);
        assert_eq!(entry.intent, Intent::Cancel);
    }

    #[test]
    fn partials_are_removed_but_not_the_finished_file() {
        let dir = std::env::temp_dir().join(format!("claws-partials-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("clip.f137.mp4");
        for name in ["clip.f137.mp4.part", "clip.f137.mp4.part-Frag3", "clip.f137.mp4.ytdl", "clip.mp4"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        remove_partials_now(&[base.to_string_lossy().into_owned()]);
        let left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, vec!["clip.mp4".to_string()]);
        let _ = std::fs::remove_dir_all(dir);
    }
}

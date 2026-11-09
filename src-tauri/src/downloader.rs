use std::collections::HashSet;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::AppHandle;
use tokio::io::BufReader;

use crate::bounded_io;
use crate::disk_budget;
use crate::download_files::{self, Publication, WorkspaceRef};
use crate::download_policy::{self, FailureKind, JobExecution, Source, VideoPreset};
use crate::error::{notify, AppError, Result};
use crate::history;
use crate::queue_writer::{QueueSnapshot, QUEUE_SCHEMA};
use crate::transcript::{self, TranscriptFormat};
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
/// A transcript run's caption files (a JSON list of paths).
const TAG_CAPTIONS: &str = "CLAWSCAPTIONS|";

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
    /// "video", "audio" or "transcript" (the site's captions as text; no media).
    pub mode: String,
    /// Largest video to fetch, as its shorter side (yt-dlp's `res`, so a 1080×1920 portrait
    /// video is 1080); `None` means the best available. Never upscaled.
    pub max_height: Option<u32>,
    /// Which streams a video keeps and what file they become. Ignored for audio.
    pub video_preset: VideoPreset,
    pub audio_format: String,
    pub subtitle_langs: Vec<String>,
    pub auto_subs: bool,
    pub embed_subs: bool,
    /// Fast Save: no metadata pass and subtitles kept as separate files, so the file is not
    /// rewritten after it downloads. The chosen video and audio streams are unchanged.
    pub fast_save: bool,
    /// What a transcript is saved as. Ignored (and kept at the default) for video and audio.
    pub transcript_format: TranscriptFormat,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            mode: "video".into(),
            max_height: None,
            video_preset: VideoPreset::Compatible,
            audio_format: "mp3".into(),
            subtitle_langs: Vec::new(),
            auto_subs: false,
            embed_subs: false,
            fast_save: false,
            transcript_format: TranscriptFormat::Txt,
        }
    }
}

/// The most subtitle languages one video download can ask for. The picker in
/// src/transcript-options.ts (`MAX_SUBTITLE_LANGS`) enforces the same number; a test ties them.
pub const MAX_SUBTITLE_LANGS: usize = 12;

impl DownloadOptions {
    /// Whether subtitles go into the video (a whole-file pass) rather than next to it.
    pub fn embeds_subtitles(&self) -> bool {
        self.embed_subs && !self.fast_save
    }

    pub(crate) fn validate(&mut self) -> Result<()> {
        if !matches!(self.mode.as_str(), "video" | "audio" | "transcript") {
            return Err(AppError::msg("Unknown download type."));
        }
        if !AUDIO_FORMATS.contains(&self.audio_format.as_str()) {
            return Err(AppError::msg("Unsupported audio format."));
        }
        if !self.video_preset.available() {
            return Err(AppError::msg("The editing preset is not available yet."));
        }
        if self.mode == "audio" {
            // No video, so no video preset: the same audio download has one duplicate key.
            self.video_preset = VideoPreset::Compatible;
        }
        // A shorter side: 4320 is 8K landscape, 8640 the most a portrait 16K could need.
        self.max_height = self.max_height.map(|h| h.clamp(144, 8640));
        let valid_lang = |l: &String| {
            !l.is_empty()
                && l.len() <= 24
                && l.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        };
        if self.mode == "transcript" {
            // One caption track, of the kind `auto_subs` names. Nothing about media applies,
            // so those choices are reset: the same transcript has one duplicate key.
            let one = match self.subtitle_langs.as_slice() {
                [code] => valid_lang(code) && code.starts_with(|c: char| c.is_ascii_alphanumeric()),
                _ => false,
            };
            if !one {
                return Err(AppError::msg("Choose one caption language for the transcript."));
            }
            self.max_height = None;
            self.video_preset = VideoPreset::Compatible;
            self.audio_format = "mp3".into();
            self.embed_subs = false;
            self.fast_save = false;
            return Ok(());
        }
        self.transcript_format = TranscriptFormat::Txt;
        // Every entry is judged before the count, and nothing is dropped: a request that
        // asks for more than the limit is refused, never quietly shortened.
        if !self.subtitle_langs.iter().all(valid_lang) {
            return Err(AppError::msg("Invalid subtitle language."));
        }
        if self.subtitle_langs.len() > MAX_SUBTITLE_LANGS {
            return Err(AppError::msg(format!(
                "Choose at most {MAX_SUBTITLE_LANGS} subtitle languages per download ({} were requested).",
                self.subtitle_langs.len()
            )));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub match_origin: Option<crate::spotify_match::MatchOrigin>,
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
    /// Only for jobs from 1.0.4 or earlier, which wrote straight into the output folder:
    /// the file names yt-dlp reported, so their exact `.part`/`.ytdl` leftovers can be
    /// removed on cancel.
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
    /// Which attempt the job is on. Trying a cancelled job again starts the next one, in a
    /// new work folder, so the cancelled attempt's cleanup can never reach its files.
    #[serde(default)]
    pub attempt: u32,
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
    pub match_origin: Option<crate::spotify_match::MatchOrigin>,
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
    pub(crate) pid: Option<u32>,
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

const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

fn queue_save_error(detail: impl std::fmt::Display) -> AppError {
    AppError::msg(format!("Claws could not save its download queue ({detail})."))
}

/// Saves the queue now and waits until it is on disk (bounded). For transitions that
/// must survive a crash: a job's start (its settings and work folder), a cancel before
/// its files are deleted, a completion before its recovery record goes, and shutdown.
/// The caller must not go on with what depends on it when this fails. Blocking.
pub fn flush_queue(app: &AppHandle) -> Result<()> {
    let state = app.state::<AppState>();
    let revision = state.queue_revision.fetch_add(1, Ordering::SeqCst) + 1;
    state.queue_writer.flush(revision, FLUSH_TIMEOUT).map_err(queue_save_error)
}

async fn flush_queue_async(app: &AppHandle) -> Result<()> {
    let app = app.clone();
    tokio::task::spawn_blocking(move || flush_queue(&app)).await.map_err(queue_save_error)?
}

/// The queue writer, saving `queue_snapshot` to queue.json.
pub fn start_queue_writer(app: &AppHandle) -> crate::queue_writer::QueueWriter {
    let writer_app = app.clone();
    let report_app = app.clone();
    crate::queue_writer::QueueWriter::start(
        Box::new(move || {
            let state = writer_app.state::<AppState>();
            let mut revision = 0;
            store::save_snapshot(&state.queue_path(), || {
                let snapshot = queue_snapshot(&state);
                revision = snapshot.revision;
                snapshot
            })
            .map(|()| revision)
            .map_err(|e| e.to_string())
        }),
        Box::new(move |e| notify(&report_app, format!("Claws could not save the queue: {e}"))),
    )
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

pub fn restore_queue(app: &AppHandle, saved: Vec<Job>) -> std::thread::JoinHandle<()> {
    let mut saved = saved;
    let mut cleanup = Vec::new();
    let mut recovered = Vec::new();
    for job in &mut saved {
        // Committed publication wins over late cancellation, even after Library failure.
        let manifest = job.workspace.as_ref().and_then(|w| download_files::verify(w, &job.id).ok());
        if let Some(manifest) = manifest.filter(|m| m.state == Publication::Published) {
            job.cancel_requested = false;
            // Identity verification can hash gigabytes on FAT/exFAT. Load a safe
            // visible state now, and verify/record it on the recovery worker below.
            job.status = JobStatus::Processing;
            job.speed = None;
            job.eta = None;
            job.error = None;
            recovered.push((job.id.clone(), manifest));
            continue;
        }
        if job.cancel_requested && job.status != JobStatus::Completed {
            job.status = JobStatus::Cancelled;
            cleanup.push(PendingCleanup::of(job));
            job.partial_files.clear();
            continue;
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
    // Claim all recovery jobs before returning: Resume/pump cannot start another
    // worker for one while its identity is being checked in the background.
    for (id, _) in &recovered {
        let workspace = find_job(app, id).and_then(|j| j.workspace);
        state.live_jobs.lock().unwrap().insert(id.clone());
        state.running.lock().unwrap().insert(id.clone(), Running {
            pid: None, intent: Intent::None, cleanup: Vec::new(), workspace,
        });
        state.workers.fetch_add(1, Ordering::SeqCst);
    }
    let app = app.clone();
    std::thread::spawn(move || {
        for (id, manifest) in recovered {
            let slot = WorkerSlot { app: app.clone(), id: id.clone() };
            let state = app.state::<AppState>();
            if !state.shutting_down.load(Ordering::SeqCst) {
                let verified = manifest.validated_media();
                if !state.shutting_down.load(Ordering::SeqCst) {
                    if let Some(mut job) = find_job(&app, &id) {
                        match verified {
                            Ok(media) => {
                                job.file_path = Some(media.to_string_lossy().into_owned());
                                update_job(&app, &id, |j| j.file_path = job.file_path.clone());
                                let known = state.history.lock().unwrap().iter().any(|h| h.id == id);
                                // An entry recorded before kept-file records existed gets its record now,
                                // while the work folder still holds what it points at.
                                let recorded = if known { history::attach_kept(&app, &job) } else { history::record(&app, &job) };
                                match recorded {
                                    Ok(()) => {
                                        if let Some(done) = update_job(&app, &id, |j| {
                                            j.status = JobStatus::Completed;
                                            j.progress = 1.0;
                                            j.progress_known = true;
                                            j.error = None;
                                        }) {
                                            if manifest.withheld().is_empty() {
                                                cleanup.push(PendingCleanup::of(&done));
                                            } else if state.history.lock().unwrap().iter().any(|h| h.id == id && h.kept_files.is_some()) {
                                                // The retained files are recorded with the Library entry: the
                                                // finished job may leave the saved queue, the work folder stays.
                                                persist_queue(&app);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let path = job.file_path.unwrap_or_default();
                                        update_job(&app, &id, |j| {
                                            j.status = JobStatus::Failed;
                                            j.error = Some(format!("The download was saved as {path}, but Claws could not add it to your Library ({e}). Press Try Again to finish; nothing is downloaded again."));
                                        });
                                    }
                                }
                            }
                            Err(e) => {
                                update_job(&app, &id, |j| {
                                    j.status = JobStatus::Failed;
                                    j.file_path = None;
                                    j.error = Some(format!("The published file could not be verified ({e}). Recovery files were kept; restore the original output before trying again."));
                                });
                            }
                        }
                    } else {
                        // Removed from the UI while verification was pending.
                        let workspace = state.running.lock().unwrap().get(&id).and_then(|r| r.workspace.clone());
                        cleanup.push(PendingCleanup { job_id: id.clone(), workspace, legacy: Vec::new() });
                    }
                }
            }
            state.running.lock().unwrap().remove(&id);
            drop(slot);
            pump(&app);
        }
        for item in cleanup {
            cleanup_when_recorded(&app, item);
        }
        retry_pending_cleanup(&app);
    })
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
        if let Some(origin) = &item.match_origin {
            crate::spotify_match::validate_match_origin(origin, &item.url, item.playlist_index, &options)?;
        }
        let key = (url.clone(), item.playlist_index.filter(|i| *i > 0), options_key(&options));
        if pending.contains(&key) || !seen.insert(key) {
            continue;
        }
        created.push(Job {
            id: uuid::Uuid::new_v4().to_string(),
            url,
            match_origin: item.match_origin,
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
            attempt: 0,
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
            update_job(app, id, |j| match j.status {
                JobStatus::Paused | JobStatus::Failed => {
                    j.status = JobStatus::Queued;
                    j.error = None;
                }
                JobStatus::Cancelled => {
                    start_new_attempt(j);
                    j.status = JobStatus::Queued;
                }
                _ => {}
            });
        }
    }
    // The reset is saved by the start's own barrier before anything is written.
    persist_queue(app);
    pump(app);
}

/// Trying a cancelled job again: its files went (or are going) with the cancel, so it
/// starts over in a new attempt and folder, with the cancel cleared. The frozen settings,
/// destination and options are kept.
fn start_new_attempt(job: &mut Job) {
    if job.workspace.take().is_some() {
        job.attempt += 1;
    }
    job.cancel_requested = false;
    job.partial_files.clear();
    job.error = None;
    job.file_path = None;
    job.progress = 0.0;
    job.progress_known = false;
    job.progress_estimated = false;
    job.downloaded_bytes = None;
    job.total_bytes = None;
    job.speed = None;
    job.eta = None;
}

pub fn cancel(app: &AppHandle, id: &str) {
    // A stale Cancel click can arrive after publication but before Library persistence.
    // Do not delete that transaction's recovery record, even when the job is Failed.
    let committed = find_job(app, id)
        .and_then(|j| j.workspace.and_then(|w| download_files::verify(&w, id).ok()))
        .is_some_and(|m| m.state == Publication::Published);
    if committed {
        update_job(app, id, |j| j.cancel_requested = false);
        persist_queue(app);
        return;
    }
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

    /// The same job's same attempt: another attempt's cleanup is a separate entry.
    fn same_files(&self, other: &PendingCleanup) -> bool {
        self.job_id == other.job_id && self.workspace == other.workspace
    }
}

/// Records a cleanup in cleanup.json before anything is deleted, so a crash midway is
/// finished at the next start. Nothing may be deleted when this fails.
fn register_cleanup(app: &AppHandle, item: &PendingCleanup) -> Result<()> {
    let state = app.state::<AppState>();
    {
        let mut pending = state.pending_cleanup.lock().unwrap();
        pending.retain(|p| !p.same_files(item));
        pending.push(item.clone());
    }
    save_pending_cleanup(app)
}

/// Deletes a job's leftovers once the state that explains the deletion and the record
/// that finishes it after a crash are both on disk; otherwise deletes nothing. Blocking.
fn cleanup_when_recorded(app: &AppHandle, item: PendingCleanup) {
    if item.is_empty() {
        return;
    }
    match flush_queue(app).and_then(|()| register_cleanup(app, &item)) {
        Ok(()) => {
            cleanup_now(app, item);
        }
        Err(e) => notify(app, format!("{e} The unfinished files were left where they are; nothing was deleted.")),
    }
}

fn spawn_cleanup(app: &AppHandle, item: PendingCleanup) {
    if item.is_empty() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = tokio::task::spawn_blocking(move || cleanup_when_recorded(&app, item)).await;
    });
}

/// Deletes a job's leftovers now. If Windows still holds a file, the cleanup is saved
/// and retried at the next start rather than reported as done.
fn cleanup_now(app: &AppHandle, item: PendingCleanup) -> bool {
    let state = app.state::<AppState>();
    remove_partials_now(&item.legacy);
    let removed = match &item.workspace {
        // A finished download's retained side file is not an unfinished leftover, even when the job
        // itself failed or was removed before the Library recorded it. The folder stays.
        Some(workspace) if download_files::holds_kept_files(workspace, &item.job_id) => {
            notify(app, format!("A subtitle file that could not be saved next to its video was left in {}. Claws did not delete it.", workspace.path.display()));
            true
        }
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
        pending.retain(|p| !p.same_files(&item));
        if !removed {
            pending.push(item);
        }
        *pending != before
    };
    if changed {
        // A stale entry only repeats a deletion that already happened.
        if let Err(e) = save_pending_cleanup(app) {
            notify(app, e.to_string());
        }
    }
    removed
}

fn save_pending_cleanup(app: &AppHandle) -> Result<()> {
    let state = app.state::<AppState>();
    store::save_snapshot(&state.cleanup_path(), || state.pending_cleanup.lock().unwrap().clone())
        .map_err(|e| AppError::msg(format!("Claws could not save its cleanup list ({e}).")))
}

/// Retries cleanups that could not finish earlier because a file was still in use.
pub fn retry_pending_cleanup(app: &AppHandle) {
    let items = app.state::<AppState>().pending_cleanup.lock().unwrap().clone();
    for item in items {
        cleanup_now(app, item);
    }
}

pub fn remove(app: &AppHandle, id: &str) -> Result<()> {
    let state = app.state::<AppState>();
    let Some(job) = find_job(app, id) else { return Ok(()) };
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
        // Recorded before the job disappears: the queue entry is what finds its files.
        register_cleanup(app, &leftovers).map_err(|e| AppError::msg(format!("{e} The download was not removed.")))?;
    }
    state.jobs.lock().unwrap().retain(|j| j.id != id);
    emit_removed(app, id);
    persist_queue(app);
    if clean {
        spawn_cleanup(app, leftovers);
    }
    Ok(())
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
    // belong to a job that comes back paused. If that save fails, nothing is deleted.
    let mut leftovers: Vec<PendingCleanup> = leftovers.into_iter().filter(|l| !l.is_empty()).collect();
    match flush_queue(app) {
        Ok(()) => leftovers.retain(|item| match register_cleanup(app, item) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("{e} The unfinished files of a cancelled download were kept.");
                false
            }
        }),
        Err(e) => {
            eprintln!("{e} The unfinished files of cancelled downloads were kept.");
            leftovers.clear();
        }
    }
    if !leftovers.is_empty() {
        // Give Windows a moment to release the killed processes' handles.
        std::thread::sleep(Duration::from_millis(300));
        for item in leftovers {
            cleanup_now(app, item);
        }
    }
    // Reported, never assumed: the writer stopping does not mean the last state is saved.
    if let Err(e) = flush_queue(app) {
        eprintln!("{e} Downloads may come back as they were before the last change.");
    }
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
    /// The work folder this run opened, so a cancel can clean it even when the job was
    /// removed before its folder was recorded anywhere else.
    workspace: Option<WorkspaceRef>,
    success: bool,
    /// The files are in the output folder; nothing (not even a cancel) undoes that.
    published: bool,
    /// The work folder still holds something of value (a withheld side file, or a
    /// completion not yet recorded), so it is not cleaned up.
    keep_workspace: bool,
    error: Option<String>,
    final_path: Option<String>,
    /// The caption files a transcript run reported; `None` until (or unless) it does.
    captions: Option<Vec<String>>,
    used_cache: bool,
    /// What kind of failure, when yt-dlp itself failed.
    kind: Option<FailureKind>,
}

impl Outcome {
    fn failed(message: impl Into<String>) -> Self {
        Self {
            workspace: None,
            success: false,
            published: false,
            keep_workspace: false,
            error: Some(message.into()),
            final_path: None,
            captions: None,
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
    if !job.options.video_preset.available() {
        // Saved by a version that can make it; never run as something else.
        return Outcome::failed("This download uses an editing preset this version of Claws cannot make. Remove it, or open the version that added it.");
    }
    let Some(ytdlp) = tools::find_ytdlp(app) else {
        return Outcome::failed("yt-dlp is not installed yet. Open Settings to install it.");
    };
    let settings = state.settings.lock().unwrap().clone();
    let (execution, workspace, manifest) = match prepare(app, &job, &settings).await {
        Ok(prepared) => prepared,
        Err(message) => return Outcome::failed(message),
    };
    let mut outcome = run_prepared(app, &job, &settings, &ytdlp, allow_cache, &execution, &workspace, &manifest).await;
    outcome.workspace = Some(workspace);
    outcome
}

/// The rest of a run, once its settings are frozen and its work folder is open and saved.
#[allow(clippy::too_many_arguments)]
async fn run_prepared(
    app: &AppHandle,
    job: &Job,
    settings: &Settings,
    ytdlp: &Path,
    allow_cache: bool,
    execution: &JobExecution,
    workspace: &WorkspaceRef,
    manifest: &download_files::Manifest,
) -> Outcome {
    let state = app.state::<AppState>();
    let id = job.id.as_str();
    // Downloaded and verified before; only the publication (or recording its completion)
    // is left to do. Registered as running (without a process) so a cancel meanwhile is
    // settled by `finish`.
    if matches!(manifest.state, Publication::Ready | Publication::Publishing | Publication::Published) {
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
            workspace: None,
            success: true,
            published: false,
            keep_workspace: false,
            error: None,
            final_path: None,
            captions: None,
            used_cache: false,
            kind: None,
        };
    }

    // Playlist items picked by index are never cached: the cache holds single videos.
    // The lease keeps the file from being pruned until this run returns.
    let info_json = (allow_cache && job.playlist_index.is_none())
        .then(|| probe::cached_info(app, &job.url))
        .flatten();
    let args = build_args(app, job, settings, execution, &workspace.path, info_json.as_ref().map(|c| c.path()));
    let mut command = tools::command(ytdlp);
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
    if job.options.mode == "transcript" {
        // Only captions are fetched: there are no media bytes to count, so the job shows
        // as processing (an indeterminate bar), never as a made-up download percentage.
        mark_processing(app, id);
    }

    // yt-dlp prints download progress on stdout but post-processing progress and its
    // errors on stderr, so both streams are read as they arrive.
    let stderr = child.stderr.take();
    let stderr_app = app.clone();
    let stderr_id = id.to_string();
    let processing = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stderr_processing = processing.clone();
    // Why the run was stopped early (not enough space), from either stream's reader.
    let stopped: std::sync::Arc<std::sync::Mutex<Option<String>>> = Default::default();
    let stderr_stopped = stopped.clone();
    let stderr_workspace = workspace.path.clone();
    // While the job runs, what reached its work folder stops being reserved. The ledger's
    // lock can wait behind a slow free-space sample, so it is only taken on a blocking
    // thread. Such a refresh may still be queued when the run ends, so it checks
    // `ticker_live` under the lock and never touches a reservation that was released.
    let ticker_live = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let ledger_ticker = {
        let app = app.clone();
        let id = id.to_string();
        let path = workspace.path.clone();
        let live = ticker_live.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(LEDGER_REFRESH).await;
                let (app, id, path, live) = (app.clone(), id.clone(), path.clone(), live.clone());
                let refreshed = tokio::task::spawn_blocking(move || {
                    let on_disk = disk_budget::bytes_in(&path);
                    let state = app.state::<AppState>();
                    let mut ledger = disk_budget::lock(&state.disk_ledger);
                    if live.load(Ordering::SeqCst) {
                        ledger.update(&id, on_disk);
                    }
                })
                .await;
                if refreshed.is_err() {
                    break;
                }
            }
        })
    };
    let stderr_task = tauri::async_runtime::spawn(async move {
        // Only the last part is kept: the final error is what explains a failure.
        let mut text = String::new();
        let Some(stream) = stderr else { return text };
        let mut reader = BufReader::new(stream);
        let mut buf = Vec::new();
        while bounded_io::read_line_capped(&mut reader, &mut buf, bounded_io::MAX_LINE).await.is_some() {
            let line = String::from_utf8_lossy(&buf);
            let line = line.trim_end();
            if let Some(rest) = line.strip_prefix(TAG_POST) {
                if rest.starts_with("started") {
                    // A pre-download postprocessor does not make a later transfer failure a processing one.
                    let final_stage = starts_final_processing(rest);
                    if final_stage {
                        stderr_processing.store(true, Ordering::SeqCst);
                    }
                    postprocessor_started(&stderr_app, &stderr_id, rest);
                    // Best-effort notification check; this cannot gate yt-dlp before processing.
                    // Only the final stage is checked: what a pre-download postprocessor leaves to
                    // be done is the whole transfer, which admission already judged, and a
                    // shortfall found now would stop the job with a processing message.
                    let short = if final_stage { recheck_processing_space(&stderr_app, &stderr_id, &stderr_workspace).await } else { None };
                    if let Some(reason) = short {
                        // A disk sample may finish after the process has been reaped.
                        if let Some(pid) = stderr_app.state::<AppState>().running.lock().unwrap().get(&stderr_id).and_then(|r| r.pid) {
                            kill_tree(pid, false);
                        }
                        stderr_stopped.lock().unwrap().get_or_insert(reason);
                    }
                }
            } else {
                text.push_str(line);
                text.push('\n');
                if text.len() > bounded_io::DIAGNOSTIC_TAIL * 2 {
                    let mut cut = text.len() - bounded_io::DIAGNOSTIC_TAIL;
                    while !text.is_char_boundary(cut) {
                        cut += 1;
                    }
                    text.drain(..cut);
                }
            }
        }
        text
    });

    let mut outcome = Outcome {
        workspace: None,
        success: false,
        published: false,
        keep_workspace: false,
        error: None,
        final_path: None,
        captions: None,
        used_cache: info_json.is_some(),
        kind: None,
    };
    let mut progress = TransferProgress::new(&job.options.mode);
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let mut budget = Budget::new(job, execution, &workspace.path);
    let pid = child.id();

    if let Some(stdout) = child.stdout.take() {
        let mut reader = BufReader::new(stdout);
        let mut buf = Vec::new();
        loop {
            // An overlong line is cut (and the rest drained), never buffered whole.
            match bounded_io::read_line_capped(&mut reader, &mut buf, bounded_io::MAX_LINE).await {
                None => break,
                Some(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    let line = line.trim_end();
                    handle_line(app, id, line, &mut progress, &mut last_emit, &mut outcome);
                    if stopped.lock().unwrap().is_none() {
                        if let Some(reason) = budget.after_line(app, id, line, &progress).await {
                            // Not enough space: stop before the transfer (or before the
                            // disk fills), keeping what is already downloaded.
                            if let Some(pid) = pid {
                                kill_tree(pid, false);
                            }
                            stopped.lock().unwrap().get_or_insert(reason);
                        }
                    }
                }
            }
        }
    }

    let status = child.wait().await;
    ledger_ticker.abort();
    ticker_live.store(false, Ordering::SeqCst);
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
    let stopped = stopped.lock().unwrap().take();
    if let Some(reason) = stopped.filter(|_| !outcome.success) {
        outcome.success = false;
        outcome.kind = Some(FailureKind::LocalIo);
        outcome.error = Some(reason);
    }
    outcome
}

/// Disk-space checks for one run: once the selection (and so the sizes) is known, the
/// estimated peak is checked and reserved; when a size is unknown, free space is watched.
struct Budget {
    workspace: std::path::PathBuf,
    volume: String,
    options: DownloadOptions,
    policy: download_policy::ProcessingPolicy,
    duration: Option<f64>,
    checked: bool,
    watching: bool,
    last_watch: Instant,
}

impl Budget {
    fn new(job: &Job, execution: &JobExecution, workspace: &Path) -> Self {
        Self {
            workspace: workspace.to_path_buf(),
            volume: disk_budget::volume_key(workspace),
            options: job.options.clone(),
            policy: download_policy::ProcessingPolicy::new(&job.options, execution),
            duration: job.duration,
            checked: false,
            watching: false,
            last_watch: Instant::now(),
        }
    }

    /// The sizing plan admission reserves from: the policy of this job's own options and
    /// frozen settings, the same one the size preview builds (estimate.rs).
    fn plan(&self) -> disk_budget::RunPlan<'_> {
        self.policy.run_plan(&self.options, self.duration)
    }

    /// A reason to stop, if the volume cannot hold this download.
    async fn after_line(&mut self, app: &AppHandle, id: &str, line: &str, progress: &TransferProgress) -> Option<String> {
        if !self.checked && line.starts_with(TAG_FORMATS) {
            self.checked = true;
            let Some(sizes) = progress.planned_sizes() else {
                self.watching = true;
                return None;
            };
            let Some(peak) = self.plan().peak(&sizes) else {
                self.watching = true;
                return None;
            };
            let (app, id, workspace, volume) = (app.clone(), id.to_string(), self.workspace.clone(), self.volume.clone());
            // The directory walk, the free-space reading and the ledger's lock can all wait
            // on a slow disk, so they run together on a blocking thread, not an async worker.
            // The reading and the decision stay together under the ledger's lock.
            let admission = tokio::task::spawn_blocking(move || {
                let on_disk = disk_budget::bytes_in(&workspace);
                let state = app.state::<AppState>();
                disk_budget::admit_sampled(&state.disk_ledger, &state.free_space, &id, &volume, &workspace, peak, on_disk)
            })
            .await;
            return match settled_admission(admission) {
                disk_budget::Admission::Admitted => None,
                // An unreadable volume is not evidence of a shortfall: watch instead.
                disk_budget::Admission::Unreadable => {
                    self.watching = true;
                    None
                }
                disk_budget::Admission::Refused(short) => Some(format!(
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
            let app = app.clone();
            let free = tokio::task::spawn_blocking(move || (app.state::<AppState>().free_space)(&workspace)).await.ok()?.ok()?;
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

/// The admission decision once its blocking thread is done. A panic there (a failing
/// free-space provider or ledger) is no evidence of room: it is logged and degrades to the
/// disk watch, as an unreadable volume does, instead of silently admitting the job.
fn settled_admission(joined: std::result::Result<disk_budget::Admission, tokio::task::JoinError>) -> disk_budget::Admission {
    joined.unwrap_or_else(|e| {
        eprintln!("Checking free space for a download failed ({e}); the disk is watched instead.");
        disk_budget::Admission::Unreadable
    })
}

/// How often a running job's work folder is measured to shrink its reservation.
const LEDGER_REFRESH: Duration = Duration::from_secs(2);

/// Refreshes the job's reservation from its work folder (best effort, on a processing
/// notification) and returns a reason to stop if what it still needs no longer fits.
async fn recheck_processing_space(app: &AppHandle, id: &str, workspace: &Path) -> Option<String> {
    // The walk, the reading and the ledger's lock run on a blocking thread (see `Budget::after_line`).
    let (app, id, sample) = (app.clone(), id.to_string(), workspace.to_path_buf());
    let short = tokio::task::spawn_blocking(move || {
        let on_disk = disk_budget::bytes_in(&sample);
        let state = app.state::<AppState>();
        disk_budget::recheck_sampled(&state.disk_ledger, &state.free_space, &id, &sample, on_disk)
    })
    .await
    // A panic there is no evidence of a shortfall: no reason to stop the job.
    .map_err(|e| eprintln!("Rechecking free space during processing failed: {e}"))
    .ok()??;
    Some(format!(
        "Not enough free space on {} to finish processing this download. It needs about {} more (estimated, including a safety margin) and about {} is available. Free up space, then press Try Again; the downloaded files are kept in the work folder.",
        disk_budget::volume_key(workspace).to_uppercase(),
        human_bytes(short.required),
        human_bytes(short.available),
    ))
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

/// Postprocessors yt-dlp runs before the download starts, in the options Claws asks for
/// (`--convert-subs` runs before the media is fetched). They announce themselves with the
/// same signal as the final stage, but a transfer still follows them.
const PRE_DOWNLOAD_PROCESSORS: [&str; 4] = ["FFmpegSubtitlesConvertor", "FFmpegThumbnailsConvertor", "MetadataParser", "SponsorBlock"];

/// Whether a `started|<postprocessor>` line of the postprocessing stream announces the
/// final processing stage (merge, audio extraction, embedding, moves), which follows the
/// last byte and is authoritative until the process exits. A pre-download postprocessor
/// leaves the transfer stage alone.
fn starts_final_processing(rest: &str) -> bool {
    let name = rest.split('|').nth(1).unwrap_or_default().trim();
    !PRE_DOWNLOAD_PROCESSORS.contains(&name)
}

fn postprocessor_started(app: &AppHandle, id: &str, rest: &str) {
    if starts_final_processing(rest) {
        mark_processing(app, id);
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
    } else if let Some(rest) = line.strip_prefix(TAG_CAPTIONS) {
        outcome.captions = serde_json::from_str::<Vec<String>>(rest.trim()).ok();
    } else if let Some(rest) = line.strip_prefix(TAG_POST) {
        if rest.starts_with("started") {
            postprocessor_started(app, id, rest);
        }
    } else if let Some(rest) = line.strip_prefix(TAG_PROGRESS) {
        // status|downloaded|total|estimate|speed|eta|fragment|fragments|format_id|filename
        // (the file name goes last and may itself contain '|').
        let fields: Vec<&str> = rest.splitn(10, '|').collect();
        // Every file this run writes is inside the work folder, which cleanup removes
        // whole, so no per-file bookkeeping is needed.
        let Some(snapshot) = progress.update(&fields) else { return };

        let finished = fields[0] == "finished";
        if finished || last_emit.elapsed() >= Duration::from_millis(150) {
            *last_emit = Instant::now();
            update_job(app, id, |j| {
                // Transfer figures belong to the transfer stage only. The two pipes are
                // read independently, so a line that was buffered while the final
                // processing began (or the job was paused or cancelled) arrives late and
                // must not restore the transfer controls, speed or ETA.
                if j.status != JobStatus::Downloading {
                    return;
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
    let attempt = job.attempt;
    let (workspace, manifest) = tokio::task::spawn_blocking(move || download_files::open_workspace(&output, &id, attempt))
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
    }
    // Every start: the job's settings, work folder and state are on disk before yt-dlp
    // may write a byte. A failed save starts nothing and keeps the work folder.
    flush_queue_async(app)
        .await
        .map_err(|e| format!("{e} The download was not started and nothing was lost. Press Try Again once the problem is fixed."))?;
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
    let captions = outcome.captions.clone();
    let options = job.options.clone();
    let wants_sidecars = !job.options.subtitle_langs.is_empty() && !job.options.embeds_subtitles();
    let app_for_reservations = app.clone();
    let id_owned = id.to_string();
    // Caption cues of a transcript that could not be read and were left out.
    let mut unreadable = 0;
    let result = tokio::task::spawn_blocking(move || -> std::result::Result<(download_files::Published, usize), String> {
        let mut manifest = download_files::verify(&workspace, &id_owned).map_err(|e| e.to_string())?;
        let mut unreadable = 0;
        if manifest.state == Publication::Active {
            let outputs = if options.mode == "transcript" {
                // The text made from the captions is the one output; the raw captions file
                // stays in the work folder and goes with it.
                let artifact = transcript::from_reported(&workspace.path, captions.as_deref(), &options)?;
                unreadable = artifact.skipped;
                vec![artifact.relative.to_string_lossy().into_owned()]
            } else {
                let reported = reported.ok_or("yt-dlp did not report the finished file.")?;
                let media = download_files::owned_relative(&workspace.path, Path::new(&reported))
                    .ok_or("yt-dlp reported a finished file outside the download's work folder.")?;
                let mut outputs = vec![media.to_string_lossy().into_owned()];
                if wants_sidecars {
                    outputs.extend(download_files::sidecars(&workspace.path, &media).iter().map(|p| p.to_string_lossy().into_owned()));
                }
                outputs
            };
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
        result
            .map(|published| (published, unreadable))
            .map_err(|e| {
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
    .unwrap_or_else(|e| Err(e.to_string()))
    .map(|(published, skipped)| {
        unreadable = skipped;
        published
    });

    match result {
        Ok(published) => {
            if unreadable > 0 {
                let cues = if unreadable == 1 { "1 caption" } else { "some captions" };
                notify(app, format!("Saved the transcript of “{}”, but {cues} in the site's file could not be read and {} left out.", job.title, if unreadable == 1 { "was" } else { "were" }));
            }
            outcome.published = true;
            outcome.final_path = Some(published.media.to_string_lossy().into_owned());
            // The work folder (and its manifest, the recovery record) stays until `finish`
            // has recorded the completion.
            if !published.skipped.is_empty() {
                outcome.keep_workspace = true;
                notify(app, skipped_notice(&job.title, &published));
            }
        }
        Err(message) => {
            outcome.success = false;
            outcome.error = Some(message);
        }
    }
}

/// What to tell the user about side files that were not published. Only a file the work
/// folder's marker records as withheld can be shown again (Show kept files); for any other
/// the notice must not promise it.
fn skipped_notice(title: &str, published: &download_files::Published) -> String {
    let names = |paths: Vec<&std::path::PathBuf>| paths.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect::<Vec<_>>().join(", ");
    let kept: Vec<_> = published.skipped.iter().filter(|p| published.kept.contains(p)).collect();
    let lost: Vec<_> = published.skipped.iter().filter(|p| !published.kept.contains(p)).collect();
    let mut text = format!("Saved “{title}”");
    if !kept.is_empty() {
        text.push_str(&format!(", but {} could not be saved next to the video (the name may be taken), so the new copy was kept in the download's work folder (Show kept files in your Library opens it).", names(kept)));
    }
    if !lost.is_empty() {
        let (count, joined) = (lost.len(), names(lost));
        let lead = if text.ends_with('.') { " " } else { ", but " };
        text.push_str(&format!("{lead}{joined} could not be saved and {} left out.", if count == 1 { "was" } else { "were" }));
    }
    text
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

    // The process is gone, so its files stop growing (publication only moves them on the
    // same volume): the claim goes before the job is settled and could be retried.
    // (On a blocking thread: the ledger's lock can wait behind a slow free-space sample.)
    {
        let (app, id) = (app.clone(), id.clone());
        // A ledger poisoned by a panicking sample still lets this claim go.
        let released = tokio::task::spawn_blocking(move || disk_budget::lock(&app.state::<AppState>().disk_ledger).release(&id))
        .await;
        if let Err(e) = released {
            eprintln!("Releasing a download's disk reservation failed: {e}");
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

/// Records a published download in the Library. Until this succeeds, the job's queue
/// entry and its work folder's manifest are what recover it.
async fn record_in_library(app: &AppHandle, id: &str, outcome: &Outcome) -> std::result::Result<(), String> {
    // Removed while finishing: the file is in the output folder; there is no entry to show.
    let Some(mut job) = find_job(app, id) else { return Ok(()) };
    job.file_path = outcome.final_path.clone();
    let path = job.file_path.clone().unwrap_or_default();
    let app = app.clone();
    tokio::task::spawn_blocking(move || history::record(&app, &job))
        .await
        .map_err(|e| AppError::msg(e.to_string()))
        .and_then(|saved| saved)
        .map_err(|e| format!("The download was saved as “{path}”, but Claws could not add it to your Library ({e}). Press Try Again to finish; nothing is downloaded again."))
}

async fn finish(app: &AppHandle, id: &str, mut outcome: Outcome) {
    let state = app.state::<AppState>();
    // Library first, then the queue, then the work folder: a crash between any two leaves
    // a record that completes the job at the next start.
    if outcome.success {
        if let Err(message) = record_in_library(app, id, &outcome).await {
            outcome.success = false;
            outcome.keep_workspace = true;
            outcome.error = Some(message);
        }
    }
    let mut completed = None;
    // Remove the entry and settle the job in one critical section. A cancel is sticky
    // (nothing downgrades it), so the files it leaves are collected right there.
    let cancelled = {
        let mut running = state.running.lock().unwrap();
        let entry = running.remove(id);
        // Clear even a cancel that arrived during the asynchronous Library save.
        // Holding running makes this reset and final settlement atomic with Cancel.
        if outcome.published {
            update_job(app, id, |j| j.cancel_requested = false);
        }
        let intent = match entry.as_ref().map(|r| r.intent) {
            Some(_) if outcome.published => Intent::None,
            Some(intent) => intent,
            None => Intent::None,
        };
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
            item.workspace = item.workspace.or_else(|| outcome.workspace.clone());
            item.legacy.sort();
            item.legacy.dedup();
            settle(app, id, intent, &outcome);
            Some(item)
        } else {
            completed = settle(app, id, intent, &outcome);
            None
        }
    };
    if let Some(item) = cancelled.filter(|i| !i.is_empty()) {
        // The process tree is gone, but Windows may release its handles a moment later;
        // `remove_workspace` retries, and a folder still in use is retried at next start.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let app = app.clone();
        let _ = tokio::task::spawn_blocking(move || cleanup_when_recorded(&app, item)).await;
    } else {
        match flush_queue_async(app).await {
            // In the Library, and retired from the queue: only the marker and disposable
            // leftovers remain in the work folder.
            Ok(()) if outcome.success && !outcome.keep_workspace => {
                let workspace = completed.as_ref().and_then(|j| j.workspace.clone()).or(outcome.workspace);
                spawn_cleanup(app, PendingCleanup { job_id: id.to_string(), workspace, legacy: Vec::new() });
            }
            Ok(()) => {}
            // The queue still has the job; its work folder stays for the next start.
            Err(e) => notify(app, e.to_string()),
        }
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

    fn budget_for(format: &str, fast_save: bool, embed_metadata: bool) -> Budget {
        let options = DownloadOptions { mode: "audio".into(), audio_format: format.into(), fast_save, ..DownloadOptions::default() };
        let job: Job = serde_json::from_value(serde_json::json!({
            "id": "budget-test", "url": "https://fixture.invalid/a", "title": "T", "thumbnail": null, "uploader": null,
            "duration": 60.0, "options": options, "status": "queued", "progress": 0.0, "speed": null, "eta": null,
            "filePath": null, "error": null, "createdAt": 1
        }))
        .unwrap();
        let settings = crate::settings::Settings { embed_metadata, ..crate::settings::Settings::default() };
        let execution = JobExecution::freeze(&settings, fast_save);
        Budget::new(&job, &execution, &std::env::temp_dir())
    }

    #[test]
    fn the_notice_about_skipped_side_files_promises_only_what_can_be_found_again() {
        let published = |skipped: &[&str], kept: &[&str]| download_files::Published {
            media: std::path::PathBuf::from("D:/Out/clip.mp4"),
            skipped: skipped.iter().map(|n| std::path::PathBuf::from("D:/Out").join(n)).collect(),
            kept: kept.iter().map(|n| std::path::PathBuf::from("D:/Out").join(n)).collect(),
        };
        // Kept and reachable: say where to find it.
        let kept = skipped_notice("Clip", &published(&["clip.en.srt"], &["clip.en.srt"]));
        assert!(kept.contains("clip.en.srt") && kept.contains("Show kept files"), "{kept}");
        // Not recorded: no promise of a copy, and the file is named as left out.
        let lost = skipped_notice("Clip", &published(&["clip.es.srt"], &[]));
        assert!(lost.contains("clip.es.srt") && lost.contains("left out"), "{lost}");
        assert!(!lost.contains("Show kept files") && !lost.contains("work folder"), "{lost}");
        // A mix names each file under what happened to it.
        let mixed = skipped_notice("Clip", &published(&["clip.en.srt", "clip.es.srt"], &["clip.en.srt"]));
        assert!(mixed.contains("clip.en.srt could not be saved next to the video") && !mixed.contains("already existed") && mixed.contains("Show kept files"), "{mixed}");
        assert!(mixed.contains("clip.es.srt could not be saved and was left out"), "{mixed}");
    }

    #[test]
    fn admission_reserves_from_the_jobs_own_processing_choices() {
        let peak = |format, fast_save, metadata| budget_for(format, fast_save, metadata).plan().peak(&[Some(100)]);
        // MP3: the cover-art pass rewrites the output, so the output is held twice (S + 2 * O).
        assert_eq!(peak("mp3", false, false), Some(300));
        // Fast Save skips it: S + O.
        assert_eq!(peak("mp3", true, false), Some(200));
        // Metadata alone rewrites an Opus output; Fast Save drops that too.
        assert_eq!(peak("opus", false, true), Some(300));
        assert_eq!(peak("opus", true, true), Some(200));
        assert_eq!(peak("opus", false, false), Some(200));
        // "Original" is extracted as well, so it is sized as a possible whole copy.
        assert_eq!(peak("best", true, false), Some(200));
    }

    #[test]
    fn a_panic_while_sampling_space_degrades_to_the_watch_not_to_admission() {
        let joined = tauri::async_runtime::block_on(async { tokio::task::spawn_blocking(|| -> disk_budget::Admission { panic!("the sample panicked") }).await });
        assert!(joined.is_err());
        assert_eq!(settled_admission(joined), disk_budget::Admission::Unreadable);
        assert_eq!(settled_admission(Ok(disk_budget::Admission::Admitted)), disk_budget::Admission::Admitted);
    }

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
    fn validation_accepts_source_resolutions_up_to_8k_and_never_the_unbuilt_preset() {
        let video = |max_height, video_preset| DownloadOptions { max_height, video_preset, ..DownloadOptions::default() };
        let mut eight_k = video(Some(4320), VideoPreset::Original);
        eight_k.validate().unwrap();
        assert_eq!(eight_k.max_height, Some(4320));
        // The cap is a shorter side; 8640 is the largest accepted, and nothing is upscaled.
        let mut huge = video(Some(20_000), VideoPreset::Compatible);
        huge.validate().unwrap();
        assert_eq!(huge.max_height, Some(8640));
        let mut tiny = video(Some(10), VideoPreset::Compatible);
        tiny.validate().unwrap();
        assert_eq!(tiny.max_height, Some(144));

        let err = video(None, VideoPreset::EditingProRes).validate().unwrap_err().to_string();
        assert_eq!(err, "The editing preset is not available yet.");
        let parsed: DownloadOptions = serde_json::from_str(r#"{"mode":"video","videoPreset":"editingProRes"}"#).unwrap();
        assert!(parsed.clone().validate().is_err(), "a preset from the frontend is checked again here");
        assert!(serde_json::from_str::<DownloadOptions>(r#"{"videoPreset":"prores4444"}"#).is_err(), "unknown presets are refused");

        // Audio has no video preset: one key for the same audio download however it is sent.
        let mut audio = DownloadOptions { mode: "audio".into(), video_preset: VideoPreset::Original, ..DownloadOptions::default() };
        audio.validate().unwrap();
        assert_eq!(audio.video_preset, VideoPreset::Compatible);
        let mut plain_audio = DownloadOptions { mode: "audio".into(), ..DownloadOptions::default() };
        plain_audio.validate().unwrap();
        assert_eq!(options_key(&audio), options_key(&plain_audio));
    }

    #[test]
    fn presets_are_part_of_the_duplicate_key_and_serialize_by_name() {
        let compatible = DownloadOptions::default();
        let original = DownloadOptions { video_preset: VideoPreset::Original, ..DownloadOptions::default() };
        assert_ne!(options_key(&compatible), options_key(&original), "the same link may be queued once per preset");
        assert!(options_key(&original).contains(r#""videoPreset":"original""#));
        assert!(options_key(&compatible).contains(r#""videoPreset":"compatible""#));
        let pro: DownloadOptions = serde_json::from_str(r#"{"videoPreset":"editingProRes"}"#).unwrap();
        assert_eq!(pro.video_preset, VideoPreset::EditingProRes);
        // A job saved before presets reads back with the same key a new Compatible job has.
        let old: DownloadOptions = serde_json::from_str(r#"{"mode":"video","maxHeight":null,"audioFormat":"mp3","subtitleLangs":[],"autoSubs":false,"embedSubs":false,"fastSave":false}"#).unwrap();
        assert_eq!(options_key(&old), options_key(&compatible));
    }

    fn transcript(langs: &[&str]) -> DownloadOptions {
        DownloadOptions { mode: "transcript".into(), subtitle_langs: langs.iter().map(|l| l.to_string()).collect(), ..DownloadOptions::default() }
    }

    #[test]
    fn a_transcript_needs_exactly_one_language_and_drops_media_choices() {
        for langs in [&[][..], &["en", "fr"][..], &["-en"][..], &["en,fr"][..], &[""][..]] {
            let err = transcript(langs).validate().unwrap_err().to_string();
            assert!(err.contains("caption language"), "{langs:?}: {err}");
        }
        let mut busy = DownloadOptions {
            max_height: Some(1080),
            video_preset: VideoPreset::Original,
            audio_format: "flac".into(),
            embed_subs: true,
            fast_save: true,
            auto_subs: true,
            transcript_format: TranscriptFormat::Srt,
            ..transcript(&["en-orig"])
        };
        busy.validate().unwrap();
        assert_eq!(busy.max_height, None);
        assert_eq!(busy.video_preset, VideoPreset::Compatible);
        assert!(!busy.embed_subs && !busy.fast_save);
        assert!(busy.auto_subs, "which kind of captions is kept");
        assert_eq!(busy.transcript_format, TranscriptFormat::Srt);
        // The same transcript has one duplicate key however the media choices were sent.
        let mut plain = DownloadOptions { auto_subs: true, transcript_format: TranscriptFormat::Srt, ..transcript(&["en-orig"]) };
        plain.validate().unwrap();
        assert_eq!(options_key(&busy), options_key(&plain));
        // Human and automatic captions are different transcripts.
        let mut human = DownloadOptions { transcript_format: TranscriptFormat::Srt, ..transcript(&["en-orig"]) };
        human.validate().unwrap();
        assert_ne!(options_key(&human), options_key(&plain));
    }

    fn video_with(langs: Vec<String>) -> DownloadOptions {
        DownloadOptions { subtitle_langs: langs, ..DownloadOptions::default() }
    }

    fn codes(count: usize) -> Vec<String> {
        (0..count).map(|i| format!("l{i}")).collect()
    }

    #[test]
    fn twelve_subtitle_languages_pass_and_more_are_refused_untouched() {
        let mut twelve = video_with(codes(12));
        twelve.validate().unwrap();
        assert_eq!(twelve.subtitle_langs, codes(12));
        for count in [13, 14, 40] {
            let mut over = video_with(codes(count));
            let err = over.validate().unwrap_err().to_string();
            assert!(err.contains("12"), "{count}: the limit is named: {err}");
            assert!(err.to_lowercase().contains("subtitle"), "{count}: {err}");
            assert_eq!(over.subtitle_langs, codes(count), "{count}: a refused request is never truncated");
        }
        // Audio downloads carry no subtitles, but the same request is not quietly accepted.
        let mut audio = DownloadOptions { mode: "audio".into(), subtitle_langs: codes(13), ..DownloadOptions::default() };
        assert!(audio.validate().is_err());
    }

    #[test]
    fn invalid_language_syntax_is_refused_wherever_it_sits() {
        for (position, bad) in [(0, ""), (11, "bad lang"), (12, "bad lang"), (13, "x;y"), (14, "en,fr"), (30, "a".repeat(25).as_str())] {
            let mut langs = codes(position.max(12) + 1);
            langs[position] = bad.to_string();
            let mut options = video_with(langs);
            let err = options.validate().unwrap_err().to_string();
            // Syntax is judged for every entry first, so a bad code past the twelfth is
            // reported as bad rather than hidden by a truncation (or by the limit).
            assert!(err.contains("Invalid subtitle language"), "{position} {bad:?}: {err}");
        }
        // Twelve valid codes with a thirteenth that is invalid are not a valid request of twelve.
        let mut thirteen = codes(12);
        thirteen.push("not valid".into());
        assert!(video_with(thirteen).validate().is_err());
    }

    #[test]
    fn the_transcript_language_rule_is_separate_from_the_video_limit() {
        // Thirteen languages: a transcript still says it needs one, not that a limit was hit.
        let err = transcript(&["l0", "l1", "l2", "l3", "l4", "l5", "l6", "l7", "l8", "l9", "l10", "l11", "l12"]).validate().unwrap_err().to_string();
        assert!(err.contains("caption language"), "{err}");
        assert!(!err.contains("12"), "{err}");
        // A video of twelve is fine; a transcript of two is not.
        assert!(video_with(codes(12)).validate().is_ok());
        assert!(transcript(&["en", "fr"]).validate().is_err());
        assert!(transcript(&["en"]).validate().is_ok());
    }

    #[test]
    fn the_subtitle_limit_matches_the_interface_constant() {
        // src/transcript-options.ts advertises the limit the picker enforces; a drift would
        // let the picker offer what validate refuses (or hide what it accepts).
        let source = include_str!("../../src/transcript-options.ts");
        let line = source
            .lines()
            .find(|l| l.trim_start().starts_with("export const MAX_SUBTITLE_LANGS"))
            .expect("transcript-options.ts exports MAX_SUBTITLE_LANGS");
        let value: usize = line.split('=').nth(1).and_then(|v| v.trim().trim_end_matches(';').trim().parse().ok()).expect("a number");
        assert_eq!(value, MAX_SUBTITLE_LANGS);
        assert_eq!(MAX_SUBTITLE_LANGS, 12);
    }

    #[test]
    fn the_transcript_format_is_ignored_outside_transcripts_and_saved_by_name() {
        let mut video = DownloadOptions { transcript_format: TranscriptFormat::Vtt, ..DownloadOptions::default() };
        video.validate().unwrap();
        assert_eq!(video.transcript_format, TranscriptFormat::Txt);
        assert_eq!(options_key(&video), options_key(&DownloadOptions::default()));
        let timed = DownloadOptions { transcript_format: TranscriptFormat::TxtTimestamps, ..transcript(&["ja"]) };
        assert!(options_key(&timed).contains(r#""transcriptFormat":"txtTimestamps""#));
        assert!(serde_json::from_str::<DownloadOptions>(r#"{"mode":"transcript","transcriptFormat":"docx"}"#).is_err(), "unknown formats are refused");
        // Saved before transcripts existed: the default, so the key is unchanged.
        let old: DownloadOptions = serde_json::from_str(r#"{"mode":"audio","maxHeight":null,"videoPreset":"compatible","audioFormat":"m4a","subtitleLangs":[],"autoSubs":false,"embedSubs":false,"fastSave":false}"#).unwrap();
        assert_eq!(old.transcript_format, TranscriptFormat::Txt);
        assert!(DownloadOptions { mode: "lyrics".into(), ..DownloadOptions::default() }.validate().is_err());
    }

    #[test]
    fn a_queue_saved_before_transcripts_still_loads_and_a_transcript_job_round_trips() {
        let saved = r#"{"schemaVersion":3,"revision":4,"jobs":[
          {"id":"j1","url":"https://example.com/v","title":"Old","thumbnail":null,"uploader":null,"duration":null,
           "options":{"mode":"video","maxHeight":2160,"videoPreset":"original","audioFormat":"mp3","subtitleLangs":["en"],"autoSubs":false,"embedSubs":true,"fastSave":false},
           "status":"paused","progress":0.5,"speed":null,"eta":null,"filePath":null,"error":null,"createdAt":1}]}"#;
        let snapshot: QueueSnapshot = serde_json::from_str(saved).unwrap();
        assert_eq!(snapshot.jobs[0].options.transcript_format, TranscriptFormat::Txt);
        assert_eq!(snapshot.jobs[0].options.video_preset, VideoPreset::Original);
        let mut job = snapshot.jobs[0].clone();
        job.options = DownloadOptions { auto_subs: true, transcript_format: TranscriptFormat::TxtTimestamps, ..transcript(&["en"]) };
        let again: Job = serde_json::from_str(&serde_json::to_string(&job).unwrap()).unwrap();
        assert_eq!(again.options.mode, "transcript");
        assert_eq!(again.options.transcript_format, TranscriptFormat::TxtTimestamps);
        assert_eq!(options_key(&again.options), options_key(&job.options));
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

    /// A transfer line that was already buffered when the job was stopped reaches the
    /// event path after the job is settled: it must change nothing.
    fn a_late_transfer_line_leaves_a_stopped_job_alone(tag: &str, stop: fn(&AppHandle, &str), stopped: JobStatus) {
        use crate::lifecycle_tests::{wait_until, TestApp};
        let Some(t) = TestApp::new(tag) else { return };
        let id = t.enqueue("https://fixture.invalid/hang/lateevent");
        t.wait_job(&id, |j| j.status == JobStatus::Downloading && j.downloaded_bytes.is_some(), "the download to run");
        stop(t.handle(), &id);
        t.wait_job(&id, |j| j.status == stopped, "the stop to settle");
        wait_until("the worker to finish", || !t.state().live_jobs.lock().unwrap().contains(&id));
        let before = t.job(&id).unwrap();
        assert_eq!(before.downloaded_bytes, Some(7));

        let mut progress = TransferProgress::new("video");
        let mut last_emit = Instant::now() - Duration::from_secs(1);
        let mut outcome = Outcome::failed("");
        for line in [
            "CLAWS|downloading|5000|9999|NA|123456|3|NA|NA|137|x.f137.mp4",
            "CLAWS|finished|9999|9999|NA|NA|NA|NA|NA|137|x.f137.mp4",
        ] {
            handle_line(t.handle(), &id, line, &mut progress, &mut last_emit, &mut outcome);
            last_emit = Instant::now() - Duration::from_secs(1);
        }
        let after = t.job(&id).unwrap();
        assert_eq!(after.status, stopped, "a late line changed the settled state");
        assert_eq!(after.downloaded_bytes, before.downloaded_bytes, "a late line overwrote the stopped job's figures");
        assert_eq!((after.total_bytes, after.progress, after.speed, after.eta), (before.total_bytes, before.progress, None, None));
    }

    #[test]
    fn a_late_transfer_line_does_not_touch_a_paused_job() {
        a_late_transfer_line_leaves_a_stopped_job_alone("w5-late-pause", pause, JobStatus::Paused);
    }

    #[test]
    fn a_late_transfer_line_does_not_touch_a_cancelled_job() {
        a_late_transfer_line_leaves_a_stopped_job_alone("w5-late-cancel", cancel, JobStatus::Cancelled);
    }

    #[test]
    fn only_the_final_stage_is_authoritative_processing() {
        // Merge, audio extraction, subtitle and thumbnail embedding, metadata and the move
        // follow the transfer; the subtitle conversion that precedes it does not.
        // The postprocessor keys yt-dlp reports (`pp_key()`), as spelled in its source.
        for name in [
            "FFmpegMerger",
            "FFmpegExtractAudio",
            "FFmpegVideoConvertor",
            "FFmpegVideoRemuxer",
            "FFmpegEmbedSubtitle",
            "FFmpegMetadata",
            "EmbedThumbnail",
            "MoveFiles",
            "FFmpegFixupM4a",
            "FFmpegFixupStretched",
            "ModifyChapters",
            "Merger",
            "NA",
            "",
        ] {
            assert!(starts_final_processing(&format!("started|{name}")), "{name}");
        }
        for name in PRE_DOWNLOAD_PROCESSORS {
            assert!(!starts_final_processing(&format!("started|{name}")), "{name}");
        }
        assert!(starts_final_processing("started"), "an unnamed signal is treated as final processing");
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

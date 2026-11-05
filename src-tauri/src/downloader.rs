use std::collections::HashSet;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::error::{notify, AppError, Result};
use crate::history;
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
        }
    }
}

impl DownloadOptions {
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
    /// Overall progress, 0.0 to 1.0.
    pub progress: f64,
    pub speed: Option<f64>,
    pub eta: Option<u64>,
    pub file_path: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    /// Every file name yt-dlp reported, so leftovers can be cleaned up after a pause,
    /// resume or restart. Saved as soon as a new one appears.
    #[serde(default)]
    pub partial_files: Vec<String>,
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
    /// Leftovers from earlier runs, handed over when a running job is removed.
    cleanup: Vec<String>,
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
    let _ = app.emit("job-update", job);
}

fn persist_queue(app: &AppHandle) {
    let state = app.state::<AppState>();
    let result = store::save_snapshot(&state.queue_path(), || {
        state
            .jobs
            .lock()
            .unwrap()
            .iter()
            .filter(|j| j.status != JobStatus::Completed && j.status != JobStatus::Cancelled)
            .cloned()
            .collect::<Vec<Job>>()
    });
    if let Err(e) = result {
        notify(app, format!("Claws could not save the queue: {e}"));
    }
}

fn update_job(app: &AppHandle, id: &str, change: impl FnOnce(&mut Job)) -> Option<Job> {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut jobs = state.jobs.lock().unwrap();
        let job = jobs.iter_mut().find(|j| j.id == id)?;
        change(job);
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
    for job in &mut saved {
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
    *app.state::<AppState>().jobs.lock().unwrap() = saved;
}

pub fn list(app: &AppHandle) -> Vec<Job> {
    app.state::<AppState>().jobs.lock().unwrap().clone()
}

pub fn enqueue(app: &AppHandle, items: Vec<EnqueueItem>, mut options: DownloadOptions) -> Result<Vec<Job>> {
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

    let mut created = Vec::with_capacity(items.len());
    for item in items {
        let url = validate_url(&item.url)?;
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
            speed: None,
            eta: None,
            file_path: None,
            error: None,
            created_at: chrono::Utc::now().timestamp_millis(),
            partial_files: Vec::new(),
        });
    }

    app.state::<AppState>()
        .jobs
        .lock()
        .unwrap()
        .extend(created.iter().cloned());
    for job in &created {
        emit_job(app, job);
    }
    persist_queue(app);
    pump(app);
    Ok(created)
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
            Vec::new()
        } else {
            let mut files = Vec::new();
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
                    files = std::mem::take(&mut j.partial_files);
                }
            });
            files
        }
    };
    // No process holds these files, so they can go right away.
    spawn_cleanup(leftovers, false);
    persist_queue(app);
}

fn job_partials(app: &AppHandle, id: &str) -> Vec<String> {
    app.state::<AppState>()
        .jobs
        .lock()
        .unwrap()
        .iter()
        .find(|j| j.id == id)
        .map(|j| j.partial_files.clone())
        .unwrap_or_default()
}

fn spawn_cleanup(files: Vec<String>, after_kill: bool) {
    if files.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move { remove_partials(files, after_kill).await });
}

pub fn remove(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    let Some(status) = job_status(app, id) else { return };
    let partials = job_partials(app, id);
    let handed_off = {
        let mut running = state.running.lock().unwrap();
        match running.get_mut(id) {
            Some(entry) => {
                // `finish` cleans up once the process is really gone.
                entry.request(Intent::Cancel);
                entry.cleanup.extend(partials.iter().cloned());
                if let Some(pid) = entry.pid {
                    kill_tree(pid, false);
                }
                true
            }
            None => false,
        }
    };
    // Finished downloads keep their file; anything unfinished loses its leftovers.
    if !handed_off && status != JobStatus::Completed {
        spawn_cleanup(partials, false);
    }
    state.jobs.lock().unwrap().retain(|j| j.id != id);
    let _ = app.emit("job-removed", id);
    persist_queue(app);
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
        let _ = app.emit("job-removed", id);
    }
    persist_queue(app);
}

/// Called once as the app exits. Raises `shutting_down` (nothing new can start), kills
/// every download and lookup tree, waits for the workers to settle, then fixes up what
/// they did not and saves the queue, so jobs come back paused.
pub fn shutdown(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.shutting_down.store(true, Ordering::SeqCst);

    let stopping: Vec<(String, Option<u32>, Intent, Vec<String>)> = {
        let mut running = state.running.lock().unwrap();
        running
            .iter_mut()
            .map(|(id, entry)| {
                entry.request(Intent::Pause);
                (id.clone(), entry.pid, entry.intent, entry.cleanup.clone())
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
                leftovers.append(&mut job.partial_files);
            } else if job.status.is_active() || job.status == JobStatus::Queued {
                job.status = JobStatus::Paused;
                job.speed = None;
                job.eta = None;
            }
        }
    }
    for (_, _, intent, cleanup) in stopping {
        if intent == Intent::Cancel {
            leftovers.extend(cleanup);
        }
    }
    if !leftovers.is_empty() {
        // Give Windows a moment to release the killed processes' handles.
        std::thread::sleep(Duration::from_millis(300));
        remove_partials_now(&leftovers);
    }
    persist_queue(app);
}

/// The source goes last: the probe's cached info, or the URL after `--` so it can
/// never be read as an option.
fn build_args(app: &AppHandle, job: &Job, settings: &Settings, info_json: Option<&Path>) -> Vec<String> {
    let mut args = tools::common_args(app, settings);
    let mut push = |items: &[&str]| args.extend(items.iter().map(|s| s.to_string()));

    push(&[
        "--newline",
        "--no-warnings",
        "--progress",
        "--no-simulate",
        "--windows-filenames",
        "--retries",
        "5",
        "--fragment-retries",
        "5",
        // 4 already saturates a fast line; 8 helps on high-latency and HLS sites.
        "--concurrent-fragments",
        "8",
    ]);
    let index = job.playlist_index.map(|i| i.to_string());
    match &index {
        // An entry without its own link: take just that item from the playlist.
        Some(index) => push(&["--yes-playlist", "--playlist-items", index]),
        None => push(&["--no-playlist"]),
    }
    push(&[
        "--progress-template",
        "download:CLAWS|%(progress.status)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s|%(progress.fragment_index)s|%(progress.fragment_count)s|%(progress.filename)s",
        "--progress-template",
        "postprocess:CLAWSPP|%(progress.status)s|%(progress.postprocessor)s",
        "--print",
        "before_dl:CLAWSFMT|%(requested_formats.0.format_id)s|%(requested_formats.1.format_id)s",
        "--print",
        "after_move:CLAWSDONE|%(filepath)s",
    ]);

    let options = &job.options;
    if options.mode == "audio" {
        push(&["-f", "ba/b", "-x", "--audio-format", &options.audio_format, "--audio-quality", "0"]);
        if matches!(options.audio_format.as_str(), "mp3" | "m4a") {
            push(&["--embed-thumbnail"]);
        }
    } else {
        let sort = match options.max_height {
            Some(h) => format!("res:{h},vcodec:h264,acodec:aac"),
            None => "res,vcodec:h264,acodec:aac".to_string(),
        };
        push(&["-f", "bv*+ba/b", "-S", &sort, "--merge-output-format", "mp4"]);
        if !options.subtitle_langs.is_empty() {
            let langs = options.subtitle_langs.join(",");
            push(&["--write-subs", "--sub-langs", &langs]);
            if options.auto_subs {
                push(&["--write-auto-subs"]);
            }
            if options.embed_subs {
                push(&["--embed-subs"]);
            } else {
                push(&["--convert-subs", "srt"]);
            }
        }
    }
    if settings.embed_metadata {
        push(&["--embed-metadata"]);
    }

    push(&["-P", &settings.output_dir, "-o", &settings.filename_template]);
    match info_json {
        Some(path) => {
            args.push("--load-info-json".into());
            args.push(path.to_string_lossy().into_owned());
        }
        None => {
            args.push("--".into());
            args.push(job.url.clone());
        }
    }
    args
}

struct Outcome {
    success: bool,
    error: Option<String>,
    final_path: Option<String>,
    partial_files: HashSet<String>,
    used_cache: bool,
}

impl Outcome {
    fn failed(message: impl Into<String>) -> Self {
        Self {
            success: false,
            error: Some(message.into()),
            final_path: None,
            partial_files: HashSet::new(),
            used_cache: false,
        }
    }
}

fn number(field: &str) -> Option<f64> {
    field.trim().parse().ok()
}

/// Subtitles, thumbnails and info files report progress too, but are not media streams.
fn is_side_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    matches!(
        ext,
        "vtt" | "srt" | "ass" | "ssa" | "ttml" | "srv1" | "srv2" | "srv3" | "json3" | "lrc" | "sbv"
            | "dfxp" | "jpg" | "jpeg" | "png" | "webp" | "json" | "description"
    )
}

/// Tracks progress across the separate video and audio streams of one download so the
/// bar moves forward once instead of restarting for every stream.
struct Progress {
    total_streams: usize,
    stream_index: usize,
    current_file: String,
    last_emit: Instant,
}

impl Progress {
    fn new(mode: &str) -> Self {
        Self {
            total_streams: if mode == "audio" { 1 } else { 2 },
            stream_index: 0,
            current_file: String::new(),
            last_emit: Instant::now() - Duration::from_secs(1),
        }
    }

    /// The stream count from yt-dlp's format selection ("NA" when only one format).
    fn set_formats(&mut self, line: &str) {
        let streams = line.split('|').filter(|f| !f.is_empty() && *f != "NA").count();
        self.total_streams = streams.max(1);
    }

    /// Overall fraction for one progress line, or `None` for a side file.
    fn overall(&mut self, fields: &[&str]) -> Option<f64> {
        let filename = fields[8].trim();
        if is_side_file(filename) {
            return None;
        }
        if filename != self.current_file {
            if !self.current_file.is_empty() {
                self.stream_index += 1;
            }
            self.current_file = filename.to_string();
        }
        let finished = fields[0] == "finished";
        let downloaded = number(fields[1]);
        let total = number(fields[2]).or_else(|| number(fields[3]));
        let fraction = if finished {
            1.0
        } else {
            match (downloaded, total) {
                (Some(done), Some(total)) if total > 0.0 => (done / total).clamp(0.0, 1.0),
                // Fragmented streams (HLS/DASH) report fragments instead of bytes.
                _ => match (number(fields[6]), number(fields[7])) {
                    (Some(index), Some(count)) if count > 0.0 => {
                        ((index - 1.0).max(0.0) / count).clamp(0.0, 1.0)
                    }
                    _ => 0.0,
                },
            }
        };
        let streams = self.total_streams.max(self.stream_index + 1);
        Some(((self.stream_index as f64 + fraction) / streams as f64).min(0.99))
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
    if let Err(e) = std::fs::create_dir_all(&settings.output_dir) {
        return Outcome::failed(format!("Cannot use the output folder: {e}"));
    }

    // Playlist items picked by index are never cached: the cache holds single videos.
    let info_json = (allow_cache && job.playlist_index.is_none())
        .then(|| probe::cached_info(app, &job.url))
        .flatten();
    let args = build_args(app, &job, &settings, info_json.as_deref());
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
        partial_files: HashSet::new(),
        used_cache: info_json.is_some(),
    };
    let mut progress = Progress::new(&job.options.mode);

    if let Some(stdout) = child.stdout.take() {
        let mut reader = BufReader::new(stdout);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    handle_line(app, id, line.trim_end(), &mut progress, &mut outcome);
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
            let message = error_text(&stderr_text);
            outcome.error = Some(if stderr_text.trim().is_empty() {
                format!("yt-dlp stopped with exit code {}.", s.code().unwrap_or(-1))
            } else {
                message
            });
        }
        Err(e) => outcome.error = Some(format!("yt-dlp stopped unexpectedly: {e}")),
    }
    outcome
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

fn handle_line(app: &AppHandle, id: &str, line: &str, progress: &mut Progress, outcome: &mut Outcome) {
    if let Some(rest) = line.strip_prefix(TAG_FORMATS) {
        progress.set_formats(rest);
    } else if let Some(rest) = line.strip_prefix(TAG_DONE) {
        if !rest.trim().is_empty() {
            outcome.final_path = Some(rest.trim().to_string());
        }
    } else if let Some(rest) = line.strip_prefix(TAG_POST) {
        if rest.starts_with("started") {
            mark_processing(app, id);
        }
    } else if let Some(rest) = line.strip_prefix(TAG_PROGRESS) {
        // status|downloaded|total|estimate|speed|eta|fragment|fragments|filename
        let fields: Vec<&str> = rest.splitn(9, '|').collect();
        if fields.len() < 9 {
            return;
        }
        let filename = fields[8].trim().to_string();
        if !filename.is_empty() && outcome.partial_files.insert(filename.clone()) {
            let added = {
                let state = app.state::<AppState>();
                let mut jobs = state.jobs.lock().unwrap();
                match jobs.iter_mut().find(|j| j.id == id) {
                    Some(job) if !job.partial_files.contains(&filename) => {
                        job.partial_files.push(filename);
                        true
                    }
                    _ => false,
                }
            };
            // Saved now, so a crash can still clean up later.
            if added {
                persist_queue(app);
            }
        }
        let Some(overall) = progress.overall(&fields) else { return };

        let finished = fields[0] == "finished";
        if finished || progress.last_emit.elapsed() >= Duration::from_millis(150) {
            progress.last_emit = Instant::now();
            let speed = number(fields[4]);
            let eta = number(fields[5]).map(|e| e as u64);
            update_job(app, id, |j| {
                if j.status.is_active() {
                    j.status = JobStatus::Downloading;
                }
                j.progress = overall;
                j.speed = speed;
                j.eta = eta;
            });
        }
    }
}

async fn run_job(app: AppHandle, id: String) {
    let state = app.state::<AppState>();
    // Waits while a tool is being replaced.
    let _engine = state.engine.read().await;

    let mut outcome = execute(&app, &id, true).await;

    // Cached media links can expire. If a cached run failed on its own (not paused or
    // cancelled), read the link again and retry once.
    let interrupted = state
        .running
        .lock()
        .unwrap()
        .get(&id)
        .is_some_and(|r| r.intent != Intent::None);
    let failed = !outcome.success && outcome.error.as_deref().is_some_and(|e| !e.is_empty());
    let closing = state.shutting_down.load(Ordering::SeqCst);
    if outcome.used_cache && failed && !interrupted && !closing {
        let retry = {
            let mut jobs = state.jobs.lock().unwrap();
            jobs.iter_mut()
                .find(|j| j.id == id && j.status.is_active())
                .map(|j| {
                    j.status = JobStatus::Downloading;
                    j.clone()
                })
        };
        if let Some(job) = retry {
            emit_job(&app, &job);
            probe::forget(&app, &job.url);
            let first_files = std::mem::take(&mut outcome.partial_files);
            outcome = execute(&app, &id, false).await;
            outcome.partial_files.extend(first_files);
        }
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
            j.speed = None;
            j.eta = None;
            j.error = None;
            j.file_path = outcome.final_path.clone();
            j.partial_files.clear();
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
    let mut cleaned = false;
    let mut completed = None;
    loop {
        // Either clean up after a cancel and look again (the intent may have changed), or
        // remove the entry and settle the job in one critical section.
        let to_clean = {
            let mut running = state.running.lock().unwrap();
            let entry = running.get(id);
            let intent = entry.map(|r| r.intent).unwrap_or(Intent::None);
            if intent == Intent::Cancel && !outcome.success && !cleaned {
                let mut files = entry.map(|r| r.cleanup.clone()).unwrap_or_default();
                files.extend(job_partials(app, id));
                files.extend(outcome.partial_files.iter().cloned());
                files.sort();
                files.dedup();
                Some(files)
            } else {
                running.remove(id);
                completed = settle(app, id, intent, &outcome);
                None
            }
        };
        match to_clean {
            Some(files) => {
                remove_partials(files, true).await;
                cleaned = true;
            }
            None => break,
        }
    }
    if let Some(job) = completed {
        history::record(app, &job);
    }
    persist_queue(app);
}

/// Removes `<name>.part`, `<name>.part-FragN` and `<name>.ytdl`; never the finished file.
pub(crate) fn remove_partials_now(files: &[String]) {
    for file in files {
        let path = Path::new(file);
        let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
            continue;
        };
        let part_prefix = format!("{name}.part");
        let ytdl_name = format!("{name}.ytdl");
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let entry_name = entry.file_name();
            let Some(entry_name) = entry_name.to_str() else {
                continue;
            };
            if entry_name.starts_with(&part_prefix) || entry_name == ytdl_name {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

async fn remove_partials(files: Vec<String>, after_kill: bool) {
    if after_kill {
        // Give Windows a moment to release the handles of the killed process.
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let _ = tokio::task::spawn_blocking(move || remove_partials_now(&files)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(status: &str, done: &str, total: &str, file: &str) -> Vec<String> {
        vec![status, done, total, "NA", "1000", "5", "NA", "NA", file]
            .into_iter()
            .map(String::from)
            .collect()
    }

    fn overall(progress: &mut Progress, fields: &[String]) -> Option<f64> {
        let refs: Vec<&str> = fields.iter().map(String::as_str).collect();
        progress.overall(&refs)
    }

    #[test]
    fn two_streams_progress_once() {
        let mut p = Progress::new("video");
        p.set_formats("137|140");
        assert_eq!(overall(&mut p, &line("downloading", "50", "100", "v.f137.mp4")), Some(0.25));
        assert_eq!(overall(&mut p, &line("finished", "100", "100", "v.f137.mp4")), Some(0.5));
        assert_eq!(overall(&mut p, &line("downloading", "50", "100", "v.f140.m4a")), Some(0.75));
    }

    #[test]
    fn single_format_counts_one_stream() {
        let mut p = Progress::new("video");
        p.set_formats("NA|NA");
        assert_eq!(overall(&mut p, &line("downloading", "50", "100", "v.mp4")), Some(0.5));
    }

    #[test]
    fn subtitles_do_not_advance_streams() {
        let mut p = Progress::new("video");
        p.set_formats("137|140");
        assert_eq!(overall(&mut p, &line("finished", "10", "10", "v.en.vtt")), None);
        assert_eq!(overall(&mut p, &line("downloading", "50", "100", "v.f137.mp4")), Some(0.25));
    }

    #[test]
    fn progress_never_reaches_one_before_done() {
        let mut p = Progress::new("audio");
        p.set_formats("NA|NA");
        assert_eq!(overall(&mut p, &line("finished", "100", "100", "a.webm")), Some(0.99));
    }

    #[test]
    fn cancel_is_not_downgraded_by_pause() {
        let mut entry = Running {
            pid: Some(1),
            intent: Intent::None,
            cleanup: Vec::new(),
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

use std::collections::HashSet;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::error::{AppError, Result};
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
    /// Every file name yt-dlp has reported for this job, so unfinished leftovers can
    /// be cleaned up even after the job was paused, resumed or restarted.
    #[serde(default)]
    pub partial_files: Vec<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EnqueueItem {
    pub url: String,
    pub title: String,
    pub thumbnail: Option<String>,
    pub uploader: Option<String>,
    pub duration: Option<f64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    None,
    Pause,
    Cancel,
}

pub struct Running {
    pid: u32,
    intent: Intent,
}

fn emit_job(app: &AppHandle, job: &Job) {
    let _ = app.emit("job-update", job);
}

fn persist_queue(app: &AppHandle) {
    let state = app.state::<AppState>();
    let pending: Vec<Job> = state
        .jobs
        .lock()
        .unwrap()
        .iter()
        .filter(|j| j.status != JobStatus::Completed && j.status != JobStatus::Cancelled)
        .cloned()
        .collect();
    let _ = store::save(&state.queue_path(), &pending);
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

pub fn restore_queue(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut saved: Vec<Job> = store::load(&state.queue_path());
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
    *state.jobs.lock().unwrap() = saved;
}

pub fn list(app: &AppHandle) -> Vec<Job> {
    app.state::<AppState>().jobs.lock().unwrap().clone()
}

pub fn enqueue(app: &AppHandle, items: Vec<EnqueueItem>, mut options: DownloadOptions) -> Result<Vec<Job>> {
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

pub fn pump(app: &AppHandle) {
    let state = app.state::<AppState>();
    let limit = state.settings.lock().unwrap().max_concurrent.max(1) as usize;
    loop {
        let claimed = {
            let mut jobs = state.jobs.lock().unwrap();
            let active = jobs
                .iter()
                .filter(|j| matches!(j.status, JobStatus::Downloading | JobStatus::Processing))
                .count();
            if active >= limit {
                None
            } else {
                jobs.iter_mut()
                    .find(|j| j.status == JobStatus::Queued)
                    .map(|job| {
                        job.status = JobStatus::Downloading;
                        job.error = None;
                        job.clone()
                    })
            }
        };
        let Some(job) = claimed else { break };
        emit_job(app, &job);
        let app = app.clone();
        tauri::async_runtime::spawn(async move { run_job(app, job.id).await });
    }
}

fn kill_tree(pid: u32) {
    // yt-dlp.exe is a launcher that starts the real interpreter (and ffmpeg) as
    // children, so the whole tree has to go.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    #[cfg(not(windows))]
    let _ = pid;
}

fn set_intent_and_kill(app: &AppHandle, id: &str, intent: Intent) -> bool {
    let state = app.state::<AppState>();
    let mut running = state.running.lock().unwrap();
    match running.get_mut(id) {
        Some(entry) => {
            entry.intent = intent;
            kill_tree(entry.pid);
            true
        }
        None => false,
    }
}

// The inner `if` kills a process, so it stays an explicit statement rather than a
// match guard with side effects.
#[allow(clippy::collapsible_match)]
pub fn pause(app: &AppHandle, id: &str) {
    let status = list(app).into_iter().find(|j| j.id == id).map(|j| j.status);
    match status {
        Some(JobStatus::Queued) => {
            update_job(app, id, |j| j.status = JobStatus::Paused);
        }
        Some(JobStatus::Downloading | JobStatus::Processing) => {
            if !set_intent_and_kill(app, id, Intent::Pause) {
                // Claimed but not spawned yet: run_job notices the status and stops.
                update_job(app, id, |j| j.status = JobStatus::Paused);
            }
        }
        _ => {}
    }
    persist_queue(app);
}

pub fn resume(app: &AppHandle, id: &str) {
    update_job(app, id, |j| {
        if matches!(
            j.status,
            JobStatus::Paused | JobStatus::Failed | JobStatus::Cancelled
        ) {
            j.status = JobStatus::Queued;
            j.error = None;
        }
    });
    persist_queue(app);
    pump(app);
}

pub fn cancel(app: &AppHandle, id: &str) {
    let status = list(app).into_iter().find(|j| j.id == id).map(|j| j.status);
    match status {
        Some(JobStatus::Downloading | JobStatus::Processing) => {
            if !set_intent_and_kill(app, id, Intent::Cancel) {
                update_job(app, id, |j| j.status = JobStatus::Cancelled);
            }
        }
        Some(JobStatus::Queued | JobStatus::Paused | JobStatus::Failed) => {
            spawn_cleanup(job_partials(app, id));
            update_job(app, id, |j| {
                j.status = JobStatus::Cancelled;
                j.speed = None;
                j.eta = None;
                j.partial_files.clear();
            });
        }
        _ => {}
    }
    persist_queue(app);
}

fn job_partials(app: &AppHandle, id: &str) -> Vec<String> {
    list(app)
        .into_iter()
        .find(|j| j.id == id)
        .map(|j| j.partial_files)
        .unwrap_or_default()
}

fn spawn_cleanup(files: Vec<String>) {
    if files.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move { remove_partials(&files).await });
}

pub fn remove(app: &AppHandle, id: &str) {
    let job = list(app).into_iter().find(|j| j.id == id);
    // Finished downloads keep their file; anything unfinished loses its leftovers.
    if job.is_some_and(|j| j.status != JobStatus::Completed) {
        spawn_cleanup(job_partials(app, id));
    }
    set_intent_and_kill(app, id, Intent::Cancel);
    let state = app.state::<AppState>();
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

/// The source goes last: the probe's cached info, or the URL after `--` so it can
/// never be read as an option.
fn build_args(app: &AppHandle, job: &Job, settings: &Settings, info_json: Option<&Path>) -> Vec<String> {
    let mut args = tools::common_args(app, settings);
    let mut push = |items: &[&str]| args.extend(items.iter().map(|s| s.to_string()));

    push(&[
        "--newline",
        "--no-warnings",
        "--no-playlist",
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

/// Tracks progress across the separate video and audio streams of one download so the
/// bar moves forward once instead of restarting for every stream.
struct Progress {
    total_streams: usize,
    stream_index: usize,
    current_file: String,
    last_emit: Instant,
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

    let info_json = allow_cache.then(|| probe::cached_info(app, &job.url)).flatten();
    let args = build_args(app, &job, &settings, info_json.as_deref());
    let mut child = match tools::command(&ytdlp)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(e) => return Outcome::failed(format!("Could not start yt-dlp: {e}")),
    };

    if let Some(pid) = child.id() {
        state.running.lock().unwrap().insert(
            id.to_string(),
            Running {
                pid,
                intent: Intent::None,
            },
        );
        // A pause or cancel may have landed before the entry existed.
        let status = list(app).into_iter().find(|j| j.id == id).map(|j| j.status);
        match status {
            Some(JobStatus::Paused) => {
                set_intent_and_kill(app, id, Intent::Pause);
            }
            Some(JobStatus::Cancelled) | None => {
                set_intent_and_kill(app, id, Intent::Cancel);
            }
            _ => {}
        }
    }

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
    let mut progress = Progress {
        total_streams: if job.options.mode == "audio" { 1 } else { 2 },
        stream_index: 0,
        current_file: String::new(),
        last_emit: Instant::now() - Duration::from_secs(1),
    };

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
        j.status = JobStatus::Processing;
        j.speed = None;
        j.eta = None;
    });
}

fn handle_line(app: &AppHandle, id: &str, line: &str, progress: &mut Progress, outcome: &mut Outcome) {
    if let Some(rest) = line.strip_prefix(TAG_FORMATS) {
        let streams = rest.split('|').filter(|f| !f.is_empty() && *f != "NA").count();
        progress.total_streams = streams.max(1);
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
        if filename != progress.current_file {
            if !progress.current_file.is_empty() {
                progress.stream_index += 1;
            }
            progress.current_file = filename.clone();
            outcome.partial_files.insert(filename.clone());
            let state = app.state::<AppState>();
            let mut jobs = state.jobs.lock().unwrap();
            if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
                if !job.partial_files.contains(&filename) {
                    job.partial_files.push(filename);
                }
            }
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
        let overall = ((progress.stream_index as f64 + fraction) / progress.total_streams as f64)
            .min(0.99);

        if finished || progress.last_emit.elapsed() >= Duration::from_millis(150) {
            progress.last_emit = Instant::now();
            let speed = number(fields[4]);
            let eta = number(fields[5]).map(|e| e as u64);
            update_job(app, id, |j| {
                j.status = JobStatus::Downloading;
                j.progress = overall;
                j.speed = speed;
                j.eta = eta;
            });
        }
    }
}

async fn run_job(app: AppHandle, id: String) {
    let mut outcome = execute(&app, &id, true).await;

    // Cached media links can expire. If a cached run failed on its own (not paused or
    // cancelled), read the link again and retry once.
    let interrupted = app
        .state::<AppState>()
        .running
        .lock()
        .unwrap()
        .get(&id)
        .is_some_and(|r| r.intent != Intent::None);
    let failed = !outcome.success && outcome.error.as_deref().is_some_and(|e| !e.is_empty());
    if outcome.used_cache && failed && !interrupted {
        if let Some(job) = update_job(&app, &id, |j| j.status = JobStatus::Downloading) {
            probe::forget(&app, &job.url);
            outcome = execute(&app, &id, false).await;
        }
    }

    finish(&app, &id, outcome).await;
    pump(&app);
}

async fn finish(app: &AppHandle, id: &str, outcome: Outcome) {
    let state = app.state::<AppState>();
    let intent = state
        .running
        .lock()
        .unwrap()
        .remove(id)
        .map(|r| r.intent)
        .unwrap_or(Intent::None);

    let still_listed = list(app).iter().any(|j| j.id == id);
    if intent == Intent::Cancel && still_listed {
        let mut files = job_partials(app, id);
        files.extend(outcome.partial_files.iter().cloned());
        files.sort();
        files.dedup();
        remove_partials(&files).await;
    }
    if !still_listed {
        return;
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
        Intent::None if outcome.success => {
            if let Some(job) = update_job(app, id, |j| {
                j.status = JobStatus::Completed;
                j.progress = 1.0;
                j.speed = None;
                j.eta = None;
                j.file_path = outcome.final_path.clone();
            }) {
                history::record(app, &job);
            }
        }
        Intent::None => {
            // An empty message means the job was paused or cancelled before it started.
            let message = outcome.error.unwrap_or_default();
            update_job(app, id, |j| {
                if j.status == JobStatus::Downloading || j.status == JobStatus::Processing {
                    j.status = JobStatus::Failed;
                    j.error = Some(message);
                }
                j.speed = None;
                j.eta = None;
            });
        }
    }
    persist_queue(app);
}

/// Removes `<name>.part`, `<name>.part-FragN` and `<name>.ytdl`; never the finished file.
async fn remove_partials(files: &[String]) {
    // Give Windows a moment to release the handles of the killed process.
    tokio::time::sleep(Duration::from_millis(500)).await;
    for file in files {
        let path = Path::new(file);
        let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
            continue;
        };
        let part_prefix = format!("{name}.part");
        let ytdl_name = format!("{name}.ytdl");
        let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let entry_name = entry.file_name();
            let Some(entry_name) = entry_name.to_str() else {
                continue;
            };
            if entry_name.starts_with(&part_prefix) || entry_name == ytdl_name {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
    }
}

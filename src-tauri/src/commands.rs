use serde::Serialize;
use tauri::{Manager, State};

use crate::{AppHandle, WebviewWindow};
use tauri_plugin_opener::OpenerExt;

use crate::desktop;
use crate::downloader::{self, DownloadOptions, EnqueueItem, Job, JobsAdded};
use crate::links::{self, CachedLink};
use crate::error::{AppError, Result};
use crate::estimate::{self, DownloadEstimate};
use crate::file_drag;
use crate::history::{self, HistoryItem, KeptReveal, RepairReport};
use crate::history_mutation;
use crate::probe::{self, ProbeResult};
use crate::settings::Settings;
use crate::state::AppState;
use crate::store;
use crate::tools::{self, ToolsStatus};
use crate::window;

/// Tool versions are fetched separately: checking them spawns processes, and the first
/// frame must not wait for that.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub settings: Settings,
    pub jobs: Vec<Job>,
    pub history: Vec<HistoryItem>,
    /// Startup problems to show once (damaged data files, no desktop blur).
    pub notices: Vec<String>,
}

#[tauri::command]
pub fn bootstrap(app: AppHandle, state: State<'_, AppState>) -> Bootstrap {
    Bootstrap {
        settings: state.settings.lock().unwrap().clone(),
        jobs: downloader::list(&app),
        history: state.history.lock().unwrap().clone(),
        notices: std::mem::take(&mut *state.notices.lock().unwrap()),
    }
}

/// Ctrl+Q: really quit, through the shutdown path.
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub fn app_ready(app: AppHandle, window: WebviewWindow, state: State<'_, AppState>) {
    // The window is listening for notices now; show what startup recovery found meanwhile.
    crate::history_mutation::release_notices(&app);
    // Opened at login: stay in the notification area until the user asks for Claws.
    if !state.start_hidden.load(std::sync::atomic::Ordering::SeqCst) {
        let _ = window.show();
    }
}

#[tauri::command]
pub async fn probe_url(app: AppHandle, url: String) -> Result<ProbeResult> {
    probe::probe(&app, &url).await
}

/// A size and free-space preview for one item with these options. Display only: the
/// download still checks the actual free space when it starts.
#[tauri::command]
pub async fn estimate_download(
    app: AppHandle,
    url: String,
    playlist_index: Option<u32>,
    options: DownloadOptions,
) -> Result<DownloadEstimate> {
    estimate::estimate(&app, &url, playlist_index, options).await
}

#[tauri::command]
pub fn cached_link(app: AppHandle, url: String) -> Option<CachedLink> {
    links::lookup(&app, &url)
}

/// `source` is the link the user looked up; its details go into the link cache.
#[tauri::command]
pub fn enqueue(
    app: AppHandle,
    items: Vec<EnqueueItem>,
    options: DownloadOptions,
    source: Option<String>,
) -> Result<JobsAdded> {
    let jobs = downloader::enqueue(&app, items, options)?;
    if let Some(source) = source {
        links::remember_download(&app, &source);
    }
    Ok(jobs)
}

#[tauri::command]
pub fn pause_job(app: AppHandle, id: String) {
    downloader::pause(&app, &id);
}

#[tauri::command]
pub fn resume_job(app: AppHandle, id: String) {
    downloader::resume(&app, &id);
}

#[tauri::command]
pub fn cancel_job(app: AppHandle, id: String) {
    downloader::cancel(&app, &id);
}

#[tauri::command]
pub fn remove_job(app: AppHandle, id: String) -> Result<()> {
    downloader::remove(&app, &id)
}

#[tauri::command]
pub fn clear_finished(app: AppHandle) {
    downloader::clear_finished(&app);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSettings {
    pub settings: Settings,
    /// Saved fine, but something did not take effect (such as the desktop blur).
    pub warning: Option<String>,
}

#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    mut settings: Settings,
) -> Result<SavedSettings> {
    // The interface always edits current-format settings.
    settings.schema_version = crate::settings::SETTINGS_SCHEMA;
    settings.validate()?;
    // Disk and memory under one lock, so two saves cannot leave them disagreeing.
    store::save_then(&state.settings_path(), &settings, || {
        *state.settings.lock().unwrap() = settings.clone();
    })?;

    window::apply_theme(&window, &settings.theme);
    let warnings: Vec<String> = [
        window::apply_backdrop(&window, &settings.backdrop).err(),
        desktop::apply(&app, &settings),
    ]
    .into_iter()
    .flatten()
    .collect();
    let warning = (!warnings.is_empty()).then(|| warnings.join(" "));
    // A higher concurrency limit can start waiting jobs right away.
    downloader::pump(&app);
    Ok(SavedSettings { settings, warning })
}

#[tauri::command]
pub fn list_history(state: State<'_, AppState>) -> Vec<HistoryItem> {
    state.history.lock().unwrap().clone()
}

fn history_item(state: &AppState, id: &str) -> Result<HistoryItem> {
    state
        .history
        .lock()
        .unwrap()
        .iter()
        .find(|h| h.id == id)
        .cloned()
        .ok_or_else(|| AppError::msg("That download is no longer in the library."))
}

/// Only paths the app itself recorded can be opened, never one supplied by the UI.
fn recorded_path(item: &HistoryItem) -> Result<String> {
    let path = item
        .file_path
        .clone()
        .ok_or_else(|| AppError::msg("The file location was not recorded."))?;
    if !std::path::Path::new(&path).exists() {
        return Err(AppError::msg("The file has been moved or deleted."));
    }
    Ok(path)
}

#[tauri::command]
pub fn open_download(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<()> {
    let path = recorded_path(&history_item(&state, &id)?)?;
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| AppError::msg(e.to_string()))
}

#[tauri::command]
pub fn reveal_download(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<()> {
    let path = recorded_path(&history_item(&state, &id)?)?;
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| AppError::msg(e.to_string()))
}

/// Shows the side files a download kept in its work folder because their names were taken.
/// The page sends the id; the folder comes from the Library entry's saved record, resolved
/// through the verified work folder. The answer tells a shown file from a folder that is
/// offline, gone or not provably this download's.
#[tauri::command]
pub async fn show_kept_files(app: AppHandle, id: String) -> Result<KeptReveal> {
    let item = history_item(&app.state::<AppState>(), &id)?;
    let (reveal, file) = tauri::async_runtime::spawn_blocking(move || history::reveal_plan(&item)).await??;
    if let Some(file) = file {
        app.opener().reveal_item_in_dir(file).map_err(|e| AppError::msg(e.to_string()))?;
    }
    Ok(reveal)
}

/// Looks again for kept side files of older downloads, in the folders that could not be read
/// before as well as the rest.
#[tauri::command]
pub async fn repair_kept_files(app: AppHandle) -> Result<RepairReport> {
    Ok(tauri::async_runtime::spawn_blocking(move || history::repair_kept_files(&app)).await?)
}

/// What the last look for kept side files found (see `repair_kept_files`).
#[tauri::command]
pub fn kept_files_report(app: AppHandle) -> RepairReport {
    history::last_repair(&app)
}

/// Starts Windows' copy-only file drag of a Library file (see file_drag). The page sends
/// the id; the path comes from the Library entry only.
#[tauri::command]
pub async fn drag_download(app: AppHandle, window: WebviewWindow, id: String) -> Result<file_drag::DragOutcome> {
    // The lock covers reading the entry only: no file-system access and no await under it.
    let recorded = {
        let state = app.state::<AppState>();
        let history = state.history.lock().unwrap();
        file_drag::recorded_location(&history, &id)?
    };
    // Off the async workers: a network location can take a while to answer.
    let path = tauri::async_runtime::spawn_blocking(move || file_drag::draggable_file(&recorded)).await??;
    file_drag::start(&app, window, path).await
}

#[tauri::command]
pub fn open_output_dir(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    let dir = state.settings.lock().unwrap().output_dir.clone();
    std::fs::create_dir_all(&dir)?;
    app.opener()
        .open_path(dir, None::<&str>)
        .map_err(|e| AppError::msg(e.to_string()))
}

/// Off the main thread: deleting a file may hash it, and the Library is saved to disk.
#[tauri::command]
pub async fn delete_history(app: AppHandle, id: String, delete_file: bool) -> Result<()> {
    tauri::async_runtime::spawn_blocking(move || history_mutation::delete_entry(&app, &id, delete_file)).await?
}

#[tauri::command]
pub async fn clear_history(app: AppHandle) -> Result<()> {
    tauri::async_runtime::spawn_blocking(move || history_mutation::clear(&app)).await?
}

#[tauri::command]
pub async fn tools_status(app: AppHandle) -> Result<ToolsStatus> {
    Ok(tools::status(&app).await)
}

#[tauri::command]
pub async fn install_ytdlp(app: AppHandle) -> Result<ToolsStatus> {
    tools::install_ytdlp(&app).await?;
    Ok(tools::status(&app).await)
}

#[tauri::command]
pub async fn update_ytdlp(app: AppHandle) -> Result<ToolsStatus> {
    let latest = tools::latest_ytdlp_version(&app).await?;
    let mut status = tools::status(&app).await;
    let current = status.ytdlp.as_ref().map(|t| t.version.clone());
    // Reinstall when outdated, when yt-dlp comes from PATH, or when the managed copy is
    // the slower single-file build older versions of Claws installed.
    let fast_build = status.ytdlp.as_ref().is_some_and(|t| t.managed && t.fast_start);
    if current.as_deref() != Some(latest.as_str()) || !fast_build {
        tools::install_ytdlp(&app).await?;
        status = tools::status(&app).await;
    }
    status.latest_ytdlp = Some(latest);
    Ok(status)
}

#[tauri::command]
pub async fn install_ffmpeg(app: AppHandle) -> Result<ToolsStatus> {
    tools::install_ffmpeg(&app).await?;
    Ok(tools::status(&app).await)
}

/// The URL is fixed here rather than taken from the UI, so this can only open the Salted
/// Flutters Polaris project page.
#[tauri::command]
pub fn open_font_page(app: AppHandle) -> Result<()> {
    app.opener()
        .open_url("https://github.com/ppolkadot/Salted-Flutters-Polaris", None::<&str>)
        .map_err(|e| AppError::msg(e.to_string()))
}

#[tauri::command]
pub fn default_output_dir(app: AppHandle) -> Result<String> {
    let base = app
        .path()
        .download_dir()
        .map_err(|e| AppError::msg(e.to_string()))?;
    Ok(base.join("Claws").to_string_lossy().into_owned())
}

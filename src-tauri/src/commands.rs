use serde::Serialize;
use tauri::{Manager, State};

use crate::{AppHandle, WebviewWindow};
use tauri_plugin_opener::OpenerExt;

use crate::desktop;
use crate::downloader::{self, DownloadOptions, EnqueueItem, Job, JobsAdded};
use crate::links::{self, CachedLink};
use crate::error::{AppError, Result};
use crate::history::{self, HistoryItem};
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
pub fn app_ready(window: WebviewWindow, state: State<'_, AppState>) {
    // Opened at login: stay in the notification area until the user asks for Claws.
    if !state.start_hidden.load(std::sync::atomic::Ordering::SeqCst) {
        let _ = window.show();
    }
}

#[tauri::command]
pub async fn probe_url(app: AppHandle, url: String) -> Result<ProbeResult> {
    probe::probe(&app, &url).await
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

#[tauri::command]
pub fn open_output_dir(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    let dir = state.settings.lock().unwrap().output_dir.clone();
    std::fs::create_dir_all(&dir)?;
    app.opener()
        .open_path(dir, None::<&str>)
        .map_err(|e| AppError::msg(e.to_string()))
}

#[tauri::command]
pub fn delete_history(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    delete_file: bool,
) -> Result<()> {
    let Ok(item) = history_item(&state, &id) else {
        return Ok(()); // already removed
    };
    // The file goes first: if it cannot be deleted, the entry stays.
    if delete_file {
        if let Some(path) = &item.file_path {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                // A file that is already gone is not an error.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(AppError::msg(format!(
                        "The file could not be deleted ({e}), so it is still in your library. Close any app using it and try again."
                    )))
                }
            }
        }
    }
    state.history.lock().unwrap().retain(|h| h.id != id);
    history::remove_thumbnail(&item);
    history::save(&app)
}

#[tauri::command]
pub fn clear_history(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    let removed: Vec<HistoryItem> = std::mem::take(&mut *state.history.lock().unwrap());
    removed.iter().for_each(history::remove_thumbnail);
    history::save(&app)
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

/// The URL is fixed here rather than taken from the UI, so this can only open Apple's
/// fonts page.
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

use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_opener::OpenerExt;

use crate::downloader::{self, DownloadOptions, EnqueueItem, Job};
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
}

#[tauri::command]
pub fn bootstrap(app: AppHandle, state: State<'_, AppState>) -> Bootstrap {
    Bootstrap {
        settings: state.settings.lock().unwrap().clone(),
        jobs: downloader::list(&app),
        history: state.history.lock().unwrap().clone(),
    }
}

#[tauri::command]
pub fn app_ready(window: WebviewWindow) {
    let _ = window.show();
}

#[tauri::command]
pub async fn probe_url(app: AppHandle, url: String) -> Result<ProbeResult> {
    probe::probe(&app, &url).await
}

#[tauri::command]
pub fn enqueue(
    app: AppHandle,
    items: Vec<EnqueueItem>,
    options: DownloadOptions,
) -> Result<Vec<Job>> {
    downloader::enqueue(&app, items, options)
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
pub fn remove_job(app: AppHandle, id: String) {
    downloader::remove(&app, &id);
}

#[tauri::command]
pub fn clear_finished(app: AppHandle) {
    downloader::clear_finished(&app);
}

#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    mut settings: Settings,
) -> Result<Settings> {
    settings.validate()?;
    store::save(&state.settings_path(), &settings)?;
    *state.settings.lock().unwrap() = settings.clone();

    window::apply_theme(&window, &settings.theme);
    window::apply_backdrop(&window, &settings.backdrop).map_err(AppError::msg)?;
    // A higher concurrency limit can start waiting jobs right away.
    downloader::pump(&app);
    Ok(settings)
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
    let removed = {
        let mut history = state.history.lock().unwrap();
        let index = history.iter().position(|h| h.id == id);
        index.map(|i| history.remove(i))
    };
    if let Some(item) = removed {
        history::remove_thumbnail(&item);
        if delete_file {
            if let Some(path) = &item.file_path {
                // A file that is already gone is not an error.
                let _ = std::fs::remove_file(path);
            }
        }
    }
    history::save(&app);
    Ok(())
}

#[tauri::command]
pub fn clear_history(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    let removed: Vec<HistoryItem> = std::mem::take(&mut *state.history.lock().unwrap());
    removed.iter().for_each(history::remove_thumbnail);
    history::save(&app);
    Ok(())
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

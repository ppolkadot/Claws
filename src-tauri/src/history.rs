use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::downloader::Job;
use crate::state::AppState;
use crate::store;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub id: String,
    pub title: String,
    pub url: String,
    pub uploader: Option<String>,
    pub duration: Option<f64>,
    /// Remote thumbnail, used until (or if) a local copy exists.
    pub thumbnail: Option<String>,
    pub thumbnail_local: Option<String>,
    pub file_path: Option<String>,
    pub file_size: Option<u64>,
    /// "video" or "audio".
    pub mode: String,
    pub quality: String,
    pub finished_at: i64,
}

pub fn save(app: &AppHandle) {
    let state = app.state::<AppState>();
    let history = state.history.lock().unwrap().clone();
    let _ = store::save(&state.history_path(), &history);
    let _ = app.emit("history-changed", ());
}

pub fn record(app: &AppHandle, job: &Job) {
    let state = app.state::<AppState>();
    let file_size = job
        .file_path
        .as_deref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len());
    let quality = match job.options.mode.as_str() {
        "audio" => job.options.audio_format.to_uppercase(),
        _ => job
            .options
            .max_height
            .map(|h| format!("{h}p"))
            .unwrap_or_else(|| "Best".into()),
    };
    let item = HistoryItem {
        id: job.id.clone(),
        title: job.title.clone(),
        url: job.url.clone(),
        uploader: job.uploader.clone(),
        duration: job.duration,
        thumbnail: job.thumbnail.clone(),
        thumbnail_local: None,
        file_path: job.file_path.clone(),
        file_size,
        mode: job.options.mode.clone(),
        quality,
        finished_at: chrono::Utc::now().timestamp_millis(),
    };
    {
        let mut history = state.history.lock().unwrap();
        history.retain(|h| h.id != item.id);
        history.insert(0, item);
    }
    save(app);

    if let Some(url) = job.thumbnail.clone() {
        let app = app.clone();
        let id = job.id.clone();
        tauri::async_runtime::spawn(async move {
            cache_thumbnail(&app, &id, &url).await;
        });
    }
}

/// Best effort: keeps the library's artwork after the site's link expires.
async fn cache_thumbnail(app: &AppHandle, id: &str, url: &str) {
    const MAX_BYTES: usize = 5 * 1024 * 1024;
    let state = app.state::<AppState>();
    if !url.starts_with("https://") {
        return;
    }
    let result = async {
        let response = state
            .http
            .get(url)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?;
        let ext = match response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
        {
            Some(t) if t.contains("png") => "png",
            Some(t) if t.contains("webp") => "webp",
            _ => "jpg",
        };
        let bytes = response.bytes().await.ok()?;
        if bytes.len() > MAX_BYTES {
            return None;
        }
        let dir = state.thumbs_dir();
        tokio::fs::create_dir_all(&dir).await.ok()?;
        // The file name is our own uuid, never anything from the remote site.
        let path: PathBuf = dir.join(format!("{id}.{ext}"));
        tokio::fs::write(&path, &bytes).await.ok()?;
        Some(path)
    }
    .await;

    if let Some(path) = result {
        {
            let mut history = state.history.lock().unwrap();
            if let Some(item) = history.iter_mut().find(|h| h.id == id) {
                item.thumbnail_local = Some(path.to_string_lossy().into_owned());
            }
        }
        save(app);
    }
}

pub fn remove_thumbnail(item: &HistoryItem) {
    if let Some(path) = &item.thumbnail_local {
        let _ = std::fs::remove_file(Path::new(path));
    }
}

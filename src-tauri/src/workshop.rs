//! Wallpaper Engine Workshop items. Details come from Valve's public API; the project comes
//! from the user's own Steam library (an item they subscribed to in Steam). A project is
//! saved as a ZIP, or — for a video project — converted with FFmpeg to MP4 or GIF. Claws
//! never writes into Steam's folders.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use tauri_plugin_opener::OpenerExt;

use crate::{
    disk_budget,
    download_files::{is_link, move_no_replace},
    downloader::kill_tree,
    encoder::{self, Encoder},
    error::{AppError, Result},
    export_progress::{self, Stage, StageTime, Tracker, Unit},
    state::AppState,
    scene_pkg,
    steam::{library_folders, library_project, locate_item, steam_root, ItemState},
    workshop_media::{self, MediaError, MediaFormat, ProjectKind},
    workshop_record,
    AppHandle,
};

pub use crate::steam::APP_ID;
/// Workshop collections are published under Steam's own collection app.
const COLLECTION_APP: u64 = 766;
const STEAM_DETAILS: &str = "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";
const STEAM_COLLECTION: &str = "https://api.steampowered.com/ISteamRemoteStorage/GetCollectionDetails/v1/";
const MAX_DETAILS: usize = 512 * 1024;
const MAX_EXPORT_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 50_000;
const PARTIAL_PREFIX: &str = ".claws-wallpaper-";
const PARTIAL_SUFFIX: &str = ".partial";

const STEAM_UNREACHABLE: &str = "Couldn't reach Steam to check this Workshop item. Check your connection, then try again.";
const RUNNING: &str = "A wallpaper download is already running.";

/// The HTTP client for Valve's public Workshop API: no redirects, short timeouts.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("Claws/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Steam HTTP client configuration is valid")
}

/// The one export that may run at a time. Leaf lock.
#[derive(Default)]
pub struct Run {
    pub active: bool,
    pub cancelled: bool,
    /// The process id while FFmpeg runs.
    pub pid: Option<u32>,
    pub partial: Option<PathBuf>,
    /// The Wallpaper Engine window a recording has open (its program and title), so a shutdown
    /// can close it.
    pub window: Option<(PathBuf, String)>,
}

// ---------------------------------------------------------------------------------------
// Links and details

fn item_id(url: &str) -> Result<u64> {
    let invalid = || AppError::msg("Paste a Steam Workshop item link with a valid item ID.");
    let url = url::Url::parse(url.trim()).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "https" | "http")
        || !matches!(url.host_str(), Some("steamcommunity.com" | "www.steamcommunity.com"))
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(
            url.path(),
            "/sharedfiles/filedetails/" | "/sharedfiles/filedetails" | "/workshop/filedetails/" | "/workshop/filedetails"
        )
    {
        return Err(invalid());
    }
    let mut ids = url.query_pairs().filter(|(key, _)| key == "id");
    let (_, id) = ids.next().ok_or_else(invalid)?;
    if ids.next().is_some() || !id.bytes().all(|c| c.is_ascii_digit()) {
        return Err(invalid());
    }
    id.parse().ok().filter(|id| *id != 0).ok_or_else(invalid)
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkshopItem {
    pub id: String,
    pub title: String,
    pub preview_url: Option<String>,
    pub file_size: Option<u64>,
    pub time_updated: Option<i64>,
    /// "Scene", "Video", "Web", "Application" or "Preset", from the item's tags.
    pub wallpaper_type: Option<String>,
    /// Other Workshop items this one needs (asset packs and the like).
    pub required_items: Vec<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum WorkshopLookup {
    Wallpaper { item: WorkshopItem },
    Collection { title: String, items: usize },
    OtherApp { app_id: u64 },
    NotFound,
}

/// Steam writes ids and sizes as numbers in some fields and as strings in others.
fn number(value: &serde_json::Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

fn preview_url(value: &serde_json::Value) -> Option<String> {
    let url = url::Url::parse(value.as_str()?).ok()?;
    let host = url.host_str()?;
    let valve = ["steamusercontent.com", "steamstatic.com", "akamaihd.net", "steampowered.com"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")));
    (url.scheme() == "https" && valve).then(|| url.to_string())
}

fn parse_details(body: &[u8], id: u64) -> Result<WorkshopLookup> {
    let value: serde_json::Value = serde_json::from_slice(body)?;
    let Some(item) = value["response"]["publishedfiledetails"]
        .as_array()
        .and_then(|items| items.iter().find(|item| number(&item["publishedfileid"]) == Some(id)))
    else {
        return Ok(WorkshopLookup::NotFound);
    };
    if item["result"].as_u64() != Some(1) {
        return Ok(WorkshopLookup::NotFound);
    }
    let consumer = number(&item["consumer_app_id"]).unwrap_or(0);
    if consumer != APP_ID {
        return Ok(WorkshopLookup::OtherApp { app_id: consumer });
    }
    let title = item["title"].as_str().unwrap_or("").trim().to_string();
    if number(&item["creator_app_id"]) == Some(COLLECTION_APP) {
        return Ok(WorkshopLookup::Collection { title, items: 0 });
    }
    let wallpaper_type = item["tags"].as_array().and_then(|tags| {
        tags.iter()
            .filter_map(|tag| tag["tag"].as_str())
            .find(|tag| ["Scene", "Video", "Web", "Application", "Preset"].contains(tag))
            .map(str::to_string)
    });
    Ok(WorkshopLookup::Wallpaper {
        item: WorkshopItem {
            id: id.to_string(),
            title,
            preview_url: preview_url(&item["preview_url"]),
            file_size: number(&item["file_size"]).filter(|size| *size > 0),
            time_updated: item["time_updated"].as_i64(),
            wallpaper_type,
            required_items: Vec::new(),
        },
    })
}

/// The children Steam lists for an item: a collection's items, or a wallpaper's required items.
fn parse_children(body: &[u8], id: u64) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else { return Vec::new() };
    value["response"]["collectiondetails"]
        .as_array()
        .and_then(|items| items.iter().find(|item| number(&item["publishedfileid"]) == Some(id)))
        .filter(|item| item["result"].as_u64() == Some(1))
        .and_then(|item| item["children"].as_array())
        .map(|children| {
            children
                .iter()
                .filter_map(|child| number(&child["publishedfileid"]))
                .filter(|child| *child != id)
                .map(|child| child.to_string())
                .collect()
        })
        .unwrap_or_default()
}

async fn post(client: &reqwest::Client, url: &str, body: String) -> Result<Vec<u8>> {
    let mut response = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await?
        .error_for_status()?;
    let mut out = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > MAX_DETAILS - out.len() {
            return Err(AppError::msg("Steam returned too much item information."));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

async fn lookup(client: &reqwest::Client, id: u64) -> Result<WorkshopLookup> {
    let details = post(client, STEAM_DETAILS, format!("itemcount=1&publishedfileids%5B0%5D={id}")).await?;
    let mut found = parse_details(&details, id)?;
    if matches!(found, WorkshopLookup::Wallpaper { .. } | WorkshopLookup::Collection { .. }) {
        // Optional: without it a wallpaper still downloads, only without the warning.
        let children = post(client, STEAM_COLLECTION, format!("collectioncount=1&publishedfileids%5B0%5D={id}"))
            .await
            .map(|body| parse_children(&body, id))
            .unwrap_or_default();
        match &mut found {
            WorkshopLookup::Wallpaper { item } => item.required_items = children,
            WorkshopLookup::Collection { items, .. } => *items = children.len(),
            _ => {}
        }
    }
    Ok(found)
}

fn not_a_wallpaper(found: &WorkshopLookup) -> AppError {
    AppError::msg(match found {
        WorkshopLookup::Collection { .. } => "This link is a Workshop collection. Open each wallpaper in it on its own.",
        WorkshopLookup::OtherApp { .. } => "Claws downloads Workshop items for Wallpaper Engine only.",
        _ => "Steam couldn't find this Workshop item. It may be private or removed.",
    })
}

#[tauri::command]
pub async fn workshop_item_details(state: State<'_, AppState>, url: String) -> Result<WorkshopLookup> {
    let id = item_id(&url)?;
    lookup(&state.steam_http, id).await.map_err(|_| AppError::msg(STEAM_UNREACHABLE))
}

#[tauri::command]
pub fn open_workshop_item(app: AppHandle, url: String) -> Result<()> {
    let id = item_id(&url)?;
    if app.opener().open_url(format!("steam://url/CommunityFilePage/{id}"), None::<&str>).is_ok() {
        return Ok(());
    }
    // No Steam client: the item's page in the browser.
    app.opener()
        .open_url(format!("https://steamcommunity.com/sharedfiles/filedetails/?id={id}"), None::<&str>)
        .map_err(|_| AppError::msg("Couldn't open the Workshop page."))
}


#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStatus {
    /// Steam is installed for this Windows user.
    pub steam: bool,
    /// Wallpaper Engine is installed in one of Steam's libraries.
    pub wallpaper_engine: bool,
    /// The item's project is in a Steam library.
    pub found: bool,
    /// True only when the installed project declares a supported video file.
    pub video: bool,
    /// What kind of project it is: video, scene, web, application, preset or unknown.
    pub kind: ProjectKind,
    /// What Steam says about it: installed, downloading (with the bytes on disk) or with an
    /// update pending. Flattened: `state`, and `bytes` while downloading.
    #[serde(flatten)]
    pub item: ItemState,
}

/// `time_updated` is the update time Valve's public details give for the item; a library copy
/// older than that has an update waiting in Steam.
#[tauri::command]
pub async fn find_wallpaper_in_library(url: String, time_updated: Option<i64>) -> Result<LibraryStatus> {
    let id = item_id(&url)?;
    tauri::async_runtime::spawn_blocking(move || {
        let Some(root) = steam_root() else {
            return LibraryStatus { steam: false, wallpaper_engine: false, found: false, video: false, kind: ProjectKind::Unknown, item: ItemState::NotInstalled };
        };
        let libraries = library_folders(&root);
        let located = locate_item(&libraries, id, time_updated);
        LibraryStatus {
            steam: true,
            wallpaper_engine: crate::steam::wallpaper_engine_exe(&libraries, &root).is_some(),
            found: located.project.is_some(),
            video: located.project.as_deref().is_some_and(|path| workshop_media::video_file(path).is_ok()),
            kind: located.project.as_deref().map_or(ProjectKind::Unknown, workshop_media::project_kind),
            item: located.state,
        }
    })
    .await
    .map_err(|_| AppError::msg("Couldn't check the Steam library."))
}

// ---------------------------------------------------------------------------------------
// Download

/// What the user asked for. ZIP works for every item and stays the default; MP4 and GIF are
/// offered only for video wallpapers.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    Zip,
    /// A video wallpaper's MP4 (copied or converted), or a scene or web wallpaper recorded.
    Mp4,
    Gif,
    /// One picture of a scene or web wallpaper.
    Png,
}

impl ExportFormat {
    fn media(self) -> Option<MediaFormat> {
        match self {
            Self::Zip => None,
            Self::Mp4 => Some(MediaFormat::Mp4),
            Self::Gif => Some(MediaFormat::Gif),
            Self::Png => None,
        }
    }
}

/// What a video export may be asked for beyond its format. Every field is optional and bounded
/// where it is used.
#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportOptions {
    /// GIF: how many seconds from the start (default 30, at most 60). Recording: how long (default
    /// 30, at most 600).
    pub length_seconds: Option<u32>,
    /// Recording: the height in pixels (default 1080, or the largest the wallpaper has).
    pub height: Option<u32>,
    /// Recording: 30 or 60.
    pub fps: Option<u32>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum WallpaperOutcome {
    /// `elapsed_ms` is the whole export, `stages` the time in each part of it.
    Saved { filename: String, path: String, elapsed_ms: u64, stages: Vec<StageTime>, note: Option<String> },
    Failed { message: String, detail: Option<String> },
    /// A video format was asked for, but the project has no video Claws can convert.
    Unsupported { message: String },
    Cancelled,
}

/// Marks the one export as running until dropped.
struct RunGuard<'a>(&'a AppState);

const STALE_WINDOW: &str = "The previous recording window is still open. Close Wallpaper Engine's pop-out window and try again.";

impl<'a> RunGuard<'a> {
    fn begin(state: &'a AppState) -> Result<Self> {
        Self::begin_with(state, |engine, title| workshop_record::close_window(engine, title, Duration::ZERO))
    }

    fn begin_with(state: &'a AppState, close: impl FnOnce(&Path, &str) -> bool) -> Result<Self> {
        let stale = {
            let run = state.workshop.lock().unwrap();
            if run.active {
                return Err(AppError::msg(RUNNING));
            }
            if state.shutting_down.load(Ordering::SeqCst) {
                return Err(AppError::msg("Claws is closing."));
            }
            run.window.clone()
        };
        if let Some((engine, title)) = stale {
            if !close(&engine, &title) {
                return Err(AppError::msg(STALE_WINDOW));
            }
            let mut run = state.workshop.lock().unwrap();
            if run.active {
                return Err(AppError::msg(RUNNING));
            }
            if run.window.as_ref().is_some_and(|entry| entry.0 == engine && entry.1 == title) {
                run.window = None;
            }
        }
        let mut run = state.workshop.lock().unwrap();
        if run.active {
            return Err(AppError::msg(RUNNING));
        }
        if run.window.is_some() {
            return Err(AppError::msg(STALE_WINDOW));
        }
        if let Some(partial) = &run.partial {
            if !remove_owned_partial(partial) {
                return Err(AppError::msg("Claws couldn't remove the previous unfinished wallpaper file. Close any app using it and try again."));
            }
            run.partial = None;
        }
        if state.shutting_down.load(Ordering::SeqCst) {
            return Err(AppError::msg("Claws is closing."));
        }
        *run = Run { active: true, cancelled: false, pid: None, partial: None, window: None };
        Ok(Self(state))
    }

    #[allow(dead_code)] // Phase 3's tracker reads it
    fn cancelled(&self) -> bool {
        self.0.workshop.lock().unwrap().cancelled || self.0.shutting_down.load(Ordering::SeqCst)
    }
}

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        let partial = self.0.workshop.lock().unwrap().partial.take();
        let partial = partial.filter(|path| !remove_owned_partial(path));
        let mut run = self.0.workshop.lock().unwrap();
        let window = run.window.take();
        *run = Run { window, partial, ..Run::default() };
    }
}

fn remove_owned_partial(path: &Path) -> bool {
    match fs::remove_file(path) {
        Ok(()) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
}

fn register_partial(state: &AppState, partial: &Path) -> bool {
    let mut run = state.workshop.lock().unwrap();
    if run.cancelled || state.shutting_down.load(Ordering::SeqCst) { return false; }
    run.partial = Some(partial.to_path_buf());
    true
}

fn publish_if_active(state: &AppState, partial: &Path, target: &Path) -> io::Result<()> {
    let run = state.workshop.lock().unwrap();
    if run.cancelled || state.shutting_down.load(Ordering::SeqCst) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "Export cancelled"));
    }
    move_no_replace(partial, target)
}

pub fn wait_for_export_cleanup(state: &AppState) {
    let active_deadline = Instant::now() + Duration::from_secs(5);
    while state.workshop.lock().unwrap().active && Instant::now() < active_deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let partial = state.workshop.lock().unwrap().partial.clone();
        let Some(partial) = partial else { return };
        if remove_owned_partial(&partial) {
            state.workshop.lock().unwrap().partial = None;
            return;
        }
        if Instant::now() >= deadline {
            eprintln!("Claws couldn't remove an unfinished wallpaper file.");
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[tauri::command]
pub fn cancel_wallpaper_download(state: State<'_, AppState>) {
    let pid = {
        let mut run = state.workshop.lock().unwrap();
        if !run.active {
            return;
        }
        run.cancelled = true;
        run.pid
    };
    if let Some(pid) = pid {
        kill_tree(pid, false);
    }
}

/// Keep an open order and its pending window atomic with cancellation/shutdown.
fn start_recording_window(
    run: &std::sync::Mutex<Run>, shutting_down: &std::sync::atomic::AtomicBool,
    engine: &Path, title: &str, launch: impl FnOnce() -> io::Result<()>,
) -> io::Result<bool> {
    let mut current = run.lock().unwrap();
    if current.cancelled || shutting_down.load(Ordering::SeqCst) { return Ok(false); }
    current.window = Some((engine.to_path_buf(), title.to_owned()));
    if let Err(error) = launch() {
        current.window = None;
        return Err(error);
    }
    Ok(true)
}

/// For shutdown: the FFmpeg process to stop, if one runs.
pub fn stop_for_shutdown(state: &AppState) -> Option<u32> {
    let mut run = state.workshop.lock().unwrap();
    run.cancelled = true;
    run.pid
}

fn is_cancelled(state: &AppState) -> bool {
    state.workshop.lock().unwrap().cancelled || state.shutting_down.load(Ordering::SeqCst)
}

fn track_pid(state: &AppState, pid: Option<u32>) -> bool {
    let mut current = state.workshop.lock().unwrap();
    if pid.is_none() {
        current.pid = None;
        true
    } else if current.cancelled || state.shutting_down.load(Ordering::SeqCst) {
        false
    } else {
        current.pid = pid;
        true
    }
}

/// The tracker for one export: its events go to the interface as `workshop-progress`.
fn new_tracker(app: &AppHandle, url: &str) -> Tracker {
    let app = app.clone();
    Tracker::new(url, move |event| {
        let _ = app.emit("workshop-progress", event);
    })
}

#[tauri::command]
pub async fn download_wallpaper(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    format: ExportFormat,
    options: Option<ExportOptions>,
) -> Result<WallpaperOutcome> {
    let options = options.unwrap_or_default();
    let id = item_id(&url)?;
    let run = RunGuard::begin(&state)?;
    let tracker = new_tracker(&app, &url);
    let outcome = export(&app, &state, &tracker, id, format, &options).await;
    drop(run);
    let outcome = outcome?;
    if let WallpaperOutcome::Saved { path, .. } = &outcome {
        let mut saved = state.wallpaper_results.lock().unwrap();
        if saved.len() >= 1024 { saved.remove(0); }
        saved.push(PathBuf::from(path));
    }
    crate::desktop::notify_export_finished(&app, &outcome);
    Ok(outcome)
}

fn path_within(path: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let normalize = |p: &Path| {
            let raw = p.to_string_lossy().replace('/', "\\").to_lowercase();
            let raw = if let Some(unc) = raw.strip_prefix(r"\\?\unc\") { format!(r"\\{unc}") }
                else { raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string() };
            raw.trim_end_matches('\\').to_string()
        };
        let path = normalize(path);
        let root = normalize(root);
        path == root || path.strip_prefix(&root).is_some_and(|rest| rest.starts_with('\\'))
    }
    #[cfg(not(windows))]
    { path.starts_with(root) }
}

/// Resolve the existing ancestor before creating anything, preserving a missing suffix.
fn destination_outside_libraries(destination: &Path, libraries: &[PathBuf]) -> io::Result<bool> {
    if !destination.is_absolute() || destination.components().any(|part| matches!(part, std::path::Component::ParentDir)) {
        return Ok(false);
    }
    let mut ancestor = destination;
    let mut missing = Vec::new();
    while !ancestor.exists() {
        let Some(name) = ancestor.file_name() else { return Ok(false) };
        missing.push(name.to_os_string());
        ancestor = ancestor.parent().ok_or_else(|| io::Error::other("No existing destination ancestor"))?;
    }
    let mut resolved = fs::canonicalize(ancestor)?;
    for part in missing.into_iter().rev() { resolved.push(part); }
    for library in libraries {
        if path_within(destination, library) { return Ok(false); }
        match fs::canonicalize(library) {
            Ok(root) if path_within(&resolved, &root) => return Ok(false),
            Ok(_) => {},
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

fn safe_export_destination(destination: &Path) -> bool {
    let libraries = steam_root().map(|root| library_folders(&root)).unwrap_or_default();
    destination_outside_libraries(destination, &libraries).unwrap_or(false)
}

/// One export, from the lookup to the published file.
async fn export(app: &AppHandle, state: &AppState, tracker: &Tracker, id: u64, format: ExportFormat, options: &ExportOptions) -> Result<WallpaperOutcome> {
    let found = lookup(&state.steam_http, id).await.map_err(|_| AppError::msg(STEAM_UNREACHABLE))?;
    let WorkshopLookup::Wallpaper { item } = found else { return Err(not_a_wallpaper(&found)) };

    let output = PathBuf::from(state.settings.lock().unwrap().output_dir.clone());
    if !output.is_absolute() {
        return Err(AppError::msg("Choose a download folder in Settings first."));
    }
    let output = tauri::async_runtime::spawn_blocking(move || -> Result<PathBuf> {
        if !safe_export_destination(&output) { return Err(AppError::msg("Choose a download folder outside your Steam libraries.")); }
        fs::create_dir_all(&output)?;
        let output = output.canonicalize()?;
        if !safe_export_destination(&output) { return Err(AppError::msg("Choose a download folder outside your Steam libraries.")); }
        Ok(output)
    })
    .await??;

    let located = tauri::async_runtime::spawn_blocking(move || steam_root().map(|root| locate_item(&library_folders(&root), id, item.time_updated)))
        .await.map_err(|_| AppError::msg("Couldn't check the Steam library."))?;
    let Some(located) = located else {
        return Ok(WallpaperOutcome::Failed { message: "Steam's library could not be found.".into(), detail: None });
    };
    if matches!(located.state, ItemState::Downloading { .. }) {
        return Ok(WallpaperOutcome::Failed { message: "Steam is still downloading this wallpaper. Wait for it to finish, then try again.".into(), detail: None });
    }
    let Some(project) = located.project else {
        return Ok(WallpaperOutcome::Failed {
            message: "This wallpaper isn't in your Steam library yet. Subscribe to it in Steam, wait for Steam to download it, then try again.".into(),
            detail: None,
        });
    };

    let title = item.title.clone();
    let kind = tauri::async_runtime::spawn_blocking({
        let project = project.clone();
        move || workshop_media::project_kind(&project)
    })
    .await?;
    let recordable = matches!(kind, ProjectKind::Scene | ProjectKind::Web);
    let saved = match (format, format.media()) {
        (ExportFormat::Zip, _) => zip_project(app, state, tracker, &project, &output, id, &title).await?,
        (ExportFormat::Mp4 | ExportFormat::Png, _) if recordable => record_media(app, state, tracker, &project, &output, (id, &title), format, options).await?,
        (ExportFormat::Gif, _) if recordable => SaveResult::Unsupported("A scene or web wallpaper can be recorded as an MP4 or a picture, not as a GIF. Choose MP4.".into()),
        (ExportFormat::Png, _) => SaveResult::Unsupported("A picture can be taken of a scene or web wallpaper only. Choose another format.".into()),
        (_, Some(media_format)) => convert_media(app, state, tracker, &project, &output, (id, &title), media_format, options).await?,
        (_, None) => zip_project(app, state, tracker, &project, &output, id, &title).await?,
    };
    let saved_note = match &saved {
        SaveResult::DoneWithNote(_, note) => Some(note.clone()),
        _ => None,
    };
    Ok(match saved {
        SaveResult::Done(filename) | SaveResult::DoneWithNote(filename, _) => {
            let note = saved_note;
            let summary = tracker.finish();
            WallpaperOutcome::Saved {
                path: output.join(&filename).to_string_lossy().into_owned(),
                filename,
                elapsed_ms: summary.elapsed_ms,
                stages: summary.stages,
                note,
            }
        }
        SaveResult::Cancelled => WallpaperOutcome::Cancelled,
        SaveResult::Unsupported(message) => WallpaperOutcome::Unsupported { message },
        SaveResult::Failed { message, detail } => WallpaperOutcome::Failed { message, detail },
    })
}

/// How a ZIP or media save ended.
enum SaveResult {
    Done(String),
    /// Saved, with something the user should know (a note under the result).
    DoneWithNote(String, String),
    Cancelled,
    /// The project is not a video Claws can convert.
    Unsupported(String),
    Failed { message: String, detail: Option<String> },
}

/// The estimate and the free-space need are made with this much room to spare.
const SPACE_ROOM_PERCENT: u64 = 120;

/// Zips the project into the output folder, counting bytes as it goes.
async fn zip_project(app: &AppHandle, state: &AppState, tracker: &Tracker, project: &Path, output: &Path, id: u64, title: &str) -> Result<SaveResult> {
    let measured = project.to_path_buf();
    let needed = tauri::async_runtime::spawn_blocking(move || disk_budget::bytes_in(&measured)).await.unwrap_or(0);
    if let Some(message) = space_shortfall(state, output, needed / 100 * SPACE_ROOM_PERCENT, "the download folder's drive") {
        return Ok(SaveResult::Failed { message, detail: None });
    }
    tracker.stage(Stage::Zipping, Some((0.0, needed as f64, Unit::Bytes)));
    let partial = output.join(format!("{PARTIAL_PREFIX}{}{PARTIAL_SUFFIX}", uuid::Uuid::new_v4()));
    if !register_partial(state, &partial) { return Ok(SaveResult::Cancelled); }
    let (export_project, export_output, title) = (project.to_path_buf(), output.to_path_buf(), title.to_string());
    let cancel_app = app.clone();
    let tracker = tracker.clone();
    let saved = tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        let state = cancel_app.state::<AppState>();
        let cancelled = || state.workshop.lock().unwrap().cancelled || state.shutting_down.load(Ordering::SeqCst);
        save_project(&export_project, &export_output, &partial, id, &title, |done, total| tracker.bytes(done, total), || tracker.stage(Stage::Finishing, None), cancelled, |from, to| publish_if_active(&state, from, to))
    })
    .await?;
    match saved {
        Ok(Some(filename)) => Ok(SaveResult::Done(filename)),
        Ok(None) => Ok(SaveResult::Cancelled),
        Err(_) if state.workshop.lock().unwrap().cancelled || state.shutting_down.load(Ordering::SeqCst) => Ok(SaveResult::Cancelled),
        Err(error) => Err(error),
    }
}

/// What the video is, for the formats row and the estimate.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VideoInfo {
    /// The original file can be saved as it is: 8-bit H.264 in an MP4 (labelled "original").
    /// Otherwise MP4 is converted.
    pub copyable: bool,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_seconds: Option<f64>,
    pub bytes: u64,
    pub has_audio: bool,
}

fn video_info(video: &workshop_media::VideoProject, probe: &workshop_media::Probe) -> VideoInfo {
    VideoInfo {
        copyable: probe.can_copy(&video.path),
        codec: probe.video_codec.clone(),
        width: probe.width,
        height: probe.height,
        fps: probe.fps,
        duration_seconds: probe.duration,
        bytes: video.bytes,
        has_audio: probe.audio_codec.is_some(),
    }
}

/// The item's project folder in the user's libraries, read off the UI thread.
async fn library_project_of(id: u64) -> Result<Option<PathBuf>> {
    tauri::async_runtime::spawn_blocking(move || steam_root().and_then(|root| library_project(&library_folders(&root), id)))
        .await
        .map_err(|_| AppError::msg("Couldn't check the Steam library."))
}

/// What the library holds for this item's video: whether it can be copied, its size and length.
/// `None` when the item has no readable video or ffprobe is not available.
#[tauri::command]
pub async fn wallpaper_video_info(app: AppHandle, url: String) -> Result<Option<VideoInfo>> {
    let id = item_id(&url)?;
    let Some(ffprobe) = crate::tools::find_ffprobe(&app) else { return Ok(None) };
    let Some(project) = library_project_of(id).await? else { return Ok(None) };
    let found = tauri::async_runtime::spawn_blocking(move || workshop_media::video_file(&project).ok())
        .await
        .map_err(|_| AppError::msg("Couldn't check the Steam library."))?;
    let Some(video) = found else { return Ok(None) };
    Ok(workshop_media::probe_cached(&ffprobe, &video).await.map(|probe| video_info(&video, &probe)))
}

// ---------------------------------------------------------------------------------------
// The estimate shown before starting

/// What the interface shows under the format row: `seconds` is `None` while nothing backs a
/// guess ("Estimate after a few seconds").
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    pub seconds: Option<f64>,
    /// The time rests on a measured or assumed rate, not on a length chosen by the user.
    pub approximate: bool,
    pub bytes: Option<u64>,
}

/// A plain copy to a fast disk runs near this speed.
const COPY_BYTES_PER_SECOND: f64 = 200.0 * 1_000_000.0;

fn estimate_copy(bytes: u64) -> Estimate {
    Estimate { seconds: Some(bytes as f64 / COPY_BYTES_PER_SECOND), approximate: true, bytes: Some(bytes) }
}

/// A conversion's time is the media length over the speed last measured for this kind of
/// conversion. With no measurement there is no estimate.
fn estimate_conversion(media_seconds: Option<f64>, speed: Option<f64>, bytes: Option<u64>) -> Estimate {
    let seconds = match (media_seconds, speed) {
        (Some(media), Some(speed)) if media > 0.0 && speed > 0.0 => Some(media / speed),
        _ => None,
    };
    Estimate { seconds, approximate: true, bytes }
}

/// The key under which a conversion's speed is remembered: the format and the source's height
/// rounded up to a common size.
pub fn speed_key(format: MediaFormat, source_height: u32) -> String {
    let bucket = [720u32, 1080, 1440, 2160].into_iter().find(|bucket| source_height <= *bucket).unwrap_or(2160);
    format!("{}-{bucket}", format.extension())
}

/// Remembers how fast the last conversion of this kind ran, as a multiple of real time.
fn remember_speed(state: &AppState, key: &str, speed: f64) {
    if !speed.is_finite() || speed <= 0.0 {
        return;
    }
    // Best effort: a speed that was not saved is only measured again next time.
    let _ = crate::store::save_transaction(&state.settings_path(), || {
        let mut next = state.settings.lock().unwrap().clone();
        if next.workshop_speeds.len() < 16 || next.workshop_speeds.contains_key(key) {
            next.workshop_speeds.insert(key.to_string(), speed);
        }
        Ok(next)
    }, |next| *state.settings.lock().unwrap() = next.clone());
}

/// A recording's time is its length, the pre-roll and a second or so to finish: exact enough to
/// say, and not "about" the length the user chose.
async fn estimate_recording(app: &AppHandle, kind: ProjectKind, scene: Option<&scene_pkg::SceneInfo>, picture: bool, options: &ExportOptions) -> Estimate {
    let pre_roll = workshop_record::pre_roll_seconds(scene.map_or(0, |scene| scene.package_bytes));
    let Some(ffmpeg) = crate::tools::find_ffmpeg(app) else { return Estimate { seconds: None, approximate: true, bytes: None } };
    let working = encoder::available(&ffmpeg).await;
    let works = |candidate: Encoder| working.contains(&candidate);
    let authored = scene.map_or(scene_pkg::Authored::Perspective, |scene| scene.authored);
    let sizes = workshop_record::size_options(authored, crate::win_window::display(), &works);
    let Ok(size) = workshop_record::choose_size(&sizes, options.height) else { return Estimate { seconds: None, approximate: true, bytes: None } };
    let _ = kind;
    if picture {
        return Estimate { seconds: Some(f64::from(pre_roll) + 1.5), approximate: false, bytes: Some(workshop_record::expected_picture_bytes(size.width, size.height)) };
    }
    let length = options.length_seconds.unwrap_or(workshop_record::DEFAULT_LENGTH).clamp(1, workshop_record::MAX_LENGTH);
    let fps = options.fps.unwrap_or(30);
    Estimate {
        seconds: Some(workshop_record::expected_seconds(length, pre_roll)),
        approximate: false,
        bytes: Some(workshop_record::expected_video_bytes((size.width, size.height), fps, length)),
    }
}

/// The time and size an export would take, from what is known now: nothing is run.
#[tauri::command]
pub async fn estimate_wallpaper_export(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    format: ExportFormat,
    options: Option<ExportOptions>,
) -> Result<Estimate> {
    let options = options.unwrap_or_default();
    let id = item_id(&url)?;
    let Some(project) = library_project_of(id).await? else {
        return Ok(Estimate { seconds: None, approximate: true, bytes: None });
    };
    let (kind, scene, _) = inspect_project(&project).await?;
    if matches!(kind, ProjectKind::Scene | ProjectKind::Web) && matches!(format, ExportFormat::Mp4 | ExportFormat::Png) {
        return Ok(estimate_recording(&app, kind, scene.as_ref(), format == ExportFormat::Png, &options).await);
    }
    let Some(media) = format.media() else {
        let bytes = tauri::async_runtime::spawn_blocking(move || disk_budget::bytes_in(&project)).await.unwrap_or(0);
        return Ok(estimate_copy(bytes));
    };
    let video = tauri::async_runtime::spawn_blocking(move || workshop_media::video_file(&project).ok())
        .await
        .map_err(|_| AppError::msg("Couldn't check the Steam library."))?;
    let Some(video) = video else { return Ok(Estimate { seconds: None, approximate: true, bytes: None }) };
    let probe = match crate::tools::find_ffprobe(&app) {
        Some(ffprobe) => workshop_media::probe_cached(&ffprobe, &video).await,
        None => None,
    };
    if media == MediaFormat::Mp4 && probe.as_ref().is_some_and(|probe| probe.can_copy(&video.path)) {
        return Ok(estimate_copy(video.bytes));
    }
    let Some(probe) = probe else { return Ok(Estimate { seconds: None, approximate: true, bytes: None }) };
    let speeds = state.settings.lock().unwrap().workshop_speeds.clone();
    Ok(estimate_probe_conversion(&probe, media, video.bytes, options.length_seconds, &speeds))
}

fn estimate_probe_conversion(
    probe: &workshop_media::Probe,
    media: MediaFormat,
    video_bytes: u64,
    length_seconds: Option<u32>,
    speeds: &std::collections::BTreeMap<String, f64>,
) -> Estimate {
    let plan_length = (media == MediaFormat::Gif).then(|| workshop_media::gif_length(length_seconds));
    let media_seconds = workshop_media::output_seconds(probe.duration, plan_length);
    let speed = speeds.get(&speed_key(media, probe.height)).copied();
    let bytes = (media == MediaFormat::Mp4).then_some(video_bytes);
    estimate_conversion(media_seconds, speed, bytes)
}

// ---------------------------------------------------------------------------------------
// Recording a scene or web wallpaper

/// What a scene's package says, when it can be read: the package, else a loose `scene.json`.
fn read_scene(project: &Path) -> std::result::Result<Option<scene_pkg::SceneInfo>, scene_pkg::PkgError> {
    let package = project.join("scene.pkg");
    if fs::symlink_metadata(&package).is_ok_and(|meta| meta.is_file()) {
        return scene_pkg::analyze_file(&package).map(Some);
    }
    let loose = project.join("scene.json");
    if fs::symlink_metadata(&loose).is_ok_and(|meta| meta.is_file()) {
        return scene_pkg::analyze_loose_scene(&loose).map(Some);
    }
    Ok(None)
}

const UNREADABLE_SCENE: &str = "Claws couldn't read this scene's files, so it can't record it. Save the ZIP instead.";

/// What the Download panel needs to offer recording for a scene or web wallpaper. `None` for any
/// other kind of project.
#[tauri::command]
pub async fn wallpaper_record_info(app: AppHandle, url: String) -> Result<Option<workshop_record::RecordInfo>> {
    let id = item_id(&url)?;
    let Some(project) = library_project_of(id).await? else { return Ok(None) };
    let inspected = project.clone();
    let (kind, scene) = tauri::async_runtime::spawn_blocking(move || {
        let kind = workshop_media::project_kind(&inspected);
        let scene = (kind == ProjectKind::Scene).then(|| read_scene(&inspected));
        (kind, scene)
    })
    .await
    .map_err(|_| AppError::msg("Couldn't check the Steam library."))?;
    let label = match kind {
        ProjectKind::Scene => "scene",
        ProjectKind::Web => "web",
        _ => return Ok(None),
    };
    let (info, unreadable) = match scene {
        Some(Ok(info)) => (info, false),
        Some(Err(_)) => (None, true),
        None => (None, false),
    };
    let ffmpeg = crate::tools::find_ffmpeg(&app);
    let mut found = workshop_record::record_info(label, info, ffmpeg.as_deref()).await;
    if unreadable {
        found.available = false;
        found.reason = Some(UNREADABLE_SCENE.to_string());
    }
    Ok(Some(found))
}

/// The project's kind and, for a scene, what its files say.
type Inspected = (ProjectKind, Option<scene_pkg::SceneInfo>, Option<scene_pkg::PkgError>);

async fn inspect_project(project: &Path) -> Result<Inspected> {
    let inspected = project.to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let kind = workshop_media::project_kind(&inspected);
        if kind != ProjectKind::Scene {
            return (kind, None, None);
        }
        match read_scene(&inspected) {
            Ok(info) => (kind, info, None),
            Err(error) => (kind, None, Some(error)),
        }
    })
    .await
    .map_err(|_| AppError::msg("Couldn't check the Steam library."))
}

/// Records the wallpaper through Wallpaper Engine and publishes the file.
#[allow(clippy::too_many_arguments)]
async fn record_media(
    app: &AppHandle,
    state: &AppState,
    tracker: &Tracker,
    project: &Path,
    output: &Path,
    item: (u64, &str),
    format: ExportFormat,
    options: &ExportOptions,
) -> Result<SaveResult> {
    const NO_FFMPEG: &str = "Recording a wallpaper needs FFmpeg. Install it in Settings, then try again.";
    let (id, title) = item;
    let cancelled = || is_cancelled(state);
    let on_spawn = |pid| track_pid(state, pid);
    if cancelled() {
        return Ok(SaveResult::Cancelled);
    }
    let Some(ffmpeg) = crate::tools::find_ffmpeg(app) else {
        return Ok(SaveResult::Failed { message: NO_FFMPEG.to_string(), detail: None });
    };
    let Some(capturer) = workshop_record::has_gfxcapture_tracked(&ffmpeg, &on_spawn, &cancelled).await else {
        return Ok(SaveResult::Cancelled);
    };
    if !capturer {
        return Ok(SaveResult::Failed { message: "This FFmpeg can't capture a window. Update FFmpeg in Settings → Tools, then try again.".into(), detail: None });
    }
    if cancelled() {
        return Ok(SaveResult::Cancelled);
    }
    let engine = match tauri::async_runtime::spawn_blocking(workshop_record::find_engine).await? {
        Ok(engine) => engine,
        Err(problem) => return Ok(SaveResult::Failed { message: problem.message().to_string(), detail: None }),
    };
    if cancelled() {
        return Ok(SaveResult::Cancelled);
    }
    let (kind, scene, problem) = inspect_project(project).await?;
    if problem.is_some() {
        return Ok(SaveResult::Failed { message: UNREADABLE_SCENE.to_string(), detail: None });
    }
    if cancelled() {
        return Ok(SaveResult::Cancelled);
    }
    let authored = scene.as_ref().map_or(scene_pkg::Authored::Perspective, |scene| scene.authored);
    let Some(working) = encoder::available_tracked(&ffmpeg, &on_spawn, &cancelled).await else {
        return Ok(SaveResult::Cancelled);
    };
    let works = |candidate: Encoder| working.contains(&candidate);
    let sizes = workshop_record::size_options(authored, crate::win_window::display(), &works);
    let size = match workshop_record::choose_size(&sizes, options.height) {
        Ok(size) => size,
        Err(message) => return Ok(SaveResult::Failed { message, detail: None }),
    };
    let picture = format == ExportFormat::Png;
    let fps = if picture { 30 } else { options.fps.unwrap_or(30) };
    if !size.fps.contains(&fps) {
        return Ok(SaveResult::Failed {
            message: format!("{fps} frames per second isn't available at {} on this PC. Choose 30.", size.label),
            detail: None,
        });
    }
    let Some(chosen) = encoder::pick_tested(&ffmpeg, &working, size.width, size.height, fps, &on_spawn).await else {
        if cancelled() { return Ok(SaveResult::Cancelled); }
        return Ok(SaveResult::Failed { message: "This PC has no video encoder that can record at that size.".into(), detail: None });
    };
    let length = options.length_seconds.unwrap_or(workshop_record::DEFAULT_LENGTH).clamp(1, workshop_record::MAX_LENGTH);
    let pre_roll = workshop_record::pre_roll_seconds(scene.as_ref().map_or(0, |scene| scene.package_bytes));
    let needed = if picture { workshop_record::expected_picture_bytes(size.width, size.height) } else { workshop_record::expected_video_bytes((size.width, size.height), fps, length) };
    if let Some(message) = space_shortfall(state, output, needed / 100 * SPACE_ROOM_PERCENT, "the download folder's drive") {
        return Ok(SaveResult::Failed { message, detail: None });
    }

    let capture = workshop_record::Capture {
        width: size.width,
        height: size.height,
        fps,
        kind: if picture { workshop_record::CaptureKind::Picture } else { workshop_record::CaptureKind::Video { length, encoder: chosen } },
    };
    let partial = output.join(format!("{PARTIAL_PREFIX}{}{PARTIAL_SUFFIX}", uuid::Uuid::new_v4()));
    if !register_partial(state, &partial) { return Ok(SaveResult::Cancelled); }
    let low_space = || low_on_space(state, output);
    let announce = |window: Option<&str>| {
        state.workshop.lock().unwrap().window = window.map(|title| (engine.exe.clone(), title.to_string()));
    };
    let project_json = project.join("project.json");
    let ffprobe = crate::tools::find_ffprobe(app);
    // Holds the engine so an install cannot replace FFmpeg while it records.
    let _engine = state.engine.read().await;
    let request = workshop_record::RecordRequest {
        ffmpeg: &ffmpeg,
        ffprobe: ffprobe.as_deref(),
        engine: &engine,
        project_json: &project_json,
        partial: &partial,
        capture,
        pre_roll,
        moves: kind == ProjectKind::Scene && scene.as_ref().is_some_and(|scene| scene.moves),
        tracker,
        cancelled: &cancelled,
        on_spawn: &on_spawn,
        low_space: &low_space,
        announce: &announce,
        open: &|title, args| start_recording_window(&state.workshop, &state.shutting_down, &engine.exe, title, || workshop_record::engine_command(&engine.exe, args)),
    };
    let recorded = workshop_record::record(&request).await;
    state.workshop.lock().unwrap().pid = None;
    let (repeated, motion_inconclusive) = match recorded {
        Ok(workshop_record::Recorded::Done { repeated_percent, motion_inconclusive }) => (repeated_percent, motion_inconclusive),
        Ok(workshop_record::Recorded::Cancelled) => return Ok(SaveResult::Cancelled),
        Ok(workshop_record::Recorded::LowSpace) => return Ok(SaveResult::Failed { message: LOW_SPACE.into(), detail: None }),
        Err(failure) => {
            if cancelled() {
                return Ok(SaveResult::Cancelled);
            }
            let detail = redact(&failure.detail);
            return Ok(SaveResult::Failed { message: failure.message, detail: (!detail.trim().is_empty()).then_some(detail) });
        }
    };
    let (output_dir, title) = (output.to_path_buf(), title.to_string());
    let extension = if picture { "png" } else { "mp4" };
    let publish_app = app.clone();
    let published = tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        let state = publish_app.state::<AppState>();
        publish_media(&partial, &output_dir, id, &title, extension, |from, to| publish_if_active(&state, from, to))
    }).await?;
    match published {
        Ok(filename) => {
            match recording_note(repeated, motion_inconclusive) {
                Some(note) => Ok(SaveResult::DoneWithNote(filename, note)),
                None => Ok(SaveResult::Done(filename)),
            }
        },
        Err(_) if cancelled() => Ok(SaveResult::Cancelled),
        Err(error) => Err(error),
    }
}

fn recording_note(repeated: f64, motion_inconclusive: bool) -> Option<String> {
    let mut notes = Vec::new();
    if repeated > 5.0 { notes.push(format!("About {repeated:.0}% of the frames are repeats: the PC was too busy to capture every frame. Record again while it is quieter for a smoother result.")); }
    if motion_inconclusive { notes.push("Claws couldn't check whether the recording moves. Review the saved file.".to_string()); }
    (!notes.is_empty()).then(|| notes.join(" "))
}

/// For shutdown: closes the Wallpaper Engine window a recording has open, if there is one.
pub fn close_recording_window(state: &AppState) {
    let window = state.workshop.lock().unwrap().window.clone();
    if let Some((engine, title)) = window {
        if workshop_record::close_window(&engine, &title, Duration::from_secs(15)) {
            let mut current = state.workshop.lock().unwrap();
            if current.window.as_ref().is_some_and(|entry| entry.0 == engine && entry.1 == title) {
                current.window = None;
            }
        }
    }
}

/// Whether the drive that holds `folder` is below the reserve the download queue keeps.
fn low_on_space(state: &AppState, folder: &Path) -> bool {
    (state.free_space)(folder).is_ok_and(|free| free < disk_budget::reserve())
}

const LOW_SPACE: &str = "The drive is almost full, so Claws stopped and removed the unfinished file. Free up space, then try again.";

/// The encoder for an MP4 conversion: a hardware one when the source is over 1440p (software
/// runs 4K at about 0.84 times real time), software otherwise, which is what it always was.
/// Returns `None` when cancelled while probing.
async fn pick_conversion_encoder(
    ffmpeg: &Path,
    height: u32,
    on_spawn: &(dyn Fn(Option<u32>) -> bool + Sync),
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Option<Encoder> {
    if cancelled() {
        return None;
    }
    if height <= encoder::SOFTWARE_MAX_HEIGHT {
        return Some(Encoder::X264);
    }
    let working = encoder::available_tracked(ffmpeg, on_spawn, cancelled).await?;
    Some(encoder::ORDER.into_iter().find(|candidate| candidate.hardware() && working.contains(candidate)).unwrap_or(Encoder::X264))
}

/// Saves the project's declared video and publishes the result. An H.264 MP4 is copied as it
/// is; anything else is converted with the managed FFmpeg. The project is only read; nothing
/// in it is run. `item` is its Workshop id and title.
#[allow(clippy::too_many_arguments)]
async fn convert_media(
    app: &AppHandle,
    state: &AppState,
    tracker: &Tracker,
    project: &Path,
    output: &Path,
    item: (u64, &str),
    format: MediaFormat,
    options: &ExportOptions,
) -> Result<SaveResult> {
    const NO_VIDEO: &str = "This wallpaper's project has no video that Claws can convert. Save it as a ZIP instead.";
    const UNSAFE: &str = "This wallpaper's project points to a file outside the downloaded item. Claws won't convert it; save it as a ZIP instead.";
    const NO_FFMPEG: &str = "Converting a wallpaper needs FFmpeg. Install it in Settings, then try again.";

    let (id, title) = item;
    let cancelled = || is_cancelled(state);
    let on_spawn = |pid| track_pid(state, pid);
    if cancelled() {
        return Ok(SaveResult::Cancelled);
    }
    let measured = project.to_path_buf();
    let checked = tauri::async_runtime::spawn_blocking(move || workshop_media::video_file(&measured)).await?;
    let video = match checked {
        Ok(video) => video,
        Err(MediaError::Unsupported) => return Ok(SaveResult::Unsupported(NO_VIDEO.to_string())),
        Err(MediaError::Unsafe) => return Ok(SaveResult::Failed { message: UNSAFE.to_string(), detail: None }),
    };
    if cancelled() {
        return Ok(SaveResult::Cancelled);
    }
    let Some(ffmpeg) = crate::tools::find_ffmpeg(app) else {
        return Ok(SaveResult::Failed { message: NO_FFMPEG.to_string(), detail: None });
    };
    let probe = match crate::tools::find_ffprobe(app) {
        Some(ffprobe) => workshop_media::probe_cached_tracked(&ffprobe, &video, &on_spawn).await,
        None => None,
    };
    if cancelled() {
        return Ok(SaveResult::Cancelled);
    }
    let partial = output.join(format!("{PARTIAL_PREFIX}{}{PARTIAL_SUFFIX}", uuid::Uuid::new_v4()));
    if !register_partial(state, &partial) { return Ok(SaveResult::Cancelled); }

    if format == MediaFormat::Mp4 && probe.as_ref().is_some_and(|probe| probe.can_copy(&video.path)) {
        // The original is already the right kind of file: copy it, byte for byte.
        if let Some(message) = space_shortfall(state, output, video.bytes / 100 * SPACE_ROOM_PERCENT, "the download folder's drive") {
            return Ok(SaveResult::Failed { message, detail: None });
        }
        tracker.stage(Stage::Copying, Some((0.0, video.bytes as f64, Unit::Bytes)));
        let (source, target) = (video.path.clone(), partial.clone());
        let cancel_app = app.clone();
        let (counting, finishing) = (tracker.clone(), tracker.clone());
        let copied = tauri::async_runtime::spawn_blocking(move || {
            use tauri::Manager;
            let state = cancel_app.state::<AppState>();
            let cancelled = || state.workshop.lock().unwrap().cancelled || state.shutting_down.load(Ordering::SeqCst);
            workshop_media::copy_original(&source, &target, |done, total| counting.bytes(done, total), || finishing.stage(Stage::Finishing, None), cancelled)
        })
        .await?;
        return match copied {
            Ok(workshop_media::CopyOutcome::Cancelled) => Ok(SaveResult::Cancelled),
            Ok(workshop_media::CopyOutcome::Done) => {
                let (output, title) = (output.to_path_buf(), title.to_string());
                let publish_app = app.clone();
                let published = tauri::async_runtime::spawn_blocking(move || {
                    use tauri::Manager;
                    let state = publish_app.state::<AppState>();
                    publish_media(&partial, &output, id, &title, "mp4", |from, to| publish_if_active(&state, from, to))
                }).await?;
                match published {
                    Ok(filename) => Ok(SaveResult::Done(filename)),
                    Err(_) if cancelled() => Ok(SaveResult::Cancelled),
                    Err(error) => Err(error),
                }
            }
            Err(_) if cancelled() => Ok(SaveResult::Cancelled),
            Err(error) => Ok(SaveResult::Failed { message: format!("Couldn't copy the wallpaper's video ({error}). Save it as a ZIP instead."), detail: None }),
        };
    }

    let mut needed = video.bytes.saturating_mul(format.space_factor());
    if format == MediaFormat::Gif {
        needed = needed.min(1024 * 1024 * 1024);
    }
    if let Some(message) = space_shortfall(state, output, needed / 100 * SPACE_ROOM_PERCENT, "the download folder's drive") {
        return Ok(SaveResult::Failed { message, detail: None });
    }

    let (width, height, fps) = probe.as_ref().map_or((1920, 1080, 30), |probe| (probe.width, probe.height, probe.fps.round().clamp(1.0, 240.0) as u32));
    let encoder = if format == MediaFormat::Mp4 {
        let Some(encoder) = pick_conversion_encoder(&ffmpeg, height, &on_spawn, &cancelled).await else {
            return Ok(SaveResult::Cancelled);
        };
        encoder
    } else {
        Encoder::X264
    };
    let plan = workshop_media::ConvertPlan {
        format,
        encoder,
        width,
        height,
        fps: fps.max(1),
        length: (format == MediaFormat::Gif).then(|| workshop_media::gif_length(options.length_seconds)),
    };
    let total_seconds = plan.output_seconds(probe.as_ref().and_then(|probe| probe.duration));

    tracker.stage(Stage::Converting, total_seconds.map(|total| (0.0, total, Unit::Seconds)));
    let converting_from = tracker.elapsed();
    // Holds the engine so an install cannot replace FFmpeg while it runs.
    let _engine = state.engine.read().await;
    let low = std::sync::atomic::AtomicBool::new(false);
    let last_space_check = std::sync::Mutex::new(Instant::now());
    let on_progress = |block: &export_progress::FfmpegProgress| {
        if let (Some(total), Some(time)) = (total_seconds, block.out_time_us) {
            tracker.converting(time as f64 / 1_000_000.0, total, block.speed);
        }
        // The drive may fill while a long conversion runs: stop before it is out of room.
        let mut last = last_space_check.lock().unwrap();
        if last.elapsed() >= Duration::from_secs(5) {
            *last = Instant::now();
            if low_on_space(state, output) {
                low.store(true, Ordering::SeqCst);
                let pid = {
                    let mut run = state.workshop.lock().unwrap();
                    run.cancelled = true;
                    run.pid
                };
                if let Some(pid) = pid {
                    kill_tree(pid, false);
                }
            }
        }
    };
    let converted = workshop_media::convert(
        &ffmpeg,
        &video.path,
        &partial,
        &plan,
        on_spawn,
        &cancelled,
        &on_progress,
    )
    .await;
    state.workshop.lock().unwrap().pid = None;
    match converted {
        Ok(workshop_media::Outcome::Cancelled) => {
            let _ = fs::remove_file(&partial);
            if low.load(Ordering::SeqCst) {
                return Ok(SaveResult::Failed { message: LOW_SPACE.into(), detail: None });
            }
            return Ok(SaveResult::Cancelled);
        }
        Ok(workshop_media::Outcome::Done) => {}
        Err(failure) => {
            let _ = fs::remove_file(&partial);
            if low.load(Ordering::SeqCst) {
                return Ok(SaveResult::Failed { message: LOW_SPACE.into(), detail: None });
            }
            if cancelled() {
                return Ok(SaveResult::Cancelled);
            }
            let detail = redact(&failure.detail);
            return Ok(SaveResult::Failed { message: failure.message, detail: (!detail.trim().is_empty()).then_some(detail) });
        }
    }

    // What this conversion taught about speed, for the next estimate.
    let spent = tracker.elapsed().saturating_sub(converting_from).as_secs_f64();
    if let Some(total) = total_seconds.filter(|total| *total >= 5.0 && spent >= 3.0) {
        remember_speed(state, &speed_key(format, height), total / spent);
    }

    tracker.stage(Stage::Finishing, None);
    let (partial_for_task, output_for_task, title) = (partial.clone(), output.to_path_buf(), title.to_string());
    let publish_app = app.clone();
    let published = tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        let state = publish_app.state::<AppState>();
        publish_media(&partial_for_task, &output_for_task, id, &title, format.extension(), |from, to| publish_if_active(&state, from, to))
    }).await?;
    match published {
        Ok(filename) => Ok(SaveResult::Done(filename)),
        Err(_) if cancelled() => Ok(SaveResult::Cancelled),
        Err(error) => Err(error),
    }
}

/// Moves a finished conversion into the output folder under the next free name. A missing,
/// linked or empty file is refused, and a failed publish removes the partial copy.
fn publish_media(partial: &Path, output: &Path, id: u64, title: &str, extension: &str, mut publish: impl FnMut(&Path, &Path) -> io::Result<()>) -> Result<String> {
    let result = (|| -> Result<String> {
        let meta = fs::symlink_metadata(partial)?;
        if !meta.is_file() || is_link(&meta) || meta.len() == 0 {
            return Err(AppError::msg("The converted video is empty, so it was not saved."));
        }
        OpenOptions::new().write(true).open(partial)?.sync_all()?;
        for copy in 0..1000 {
            let name = export_name(title, id, copy, extension);
            match publish(partial, &output.join(&name)) {
                Ok(()) => return Ok(name),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(AppError::msg("Too many copies of this wallpaper in the download folder."))
    })();
    if result.is_err() {
        let _ = fs::remove_file(partial);
    }
    result
}

fn space_shortfall(state: &AppState, folder: &Path, bytes: u64, place: &str) -> Option<String> {
    let free = (state.free_space)(folder).ok()?;
    disk_budget::check(bytes, free, 0).err().map(|short| {
        format!(
            "There isn't enough free space on {place}. It needs {} more. Free up space, then try again.",
            format_bytes(short.required.saturating_sub(short.available))
        )
    })
}

/// FFmpeg's error text without the user's home folder, which it prints in paths.
fn redact(text: &str) -> String {
    let Some(home) = std::env::var("USERPROFILE").ok().filter(|home| home.len() > 3) else { return text.to_string() };
    redact_with(text, &home)
}

fn redact_with(text: &str, home: &str) -> String {
    let mut out = text.to_string();
    for form in [home.to_string(), home.replace('\\', "/")] {
        let lower = out.to_ascii_lowercase();
        let needle = form.to_ascii_lowercase();
        let mut next = String::with_capacity(out.len());
        let mut at = 0;
        while let Some(found) = lower[at..].find(&needle) {
            next.push_str(&out[at..at + found]);
            next.push_str("%USERPROFILE%");
            at += found + needle.len();
        }
        next.push_str(&out[at..]);
        out = next;
    }
    out
}

fn format_bytes(bytes: u64) -> String {
    let units = ["bytes", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < units.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{value:.1} {}", units[unit])
    }
}




// ---------------------------------------------------------------------------------------
// ZIP export

/// A file name made from the item title: `<title> (<id>).<extension>`.
fn export_name(title: &str, id: u64, copy: u32, extension: &str) -> String {
    let mut clean = String::new();
    for c in title.chars() {
        let c = if c.is_control() || r#"<>:"/\|?*"#.contains(c) { ' ' } else { c };
        if c == ' ' && clean.ends_with(' ') {
            continue;
        }
        clean.push(c);
    }
    let mut clean: String = clean.trim().trim_end_matches(['.', ' ']).chars().take(80).collect();
    clean = clean.trim_end_matches(['.', ' ']).to_string();
    let stem = clean.split('.').next().unwrap_or("").trim().to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    let base = if clean.is_empty() || reserved { format!("Wallpaper Engine {id}") } else { format!("{clean} ({id})") };
    if copy == 0 {
        format!("{base}.{extension}")
    } else {
        format!("{base} ({copy}).{extension}")
    }
}

fn compresses(name: &str) -> bool {
    let extension = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    ["json", "html", "htm", "js", "css", "txt", "frag", "vert", "xml", "svg", "md", "glsl"].contains(&extension.as_str())
}

/// Opens a file without following a link that replaced it after it was checked.
pub(crate) fn open_plain(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || is_link(&meta) {
        return Err(io::Error::other("The wallpaper contains a linked file."));
    }
    Ok(file)
}

/// Zips the project into the output folder. `Ok(None)` when cancelled.
#[allow(clippy::too_many_arguments)]
fn save_project(
    project: &Path,
    output: &Path,
    partial: &Path,
    id: u64,
    title: &str,
    mut report: impl FnMut(u64, u64),
    on_finish: impl FnOnce(),
    cancelled: impl Fn() -> bool,
    mut publish: impl FnMut(&Path, &Path) -> io::Result<()>,
) -> Result<Option<String>> {
    let meta = fs::symlink_metadata(project.join("project.json"))
        .map_err(|_| AppError::msg("The download doesn't contain a Wallpaper Engine project."))?;
    if !meta.is_file() || meta.len() == 0 || is_link(&meta) {
        return Err(AppError::msg("The download doesn't contain a Wallpaper Engine project."));
    }
    let mut files = Vec::new();
    collect(project, "", 0, &mut files)?;
    let total: u64 = files.iter().map(|entry| entry.size).sum();
    if total > MAX_EXPORT_BYTES {
        return Err(AppError::msg("This wallpaper is too large to export."));
    }

    let file = OpenOptions::new().write(true).create_new(true).open(partial)?;
    let result = (|| -> Result<Option<String>> {
        let mut zip = zip::ZipWriter::new(BufWriter::with_capacity(256 * 1024, file));
        let mut buffer = vec![0u8; 256 * 1024];
        let mut done = 0u64;
        report(0, total);
        for entry in &files {
            if cancelled() {
                return Ok(None);
            }
            let options = zip::write::SimpleFileOptions::default().compression_method(if entry.is_dir {
                zip::CompressionMethod::Stored
            } else if compresses(&entry.name) {
                zip::CompressionMethod::Deflated
            } else {
                zip::CompressionMethod::Stored
            });
            if entry.is_dir {
                zip.add_directory(&entry.name, options)?;
                continue;
            }
            zip.start_file(&entry.name, options.large_file(entry.size >= u32::MAX as u64))?;
            let mut source = open_plain(&entry.path)?;
            loop {
                let read = source.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                zip.write_all(&buffer[..read])?;
                done += read as u64;
                report(done, total);
                if cancelled() {
                    return Ok(None);
                }
            }
        }
        on_finish();
        let writer = zip.finish()?;
        writer.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        for copy in 0..1000 {
            let name = export_name(title, id, copy, "zip");
            if cancelled() { return Ok(None); }
            match publish(partial, &output.join(&name)) {
                Ok(()) => return Ok(Some(name)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(AppError::msg("Too many copies of this wallpaper in the download folder."))
    })();
    if !matches!(result, Ok(Some(_))) {
        let _ = fs::remove_file(partial);
    }
    result
}

struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    size: u64,
}

fn collect(folder: &Path, prefix: &str, depth: u8, out: &mut Vec<Entry>) -> Result<()> {
    if depth > 48 {
        return Err(AppError::msg("This wallpaper has too many nested folders."));
    }
    let mut children = fs::read_dir(folder)?.collect::<io::Result<Vec<_>>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        if out.len() >= MAX_ENTRIES {
            return Err(AppError::msg("This wallpaper has too many files."));
        }
        let path = entry.path();
        let meta = fs::symlink_metadata(&path)?;
        let base = entry.file_name();
        let name = base
            .to_str()
            .filter(|s| !s.contains(['\\', '/', ':']))
            .ok_or_else(|| AppError::msg("A wallpaper file has an unsupported name."))?;
        let name = format!("{prefix}{name}");
        if is_link(&meta) {
            return Err(AppError::msg("The wallpaper contains a linked file or folder."));
        }
        if meta.is_dir() {
            let prefix = format!("{name}/");
            out.push(Entry { name: prefix.clone(), path: path.clone(), is_dir: true, size: 0 });
            collect(&path, &prefix, depth + 1, out)?;
        } else if meta.is_file() {
            out.push(Entry { name, path, is_dir: false, size: meta.len() });
        } else {
            return Err(AppError::msg("The wallpaper contains an unsupported file."));
        }
    }
    Ok(())
}

/// Removes exports a crash left half-written in the download folder.
pub fn sweep_partials(dir: &Path) {
    let libraries = steam_root().map(|root| library_folders(&root)).unwrap_or_default();
    sweep_partials_in_libraries(dir, &libraries);
}

fn sweep_partials_in_libraries(dir: &Path, libraries: &[PathBuf]) {
    if !destination_outside_libraries(dir, libraries).unwrap_or(false) { return; }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(middle) = name.strip_prefix(PARTIAL_PREFIX).and_then(|rest| rest.strip_suffix(PARTIAL_SUFFIX)) else {
            continue;
        };
        if uuid::Uuid::parse_str(middle).is_ok() && entry.file_type().is_ok_and(|t| t.is_file()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    #[cfg(windows)]
    use std::process::Stdio;
    #[cfg(windows)]
    use tokio::{process::Command, time::timeout};

    #[test]
    fn classifies_wallpapers_collections_and_other_items() {
        let body = br#"{"response":{"publishedfiledetails":[
            {"publishedfileid":"123","consumer_app_id":431960,"creator_app_id":431960,"result":1,"title":" Ocean dusk ",
             "file_size":"31224037","time_updated":1602087909,
             "preview_url":"https://images.steamusercontent.com/ugc/1/ABC/",
             "tags":[{"tag":"Wallpaper"},{"tag":"Scene"},{"tag":"Approved"}]},
            {"publishedfileid":"456","consumer_app_id":480,"result":1},
            {"publishedfileid":"789","consumer_app_id":431960,"result":9},
            {"publishedfileid":"321","consumer_app_id":431960,"creator_app_id":766,"result":1,"title":"Set"},
            {"publishedfileid":"654","consumer_app_id":431960,"creator_app_id":431960,"result":1,"title":"x",
             "preview_url":"https://evil.example/preview.png"}
        ]}}"#;
        let WorkshopLookup::Wallpaper { item } = parse_details(body, 123).unwrap() else { panic!("wallpaper") };
        assert_eq!(item.title, "Ocean dusk");
        assert_eq!(item.file_size, Some(31224037));
        assert_eq!(item.wallpaper_type.as_deref(), Some("Scene"));
        assert_eq!(item.preview_url.as_deref(), Some("https://images.steamusercontent.com/ugc/1/ABC/"));
        assert_eq!(parse_details(body, 456).unwrap(), WorkshopLookup::OtherApp { app_id: 480 });
        assert_eq!(parse_details(body, 789).unwrap(), WorkshopLookup::NotFound);
        assert_eq!(parse_details(body, 999).unwrap(), WorkshopLookup::NotFound);
        assert_eq!(parse_details(body, 321).unwrap(), WorkshopLookup::Collection { title: "Set".into(), items: 0 });
        let WorkshopLookup::Wallpaper { item } = parse_details(body, 654).unwrap() else { panic!("wallpaper") };
        assert_eq!(item.preview_url, None, "previews only from Valve's hosts");
    }

    #[test]
    fn reads_children_and_required_items() {
        let body = br#"{"response":{"collectiondetails":[{"publishedfileid":"321","result":1,
            "children":[{"publishedfileid":"1","filetype":0},{"publishedfileid":"2","filetype":0}]}]}}"#;
        assert_eq!(parse_children(body, 321), ["1", "2"]);
        let none = br#"{"response":{"collectiondetails":[{"publishedfileid":"123","result":9}]}}"#;
        assert!(parse_children(none, 123).is_empty());
        assert!(parse_children(b"not json", 1).is_empty());
    }

    #[test]
    fn lookup_serializes_for_the_interface() {
        let json = serde_json::to_value(WorkshopLookup::OtherApp { app_id: 480 }).unwrap();
        assert_eq!(json, serde_json::json!({"kind":"otherApp","appId":480}));
        let json = serde_json::to_value(WallpaperOutcome::Failed { message: "m".into(), detail: None }).unwrap();
        assert_eq!(json, serde_json::json!({"status":"failed","message":"m","detail":null}));
        let json = serde_json::to_value(WallpaperOutcome::Unsupported { message: "u".into() }).unwrap();
        assert_eq!(json, serde_json::json!({"status":"unsupported","message":"u"}));
        assert_eq!(serde_json::to_value(ExportFormat::Zip).unwrap(), "zip");
        assert_eq!(serde_json::to_value(ExportFormat::Mp4).unwrap(), "mp4");
        assert_eq!(serde_json::to_value(ExportFormat::Gif).unwrap(), "gif");
        assert_eq!(serde_json::to_value(ExportFormat::Png).unwrap(), "png");
        let saved = WallpaperOutcome::Saved { filename: "f".into(), path: "p".into(), elapsed_ms: 63_000, stages: vec![StageTime { name: "copying", ms: 62_000 }], note: None };
        assert_eq!(
            serde_json::to_value(saved).unwrap(),
            serde_json::json!({"status":"saved","filename":"f","path":"p","elapsedMs":63000,"stages":[{"name":"copying","ms":62000}],"note":null})
        );
    }

    #[test]
    fn parses_a_single_workshop_id() {
        let url = "https://steamcommunity.com/sharedfiles/filedetails/?id=123&searchtext=blue";
        assert_eq!(item_id(url).unwrap(), 123);
        for invalid in [
            "https://steamcommunity.com.evil.test/sharedfiles/filedetails/?id=123",
            "https://steamcommunity.com/sharedfiles/filedetails/?id=123&id=456",
            "https://steamcommunity.com/workshop/browse/?id=123",
            "https://steamcommunity.com/sharedfiles/filedetails/?id=%2Bquit",
            "https://steamcommunity.com/sharedfiles/filedetails/?id=0",
        ] {
            assert!(item_id(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn redacts_the_home_folder() {
        let text = r"Error opening C:\Users\Ana\Videos\x.mp4 and c:/users/ana/y";
        assert_eq!(redact_with(text, r"C:\Users\Ana"), r"Error opening %USERPROFILE%\Videos\x.mp4 and %USERPROFILE%/y");
        assert_eq!(redact_with("nothing here", r"C:\Users\Ana"), "nothing here");
    }

    #[test]
    fn names_exports_after_the_title() {
        assert_eq!(export_name("Ocean: dusk / 4K", 7, 0, "zip"), "Ocean dusk 4K (7).zip");
        assert_eq!(export_name("Ocean", 7, 2, "zip"), "Ocean (7) (2).zip");
        assert_eq!(export_name("  ...  ", 7, 0, "zip"), "Wallpaper Engine 7.zip");
        assert_eq!(export_name("con", 7, 0, "zip"), "Wallpaper Engine 7.zip");
        assert_eq!(export_name("COM3.scene", 7, 0, "zip"), "Wallpaper Engine 7.zip");
        assert_eq!(export_name("Trailing dots...", 7, 0, "zip"), "Trailing dots (7).zip");
        let long = "é".repeat(200);
        assert_eq!(export_name(&long, 7, 0, "zip").chars().count(), 80 + " (7).zip".len());
        assert_eq!(export_name("Ocean", 7, 0, "mp4"), "Ocean (7).mp4");
        assert_eq!(export_name("Ocean", 7, 1, "gif"), "Ocean (7) (1).gif");
    }


    #[test]
    fn inconclusive_motion_is_reported_on_the_saved_recording() {
        assert!(recording_note(0.0, true).unwrap().contains("couldn't check"));
        assert!(recording_note(0.0, false).is_none());
        assert!(recording_note(10.0, true).unwrap().contains("repeats"));
    }

    #[test]
    fn shutdown_cancellation_refuses_publish_and_owner_removes_partial() {
        use tauri::Manager;
        let dir = crate::download_tests::temp_dir("export-owner");
        let app = tauri::test::mock_builder().build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let writer = crate::downloader::start_queue_writer(app.handle());
        app.manage(AppState::new(dir.clone(), Default::default(), Vec::new(), Vec::new(), writer, false));
        let state = app.state::<AppState>();
        let guard = RunGuard::begin(&state).unwrap();
        let partial = dir.join("owned.partial");
        let target = dir.join("saved.mp4");
        assert!(register_partial(&state, &partial));
        fs::write(&partial, b"video").unwrap();
        stop_for_shutdown(&state);
        assert_eq!(publish_if_active(&state, &partial, &target).unwrap_err().kind(), io::ErrorKind::Interrupted);
        drop(guard);
        wait_for_export_cleanup(&state);
        assert!(!partial.exists() && !target.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn locked_partial_is_retained_for_shutdown_retry() {
        use std::os::windows::fs::OpenOptionsExt;
        use tauri::Manager;
        let dir = crate::download_tests::temp_dir("export-locked-partial");
        let app = tauri::test::mock_builder().build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let writer = crate::downloader::start_queue_writer(app.handle());
        app.manage(AppState::new(dir.clone(), Default::default(), Vec::new(), Vec::new(), writer, false));
        let state = app.state::<AppState>();
        let guard = RunGuard::begin(&state).unwrap();
        let partial = dir.join("locked.partial");
        fs::write(&partial, b"data").unwrap();
        assert!(register_partial(&state, &partial));
        let held = OpenOptions::new().read(true).share_mode(1).open(&partial).unwrap();
        drop(guard);
        assert_eq!(state.workshop.lock().unwrap().partial.as_deref(), Some(partial.as_path()));
        drop(held);
        wait_for_export_cleanup(&state);
        assert!(!partial.exists());
        assert!(state.workshop.lock().unwrap().partial.is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn wait_for_export_cleanup_bounds_a_stuck_export_and_still_removes_partial() {
        use tauri::Manager;
        let dir = crate::download_tests::temp_dir("export-cleanup-timeout");
        let app = tauri::test::mock_builder().build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let writer = crate::downloader::start_queue_writer(app.handle());
        app.manage(AppState::new(dir.clone(), Default::default(), Vec::new(), Vec::new(), writer, false));
        let state = app.state::<AppState>();
        let partial = dir.join("stuck.partial");
        fs::write(&partial, b"partial").unwrap();
        {
            let mut run = state.workshop.lock().unwrap();
            run.active = true;
            run.partial = Some(partial.clone());
        }
        let started = Instant::now();
        wait_for_export_cleanup(&state);
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_secs(5), "waited {elapsed:?}");
        assert!(elapsed < Duration::from_secs(8), "timed out too late: {elapsed:?}");
        assert!(!partial.exists());
        assert!(state.workshop.lock().unwrap().partial.is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn stale_recording_window_is_rechecked_and_cleared_or_rejected() {
        use tauri::Manager;
        let dir = crate::download_tests::temp_dir("export-stale-window");
        let app = tauri::test::mock_builder().build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let writer = crate::downloader::start_queue_writer(app.handle());
        app.manage(AppState::new(dir.clone(), Default::default(), Vec::new(), Vec::new(), writer, false));
        let state = app.state::<AppState>();
        let stale = (PathBuf::from(r"C:\no such folder\wallpaper64.exe"), "Claws stale window".to_string());
        state.workshop.lock().unwrap().window = Some(stale.clone());

        let Err(err) = RunGuard::begin_with(&state, |_, _| false) else { panic!("still-open window must reject") };
        assert!(err.to_string().contains("still open"), "{err}");
        {
            let run = state.workshop.lock().unwrap();
            assert!(!run.active);
            assert_eq!(run.window.as_ref(), Some(&stale));
        }

        let guard = RunGuard::begin(&state).expect("gone stale window is cleared and begin succeeds");
        assert!(state.workshop.lock().unwrap().active);
        assert!(state.workshop.lock().unwrap().window.is_none());
        drop(guard);
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_library_comparison_handles_case_and_extended_unc_paths() {
        assert!(path_within(Path::new(r"c:\steam\workshop"), Path::new(r"C:\Steam")));
        assert!(!path_within(Path::new(r"C:\Steam-old"), Path::new(r"C:\Steam")));
        assert!(path_within(Path::new(r"\\?\UNC\server\share\library\item"), Path::new(r"\\server\share\library")));
    }

    #[test]
    fn steam_library_destinations_are_rejected_before_creation() {
        let dir = crate::download_tests::temp_dir("destination-safety");
        let library = dir.join("library");
        let steamapps = library.join("steamapps");
        fs::create_dir_all(&steamapps).unwrap();
        let inside = steamapps.join("new").join("output");
        assert!(!destination_outside_libraries(&inside, std::slice::from_ref(&library)).unwrap());
        assert!(!inside.exists());
        let partial = steamapps.join(format!("{PARTIAL_PREFIX}{}{PARTIAL_SUFFIX}", uuid::Uuid::new_v4()));
        fs::write(&partial, b"keep").unwrap();
        sweep_partials_in_libraries(&steamapps, std::slice::from_ref(&library));
        assert!(partial.exists(), "startup sweep must leave Steam files alone");
        assert!(destination_outside_libraries(&dir.join("elsewhere"), std::slice::from_ref(&library)).unwrap());
        let offline = dir.join("offline-library");
        assert!(destination_outside_libraries(&dir.join("elsewhere"), std::slice::from_ref(&offline)).unwrap());
        assert!(!destination_outside_libraries(&offline.join("new"), &[offline]).unwrap());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn shutdown_and_open_order_are_serialized() {
        use std::sync::{Arc, Mutex, atomic::AtomicBool, mpsc};
        let run = Arc::new(Mutex::new(Run { active: true, ..Run::default() }));
        let shutdown = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (run2, shutdown2) = (run.clone(), shutdown.clone());
        let opener = std::thread::spawn(move || start_recording_window(&run2, &shutdown2, Path::new("engine"), "title", || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        }).unwrap());
        entered_rx.recv().unwrap();
        shutdown.store(true, Ordering::SeqCst);
        release_tx.send(()).unwrap();
        assert!(opener.join().unwrap());
        let mut current = run.lock().unwrap();
        current.cancelled = true;
        assert_eq!(current.window.as_ref().map(|w| w.1.as_str()), Some("title"));
        drop(current);
        assert!(!start_recording_window(&run, &shutdown, Path::new("engine"), "late", || panic!("must not launch")).unwrap());
    }

    #[test]
    fn exports_the_project_tree_without_replacing_an_existing_backup() {
        let dir = crate::download_tests::temp_dir("workshop-export");
        let project = dir.join("steamapps/workshop/content/431960/123");
        let output = dir.join("output");
        fs::create_dir_all(project.join("assets")).unwrap();
        fs::create_dir(&output).unwrap();
        fs::write(project.join("project.json"), r#"{"type":"web","file":"assets/index.html"}"#).unwrap();
        fs::write(project.join("assets/index.html"), b"<title>Fixture</title>").unwrap();
        fs::write(project.join("assets/video.mp4"), vec![7u8; 300_000]).unwrap();
        fs::write(output.join("Fixture (123).zip"), b"existing").unwrap();

        let project = crate::steam::workshop_folder(&dir, 123).unwrap();
        assert_eq!(project, fs::canonicalize(dir.join("steamapps/workshop/content/431960/123")).unwrap());
        let mut steps: Vec<(u64, u64)> = Vec::new();
        let mut finished = false;
        let name = save_project(&project, &output, &output.join("test.partial"), 123, "Fixture", |done, total| steps.push((done, total)), || finished = true, || false, move_no_replace).unwrap().unwrap();
        assert!(finished, "the finishing step is announced once every byte is written");
        assert_eq!(name, "Fixture (123) (1).zip");
        let total = steps.last().unwrap().1;
        assert_eq!(steps.first(), Some(&(0, total)));
        assert_eq!(steps.last(), Some(&(total, total)));
        assert!(steps.windows(2).all(|pair| pair[0].0 <= pair[1].0), "never backwards");
        assert_eq!(fs::read(output.join("Fixture (123).zip")).unwrap(), b"existing");
        {
            let mut zip = zip::ZipArchive::new(File::open(output.join(&name)).unwrap()).unwrap();
            let names: Vec<String> = zip.file_names().map(|name| name.unwrap().into_owned()).collect();
            assert_eq!(names, ["assets/", "assets/index.html", "assets/video.mp4", "project.json"]);
            assert_eq!(zip.by_name("project.json").unwrap().compression(), zip::CompressionMethod::Deflated);
            assert_eq!(zip.by_name("assets/video.mp4").unwrap().compression(), zip::CompressionMethod::Stored);
            let mut html = String::new();
            zip.by_name("assets/index.html").unwrap().read_to_string(&mut html).unwrap();
            assert_eq!(html, "<title>Fixture</title>");
        }
        assert_eq!(fs::read_dir(&output).unwrap().count(), 2);

        // Cancellation during ZIP finalisation must prevent publication.
        let cancel = std::cell::Cell::new(false);
        assert!(save_project(&project, &output, &output.join("test.partial"), 123, "Fixture", |_, _| {}, || cancel.set(true), || cancel.get(), move_no_replace).unwrap().is_none());
        assert_eq!(fs::read_dir(&output).unwrap().count(), 2);

        // Cancelled part-way: nothing new is left in the folder.
        assert!(save_project(&project, &output, &output.join("test.partial"), 123, "Fixture", |_, _| {}, || {}, || true, move_no_replace).unwrap().is_none());
        assert_eq!(fs::read_dir(&output).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn publishes_only_a_complete_media_file_and_never_replaces_one() {
        let dir = crate::download_tests::temp_dir("workshop-media-publish");
        let output = dir.join("output");
        fs::create_dir_all(&output).unwrap();
        let partial = dir.join(format!("{PARTIAL_PREFIX}{}{PARTIAL_SUFFIX}", uuid::Uuid::new_v4()));

        // Empty: refused, the partial removed, nothing published.
        fs::write(&partial, b"").unwrap();
        assert!(publish_media(&partial, &output, 5, "Loop", "gif", move_no_replace).is_err());
        assert!(!partial.exists());
        assert_eq!(fs::read_dir(&output).unwrap().count(), 0);

        // Cancellation during the final flush: the publish callback refuses the move.
        fs::write(&partial, b"video").unwrap();
        assert!(publish_media(&partial, &output, 5, "Loop", "mp4", |_, _| Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))).is_err());
        assert!(!partial.exists());
        assert_eq!(fs::read_dir(&output).unwrap().count(), 0);

        // Complete: the next free name, and the existing export stays untouched.
        fs::write(output.join("Loop (5).mp4"), b"existing").unwrap();
        fs::write(&partial, b"video").unwrap();
        let name = publish_media(&partial, &output, 5, "Loop", "mp4", move_no_replace).unwrap();
        assert_eq!(name, "Loop (5) (1).mp4");
        assert_eq!(fs::read(output.join("Loop (5).mp4")).unwrap(), b"existing");
        assert_eq!(fs::read(output.join(&name)).unwrap(), b"video");
        assert!(!partial.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    /// Live: needs ffmpeg. The real path end to end: the project's declared video found by
    /// `video_file` (a canonical `\\?\` path on Windows), converted into a partial file in the
    /// canonical download folder, then published beside an existing export without touching it.
    #[tokio::test]
    async fn converts_a_declared_video_through_canonical_paths_and_publishes_it() {
        let Some(tools) = crate::download_tests::ffmpeg_dir() else {
            eprintln!("skipped: needs ffmpeg");
            return;
        };
        let ffmpeg = tools.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
        let dir = crate::download_tests::temp_dir("workshop-media-e2e");
        let project = dir.join("project");
        fs::create_dir_all(project.join("media")).unwrap();
        let made = std::process::Command::new(&ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=r=10:d=1:s=48x64"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(project.join("media").join("clip.mp4"))
            .status()
            .unwrap();
        assert!(made.success());
        fs::write(project.join("project.json"), r#"{"type":"video","file":"media\\clip.mp4"}"#).unwrap();
        let output = dir.join("output");
        fs::create_dir_all(&output).unwrap();
        let output = output.canonicalize().unwrap();
        fs::write(output.join("Loop (9).mp4"), b"existing").unwrap();

        let video = workshop_media::video_file(&project).unwrap();
        for (format, expected) in [(MediaFormat::Mp4, "Loop (9) (1).mp4"), (MediaFormat::Gif, "Loop (9).gif")] {
            let partial = output.join(format!("{PARTIAL_PREFIX}{}{PARTIAL_SUFFIX}", uuid::Uuid::new_v4()));
            let done = workshop_media::convert(&ffmpeg, &video.path, &partial, &workshop_media::ConvertPlan::basic(format), |_| true, &|| false, &|_| {}).await;
            assert_eq!(done, Ok(workshop_media::Outcome::Done), "{format:?}");
            let name = publish_media(&partial, &output, 9, "Loop", format.extension(), move_no_replace).unwrap();
            assert_eq!(name, expected);
            assert!(fs::metadata(output.join(&name)).unwrap().len() > 0);
        }
        assert_eq!(fs::read(output.join("Loop (9).mp4")).unwrap(), b"existing");
        let leftovers = fs::read_dir(&output).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with(PARTIAL_PREFIX)).count();
        assert_eq!(leftovers, 0, "no partial file may remain");
        fs::remove_dir_all(dir).unwrap();
    }

    /// Live, read only on the Steam library: set `CLAWS_TEST_LIBRARY_VIDEO` to one or more Workshop
    /// item ids separated by `;`. For each, reports what Steam says about it, what ffprobe says
    /// about its video and, when the original can be copied, copies it into a temporary folder
    /// with the real progress code and prints how well the time left predicted the copy.
    #[tokio::test]
    async fn copies_real_library_videos_when_given() {
        let Some(ids) = std::env::var_os("CLAWS_TEST_LIBRARY_VIDEO") else { return };
        let Some(tools) = crate::download_tests::ffmpeg_dir() else { panic!("needs the managed ffprobe") };
        let ffprobe = tools.join(format!("ffprobe{}", std::env::consts::EXE_SUFFIX));
        let root = steam_root().expect("Steam is installed");
        let libraries = library_folders(&root);
        for id in ids.to_string_lossy().split(';').filter(|id| !id.is_empty()) {
            let id: u64 = id.parse().expect("an item id");
            let located = crate::steam::locate_item(&libraries, id, None);
            eprintln!("item {id}: {:?}", located.state);
            let project = located.project.expect("the item is in the library");
            let video = workshop_media::video_file(&project).expect("a video project");
            let probe = workshop_media::probe(&ffprobe, &video.path).await.expect("ffprobe reads it");
            eprintln!("  {} bytes, {}x{} {} {:.1} fps, {:?}s, audio {:?}, container {}", video.bytes, probe.width, probe.height, probe.video_codec, probe.fps, probe.duration, probe.audio_codec, probe.format_name);
            let copyable = probe.can_copy(&video.path);
            eprintln!("  copyable: {copyable}");
            if !copyable {
                continue;
            }
            let dir = crate::download_tests::temp_dir("live-copy");
            let partial = dir.join(format!("{PARTIAL_PREFIX}{}{PARTIAL_SUFFIX}", uuid::Uuid::new_v4()));
            let started = Instant::now();
            let mut samples: Vec<(Duration, u64)> = Vec::new();
            let source = video.path.clone();
            let target = partial.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                let clock = Instant::now();
                let result = workshop_media::copy_original(&source, &target, |done, _| samples.push((clock.elapsed(), done)), || {}, || false);
                (result, samples)
            })
            .await
            .unwrap();
            let (result, samples) = outcome;
            assert_eq!(result.unwrap(), workshop_media::CopyOutcome::Done);
            let total = started.elapsed();
            assert_eq!(fs::metadata(&partial).unwrap().len(), video.bytes);
            eprintln!("  copied in {:.2}s ({:.0} MB/s)", total.as_secs_f64(), video.bytes as f64 / total.as_secs_f64() / 1e6);
            // How well would the time left have predicted the copy? Replay the samples.
            let finish = samples.last().unwrap().0;
            for percent in [10u64, 25, 50, 75, 90] {
                let target_done = video.bytes * percent / 100;
                let Some(at) = samples.iter().position(|(_, done)| *done >= target_done) else { continue };
                let mut window = export_progress::RateWindow::new(Duration::from_secs(3));
                for (elapsed, done) in &samples[..=at] {
                    window.push(*elapsed, *done);
                }
                let (elapsed, done) = samples[at];
                let eta = export_progress::eta_from_rate(done, video.bytes, window.rate(), elapsed, Duration::from_secs(1));
                let actual = finish.saturating_sub(elapsed).as_secs_f64();
                eprintln!("  at {percent:>2}%: elapsed {:.2}s, predicted left {:?}, actual left {actual:.2}s", elapsed.as_secs_f64(), eta.map(|e| (e * 100.0).round() / 100.0));
            }
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn a_copy_is_estimated_from_the_size_and_a_conversion_only_from_a_measured_speed() {
        let copy = estimate_copy(5_856_547_590);
        assert!(copy.approximate);
        assert!((copy.seconds.unwrap() - 29.28).abs() < 0.01, "{:?}", copy.seconds);
        assert_eq!(copy.bytes, Some(5_856_547_590));

        // No speed measured yet: no estimate, rather than a made-up one.
        assert_eq!(estimate_conversion(Some(294.0), None, Some(5)).seconds, None);
        assert_eq!(estimate_conversion(None, Some(2.0), None).seconds, None, "no length either");
        assert_eq!(estimate_conversion(Some(300.0), Some(0.0), None).seconds, None);
        let known = estimate_conversion(Some(294.0), Some(2.0), Some(7));
        assert_eq!((known.seconds, known.approximate, known.bytes), (Some(147.0), true, Some(7)));

        // Unknown GIF duration: never invent an estimate from the 30/60 s cap.
        let unknown_gif_probe = workshop_media::Probe {
            video_codec: "h264".into(),
            pix_fmt: "yuv420p".into(),
            width: 1920,
            height: 1080,
            fps: 30.0,
            duration: None,
            audio_codec: None,
            format_name: "matroska,webm".into(),
            frames: None,
        };
        let speeds = std::collections::BTreeMap::from([("gif-1080".to_string(), 2.0)]);
        assert_eq!(estimate_probe_conversion(&unknown_gif_probe, MediaFormat::Gif, 5_000, Some(30), &speeds).seconds, None);
        assert_eq!(estimate_probe_conversion(&unknown_gif_probe, MediaFormat::Gif, 5_000, None, &speeds).seconds, None);
    }

    #[test]
    fn a_remembered_speed_belongs_to_a_format_and_a_size_bucket() {
        assert_eq!(speed_key(MediaFormat::Mp4, 480), "mp4-720");
        assert_eq!(speed_key(MediaFormat::Mp4, 720), "mp4-720");
        assert_eq!(speed_key(MediaFormat::Mp4, 1080), "mp4-1080");
        assert_eq!(speed_key(MediaFormat::Mp4, 1081), "mp4-1440");
        assert_eq!(speed_key(MediaFormat::Mp4, 2160), "mp4-2160");
        assert_eq!(speed_key(MediaFormat::Gif, 4320), "gif-2160");
    }

    #[test]
    fn the_export_event_and_outcome_describe_the_same_stages() {
        // Every stage the tracker can announce has a name the interface knows.
        for stage in [Stage::Checking, Stage::Opening, Stage::Loading, Stage::Recording, Stage::Finishing, Stage::Copying, Stage::Converting, Stage::Zipping] {
            assert_eq!(serde_json::to_value(stage).unwrap(), stage.name());
        }
    }

    /// Live, end to end: `CLAWS_TEST_EXPORT` is `id;format;length;height;fps` (the last three may be
    /// left out; several runs are separated by `|`). Runs the real export, with a lookup at Valve's
    /// public API, the Steam library, Wallpaper Engine and Claws' FFmpeg, into a temporary folder.
    /// Prints what the interface would have been told and checks the published file.
    #[tokio::test]
    async fn runs_real_exports_end_to_end_when_given() {
        use tauri::Manager;
        let Some(plan) = std::env::var_os("CLAWS_TEST_EXPORT") else { return };
        let tools = crate::download_tests::ffmpeg_dir().expect("needs Claws' ffmpeg and ffprobe");
        let dir = crate::download_tests::temp_dir("live-export");
        let out = dir.join("out");
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        for name in ["ffmpeg.exe", "ffprobe.exe"] {
            // A link, not a copy, and the originals stay where they are.
            let source = tools.join(name);
            if fs::hard_link(&source, bin.join(name)).is_err() {
                fs::copy(&source, bin.join(name)).unwrap();
            }
        }
        let app = tauri::test::mock_builder().build(tauri::test::mock_context(tauri::test::noop_assets())).expect("mock app");
        let settings = crate::settings::Settings { output_dir: out.to_string_lossy().into_owned(), ..Default::default() };
        let writer = crate::downloader::start_queue_writer(app.handle());
        app.manage(AppState::new(dir.clone(), settings, Vec::new(), Vec::new(), writer, false));
        let state = app.state::<AppState>();

        for run in plan.to_string_lossy().split('|').filter(|run| !run.is_empty()) {
            let mut parts = run.split(';');
            let id: u64 = parts.next().unwrap().parse().expect("an item id");
            let format = match parts.next().unwrap_or("mp4") {
                "zip" => ExportFormat::Zip,
                "gif" => ExportFormat::Gif,
                "png" => ExportFormat::Png,
                _ => ExportFormat::Mp4,
            };
            let number = |part: Option<&str>| part.and_then(|part| part.parse::<u32>().ok());
            let options = ExportOptions { length_seconds: number(parts.next()), height: number(parts.next()), fps: number(parts.next()) };
            let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<export_progress::ProgressEvent>::new()));
            let sink = events.clone();
            let tracker = Tracker::new("live", move |event| sink.lock().unwrap().push(event));
            let guard = RunGuard::begin(&state).expect("no other export is running");
            let started = Instant::now();
            let outcome = export(app.handle(), &state, &tracker, id, format, &options).await.expect("the export ends in an outcome");
            drop(guard);
            let took = started.elapsed();
            let events = events.lock().unwrap().clone();
            let stages = events.iter().map(|event| event.stage.name()).fold(Vec::<&str>::new(), |mut seen, stage| {
                if seen.last() != Some(&stage) {
                    seen.push(stage);
                }
                seen
            });
            match outcome {
                WallpaperOutcome::Saved { filename, path, elapsed_ms, stages: times, note } => {
                    let size = fs::metadata(&path).expect("the file was published").len();
                    eprintln!("export {run}: saved {filename} ({size} bytes) in {:.1}s (reported {:.1}s), stages {stages:?}, times {:?}, note {note:?}", took.as_secs_f64(), elapsed_ms as f64 / 1000.0, times.iter().map(|t| (t.name, t.ms)).collect::<Vec<_>>());
                    assert!(size > 0);
                    assert!(elapsed_ms as f64 / 1000.0 <= took.as_secs_f64() + 0.5);
                    assert_eq!(times.iter().map(|t| t.ms).sum::<u64>(), elapsed_ms);
                    let progress: Vec<f64> = events.iter().filter_map(|event| event.done).collect();
                    eprintln!("  {} events, {} with a count", events.len(), progress.len());
                    if std::env::var_os("CLAWS_TEST_SERIES").is_some() {
                        for event in events.iter().filter(|event| event.done.is_some()) {
                            eprintln!("series {} {} {}", event.elapsed_ms, event.done.unwrap_or(0.0), event.total.unwrap_or(0.0));
                        }
                    }
                    // How well did the time left predict the rest of the counted stage?
                    if let Some(counted) = events.iter().find(|event| event.done.is_some()).map(|event| event.stage) {
                        let end = events.iter().rev().find(|event| event.stage == counted).map(|event| event.elapsed_ms);
                        let after = events.iter().find(|event| matches!(event.stage, Stage::Finishing)).map(|event| event.elapsed_ms).or(end);
                        for share in [0.1, 0.25, 0.5, 0.75, 0.9] {
                            let at = events.iter().find(|event| event.stage == counted && event.done.zip(event.total).is_some_and(|(done, total)| total > 0.0 && done / total >= share));
                            if let (Some(event), Some(after)) = (at, after) {
                                let actual = after.saturating_sub(event.elapsed_ms) as f64 / 1000.0;
                                eprintln!("  at {:>3.0}%: elapsed {:.1}s, predicted left {:?}{}, actual left {actual:.1}s", share * 100.0, event.elapsed_ms as f64 / 1000.0, event.eta_ms.map(|eta| eta as f64 / 1000.0), if event.approximate { " (about)" } else { "" });
                            }
                        }
                    }
                }
                other => panic!("export {run} did not save: {other:?}"),
            }
        }
        let leftovers: Vec<_> = fs::read_dir(&out).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(PARTIAL_PREFIX)).collect();
        assert!(leftovers.is_empty(), "no partial file may remain: {leftovers:?}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sweeps_only_its_own_partial_files() {
        let dir = crate::download_tests::temp_dir("workshop-sweep");
        let ours = dir.join(format!("{PARTIAL_PREFIX}{}{PARTIAL_SUFFIX}", uuid::Uuid::new_v4()));
        let other = dir.join(".claws-wallpaper-notes.partial");
        let video = dir.join("video.mp4");
        for file in [&ours, &other, &video] {
            fs::write(file, b"x").unwrap();
        }
        sweep_partials_in_libraries(&dir, &[]);
        assert!(!ours.exists());
        assert!(other.exists() && video.exists());
        fs::remove_dir_all(dir).unwrap();
    }


    /// Live: set CLAWS_TEST_NETWORK=1 to ask Valve's API about a known wallpaper and collection.
    #[tokio::test]
    async fn reads_real_workshop_details_when_online() {
        if std::env::var_os("CLAWS_TEST_NETWORK").is_none() {
            return;
        }
        let client = http_client();
        let WorkshopLookup::Wallpaper { item } = lookup(&client, 2250845956).await.unwrap() else { panic!("wallpaper") };
        assert!(!item.title.is_empty() && item.file_size.is_some() && item.preview_url.is_some());
        assert!(matches!(lookup(&client, 3844862544).await.unwrap(), WorkshopLookup::Collection { items, .. } if items > 0));
    }


    #[cfg(windows)]
    #[tokio::test]
    async fn stops_a_long_running_process_tree() {
        let mut child = Command::new("cmd")
            .args(["/c", "ping", "-n", "60", "127.0.0.1"])
            .stdout(Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let started = Instant::now();
        kill_tree(pid, true);
        let status = timeout(Duration::from_secs(10), child.wait()).await.expect("stopped").unwrap();
        assert!(!status.success());
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn cancelling_during_preflight_probe_kills_the_child_promptly() {
        use tauri::Manager;
        let dir = crate::download_tests::temp_dir("preflight-cancel");
        let app = tauri::test::mock_builder().build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let writer = crate::downloader::start_queue_writer(app.handle());
        app.manage(AppState::new(dir.clone(), Default::default(), Vec::new(), Vec::new(), writer, false));
        let state = app.state::<AppState>();

        let gfx_probe = dir.join("fake-ffmpeg-gfx.cmd");
        let enc_probe = dir.join("fake-ffmpeg-enc.cmd");
        let media_probe = dir.join("fake-ffprobe.cmd");
        let video_file = dir.join("sample.mp4");
        fs::write(&video_file, b"not-a-real-mp4").unwrap();
        for script in [&gfx_probe, &enc_probe, &media_probe] {
            fs::write(script, "@echo off\r\nping -n 30 127.0.0.1 >nul\r\n").unwrap();
        }

        // 1. has_gfxcapture_tracked registers PID, gets killed promptly on cancel, and does not cache false.
        let _guard = RunGuard::begin(&state).unwrap();
        let app_for_task = app.handle().clone();
        let gfx_path = gfx_probe.clone();
        let task = tokio::spawn(async move {
            let state = app_for_task.state::<AppState>();
            workshop_record::has_gfxcapture_tracked(&gfx_path, &|pid| track_pid(&state, pid), &|| is_cancelled(&state)).await
        });
        let wait_pid = Instant::now();
        while state.workshop.lock().unwrap().pid.is_none() && wait_pid.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        assert!(state.workshop.lock().unwrap().pid.is_some(), "gfxcapture probe PID must be registered");
        let cancel_started = Instant::now();
        cancel_wallpaper_download(app.state());
        let res = timeout(Duration::from_secs(5), task).await.expect("gfxcapture probe stopped promptly").unwrap();
        assert_eq!(res, None);
        assert!(cancel_started.elapsed() < Duration::from_secs(5));
        assert!(state.workshop.lock().unwrap().pid.is_none());
        drop(_guard);

        // Replace with a fast script outputting gfxcapture: proves the cancelled run did not poison the cache.
        fs::write(&gfx_probe, "@echo off\r\necho gfxcapture\r\n").unwrap();
        assert!(workshop_record::has_gfxcapture(&gfx_probe).await);

        // 2. encoder::available_tracked registers PID, stops on cancel without waiting through all encoders.
        let _guard = RunGuard::begin(&state).unwrap();
        let app_for_task = app.handle().clone();
        let enc_path = enc_probe.clone();
        let task = tokio::spawn(async move {
            let state = app_for_task.state::<AppState>();
            encoder::available_tracked(&enc_path, &|pid| track_pid(&state, pid), &|| is_cancelled(&state)).await
        });
        let wait_pid = Instant::now();
        while state.workshop.lock().unwrap().pid.is_none() && wait_pid.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        assert!(state.workshop.lock().unwrap().pid.is_some(), "encoder probe PID must be registered");
        let cancel_started = Instant::now();
        cancel_wallpaper_download(app.state());
        let res = timeout(Duration::from_secs(5), task).await.expect("encoder probe stopped promptly").unwrap();
        assert_eq!(res, None);
        assert!(cancel_started.elapsed() < Duration::from_secs(5));
        assert!(state.workshop.lock().unwrap().pid.is_none());
        drop(_guard);

        // 3. probe_cached_tracked registers PID and kills child promptly on cancel.
        let _guard = RunGuard::begin(&state).unwrap();
        let app_for_task = app.handle().clone();
        let probe_path = media_probe.clone();
        let video = workshop_media::VideoProject { path: video_file, bytes: 14 };
        let task = tokio::spawn(async move {
            let state = app_for_task.state::<AppState>();
            workshop_media::probe_cached_tracked(&probe_path, &video, &|pid| track_pid(&state, pid)).await
        });
        let wait_pid = Instant::now();
        while state.workshop.lock().unwrap().pid.is_none() && wait_pid.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        assert!(state.workshop.lock().unwrap().pid.is_some(), "ffprobe PID must be registered");
        let cancel_started = Instant::now();
        cancel_wallpaper_download(app.state());
        let res = timeout(Duration::from_secs(5), task).await.expect("ffprobe stopped promptly").unwrap();
        assert_eq!(res, None);
        assert!(cancel_started.elapsed() < Duration::from_secs(5));
        assert!(state.workshop.lock().unwrap().pid.is_none());
        drop(_guard);

        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn shutdown_before_open_does_not_record_window_or_wait() {
        use tauri::Manager;
        let dir = crate::download_tests::temp_dir("shutdown-before-open");
        let project_json = dir.join("project.json");
        fs::write(&project_json, r#"{"type":"scene"}"#).unwrap();
        let app = tauri::test::mock_builder().build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
        let writer = crate::downloader::start_queue_writer(app.handle());
        app.manage(AppState::new(dir.clone(), Default::default(), Vec::new(), Vec::new(), writer, false));
        let state = app.state::<AppState>();
        let _guard = RunGuard::begin(&state).unwrap();

        let engine = workshop_record::Engine { exe: PathBuf::from(r"C:\no such folder\wallpaper64.exe") };
        let tracker = Tracker::new("test", |_| {});
        let partial = dir.join("recording.partial");
        let cancelled = || is_cancelled(&state);
        let on_spawn = |pid| track_pid(&state, pid);
        let low_space = || false;
        let announce = |window: Option<&str>| {
            state.workshop.lock().unwrap().window = window.map(|title| (engine.exe.clone(), title.to_string()));
        };
        let shutdown_elapsed = std::sync::Mutex::new(None);
        let request = workshop_record::RecordRequest {
            ffmpeg: Path::new("ffmpeg"),
            ffprobe: None,
            engine: &engine,
            project_json: &project_json,
            partial: &partial,
            capture: workshop_record::Capture { width: 1280, height: 720, fps: 30, kind: workshop_record::CaptureKind::Picture },
            pre_roll: 2,
            moves: false,
            tracker: &tracker,
            cancelled: &cancelled,
            on_spawn: &on_spawn,
            low_space: &low_space,
            announce: &announce,
            open: &|title, _args| {
                assert!(state.workshop.lock().unwrap().window.is_none(), "WindowGuard::new must not announce before open");
                let started = Instant::now();
                stop_for_shutdown(&state);
                close_recording_window(&state);
                *shutdown_elapsed.lock().unwrap() = Some(started.elapsed());
                start_recording_window(&state.workshop, &state.shutting_down, &engine.exe, title, || panic!("must not launch after shutdown"))
            },
        };
        let outcome = workshop_record::record(&request).await;
        assert_eq!(outcome, Ok(workshop_record::Recorded::Cancelled));
        let elapsed = shutdown_elapsed.lock().unwrap().expect("open hook ran");
        assert!(elapsed < Duration::from_secs(2), "close_recording_window waited too long: {elapsed:?}");
        assert!(state.workshop.lock().unwrap().window.is_none());
        drop(_guard);
        let _ = fs::remove_dir_all(dir);
    }
}

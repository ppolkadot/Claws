use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::Manager;

use crate::AppHandle;

use crate::bounded_io;
use crate::downloader::kill_tree;
use crate::error::{AppError, Result};
use crate::links;
use crate::state::AppState;
use crate::tools;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleLang {
    pub code: String,
    pub name: String,
    pub auto: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EntrySummary {
    /// The entry's own link, or the playlist's link when the site gave only an id; then
    /// `playlist_index` says which item to take.
    pub url: String,
    pub playlist_index: Option<u32>,
    pub title: String,
    pub duration: Option<f64>,
    pub thumbnail: Option<String>,
    pub uploader: Option<String>,
}

/// One picture size the source offers. Choices are made by the shorter side, because that is
/// what yt-dlp's `-S res:N` caps: a portrait 1080×1920 stream is a 1080 choice.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    pub short_side: u32,
    /// Absent when the site gives only a height; then `short_side` is that height.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// What the site says about the stream at this size, when it says anything.
    pub vcodec: Option<String>,
    pub dynamic_range: Option<String>,
    pub fps: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    /// "video" or "playlist".
    pub kind: String,
    pub url: String,
    pub title: String,
    pub uploader: Option<String>,
    pub thumbnail: Option<String>,
    pub duration: Option<f64>,
    pub site: Option<String>,
    /// The shorter sides of `resolutions`, largest first. Kept for saved links and older
    /// readers; `resolutions` has the detail.
    pub heights: Vec<u32>,
    /// Every picture size the source offers, one per shorter side, largest first. Empty in
    /// links saved before it existed (their `heights` still apply) and for flat playlists.
    #[serde(default)]
    pub resolutions: Vec<Resolution>,
    pub subtitles: Vec<SubtitleLang>,
    /// Caption tracks a transcript can be made from, human and automatic kept apart (the
    /// same language may be in both). `None` in links saved before transcripts existed;
    /// an empty list means the video has no captions.
    #[serde(default)]
    pub captions: Option<Vec<SubtitleLang>>,
    pub entries: Vec<EntrySummary>,
}

/// Accepts only plain http(s) links. Anything else (file:, javascript:, option-like
/// strings) is rejected before it can reach yt-dlp.
pub fn validate_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let parsed = url::Url::parse(trimmed)
        .map_err(|_| AppError::msg("That does not look like a link. Paste a full http(s) address."))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(AppError::msg("Only http and https links are supported."));
    }
    Ok(parsed.to_string())
}

fn str_field(value: &Value, key: &str) -> Option<String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn pick_thumbnail(value: &Value) -> Option<String> {
    let https = |s: &str| s.starts_with("https://");
    if let Some(url) = value["thumbnail"].as_str().filter(|s| https(s)) {
        return Some(url.to_string());
    }
    value["thumbnails"]
        .as_array()?
        .iter()
        .rev()
        .filter_map(|t| t["url"].as_str())
        .find(|u| https(u))
        .map(str::to_string)
}

/// Common manual-subtitle languages worth offering from the automatic captions,
/// which on some sites list over a hundred translations.
const AUTO_CAPTION_LANGS: &[&str] = &["en", "es", "pt", "fr", "de", "it", "ja", "ko", "zh-Hans", "ru"];

fn collect_subtitles(value: &Value) -> Vec<SubtitleLang> {
    let mut out = Vec::new();
    if let Some(map) = value["subtitles"].as_object() {
        for (code, tracks) in map {
            if code == "live_chat" {
                continue;
            }
            let name = tracks
                .get(0)
                .and_then(|t| t["name"].as_str())
                .unwrap_or(code)
                .to_string();
            out.push(SubtitleLang {
                code: code.clone(),
                name,
                auto: false,
            });
        }
    }
    if let Some(map) = value["automatic_captions"].as_object() {
        for code in AUTO_CAPTION_LANGS {
            if let Some(tracks) = map.get(*code) {
                if out.iter().any(|s| s.code == *code) {
                    continue;
                }
                let name = tracks
                    .get(0)
                    .and_then(|t| t["name"].as_str())
                    .unwrap_or(code)
                    .to_string();
                out.push(SubtitleLang {
                    code: (*code).to_string(),
                    name: format!("{name} (auto)"),
                    auto: true,
                });
            }
        }
    }
    out
}

/// Sites with at most this many automatic tracks (and no `-orig` track) have all of them
/// offered for transcripts. A longer list is YouTube's machine translations, which are not
/// a transcript of what was said.
const ALL_AUTO_CAPTIONS_UP_TO: usize = 12;

fn track_name(tracks: &Value, code: &str) -> String {
    tracks.get(0).and_then(|t| t["name"].as_str()).filter(|n| !n.is_empty()).unwrap_or(code).to_string()
}

/// The caption tracks a transcript can be made from: every human track, then the automatic
/// tracks that transcribe the speech itself. A language can be in both lists; `auto` keeps
/// them apart.
///
/// On YouTube the speech-recognition track is the one ending in `-orig`; every other
/// automatic code is a machine translation of it (or of the human captions, when the video
/// has no `-orig` track). So when a `-orig` track exists only it is offered; with none, a
/// short list (a site's own per-language tracks) is offered whole and a long one not at
/// all.
fn collect_captions(value: &Value) -> Vec<SubtitleLang> {
    let mut out = Vec::new();
    for (code, tracks) in value["subtitles"].as_object().into_iter().flatten() {
        if code != "live_chat" {
            out.push(SubtitleLang { code: code.clone(), name: track_name(tracks, code), auto: false });
        }
    }
    if let Some(map) = value["automatic_captions"].as_object() {
        let tracks = || map.iter().filter(|(code, _)| *code != "live_chat");
        let has_original = tracks().any(|(code, _)| code.ends_with("-orig"));
        // Only tracks a transcript could use count toward "a short list": the chat replay
        // entry is not one.
        let usable = tracks().count();
        let wanted = |code: &str| if has_original { code.ends_with("-orig") } else { usable <= ALL_AUTO_CAPTIONS_UP_TO };
        let mut automatic: Vec<(&String, &Value)> = tracks().filter(|(code, _)| wanted(code)).collect();
        // The common languages in their usual order, then the rest by code.
        let rank = |code: &str| AUTO_CAPTION_LANGS.iter().position(|c| *c == code).unwrap_or(AUTO_CAPTION_LANGS.len());
        automatic.sort_by_key(|(code, _)| rank(code));
        out.extend(automatic.into_iter().map(|(code, tracks)| SubtitleLang { code: code.clone(), name: track_name(tracks, code), auto: true }));
    }
    out
}

/// The smallest size offered as a choice.
const MIN_SHORT_SIDE: u32 = 144;

fn dimension(value: &Value) -> Option<u32> {
    value.as_u64().filter(|n| *n > 0).and_then(|n| u32::try_from(n).ok())
}

/// The picture sizes the formats offer, one per shorter side, largest first. A format counts
/// as video unless its codec is "none": yt-dlp's `bv*` takes a stream whose codec the site
/// does not name, so its size is offered too. A format with only a width is skipped, since
/// yt-dlp's `res` cannot place it either.
fn collect_resolutions(value: &Value) -> Vec<Resolution> {
    let mut by_side: BTreeMap<u32, Resolution> = BTreeMap::new();
    for format in value["formats"].as_array().into_iter().flatten() {
        if format["vcodec"].as_str() == Some("none") {
            continue;
        }
        let (width, height) = (dimension(&format["width"]), dimension(&format["height"]));
        let Some(height) = height else { continue };
        let short_side = width.map_or(height, |w| w.min(height));
        if short_side < MIN_SHORT_SIDE {
            continue;
        }
        let text = |key: &str| format[key].as_str().filter(|s| !s.is_empty() && *s != "none").map(str::to_string);
        let candidate = Resolution {
            short_side,
            width,
            height: Some(height),
            vcodec: text("vcodec"),
            dynamic_range: text("dynamic_range"),
            fps: format["fps"].as_f64().filter(|f| f.is_finite() && *f > 0.0),
        };
        match by_side.get_mut(&short_side) {
            Some(kept) if detail(&candidate) > detail(kept) => *kept = candidate,
            Some(_) => {}
            None => {
                by_side.insert(short_side, candidate);
            }
        }
    }
    by_side.into_values().rev().collect()
}

/// Which of two streams with the same shorter side describes it better: the one with both
/// dimensions, then the larger picture, then the one naming its codec, HDR and frame rate.
fn detail(r: &Resolution) -> (bool, u32, bool, bool, u64) {
    let long_side = r.width.unwrap_or(0).max(r.height.unwrap_or(0));
    let hdr = r.dynamic_range.as_deref().is_some_and(|d| d != "SDR");
    (r.width.is_some(), long_side, r.vcodec.is_some(), hdr, r.fps.map_or(0, |f| f.round() as u64))
}

fn parse(url: &str, value: &Value) -> ProbeResult {
    let is_playlist = value["_type"].as_str() == Some("playlist");
    let playlist_url = str_field(value, "webpage_url")
        .and_then(|u| validate_url(&u).ok())
        .unwrap_or_else(|| url.to_string());
    let entries = if is_playlist {
        value["entries"]
            .as_array()
            .map(|list| {
                list.iter()
                    .enumerate()
                    .filter(|(_, e)| e.is_object())
                    .map(|(position, e)| {
                        // Flat entries usually carry a link, but some extractors give only an
                        // id. Those are fetched by their position in the playlist instead.
                        let own_url = str_field(e, "webpage_url")
                            .and_then(|u| validate_url(&u).ok())
                            .or_else(|| str_field(e, "url").and_then(|u| validate_url(&u).ok()));
                        let (entry_url, playlist_index) = match own_url {
                            Some(u) => (u, None),
                            None => {
                                let index = e["playlist_index"]
                                    .as_u64()
                                    .filter(|i| *i > 0)
                                    .unwrap_or(position as u64 + 1);
                                (playlist_url.clone(), Some(index as u32))
                            }
                        };
                        EntrySummary {
                            url: entry_url,
                            playlist_index,
                            title: str_field(e, "title").unwrap_or_else(|| "Untitled".into()),
                            duration: e["duration"].as_f64(),
                            thumbnail: pick_thumbnail(e),
                            uploader: str_field(e, "uploader").or_else(|| str_field(e, "channel")),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let resolutions = collect_resolutions(value);
    ProbeResult {
        kind: if is_playlist { "playlist" } else { "video" }.into(),
        url: if is_playlist {
            playlist_url
        } else {
            str_field(value, "webpage_url")
                .and_then(|u| validate_url(&u).ok())
                .unwrap_or_else(|| url.to_string())
        },
        title: str_field(value, "title").unwrap_or_else(|| "Untitled".into()),
        uploader: str_field(value, "uploader")
            .or_else(|| str_field(value, "channel"))
            .or_else(|| str_field(value, "uploader_id")),
        thumbnail: pick_thumbnail(value),
        duration: value["duration"].as_f64(),
        site: str_field(value, "extractor_key").or_else(|| str_field(value, "extractor")),
        heights: resolutions.iter().map(|r| r.short_side).collect(),
        resolutions,
        subtitles: collect_subtitles(value),
        // A flat playlist's entries carry no captions; transcripts are made per video.
        captions: (!is_playlist).then(|| collect_captions(value)),
        entries,
    }
}

/// Removes a lookup from `AppState::probes` however `probe` returns.
struct ProbeSlot<'a> {
    state: &'a AppState,
    id: u64,
}

impl Drop for ProbeSlot<'_> {
    fn drop(&mut self) {
        self.state.probes.lock().unwrap().remove(&self.id);
    }
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(90);

pub async fn probe(app: &AppHandle, raw_url: &str) -> Result<ProbeResult> {
    let url = validate_url(raw_url)?;
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();
    // Waits while yt-dlp is being replaced, and keeps it from being replaced mid-probe.
    let _engine = state.engine.read().await;
    let ytdlp = tools::find_ytdlp(app)
        .ok_or_else(|| AppError::msg("yt-dlp is not installed yet."))?;

    let mut args = tools::common_args(app, &settings);
    args.extend(
        [
            "--dump-single-json",
            "--flat-playlist",
            "--skip-download",
            "--no-warnings",
            "--socket-timeout",
            "20",
            "--",
        ]
        .map(String::from),
    );
    args.push(url.clone());

    let stdout = run_lookup(app, &ytdlp, &args).await?;
    let value: Value = serde_json::from_slice(&stdout)
        .map_err(|_| AppError::msg("yt-dlp returned something that could not be read."))?;
    let result = parse(&url, &value);
    if result.kind == "video" {
        remember(app, &result.url, &stdout).await;
    }
    remember_recent(&state, &url, &result);
    // A link the user downloaded before keeps its saved details current.
    links::refresh(app, &[url.as_str(), result.url.as_str()], &result);
    Ok(result)
}

/// Runs one yt-dlp lookup (a probe or a size preview) and returns what it printed. The
/// process is registered under `AppState::probes`, so shutdown stops it; its output is
/// capped and it is stopped after PROBE_TIMEOUT. The caller holds the engine lock.
pub(crate) async fn run_lookup(app: &AppHandle, ytdlp: &Path, args: &[String]) -> Result<Vec<u8>> {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let state = app.state::<AppState>();

    let mut command = tools::command(ytdlp);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    // Registered under `probes` after checking the shutdown flag under the same lock.
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (mut child, pid) = {
        let mut probes = state.probes.lock().unwrap();
        if state.shutting_down.load(Ordering::SeqCst) {
            return Err(AppError::msg("Claws is closing."));
        }
        let child = command.spawn()?;
        let pid = child.id();
        if let Some(pid) = pid {
            probes.insert(id, pid);
        }
        (child, pid)
    };
    let _slot = ProbeSlot { state: &state, id };

    // The metadata is capped and only the end of the error output is kept, so neither an
    // enormous answer nor a noisy failure can grow memory without bound.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout = tauri::async_runtime::spawn(async move {
        match stdout {
            Some(stream) => bounded_io::read_capped(stream, bounded_io::MAX_METADATA).await,
            None => Ok(Vec::new()),
        }
    });
    let stderr = tauri::async_runtime::spawn(async move {
        match stderr {
            Some(stream) => bounded_io::read_tail(stream, bounded_io::DIAGNOSTIC_TAIL).await,
            None => Vec::new(),
        }
    });
    let finished = tokio::time::timeout(PROBE_TIMEOUT, async {
        // An oversized answer ends the read early; the process must then be stopped,
        // because nothing drains its output any more.
        let stdout = stdout.await.unwrap_or_else(|_| Ok(Vec::new()));
        if stdout.is_err() {
            return Err(());
        }
        Ok((stdout, child.wait().await))
    })
    .await;
    let stop = |child_pid: Option<u32>| async move {
        // Kill the tree first: dropping the child alone would orphan yt-dlp's interpreter.
        if let Some(pid) = child_pid {
            let _ = tokio::task::spawn_blocking(move || kill_tree(pid, true)).await;
        }
    };
    let (stdout, status) = match finished {
        Ok(Ok((stdout, status))) => (stdout.unwrap_or_default(), status?),
        Ok(Err(())) => {
            stop(pid).await;
            let _ = child.kill().await;
            return Err(AppError::msg(format!(
                "That link returned more information than Claws accepts ({} MB). For a very large playlist or channel, try a smaller part of it.",
                bounded_io::MAX_METADATA / (1024 * 1024)
            )));
        }
        Err(_) => {
            stop(pid).await;
            let _ = child.kill().await;
            return Err(AppError::msg("Timed out while reading that link."));
        }
    };
    let stderr = stderr.await.unwrap_or_default();

    if !status.success() {
        return Err(AppError::msg(error_text(&String::from_utf8_lossy(&stderr))));
    }
    Ok(stdout)
}

/// Kept briefly so `enqueue` can save them in the link cache without another lookup.
fn remember_recent(state: &AppState, url: &str, result: &ProbeResult) {
    const KEEP: usize = 16;
    let mut recent = state.recent_probes.lock().unwrap();
    recent.retain(|(key, _)| key != url && *key != result.url);
    recent.push((url.to_string(), result.clone()));
    if result.url != url {
        recent.push((result.url.clone(), result.clone()));
    }
    let excess = recent.len().saturating_sub(KEEP);
    recent.drain(..excess);
}

pub fn recent(state: &AppState, url: &str) -> Option<ProbeResult> {
    state
        .recent_probes
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|(key, _)| key == url)
        .map(|(_, result)| result.clone())
}

// The probe's JSON is reused by the download (--load-info-json) to skip a second
// extraction. Media links expire, so entries are short-lived and a failed run retries.

const CACHE_MAX_AGE: Duration = Duration::from_secs(20 * 60);
/// The cache's total size. A pasted list of links is looked up one after another and each
/// answer can be megabytes, so without a budget one batch could leave gigabytes behind.
/// Evicting an entry only costs that download a second extraction.
const CACHE_BUDGET: u64 = 256 * 1024 * 1024;

fn cache_path(dir: &Path, url: &str) -> PathBuf {
    let digest = Sha256::digest(url.as_bytes());
    let name: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    dir.join(format!("{name}.json"))
}

async fn remember(app: &AppHandle, url: &str, json: &[u8]) {
    // Also pruned here, after every write: a session can last days, entries are megabytes,
    // and a batch writes one entry per link. Listing a few hundred files is cheap next to
    // the yt-dlp run that produced the entry.
    let dir = app.state::<AppState>().probe_cache_dir();
    if tokio::fs::create_dir_all(&dir).await.is_ok() {
        let _ = tokio::fs::write(cache_path(&dir, url), json).await;
    }
    let _ = tokio::task::spawn_blocking(move || prune_cache(&dir)).await;
}

/// Cache files a download run is using. A prune or `forget` leaves them alone until the run
/// ends, so a lookup finishing elsewhere never deletes the file yt-dlp was just handed
/// (it opens it only after starting up, which can take seconds).
static LEASES: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

fn leases() -> std::sync::MutexGuard<'static, Vec<PathBuf>> {
    LEASES.lock().unwrap_or_else(|e| e.into_inner())
}

/// A cache entry held for one download run; it can be pruned again once this is dropped.
pub struct CachedInfo {
    path: PathBuf,
}

impl CachedInfo {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for CachedInfo {
    fn drop(&mut self) {
        let mut leases = leases();
        if let Some(i) = leases.iter().position(|p| *p == self.path) {
            leases.swap_remove(i);
        }
    }
}

/// Holds `path` if it is younger than `max_age`. It is leased before it is checked, so a
/// prune either sees the lease or has already removed the file (and the check fails).
fn lease_fresh(path: PathBuf, max_age: Duration) -> Option<CachedInfo> {
    leases().push(path.clone());
    let lease = CachedInfo { path };
    let age = std::fs::metadata(&lease.path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())?;
    (age < max_age).then_some(lease)
}

/// The cached info for `url`, if it is recent enough to still have working links. The file
/// is kept until the returned lease is dropped.
pub fn cached_info(app: &AppHandle, url: &str) -> Option<CachedInfo> {
    lease_fresh(cache_path(&app.state::<AppState>().probe_cache_dir(), url), CACHE_MAX_AGE)
}

pub fn forget(app: &AppHandle, url: &str) {
    remove_unleased(&cache_path(&app.state::<AppState>().probe_cache_dir(), url));
}

/// Removes a cache file unless a download run holds it. A file already gone counts as
/// removed (another prune got there first).
fn remove_unleased(path: &Path) -> bool {
    let leases = leases();
    if leases.iter().any(|p| p == path) {
        return false;
    }
    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

pub fn prune_cache(dir: &Path) {
    // One at a time: two prunes listing the folder together would each evict the whole
    // excess. One skipped here is made up by the next write's prune.
    static PRUNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    struct Running;
    impl Drop for Running {
        fn drop(&mut self) {
            PRUNING.store(false, Ordering::Release);
        }
    }
    if PRUNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let _running = Running;
    prune_cache_within(dir, CACHE_MAX_AGE, CACHE_BUDGET);
}

/// Removes entries older than `max_age`, then the oldest of the rest until they fit in
/// `budget` bytes. Entries a download run holds are skipped.
fn prune_cache_within(dir: &Path, max_age: Duration, budget: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut kept: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let meta = entry.metadata().ok();
        let modified = meta.as_ref().and_then(|m| m.modified().ok());
        let stale = modified
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_none_or(|age| age >= max_age);
        if stale {
            remove_unleased(&entry.path());
        } else if let (Some(meta), Some(modified)) = (meta, modified) {
            kept.push((modified, meta.len(), entry.path()));
        }
    }
    evict_oldest(kept, budget, remove_unleased);
}

/// Removes the oldest `entries` (modified, length, path) until the rest fit in `budget`.
/// `remove` says whether the file is gone afterwards.
fn evict_oldest(mut entries: Vec<(SystemTime, u64, PathBuf)>, budget: u64, mut remove: impl FnMut(&Path) -> bool) {
    let mut total: u64 = entries.iter().map(|(_, len, _)| *len).sum();
    if total <= budget {
        return;
    }
    entries.sort_by_key(|(modified, _, _)| *modified);
    for (_, len, path) in entries {
        if total <= budget {
            break;
        }
        if remove(&path) {
            total = total.saturating_sub(len);
        }
    }
}

pub fn error_text(stderr: &str) -> String {
    let errors: Vec<&str> = stderr
        .lines()
        .filter_map(|line| line.trim().strip_prefix("ERROR:"))
        .map(str::trim)
        .collect();
    if !errors.is_empty() {
        return errors.join("\n");
    }
    stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("yt-dlp failed without a message.")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn probe_cache_drops_stale_entries_then_the_oldest_beyond_its_budget() {
        let dir = std::env::temp_dir().join(format!("claws-probe-cache-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let now = SystemTime::now();
        let write = |name: &str, len: usize, age_secs: u64| {
            let path = dir.join(name);
            std::fs::write(&path, vec![b'x'; len]).unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(now - Duration::from_secs(age_secs)).unwrap();
        };
        write("stale.json", 10, 3_600);
        write("oldest.json", 400, 300);
        write("older.json", 400, 200);
        write("newer.json", 400, 100);
        write("newest.json", 400, 0);

        prune_cache_within(&dir, Duration::from_secs(20 * 60), 1_000);

        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["newer.json", "newest.json"], "stale first, then the oldest until 1000 bytes fit");

        // Within its budget, nothing fresh is touched.
        prune_cache_within(&dir, Duration::from_secs(20 * 60), 1_000);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cache_file_handed_to_a_download_survives_prunes_until_the_run_ends() {
        let dir = std::env::temp_dir().join(format!("claws-probe-lease-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let now = SystemTime::now();
        let write = |name: &str, age_secs: u64| {
            let path = dir.join(name);
            std::fs::write(&path, vec![b'x'; 400]).unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(now - Duration::from_secs(age_secs)).unwrap();
            path
        };
        let in_use = write("in-use.json", 300); // the oldest: first in line for eviction
        write("older.json", 200);
        write("newer.json", 100);
        let max_age = Duration::from_secs(20 * 60);

        // What run_prepared does: take the entry, then (later) yt-dlp opens it.
        let lease = lease_fresh(in_use.clone(), max_age).expect("fresh entry");
        // Lookups finishing meanwhile push the cache over its budget.
        prune_cache_within(&dir, max_age, 500);
        assert!(lease.path().exists(), "the file a download was handed is never evicted under it");
        assert!(!dir.join("older.json").exists() && !dir.join("newer.json").exists());
        // `forget` (a stale retry of the same link elsewhere) leaves it too.
        assert!(!remove_unleased(&in_use));
        assert!(in_use.exists());

        // Once the run ends it is an ordinary entry again.
        drop(lease);
        prune_cache_within(&dir, max_age, 0);
        assert!(!in_use.exists());
        // A lease on a file that is gone, or too old, is refused (and not left behind).
        assert!(lease_fresh(in_use.clone(), max_age).is_none());
        assert!(!leases().contains(&in_use));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn eviction_counts_a_file_another_prune_already_removed_as_gone() {
        // Two prunes listed the folder together; the other removed the oldest first. This
        // one must not then evict newer entries to make up for files that are already gone.
        let now = SystemTime::now();
        let at = |secs: u64| now - Duration::from_secs(secs);
        let entries = vec![
            (at(400), 400, PathBuf::from("a")),
            (at(300), 400, PathBuf::from("b")),
            (at(200), 400, PathBuf::from("c")),
            (at(100), 400, PathBuf::from("d")),
        ];
        let mut asked = Vec::new();
        // "a" is already gone: reported as removed, as remove_unleased does for NotFound.
        evict_oldest(entries.clone(), 1_000, |p| {
            asked.push(p.to_path_buf());
            true
        });
        assert_eq!(asked, [PathBuf::from("a"), PathBuf::from("b")], "1600 bytes over a 1000 budget: two go, not more");

        // A file that cannot be removed (held open) is skipped and the next one goes.
        let mut asked = Vec::new();
        evict_oldest(entries, 1_000, |p| {
            asked.push(p.to_path_buf());
            p != Path::new("a")
        });
        assert_eq!(asked, [PathBuf::from("a"), PathBuf::from("b"), PathBuf::from("c")]);
    }

    #[test]
    fn remove_unleased_reports_a_missing_file_as_removed() {
        let path = std::env::temp_dir().join(format!("claws-probe-missing-{}.json", uuid::Uuid::new_v4().simple()));
        assert!(remove_unleased(&path));
    }

    #[test]
    fn only_http_links_pass() {
        assert!(validate_url("https://example.com/v?id=1").is_ok());
        assert!(validate_url("  http://example.com  ").is_ok());
        for bad in ["file:///C:/x", "javascript:alert(1)", "--exec=calc", "example.com", "https://", ""] {
            assert!(validate_url(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn playlist_entries_without_links_use_their_index() {
        let value = json!({
            "_type": "playlist",
            "title": "List",
            "webpage_url": "https://example.com/list/1",
            "entries": [
                { "url": "https://example.com/v/a", "title": "A" },
                { "url": "abc123", "id": "abc123", "title": "B" },
                { "id": "xyz", "title": "C", "playlist_index": 7 },
                null
            ]
        });
        let result = parse("https://example.com/list/1", &value);
        assert_eq!(result.kind, "playlist");
        assert_eq!(result.entries.len(), 3);
        assert_eq!(result.entries[0].url, "https://example.com/v/a");
        assert_eq!(result.entries[0].playlist_index, None);
        assert_eq!(result.entries[1].url, "https://example.com/list/1");
        assert_eq!(result.entries[1].playlist_index, Some(2));
        assert_eq!(result.entries[2].playlist_index, Some(7));
    }

    #[test]
    fn heights_and_subtitles() {
        let value = json!({
            "title": "V",
            "formats": [
                { "vcodec": "avc1", "height": 1080 },
                { "vcodec": "avc1", "height": 720 },
                { "vcodec": "none", "height": 0 },
                { "vcodec": "avc1", "height": 90 }
            ],
            "subtitles": { "en": [{ "name": "English" }], "live_chat": [{}] },
            "automatic_captions": { "en": [{ "name": "English" }], "fr": [{ "name": "French" }], "xx": [{}] }
        });
        let result = parse("https://example.com/v", &value);
        assert_eq!(result.heights, vec![1080, 720]);
        let codes: Vec<_> = result.subtitles.iter().map(|s| (s.code.as_str(), s.auto)).collect();
        assert_eq!(codes, vec![("en", false), ("fr", true)]);
    }

    fn caption_kinds(result: &ProbeResult) -> Vec<(String, bool)> {
        result.captions.clone().unwrap_or_default().iter().map(|s| (s.code.clone(), s.auto)).collect()
    }

    #[test]
    fn captions_keep_human_and_automatic_tracks_apart() {
        let mut automatic = serde_json::Map::new();
        // A YouTube-like list: the original-language track, then many translations.
        automatic.insert("de-orig".into(), json!([{ "name": "German (Original)" }]));
        for code in ["af", "ar", "de", "en", "es", "fr", "ja", "ko", "pt", "ru", "zh-Hans", "zh-Hant", "zu"] {
            automatic.insert(code.into(), json!([{ "name": code }]));
        }
        let value = json!({
            "title": "V",
            "subtitles": { "en": [{ "name": "English" }], "de": [{ "name": "German" }], "live_chat": [{}] },
            "automatic_captions": automatic
        });
        let result = parse("https://example.com/v", &value);
        let kinds = caption_kinds(&result);
        let human: Vec<_> = kinds.iter().filter(|(_, auto)| !auto).map(|(c, _)| c.as_str()).collect();
        assert_eq!(human, ["de", "en"]);
        let automatic: Vec<_> = kinds.iter().filter(|(_, auto)| *auto).map(|(c, _)| c.as_str()).collect();
        assert_eq!(automatic, ["de-orig"], "only the speech-recognition track; the rest are machine translations");
        assert!(!kinds.iter().any(|(c, _)| c == "live_chat"), "{kinds:?}");
        assert!(kinds.iter().position(|(_, a)| *a) > kinds.iter().rposition(|(_, a)| !*a), "human tracks first");
        // The video-mode subtitle list is unchanged.
        assert!(result.subtitles.iter().all(|s| !(s.code == "en" && s.auto)));

        // A video with human captions and no `-orig` track: YouTube's automatic list is
        // translations of those captions, so none is offered.
        let mut translations = serde_json::Map::new();
        for code in ["af", "ar", "de", "en", "es", "fr", "ja", "ko", "pt", "ru", "zh-Hans", "zh-Hant", "zu"] {
            translations.insert(code.into(), json!([{ "name": code }]));
        }
        let translated = json!({ "subtitles": { "en": [{ "name": "English" }] }, "automatic_captions": translations });
        assert_eq!(caption_kinds(&parse("https://example.com/t", &translated)), [("en".to_string(), false)]);
        // A site with a few automatic tracks offers all of them.
        let few = json!({ "automatic_captions": { "nl": [{ "name": "Dutch" }], "sv": [{}] } });
        assert_eq!(caption_kinds(&parse("https://example.com/x", &few)), [("nl".to_string(), true), ("sv".to_string(), true)]);
        // No captions at all: an empty list, which the interface shows as unavailable.
        assert_eq!(parse("https://example.com/y", &json!({ "title": "Silent" })).captions.map(|c| c.len()), Some(0));
    }

    /// `count` automatic tracks named l0.. plus, optionally, YouTube's chat replay entry.
    fn automatic_tracks(count: usize, live_chat: bool) -> Value {
        let mut map = serde_json::Map::new();
        for i in 0..count {
            map.insert(format!("l{i}"), json!([{ "name": format!("Language {i}") }]));
        }
        if live_chat {
            map.insert("live_chat".into(), json!([{}]));
        }
        Value::Object(map)
    }

    #[test]
    fn the_twelve_track_threshold_counts_only_usable_tracks() {
        let auto_codes = |value: Value| -> Vec<String> {
            caption_kinds(&parse("https://example.com/v", &value)).into_iter().filter(|(_, auto)| *auto).map(|(code, _)| code).collect()
        };
        // Twelve real tracks and a chat replay entry: the chat is not a track, so all twelve stay.
        let twelve = auto_codes(json!({ "automatic_captions": automatic_tracks(12, true) }));
        assert_eq!(twelve.len(), 12, "{twelve:?}");
        assert!(!twelve.iter().any(|c| c == "live_chat"));
        // Thirteen real tracks are a long machine-translated list.
        assert!(auto_codes(json!({ "automatic_captions": automatic_tracks(13, false) })).is_empty());
        assert!(auto_codes(json!({ "automatic_captions": automatic_tracks(13, true) })).is_empty());
        // A -orig track decides it however long the list: only that track, chat or not.
        let mut with_orig = automatic_tracks(12, true);
        with_orig.as_object_mut().unwrap().insert("l3-orig".into(), json!([{ "name": "Original" }]));
        assert_eq!(auto_codes(json!({ "automatic_captions": with_orig })), ["l3-orig"]);
        let mut short_orig = automatic_tracks(2, false);
        short_orig.as_object_mut().unwrap().insert("fr-orig".into(), json!([{}]));
        assert_eq!(auto_codes(json!({ "automatic_captions": short_orig })), ["fr-orig"]);
        // Only a chat entry: nothing to offer, and the video has no captions.
        assert!(auto_codes(json!({ "automatic_captions": automatic_tracks(0, true) })).is_empty());
    }

    #[test]
    fn human_spanish_hides_automatic_spanish_and_automatic_english_stays() {
        // The picker's identity is the code: human Spanish is the one Spanish choice, and
        // English is offered automatic. (yt-dlp's own precedence is checked below when it is present.)
        let value = json!({
            "subtitles": { "es": [{ "name": "Spanish" }] },
            "automatic_captions": { "es": [{ "name": "Spanish" }], "en": [{ "name": "English" }] }
        });
        let result = parse("https://example.com/v", &value);
        let offered: Vec<_> = result.subtitles.iter().map(|s| (s.code.as_str(), s.auto)).collect();
        assert_eq!(offered, [("es", false), ("en", true)]);
        // Transcripts still keep the two Spanish kinds apart.
        let kinds = caption_kinds(&result);
        assert!(kinds.contains(&("es".to_string(), false)) && kinds.contains(&("es".to_string(), true)) && kinds.contains(&("en".to_string(), true)), "{kinds:?}");
    }

    /// Runs the real yt-dlp over a generated info file (no network: nothing is fetched, only
    /// the subtitle selection is printed). Needs `CLAWS_TEST_YTDLP` or a `yt-dlp` on PATH;
    /// without either it says so and passes, so the suite stays hermetic.
    #[test]
    fn yt_dlp_picks_human_spanish_once_beside_automatic_english() {
        let Some(ytdlp) = std::env::var_os("CLAWS_TEST_YTDLP").map(PathBuf::from).or_else(|| which::which("yt-dlp").ok()) else {
            eprintln!("SKIPPED: no yt-dlp (set CLAWS_TEST_YTDLP) - upstream subtitle precedence was not exercised");
            return;
        };
        let dir = std::env::temp_dir().join(format!("claws-subtitle-precedence-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let track = |name: &str| json!([{ "ext": "vtt", "url": format!("https://example.invalid/{name}.vtt"), "name": name }]);
        let info = json!({
            "id": "fixture", "title": "Fixture", "extractor": "fixture", "extractor_key": "Fixture",
            "webpage_url": "https://example.invalid/watch", "url": "https://example.invalid/video.mp4", "ext": "mp4",
            "subtitles": { "es": track("human-es") },
            "automatic_captions": { "es": track("auto-es"), "en": track("auto-en") }
        });
        let info_path = dir.join("info.json");
        std::fs::write(&info_path, info.to_string()).unwrap();
        let output = std::process::Command::new(&ytdlp)
            .current_dir(&dir)
            .args(["--ignore-config", "--no-cache-dir", "--skip-download", "--write-subs", "--write-auto-subs", "--sub-langs", "es,en", "--dump-json", "--load-info-json"])
            .arg(&info_path)
            .output()
            .expect("yt-dlp runs");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let printed: Value = serde_json::from_slice(&output.stdout).expect("yt-dlp prints its info");
        let chosen = printed["requested_subtitles"].as_object().expect("requested_subtitles");
        let urls: BTreeMap<&str, &str> = chosen.iter().map(|(code, t)| (code.as_str(), t["name"].as_str().unwrap_or(""))).collect();
        assert_eq!(urls, BTreeMap::from([("en", "auto-en"), ("es", "human-es")]), "one Spanish track, the human one");
    }

    #[test]
    fn a_probe_saved_before_captions_still_reads() {
        let saved = r#"{"kind":"video","url":"https://example.com/v","title":"V","uploader":null,"thumbnail":null,"duration":10.0,"site":"Example","heights":[1080],"resolutions":[],"subtitles":[{"code":"en","name":"English","auto":false}],"entries":[]}"#;
        let result: ProbeResult = serde_json::from_str(saved).unwrap();
        assert!(result.captions.is_none(), "unknown, so the interface falls back to the subtitle list");
    }

    fn video_format(id: &str, vcodec: &str, width: u32, height: u32) -> Value {
        json!({ "format_id": id, "vcodec": vcodec, "acodec": "none", "width": width, "height": height, "ext": "mp4" })
    }

    fn sides(result: &ProbeResult) -> Vec<(u32, Option<u32>, Option<u32>)> {
        result.resolutions.iter().map(|r| (r.short_side, r.width, r.height)).collect()
    }

    #[test]
    fn choices_reach_as_high_as_the_source_goes() {
        let value = json!({
            "title": "V",
            "formats": [
                { "format_id": "sb0", "vcodec": "none", "acodec": "none", "width": 320, "height": 180, "ext": "mhtml" },
                { "format_id": "a", "vcodec": "none", "acodec": "opus", "ext": "webm" },
                video_format("144", "avc1.4d400c", 256, 144),
                video_format("1080", "avc1.640028", 1920, 1080),
                { "format_id": "2160h", "vcodec": "vp09.02.51.10", "acodec": "none", "width": 3840, "height": 2160, "dynamic_range": "HDR10", "fps": 60.0 },
                video_format("2160", "vp09.00.51.08", 3840, 2160),
                video_format("2880", "vp09.00.61.08", 5120, 2880),
                video_format("3240", "vp09.00.61.08", 5760, 3240),
                { "format_id": "4320", "vcodec": "av01.0.17M.08", "acodec": "none", "width": 7680, "height": 4320, "fps": 30.0, "dynamic_range": "SDR" },
                video_format("tiny", "avc1", 160, 90)
            ]
        });
        let result = parse("https://example.com/v", &value);
        assert_eq!(result.heights, vec![4320, 3240, 2880, 2160, 1080, 144], "no 2160 ceiling and no fixed ladder");
        assert_eq!(
            sides(&result),
            vec![
                (4320, Some(7680), Some(4320)),
                (3240, Some(5760), Some(3240)),
                (2880, Some(5120), Some(2880)),
                (2160, Some(3840), Some(2160)),
                (1080, Some(1920), Some(1080)),
                (144, Some(256), Some(144)),
            ]
        );
        let eight_k = &result.resolutions[0];
        assert_eq!(eight_k.vcodec.as_deref(), Some("av01.0.17M.08"));
        assert_eq!(eight_k.fps, Some(30.0));
        // Two 2160 streams: the one that says more (HDR, frame rate) describes the size.
        assert_eq!(result.resolutions[3].dynamic_range.as_deref(), Some("HDR10"));
        assert_eq!(result.resolutions[3].fps, Some(60.0));
    }

    #[test]
    fn portrait_video_is_chosen_by_its_shorter_side() {
        let value = json!({
            "title": "Portrait",
            "formats": [video_format("p1920", "avc1", 1080, 1920), video_format("p1280", "avc1", 720, 1280), video_format("p3840", "vp9", 2160, 3840)]
        });
        let result = parse("https://example.com/p", &value);
        // yt-dlp's `res:1080` takes the 1080×1920 stream; a "1920" cap would ask for 4K.
        assert_eq!(result.heights, vec![2160, 1080, 720]);
        assert_eq!(sides(&result), vec![(2160, Some(2160), Some(3840)), (1080, Some(1080), Some(1920)), (720, Some(720), Some(1280))]);
    }

    #[test]
    fn streams_without_codec_fields_still_offer_their_size() {
        // yt-dlp leaves a missing codec missing, and `bv*` treats such a stream as video.
        let value = json!({
            "title": "Generic",
            "formats": [
                { "format_id": "hls-2160", "width": 3840, "height": 2160, "ext": "mp4" },
                { "format_id": "hls-1440", "height": 1440, "ext": "mp4" },
                { "format_id": "hls-wide", "width": 7680, "ext": "mp4" },
                { "format_id": "audio", "acodec": "mp4a.40.2", "vcodec": "none", "ext": "m4a" },
                { "format_id": "unknown", "ext": "mp4" }
            ]
        });
        let result = parse("https://example.com/g", &value);
        assert_eq!(sides(&result), vec![(2160, Some(3840), Some(2160)), (1440, None, Some(1440))], "a width alone says nothing about res");
        assert_eq!(result.resolutions[0].vcodec, None);
        // The same size given once with and once without its width keeps the width.
        let value = json!({ "formats": [{ "format_id": "a", "height": 2160 }, { "format_id": "b", "width": 4096, "height": 2160, "vcodec": "hvc1" }] });
        assert_eq!(sides(&parse("https://example.com/x", &value)), vec![(2160, Some(4096), Some(2160))]);
    }

    #[test]
    fn a_probe_saved_before_resolutions_still_reads() {
        let saved = r#"{"kind":"video","url":"https://example.com/v","title":"V","uploader":null,"thumbnail":null,"duration":10.0,"site":"Example","heights":[2160,1080],"subtitles":[],"entries":[]}"#;
        let result: ProbeResult = serde_json::from_str(saved).unwrap();
        assert!(result.resolutions.is_empty());
        assert_eq!(result.heights, vec![2160, 1080], "the old heights still load; the UI offers them only as caps");
        // Its own shape round-trips with the new field.
        let again: ProbeResult = serde_json::from_str(&serde_json::to_string(&result).unwrap()).unwrap();
        assert_eq!(again.heights, result.heights);
    }

    #[test]
    fn error_text_prefers_error_lines() {
        assert_eq!(error_text("noise
ERROR: [x] Private video
more"), "[x] Private video");
        assert_eq!(error_text("one
 last line 
"), "last line");
        assert_eq!(error_text(""), "yt-dlp failed without a message.");
    }
}

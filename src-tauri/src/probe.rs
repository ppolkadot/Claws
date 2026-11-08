use std::collections::BTreeSet;
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
    /// Available video heights, tallest first.
    pub heights: Vec<u32>,
    pub subtitles: Vec<SubtitleLang>,
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

fn collect_heights(value: &Value) -> Vec<u32> {
    let mut heights = BTreeSet::new();
    if let Some(formats) = value["formats"].as_array() {
        for format in formats {
            let has_video = format["vcodec"].as_str().is_some_and(|c| c != "none");
            if let (true, Some(h)) = (has_video, format["height"].as_u64()) {
                if h >= 144 {
                    heights.insert(h as u32);
                }
            }
        }
    }
    heights.into_iter().rev().collect()
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
        heights: collect_heights(value),
        subtitles: collect_subtitles(value),
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
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

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

    let mut command = tools::command(&ytdlp);
    command
        .args(&args)
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

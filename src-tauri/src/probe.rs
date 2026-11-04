use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

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

async fn read_all<R: tokio::io::AsyncRead + Unpin>(stream: Option<R>) -> Vec<u8> {
    let mut out = Vec::new();
    if let Some(mut stream) = stream {
        let _ = stream.read_to_end(&mut out).await;
    }
    out
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

    let stdout = tauri::async_runtime::spawn(read_all(child.stdout.take()));
    let stderr = tauri::async_runtime::spawn(read_all(child.stderr.take()));
    let status = match tokio::time::timeout(PROBE_TIMEOUT, child.wait()).await {
        Ok(status) => status?,
        Err(_) => {
            // Kill the tree first: dropping the child alone would orphan yt-dlp's interpreter.
            if let Some(pid) = pid {
                let _ = tokio::task::spawn_blocking(move || kill_tree(pid, true)).await;
            }
            let _ = child.kill().await;
            return Err(AppError::msg("Timed out while reading that link."));
        }
    };
    let stdout = stdout.await.unwrap_or_default();
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

fn cache_path(dir: &Path, url: &str) -> PathBuf {
    let digest = Sha256::digest(url.as_bytes());
    let name: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    dir.join(format!("{name}.json"))
}

async fn remember(app: &AppHandle, url: &str, json: &[u8]) {
    // Also pruned here (every few minutes): a session can last days, and entries are
    // megabytes.
    static LAST_PRUNE: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
    let dir = app.state::<AppState>().probe_cache_dir();
    if tokio::fs::create_dir_all(&dir).await.is_ok() {
        let _ = tokio::fs::write(cache_path(&dir, url), json).await;
    }
    let due = {
        let mut last = LAST_PRUNE.lock().unwrap();
        let due = last.is_none_or(|t| t.elapsed() >= Duration::from_secs(5 * 60));
        if due {
            *last = Some(std::time::Instant::now());
        }
        due
    };
    if due {
        let _ = tokio::task::spawn_blocking(move || prune_cache(&dir)).await;
    }
}

/// The cached info for `url`, if it is recent enough to still have working links.
pub fn cached_info(app: &AppHandle, url: &str) -> Option<PathBuf> {
    let path = cache_path(&app.state::<AppState>().probe_cache_dir(), url);
    let age = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())?;
    (age < CACHE_MAX_AGE).then_some(path)
}

pub fn forget(app: &AppHandle, url: &str) {
    let _ = std::fs::remove_file(cache_path(&app.state::<AppState>().probe_cache_dir(), url));
}

pub fn prune_cache(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_none_or(|age| age >= CACHE_MAX_AGE);
        if stale {
            let _ = std::fs::remove_file(entry.path());
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

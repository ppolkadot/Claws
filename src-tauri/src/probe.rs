use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::error::{AppError, Result};
use crate::state::AppState;
use crate::tools;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleLang {
    pub code: String,
    pub name: String,
    pub auto: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EntrySummary {
    pub url: String,
    pub title: String,
    pub duration: Option<f64>,
    pub thumbnail: Option<String>,
    pub uploader: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
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
    let entries = if is_playlist {
        value["entries"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|e| {
                        let entry_url = str_field(e, "webpage_url").or_else(|| str_field(e, "url"))?;
                        // Flat playlist entries can carry non-link ids for some extractors.
                        let entry_url = validate_url(&entry_url).ok()?;
                        Some(EntrySummary {
                            url: entry_url,
                            title: str_field(e, "title").unwrap_or_else(|| "Untitled".into()),
                            duration: e["duration"].as_f64(),
                            thumbnail: pick_thumbnail(e),
                            uploader: str_field(e, "uploader").or_else(|| str_field(e, "channel")),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    ProbeResult {
        kind: if is_playlist { "playlist" } else { "video" }.into(),
        url: str_field(value, "webpage_url").unwrap_or_else(|| url.to_string()),
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

pub async fn probe(app: &AppHandle, raw_url: &str) -> Result<ProbeResult> {
    let url = validate_url(raw_url)?;
    let ytdlp = tools::find_ytdlp(app)
        .ok_or_else(|| AppError::msg("yt-dlp is not installed yet."))?;
    let settings = app.state::<AppState>().settings.lock().unwrap().clone();

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

    let output = tokio::time::timeout(
        Duration::from_secs(90),
        tools::command(&ytdlp).args(&args).kill_on_drop(true).output(),
    )
    .await
    .map_err(|_| AppError::msg("Timed out while reading that link."))??;

    if !output.status.success() {
        return Err(AppError::msg(error_text(&String::from_utf8_lossy(&output.stderr))));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| AppError::msg("yt-dlp returned something that could not be read."))?;
    let result = parse(&url, &value);
    if result.kind == "video" {
        remember(app, &result.url, &output.stdout).await;
    }
    Ok(result)
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
    let dir = app.state::<AppState>().probe_cache_dir();
    if tokio::fs::create_dir_all(&dir).await.is_ok() {
        let _ = tokio::fs::write(cache_path(&dir, url), json).await;
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

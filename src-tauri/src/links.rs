//! The link cache: the details of every downloaded link, so looking it up again is
//! instant. Unlike the 20-minute probe cache in `probe.rs` (yt-dlp's full info, whose
//! media URLs expire), these are kept for a long time.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::Manager;

use crate::AppHandle;

use crate::probe::{self, validate_url, ProbeResult};
use crate::state::AppState;
use crate::store;

const MAX_ENTRIES: usize = 2000;
const MAX_AGE: Duration = Duration::from_secs(365 * 24 * 60 * 60);

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CachedLink {
    /// When these details were last read from the site, in milliseconds.
    pub saved_at: i64,
    pub result: ProbeResult,
}

fn dir(app: &AppHandle) -> PathBuf {
    cache_dir(&app.state::<AppState>().data_dir)
}

/// One file per normalised link, named by its hash, so no remote text reaches a path.
fn path_for(dir: &Path, url: &str) -> PathBuf {
    let digest = Sha256::digest(url.as_bytes());
    let name: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    dir.join(format!("{name}.json"))
}

fn read(path: &Path) -> Option<CachedLink> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(entry) => Some(entry),
        Err(_) => {
            // Written by an older version or damaged: a fresh lookup replaces it.
            let _ = std::fs::remove_file(path);
            None
        }
    }
}

fn write(dir: &Path, keys: &[&str], result: &ProbeResult) {
    let entry = CachedLink {
        saved_at: chrono::Utc::now().timestamp_millis(),
        result: result.clone(),
    };
    let mut written: Vec<String> = Vec::new();
    for key in keys {
        let Ok(key) = validate_url(key) else { continue };
        if written.contains(&key) {
            continue;
        }
        // Best effort: a failed write only means the next lookup is not instant.
        let _ = store::save_snapshot(&path_for(dir, &key), || entry.clone());
        written.push(key);
    }
}

/// The saved details for `url`, if this link was downloaded before.
pub fn lookup(app: &AppHandle, url: &str) -> Option<CachedLink> {
    let url = validate_url(url).ok()?;
    read(&path_for(&dir(app), &url))
}

/// Saves the details a download was started from: from the recent lookup, or from this
/// cache when they were shown from it and not refreshed yet.
pub fn remember_download(app: &AppHandle, source: &str) {
    let Ok(source) = validate_url(source) else { return };
    let state = app.state::<AppState>();
    let result = probe::recent(&state, &source).or_else(|| lookup(app, &source).map(|c| c.result));
    if let Some(result) = result {
        write(&dir(app), &[source.as_str(), result.url.as_str()], &result);
    }
}

/// Updates the details of a link downloaded before; links only looked up are not saved.
pub fn refresh(app: &AppHandle, keys: &[&str], result: &ProbeResult) {
    let dir = dir(app);
    let known = keys
        .iter()
        .filter_map(|k| validate_url(k).ok())
        .any(|k| path_for(&dir, &k).is_file());
    if known {
        write(&dir, keys, result);
    }
}

/// Drops entries older than a year, then the oldest beyond `MAX_ENTRIES`.
pub fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    let mut files: Vec<(SystemTime, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let modified = entry.metadata().and_then(|m| m.modified()).unwrap_or(now);
        if now.duration_since(modified).unwrap_or_default() > MAX_AGE {
            let _ = std::fs::remove_file(&path);
        } else {
            files.push((modified, path));
        }
    }
    if files.len() > MAX_ENTRIES {
        files.sort_by_key(|f| std::cmp::Reverse(f.0));
        for (_, path) in files.drain(MAX_ENTRIES..) {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("cache").join("links")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(url: &str) -> ProbeResult {
        serde_json::from_value(serde_json::json!({
            "kind": "video", "url": url, "title": "T", "uploader": null, "thumbnail": null,
            "duration": 10.0, "site": "Example", "heights": [1080, 720],
            "subtitles": [], "entries": []
        }))
        .unwrap()
    }

    #[test]
    fn saves_under_every_key_and_reads_back() {
        let dir = std::env::temp_dir().join(format!("claws-links-{}", uuid::Uuid::new_v4().simple()));
        let result = sample("https://example.com/watch?v=1");
        write(&dir, &["https://youtu.be/1", "https://example.com/watch?v=1", "not a link"], &result);
        let a = read(&path_for(&dir, &validate_url("https://youtu.be/1").unwrap())).unwrap();
        let b = read(&path_for(&dir, &validate_url("https://example.com/watch?v=1").unwrap())).unwrap();
        assert_eq!(a.result.heights, vec![1080, 720]);
        assert_eq!(b.result.title, "T");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_entry_saved_by_1_0_5_still_loads() {
        // The exact shape 1.0.5 wrote, playlist entries included. A field added to
        // ProbeResult later must be optional, or every saved link is discarded on read.
        let dir = std::env::temp_dir().join(format!("claws-links-legacy-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = path_for(&dir, "https://example.com/list");
        let saved = r#"{"savedAt":1762500000000,"result":{"kind":"playlist","url":"https://example.com/list","title":"L","uploader":"U","thumbnail":null,"duration":null,"site":"Example","heights":[],"subtitles":[{"code":"en","name":"English","auto":false}],"entries":[{"url":"https://example.com/list","playlistIndex":2,"title":"B","duration":61.0,"thumbnail":null,"uploader":null}]}}"#;
        std::fs::write(&path, saved).unwrap();
        let entry = read(&path).expect("an old entry loads");
        assert!(path.exists(), "and is not discarded");
        assert_eq!(entry.result.entries[0].playlist_index, Some(2));
        assert_eq!(entry.result.subtitles[0].code, "en");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_video_saved_before_resolutions_existed_keeps_its_heights() {
        let dir = std::env::temp_dir().join(format!("claws-links-heights-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = path_for(&dir, "https://example.com/v");
        let saved = r#"{"savedAt":1762500000000,"result":{"kind":"video","url":"https://example.com/v","title":"V","uploader":null,"thumbnail":null,"duration":12.0,"site":"Example","heights":[2160,1440,1080],"subtitles":[],"entries":[]}}"#;
        std::fs::write(&path, saved).unwrap();
        let entry = read(&path).expect("an entry without resolutions loads");
        assert!(path.exists());
        assert!(entry.result.resolutions.is_empty());
        assert_eq!(entry.result.heights, vec![2160, 1440, 1080]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn damaged_entry_is_dropped() {
        let dir = std::env::temp_dir().join(format!("claws-links-bad-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = path_for(&dir, "https://example.com/");
        std::fs::write(&path, "{ nope").unwrap();
        assert!(read(&path).is_none());
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn prune_keeps_the_newest() {
        let dir = std::env::temp_dir().join(format!("claws-links-prune-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..(MAX_ENTRIES + 5) {
            std::fs::write(dir.join(format!("{i:05}.json")), "{}").unwrap();
        }
        prune(&dir);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), MAX_ENTRIES);
        let _ = std::fs::remove_dir_all(dir);
    }
}

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::error::{AppError, Result};
use crate::settings::Settings;
use crate::state::AppState;

// The folder build starts about 3x faster: the single-file exe unpacks itself on
// every launch.
const YTDLP_ZIP_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_win.zip";
const YTDLP_ZIP_NAME: &str = "yt-dlp_win.zip";
const YTDLP_SUMS_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS";
const YTDLP_LATEST_API: &str = "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest";
const FFMPEG_URL: &str = "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-win64-gpl.zip";
const FFMPEG_SUMS_URL: &str = "https://github.com/yt-dlp/FFmpeg-Builds/releases/latest/download/checksums.sha256";
const FFMPEG_ZIP_NAME: &str = "ffmpeg-master-latest-win64-gpl.zip";

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub path: String,
    pub version: String,
    /// True when the binary lives in the app's own folder (so the app can update it).
    pub managed: bool,
    /// yt-dlp only: the folder build, which starts roughly three times faster.
    pub fast_start: bool,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToolsStatus {
    pub ytdlp: Option<ToolInfo>,
    pub ffmpeg: Option<ToolInfo>,
    pub latest_ytdlp: Option<String>,
    /// "Deno" or "Node.js": the JavaScript runtime yt-dlp uses for sites such as YouTube.
    pub js_runtime: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ToolProgress {
    tool: &'static str,
    received: u64,
    total: u64,
    stage: &'static str,
}

/// A command that never flashes a console window.
pub fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(std::process::Stdio::null());
    // Keep yt-dlp's piped output in UTF-8 instead of the console code page.
    cmd.env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1");
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

pub fn find_ytdlp(app: &AppHandle) -> Option<PathBuf> {
    let bin = app.state::<AppState>().bin_dir();
    // Folder build first, then the single-file build older versions installed.
    for managed in [bin.join("yt-dlp").join("yt-dlp.exe"), bin.join("yt-dlp.exe")] {
        if managed.is_file() {
            return Some(managed);
        }
    }
    which::which("yt-dlp").ok()
}

/// PATH does not change while the app runs, so each lookup is done once.
fn on_path(name: &'static str) -> Option<&'static PathBuf> {
    static FFMPEG: OnceLock<Option<PathBuf>> = OnceLock::new();
    static DENO: OnceLock<Option<PathBuf>> = OnceLock::new();
    static NODE: OnceLock<Option<PathBuf>> = OnceLock::new();
    let cell = match name {
        "ffmpeg" => &FFMPEG,
        "deno" => &DENO,
        _ => &NODE,
    };
    cell.get_or_init(|| which::which(name).ok()).as_ref()
}

pub fn find_ffmpeg(app: &AppHandle) -> Option<PathBuf> {
    let managed = app.state::<AppState>().bin_dir().join("ffmpeg.exe");
    if managed.is_file() {
        return Some(managed);
    }
    on_path("ffmpeg").cloned()
}

pub fn common_args(app: &AppHandle, settings: &Settings) -> Vec<String> {
    let mut args: Vec<String> = vec!["--no-colors".into(), "--ignore-config".into()];

    match settings.cookie_source.kind.as_str() {
        "browser" => {
            args.push("--cookies-from-browser".into());
            args.push(settings.cookie_source.value.clone());
        }
        "file" => {
            args.push("--cookies".into());
            args.push(settings.cookie_source.value.clone());
        }
        _ => {}
    }

    if let Some(ffmpeg) = find_ffmpeg(app) {
        if let Some(dir) = ffmpeg.parent() {
            args.push("--ffmpeg-location".into());
            args.push(dir.to_string_lossy().into_owned());
        }
    }

    // Recent YouTube extraction needs a JavaScript runtime. yt-dlp finds Deno on its
    // own; point it at Node when that is all there is.
    if on_path("deno").is_none() && on_path("node").is_some() {
        args.push("--js-runtimes".into());
        args.push("node".into());
    }
    args
}

async fn run_version(program: &Path, arg: &str) -> Option<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        command(program).arg(arg).kill_on_drop(true).output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().next().map(|line| line.trim().to_string())
}

fn is_managed(app: &AppHandle, path: &Path) -> bool {
    path.starts_with(app.state::<AppState>().bin_dir())
}

pub async fn status(app: &AppHandle) -> ToolsStatus {
    // Never run a tool while it is being replaced: the running exe would block the swap,
    // and a half-swapped folder reads as "not installed".
    let state = app.state::<AppState>();
    let _engine = state.engine.read().await;
    let (ytdlp, ffmpeg) = tokio::join!(ytdlp_info(app), ffmpeg_info(app));
    ToolsStatus {
        ytdlp,
        ffmpeg,
        latest_ytdlp: None,
        js_runtime: js_runtime().map(str::to_string),
    }
}

/// The runtime `common_args` makes yt-dlp use: Deno when present, else Node.js.
pub fn js_runtime() -> Option<&'static str> {
    if on_path("deno").is_some() {
        Some("Deno")
    } else if on_path("node").is_some() {
        Some("Node.js")
    } else {
        None
    }
}

/// Exclusive use of the engine for an install. Fails at once (rather than waiting for
/// downloads that may run for an hour) when anything is using yt-dlp, and also when
/// another install is already in progress.
fn reserve_engine(state: &AppState) -> Result<tokio::sync::RwLockWriteGuard<'_, ()>> {
    state.engine.try_write().map_err(|_| {
        AppError::msg(
            "yt-dlp is in use right now (a download, a link lookup or another update). \
             Pause or finish the downloads, then try again.",
        )
    })
}

async fn ytdlp_info(app: &AppHandle) -> Option<ToolInfo> {
    let path = find_ytdlp(app)?;
    run_version(&path, "--version").await.map(|version| ToolInfo {
        managed: is_managed(app, &path),
        fast_start: path.parent().and_then(|p| p.file_name()).is_some_and(|n| n == "yt-dlp"),
        path: path.to_string_lossy().into_owned(),
        version,
    })
}

async fn ffmpeg_info(app: &AppHandle) -> Option<ToolInfo> {
    match find_ffmpeg(app) {
        Some(path) => run_version(&path, "-version").await.map(|line| ToolInfo {
            managed: is_managed(app, &path),
            fast_start: false,
            path: path.to_string_lossy().into_owned(),
            // "ffmpeg version N-1234-gabcdef Copyright ..." -> the version token.
            version: line
                .split_whitespace()
                .nth(2)
                .unwrap_or("unknown")
                .to_string(),
        }),
        None => None,
    }
}

fn emit_progress(app: &AppHandle, tool: &'static str, received: u64, total: u64, stage: &'static str) {
    let _ = app.emit(
        "tool-progress",
        ToolProgress {
            tool,
            received,
            total,
            stage,
        },
    );
}

/// Streams a URL to `dest`, reporting progress, and returns the SHA-256 of what was written.
async fn download_to(
    app: &AppHandle,
    tool: &'static str,
    url: &str,
    dest: &Path,
) -> Result<String> {
    let state = app.state::<AppState>();
    let response = state.http.get(url).send().await?.error_for_status()?;
    let total = response.content_length().unwrap_or(0);

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::File::create(dest).await?;
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    let mut last_emit = Instant::now();
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        received += chunk.len() as u64;
        if last_emit.elapsed() >= Duration::from_millis(120) {
            emit_progress(app, tool, received, total, "download");
            last_emit = Instant::now();
        }
    }
    file.flush().await?;
    emit_progress(app, tool, received, total.max(received), "download");
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Looks up `file_name` in a "<sha256>  <name>" checksum listing.
async fn expected_checksum(app: &AppHandle, sums_url: &str, file_name: &str) -> Result<String> {
    let state = app.state::<AppState>();
    let text = state
        .http
        .get(sums_url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?.trim_start_matches('*');
            (name == file_name).then(|| hash.to_ascii_lowercase())
        })
        .next()
        .ok_or_else(|| AppError::msg(format!("No published checksum for {file_name}.")))
}

pub async fn latest_ytdlp_version(app: &AppHandle) -> Result<String> {
    let state = app.state::<AppState>();
    let release: serde_json::Value = state
        .http
        .get(YTDLP_LATEST_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    release["tag_name"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| AppError::msg("Could not read the latest yt-dlp version."))
}

pub async fn install_ytdlp(app: &AppHandle) -> Result<()> {
    let state = app.state::<AppState>();
    let _engine = reserve_engine(&state)?;
    let bin = state.bin_dir();
    let zip_path = bin.join("yt-dlp.zip.part");

    let expected = expected_checksum(app, YTDLP_SUMS_URL, YTDLP_ZIP_NAME).await?;
    let actual = match download_to(app, "ytdlp", YTDLP_ZIP_URL, &zip_path).await {
        Ok(actual) => actual,
        Err(e) => {
            let _ = tokio::fs::remove_file(&zip_path).await;
            return Err(e);
        }
    };
    if actual != expected {
        let _ = tokio::fs::remove_file(&zip_path).await;
        return Err(AppError::msg(
            "The downloaded yt-dlp did not match its published checksum, so it was discarded.",
        ));
    }

    emit_progress(app, "ytdlp", 0, 0, "extract");
    let bin_for_task = bin.clone();
    let zip_for_task = zip_path.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<()> {
        let staging = bin_for_task.join("yt-dlp.new");
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging)?;
        if let Err(e) = extract_all(&zip_for_task, &staging) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
        let target = bin_for_task.join("yt-dlp");
        swap_in(&staging, &target, &backup_of(&target))?;
        // The single-file build from older versions is no longer needed.
        let _ = std::fs::remove_file(bin_for_task.join("yt-dlp.exe"));
        Ok(())
    })
    .await
    .map_err(|e| AppError::msg(e.to_string()))?;

    let _ = tokio::fs::remove_file(&zip_path).await;
    result
}

fn extract_all(zip_path: &Path, staging: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path)?)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        // enclosed_name rejects absolute paths and "..", so nothing lands outside the
        // staging folder.
        let Some(relative) = entry.enclosed_name() else { continue };
        let out = staging.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::io::copy(&mut entry, &mut std::fs::File::create(&out)?)?;
    }
    if !staging.join("yt-dlp.exe").is_file() {
        return Err(AppError::msg("The yt-dlp archive did not contain yt-dlp.exe."));
    }
    Ok(())
}

/// Replaces `target` with `staging`, keeping the old copy until the new one is in place
/// and putting it back if the swap fails, so a failed update never leaves no yt-dlp.
fn swap_in(staging: &Path, target: &Path, backup: &Path) -> Result<()> {
    let _ = std::fs::remove_dir_all(backup);
    let had_old = target.exists();
    if had_old {
        if let Err(e) = std::fs::rename(target, backup) {
            let _ = std::fs::remove_dir_all(staging);
            return Err(AppError::msg(format!(
                "Could not replace yt-dlp because its files are in use ({e}). Close anything using it and try again."
            )));
        }
    }
    if let Err(e) = std::fs::rename(staging, target) {
        if had_old {
            let _ = std::fs::rename(backup, target);
        }
        let _ = std::fs::remove_dir_all(staging);
        return Err(AppError::msg(format!("Could not install the new yt-dlp: {e}")));
    }
    let _ = std::fs::remove_dir_all(backup);
    Ok(())
}

/// Installs ffmpeg and ffprobe from the yt-dlp project's own FFmpeg builds.
pub async fn install_ffmpeg(app: &AppHandle) -> Result<()> {
    let state = app.state::<AppState>();
    let _engine = reserve_engine(&state)?;
    let bin = state.bin_dir();
    let zip_path = bin.join("ffmpeg.zip.part");

    let expected = expected_checksum(app, FFMPEG_SUMS_URL, FFMPEG_ZIP_NAME).await?;
    let actual = match download_to(app, "ffmpeg", FFMPEG_URL, &zip_path).await {
        Ok(actual) => actual,
        Err(e) => {
            let _ = tokio::fs::remove_file(&zip_path).await;
            return Err(e);
        }
    };
    if actual != expected {
        let _ = tokio::fs::remove_file(&zip_path).await;
        return Err(AppError::msg(
            "The downloaded ffmpeg did not match its published checksum, so it was discarded.",
        ));
    }

    emit_progress(app, "ffmpeg", 0, 0, "extract");
    let bin_for_task = bin.clone();
    let zip_for_task = zip_path.clone();
    let extracted = tokio::task::spawn_blocking(move || -> Result<()> {
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_for_task)?)?;
        // Both executables are unpacked to temp names first, then swapped in as a set
        // with backups (`replace_all`), so a failure never leaves a mismatched pair.
        let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();
        let discard = |staged: &[(PathBuf, PathBuf)]| {
            for (tmp, _) in staged {
                let _ = std::fs::remove_file(tmp);
            }
        };
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            // Only take the two executables we need, by file name, never by the
            // archive's own path.
            let name = entry.name().replace('\\', "/");
            let file_name = match name.rsplit('/').next() {
                Some(n) if name.contains("/bin/") => n.to_string(),
                _ => continue,
            };
            if file_name != "ffmpeg.exe" && file_name != "ffprobe.exe" {
                continue;
            }
            let out_path = bin_for_task.join(&file_name);
            if staged.iter().any(|(_, out)| *out == out_path) {
                continue;
            }
            let tmp = bin_for_task.join(format!("{file_name}.part"));
            let copied = std::fs::File::create(&tmp).and_then(|mut f| std::io::copy(&mut entry, &mut f));
            if let Err(e) = copied {
                let _ = std::fs::remove_file(&tmp);
                discard(&staged);
                return Err(e.into());
            }
            staged.push((tmp, out_path));
        }
        if staged.len() < 2 {
            discard(&staged);
            return Err(AppError::msg("The ffmpeg archive did not contain the expected files."));
        }
        if let Err(e) = replace_all(&staged) {
            discard(&staged);
            return Err(AppError::msg(format!(
                "Could not install ffmpeg because a file is in use ({e}). The previous copy was kept."
            )));
        }
        Ok(())
    })
    .await
    .map_err(|e| AppError::msg(e.to_string()))?;

    let _ = tokio::fs::remove_file(&zip_path).await;
    extracted
}

/// Startup repair after an exit in the middle of an install: puts back a tool that was
/// left renamed to `.old` and removes unpacked or downloaded `.part` leftovers.
pub fn recover_bin(bin: &Path) {
    for name in ["yt-dlp", "ffmpeg.exe", "ffprobe.exe"] {
        let target = bin.join(name);
        let backup = backup_of(&target);
        if !target.exists() && backup.exists() {
            let _ = std::fs::rename(&backup, &target);
        }
    }
    let _ = std::fs::remove_dir_all(bin.join("yt-dlp.new"));
    if let Ok(entries) = std::fs::read_dir(bin) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().ends_with(".part") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn backup_of(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

/// Moves each `(staged, target)` into place as one unit. Every existing target is first
/// renamed to `<target>.old` (Windows allows renaming a running executable, but not
/// overwriting it). If any step fails, the targets already replaced get their old copy
/// back, so the set is either all new or all old. Backups are removed on success.
fn replace_all(pairs: &[(PathBuf, PathBuf)]) -> std::io::Result<()> {
    // (index, whether that target had an old copy that is now in its backup)
    let mut done: Vec<(usize, bool)> = Vec::new();
    let restore = |done: &[(usize, bool)]| {
        for &(index, had_old) in done.iter().rev() {
            let target = &pairs[index].1;
            let _ = std::fs::remove_file(target);
            if had_old {
                let _ = std::fs::rename(backup_of(target), target);
            }
        }
    };
    for (index, (staged, target)) in pairs.iter().enumerate() {
        let backup = backup_of(target);
        let _ = std::fs::remove_file(&backup);
        let had_old = target.exists();
        if had_old {
            if let Err(e) = std::fs::rename(target, &backup) {
                restore(&done);
                return Err(e);
            }
        }
        if let Err(e) = std::fs::rename(staged, target) {
            if had_old {
                let _ = std::fs::rename(&backup, target);
            }
            restore(&done);
            return Err(e);
        }
        done.push((index, had_old));
    }
    for (_, target) in pairs {
        // A backup still held by a running ffmpeg stays; the next install replaces it.
        let _ = std::fs::remove_file(backup_of(target));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recover_bin_restores_and_sweeps() {
        let dir = temp_dir("recover");
        std::fs::write(dir.join("ffmpeg.exe.old"), b"old").unwrap();
        std::fs::write(dir.join("ffprobe.exe"), b"cur").unwrap();
        std::fs::write(dir.join("ffprobe.exe.old"), b"stale").unwrap();
        std::fs::write(dir.join("ffmpeg.zip.part"), b"x").unwrap();
        std::fs::create_dir_all(dir.join("yt-dlp.old")).unwrap();
        recover_bin(&dir);
        assert_eq!(std::fs::read(dir.join("ffmpeg.exe")).unwrap(), b"old");
        assert_eq!(std::fs::read(dir.join("ffprobe.exe")).unwrap(), b"cur");
        assert!(dir.join("yt-dlp").is_dir());
        assert!(!dir.join("ffmpeg.zip.part").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replace_all_swaps_the_whole_set() {
        let dir = temp_dir("set");
        let pairs: Vec<(PathBuf, PathBuf)> = ["ffmpeg.exe", "ffprobe.exe"]
            .iter()
            .map(|name| {
                std::fs::write(dir.join(name), b"old").unwrap();
                std::fs::write(dir.join(format!("{name}.part")), b"new").unwrap();
                (dir.join(format!("{name}.part")), dir.join(name))
            })
            .collect();
        replace_all(&pairs).unwrap();
        for (_, target) in &pairs {
            assert_eq!(std::fs::read(target).unwrap(), b"new");
            assert!(!backup_of(target).exists());
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replace_all_restores_the_first_when_the_second_fails() {
        let dir = temp_dir("set-fail");
        std::fs::write(dir.join("ffmpeg.exe"), b"old-ffmpeg").unwrap();
        std::fs::write(dir.join("ffprobe.exe"), b"old-ffprobe").unwrap();
        std::fs::write(dir.join("ffmpeg.exe.part"), b"new-ffmpeg").unwrap();
        // The second staged file is missing, so its rename fails after the first succeeded.
        let pairs = vec![
            (dir.join("ffmpeg.exe.part"), dir.join("ffmpeg.exe")),
            (dir.join("ffprobe.exe.part"), dir.join("ffprobe.exe")),
        ];
        assert!(replace_all(&pairs).is_err());
        assert_eq!(std::fs::read(dir.join("ffmpeg.exe")).unwrap(), b"old-ffmpeg");
        assert_eq!(std::fs::read(dir.join("ffprobe.exe")).unwrap(), b"old-ffprobe");
        assert!(!dir.join("ffmpeg.exe.old").exists() && !dir.join("ffprobe.exe.old").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("claws-tools-{tag}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn swap_replaces_and_drops_backup() {
        let dir = temp_dir("swap");
        let (staging, target, backup) = (dir.join("new"), dir.join("cur"), dir.join("old"));
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("yt-dlp.exe"), b"new").unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("yt-dlp.exe"), b"old").unwrap();
        swap_in(&staging, &target, &backup).unwrap();
        assert_eq!(std::fs::read(target.join("yt-dlp.exe")).unwrap(), b"new");
        assert!(!backup.exists() && !staging.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_swap_restores_old_copy() {
        let dir = temp_dir("rollback");
        let (staging, target, backup) = (dir.join("missing"), dir.join("cur"), dir.join("old"));
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("yt-dlp.exe"), b"old").unwrap();
        assert!(swap_in(&staging, &target, &backup).is_err());
        assert_eq!(std::fs::read(target.join("yt-dlp.exe")).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(dir);
    }
}

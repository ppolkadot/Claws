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
        command(program).arg(arg).output(),
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
    let (ytdlp, ffmpeg) = tokio::join!(ytdlp_info(app), ffmpeg_info(app));
    ToolsStatus {
        ytdlp,
        ffmpeg,
        latest_ytdlp: None,
    }
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
    if !state.running.lock().unwrap().is_empty() {
        return Err(AppError::msg(
            "Pause or finish the running downloads first, they are using yt-dlp right now.",
        ));
    }
    let bin = state.bin_dir();
    let zip_path = bin.join("yt-dlp.zip.part");

    let expected = expected_checksum(app, YTDLP_SUMS_URL, YTDLP_ZIP_NAME).await?;
    let actual = download_to(app, "ytdlp", YTDLP_ZIP_URL, &zip_path).await?;
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
        let target = bin_for_task.join("yt-dlp");
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging)?;

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_for_task)?)?;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            // enclosed_name rejects absolute paths and "..", so nothing lands outside
            // the staging folder.
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
            let _ = std::fs::remove_dir_all(&staging);
            return Err(AppError::msg("The yt-dlp archive did not contain yt-dlp.exe."));
        }

        let _ = std::fs::remove_dir_all(&target);
        std::fs::rename(&staging, &target)?;
        // The single-file build from older versions is no longer needed.
        let _ = std::fs::remove_file(bin_for_task.join("yt-dlp.exe"));
        Ok(())
    })
    .await
    .map_err(|e| AppError::msg(e.to_string()))?;

    let _ = tokio::fs::remove_file(&zip_path).await;
    result
}

/// Installs ffmpeg and ffprobe from the yt-dlp project's own FFmpeg builds.
pub async fn install_ffmpeg(app: &AppHandle) -> Result<()> {
    let bin = app.state::<AppState>().bin_dir();
    let zip_path = bin.join("ffmpeg.zip.part");

    let expected = expected_checksum(app, FFMPEG_SUMS_URL, FFMPEG_ZIP_NAME).await?;
    let actual = download_to(app, "ffmpeg", FFMPEG_URL, &zip_path).await?;
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
        let mut found = 0;
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
            let tmp = bin_for_task.join(format!("{file_name}.part"));
            std::io::copy(&mut entry, &mut std::fs::File::create(&tmp)?)?;
            std::fs::rename(&tmp, &out_path)?;
            found += 1;
        }
        if found < 2 {
            return Err(AppError::msg("The ffmpeg archive did not contain the expected files."));
        }
        Ok(())
    })
    .await
    .map_err(|e| AppError::msg(e.to_string()))?;

    let _ = tokio::fs::remove_file(&zip_path).await;
    extracted
}

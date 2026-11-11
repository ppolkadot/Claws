use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{Emitter, Manager};

use crate::AppHandle;
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
// Deno's per-archive checksum file uses PowerShell's Get-FileHash layout; see `checksum_in`.
const DENO_URL: &str = "https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip";
const DENO_SUMS_URL: &str = "https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip.sha256sum";
const DENO_ZIP_NAME: &str = "deno-x86_64-pc-windows-msvc.zip";

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
    pub deno: Option<ToolInfo>,
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
    // Request UTF-8 from Python child tools; frozen yt-dlp also needs --encoding
    // in common_args below because it can override these environment variables.
    cmd.env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1");
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

pub fn find_ytdlp(app: &AppHandle) -> Option<PathBuf> {
    let bin = app.state::<AppState>().bin_dir();
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

fn managed_deno(app: &AppHandle) -> Option<PathBuf> {
    let managed = app.state::<AppState>().bin_dir().join("deno.exe");
    managed.is_file().then_some(managed)
}

pub fn find_deno(app: &AppHandle) -> Option<PathBuf> {
    managed_deno(app).or_else(|| on_path("deno").cloned())
}

/// Claws' own Deno is named by path (not on PATH); yt-dlp finds a PATH Deno unaided; Node.js only as a fallback.
fn js_runtime_args(managed_deno: Option<&Path>, deno_on_path: bool, node_on_path: bool) -> Vec<String> {
    match managed_deno {
        Some(path) => vec!["--js-runtimes".into(), format!("deno:{}", path.display())],
        None if !deno_on_path && node_on_path => vec!["--js-runtimes".into(), "node".into()],
        None => Vec::new(),
    }
}

pub fn common_args(app: &AppHandle, settings: &Settings) -> Vec<String> {
    // The Windows frozen executable can choose its legacy output code page despite
    // PYTHONIOENCODING/PYTHONUTF8. Its machine-readable paths must match the Unicode
    // filenames on disk, including finished media and caption paths.
    let mut args: Vec<String> = vec!["--no-colors".into(), "--ignore-config".into(), "--encoding".into(), "utf-8".into()];

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

    // Recent YouTube extraction needs a JavaScript runtime.
    args.extend(js_runtime_args(managed_deno(app).as_deref(), on_path("deno").is_some(), on_path("node").is_some()));
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
    // A running exe would block an install's swap, and a half-swapped folder reads as
    // "not installed".
    let state = app.state::<AppState>();
    let _engine = state.engine.read().await;
    let (ytdlp, ffmpeg, deno) = tokio::join!(ytdlp_info(app), ffmpeg_info(app), deno_info(app));
    let js_runtime = if deno.is_some() {
        Some("Deno")
    } else if on_path("node").is_some() {
        Some("Node.js")
    } else {
        None
    };
    ToolsStatus {
        ytdlp,
        ffmpeg,
        deno,
        latest_ytdlp: None,
        js_runtime: js_runtime.map(str::to_string),
    }
}

/// Exclusive use of the engine for an install. Fails at once, rather than waiting for
/// downloads that may run for an hour, when anything is using yt-dlp. An install takes it
/// briefly before downloading, so a busy engine is reported before a long transfer, and
/// again for the swap, which is the check that counts.
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

async fn deno_info(app: &AppHandle) -> Option<ToolInfo> {
    let path = find_deno(app)?;
    run_version(&path, "--version").await.map(|line| ToolInfo {
        managed: is_managed(app, &path),
        fast_start: false,
        path: path.to_string_lossy().into_owned(),
        // "deno 2.9.8 (stable, release, x86_64-pc-windows-msvc)" -> the version token.
        version: line.split_whitespace().nth(1).unwrap_or("unknown").to_string(),
    })
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

/// How long a network step may take. Production values are `TIMING`; tests inject short ones.
#[derive(Clone, Copy)]
struct Timing {
    /// Longest wait for response headers or for the next piece of a body.
    idle: Duration,
    /// Longest a whole archive transfer may take, from request to last byte.
    artifact_total: Duration,
    /// The same for checksum listings and release information.
    text_total: Duration,
}

const TIMING: Timing = Timing {
    idle: Duration::from_secs(30),
    artifact_total: Duration::from_secs(15 * 60),
    text_total: Duration::from_secs(60),
};

#[derive(Clone, Copy)]
struct Bounds {
    idle: Duration,
    total: Duration,
    max_bytes: u64,
}

/// Compressed download caps, far above the real archives (about 20 MB and 150 MB).
const YTDLP_ZIP_MAX: u64 = 256 * 1024 * 1024;
const FFMPEG_ZIP_MAX: u64 = 512 * 1024 * 1024;
const DENO_ZIP_MAX: u64 = 256 * 1024 * 1024;
const CHECKSUM_MAX: u64 = 1024 * 1024;
const RELEASE_JSON_MAX: u64 = 2 * 1024 * 1024;

struct Release<'a> {
    tool: &'static str,
    archive_url: &'a str,
    sums_url: &'a str,
    archive_name: &'a str,
    archive_max: u64,
}

const YTDLP_RELEASE: Release<'static> = Release {
    tool: "ytdlp",
    archive_url: YTDLP_ZIP_URL,
    sums_url: YTDLP_SUMS_URL,
    archive_name: YTDLP_ZIP_NAME,
    archive_max: YTDLP_ZIP_MAX,
};

const FFMPEG_RELEASE: Release<'static> = Release {
    tool: "ffmpeg",
    archive_url: FFMPEG_URL,
    sums_url: FFMPEG_SUMS_URL,
    archive_name: FFMPEG_ZIP_NAME,
    archive_max: FFMPEG_ZIP_MAX,
};

const DENO_RELEASE: Release<'static> = Release {
    tool: "deno",
    archive_url: DENO_URL,
    sums_url: DENO_SUMS_URL,
    archive_name: DENO_ZIP_NAME,
    archive_max: DENO_ZIP_MAX,
};

fn size_text(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes >= MIB && bytes % MIB == 0 {
        format!("{} MiB", bytes / MIB)
    } else {
        format!("{bytes} bytes")
    }
}

fn too_big(max_bytes: u64) -> AppError {
    AppError::msg(format!(
        "The server sent more data than allowed (over {}), so the transfer was stopped.",
        size_text(max_bytes)
    ))
}

fn stalled(idle: Duration) -> AppError {
    AppError::msg(format!(
        "The connection stalled (no data for {idle:?}), so the transfer was stopped. Try again later."
    ))
}

fn too_slow(total: Duration) -> AppError {
    AppError::msg(format!(
        "The transfer took longer than {total:?} in total, so it was stopped. Try again later."
    ))
}

/// A request error without its address: after a redirect that is a signed download link.
fn net_error(error: reqwest::Error) -> AppError {
    AppError::from(error.without_url())
}

/// Adds a chunk to the running byte count, refusing a total over `max` (or one that
/// would overflow) before the chunk is used.
fn add_received(received: u64, chunk: usize, max: u64) -> Result<u64> {
    u64::try_from(chunk)
        .ok()
        .and_then(|chunk| received.checked_add(chunk))
        .filter(|total| *total <= max)
        .ok_or_else(|| too_big(max))
}

async fn within_idle<T>(idle: Duration, step: impl std::future::Future<Output = reqwest::Result<T>>) -> Result<T> {
    tokio::time::timeout(idle, step).await.map_err(|_| stalled(idle))?.map_err(net_error)
}

/// Sends the request and checks the status and any declared length against the cap.
async fn open_response(client: &reqwest::Client, url: &str, accept: Option<&str>, bounds: Bounds) -> Result<reqwest::Response> {
    let mut request = client.get(url);
    if let Some(accept) = accept {
        request = request.header("Accept", accept);
    }
    let response = within_idle(bounds.idle, request.send()).await?;
    let response = response.error_for_status().map_err(net_error)?;
    if response.content_length().is_some_and(|declared| declared > bounds.max_bytes) {
        return Err(too_big(bounds.max_bytes));
    }
    Ok(response)
}

/// Streams a URL to `dest`, reporting progress, and returns the SHA-256 of what was
/// written. The whole transfer has a deadline, every wait has an idle deadline, and the
/// size is capped; on any failure the partial file is removed.
async fn fetch_to_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    bounds: Bounds,
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<String> {
    let outcome = tokio::time::timeout(bounds.total, stream_to_file(client, url, dest, bounds, progress)).await;
    match outcome {
        Ok(Ok(digest)) => Ok(digest),
        failed => {
            // The transfer future is gone by now. Its file handle is normally closed too;
            // if a write is still finishing on the blocking pool the removal can fail, and
            // the attempt folder's cleanup (or the startup sweep) removes what is left.
            let _ = tokio::fs::remove_file(dest).await;
            match failed {
                Ok(Err(e)) => Err(e),
                _ => Err(too_slow(bounds.total)),
            }
        }
    }
}

async fn stream_to_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    bounds: Bounds,
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<String> {
    let mut response = open_response(client, url, None, bounds).await?;
    let total = response.content_length().unwrap_or(0);

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::File::create(dest).await?;
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    let mut last_emit = Instant::now();

    while let Some(chunk) = within_idle(bounds.idle, response.chunk()).await? {
        received = add_received(received, chunk.len(), bounds.max_bytes)?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        if last_emit.elapsed() >= Duration::from_millis(120) {
            progress(received, total);
            last_emit = Instant::now();
        }
    }
    file.flush().await?;
    progress(received, total.max(received));
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Fetches a small document under the same deadlines and a byte cap.
async fn fetch_capped(client: &reqwest::Client, url: &str, accept: Option<&str>, bounds: Bounds) -> Result<Vec<u8>> {
    let read = async {
        let mut response = open_response(client, url, accept, bounds).await?;
        let mut body: Vec<u8> = Vec::new();
        while let Some(chunk) = within_idle(bounds.idle, response.chunk()).await? {
            add_received(body.len() as u64, chunk.len(), bounds.max_bytes)?;
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    };
    tokio::time::timeout(bounds.total, read).await.unwrap_or_else(|_| Err(too_slow(bounds.total)))
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Accepts a "<sha256>  <name>" listing (yt-dlp, FFmpeg-Builds) or PowerShell Get-FileHash output
/// (Deno), whose "Path" line must name `file_name`.
fn checksum_in(text: &str, file_name: &str) -> Option<String> {
    let listed = text.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == file_name && is_sha256(hash)).then(|| hash.to_ascii_lowercase())
    });
    if listed.is_some() {
        return listed;
    }
    let field = |key: &str| {
        text.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.trim() == key).then(|| value.trim().to_string())
        })
    };
    let hash = field("Hash").filter(|hash| is_sha256(hash))?;
    let path = field("Path")?;
    let named = path.rsplit(['\\', '/']).next()?;
    (field("Algorithm").as_deref() == Some("SHA256") && named == file_name).then(|| hash.to_ascii_lowercase())
}

async fn expected_checksum(client: &reqwest::Client, sums_url: &str, file_name: &str, timing: Timing) -> Result<String> {
    let bounds = Bounds { idle: timing.idle, total: timing.text_total, max_bytes: CHECKSUM_MAX };
    let bytes = fetch_capped(client, sums_url, None, bounds).await?;
    checksum_in(&String::from_utf8_lossy(&bytes), file_name)
        .ok_or_else(|| AppError::msg(format!("No published checksum for {file_name}.")))
}

async fn latest_version_from(client: &reqwest::Client, url: &str, timing: Timing) -> Result<String> {
    let bounds = Bounds { idle: timing.idle, total: timing.text_total, max_bytes: RELEASE_JSON_MAX };
    let bytes = fetch_capped(client, url, Some("application/vnd.github+json"), bounds).await?;
    let release: serde_json::Value = serde_json::from_slice(&bytes)?;
    release["tag_name"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| AppError::msg("Could not read the latest yt-dlp version."))
}

pub async fn latest_ytdlp_version(app: &AppHandle) -> Result<String> {
    let state = app.state::<AppState>();
    latest_version_from(&state.http, YTDLP_LATEST_API, TIMING).await
}

/// Prefix of the private folder one install attempt works in, inside the tools folder so
/// that finished files can be renamed into place.
const ATTEMPT_PREFIX: &str = ".install-";

/// One install attempt's own folder: the archive, its extracted files and any staged
/// executables live here, never under a name another attempt could share. Removed when the
/// attempt ends by any route (success, failure or cancellation); `recover_bin` sweeps what
/// a crash leaves.
struct Attempt {
    dir: PathBuf,
}

impl Attempt {
    async fn create(bin: &Path) -> Result<Self> {
        let dir = bin.join(format!("{ATTEMPT_PREFIX}{}", uuid::Uuid::new_v4().simple()));
        tokio::fs::create_dir_all(&dir).await?;
        Ok(Self { dir })
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// One install at a time. Fails at once when another is running; nothing ever waits on
/// this lock, so it cannot deadlock with the engine lock.
fn reserve_install(state: &AppState) -> Result<tokio::sync::MutexGuard<'_, ()>> {
    state
        .tool_install
        .try_lock()
        .map_err(|_| AppError::msg("A tool install is already running. Wait for it to finish, then try again."))
}

/// Downloads the archive into `archive` and checks it against the published checksum.
/// Touches nothing outside the attempt folder, so it needs no engine lock.
async fn fetch_verified(
    app: &AppHandle,
    release: &Release<'_>,
    timing: Timing,
    label: &str,
    archive: &Path,
) -> Result<()> {
    let state = app.state::<AppState>();
    let expected = expected_checksum(&state.http, release.sums_url, release.archive_name, timing).await?;
    let bounds = Bounds { idle: timing.idle, total: timing.artifact_total, max_bytes: release.archive_max };
    let actual = fetch_to_file(&state.http, release.archive_url, archive, bounds, &mut |received, total| {
        emit_progress(app, release.tool, received, total, "download")
    })
    .await?;
    if actual != expected {
        let _ = tokio::fs::remove_file(archive).await;
        return Err(AppError::msg(format!(
            "The downloaded {label} did not match its published checksum, so it was discarded."
        )));
    }
    Ok(())
}

fn blocking_failed(error: tokio::task::JoinError) -> AppError {
    AppError::msg(error.to_string())
}

pub async fn install_ytdlp(app: &AppHandle) -> Result<()> {
    install_ytdlp_from(app, &YTDLP_RELEASE, TIMING).await
}

/// The network transfer, checksum and extraction run beside the engine, in the attempt's
/// folder; only the final swap takes the engine, and fails at once rather than wait.
async fn install_ytdlp_from(app: &AppHandle, release: &Release<'_>, timing: Timing) -> Result<()> {
    let state = app.state::<AppState>();
    let _install = reserve_install(&state)?;
    drop(reserve_engine(&state)?);
    let bin = state.bin_dir();
    let attempt = Attempt::create(&bin).await?;
    let archive = attempt.dir.join("archive.zip.part");

    fetch_verified(app, release, timing, "yt-dlp", &archive).await?;

    emit_progress(app, release.tool, 0, 0, "extract");
    let staging = attempt.dir.join("yt-dlp.new");
    let (archive_for_task, staging_for_task) = (archive.clone(), staging.clone());
    tokio::task::spawn_blocking(move || -> Result<()> {
        std::fs::create_dir_all(&staging_for_task)?;
        extract_all(&archive_for_task, &staging_for_task, &YTDLP_LIMITS)
    })
    .await
    .map_err(blocking_failed)??;
    let _ = tokio::fs::remove_file(&archive).await;

    let _engine = reserve_engine(&state)?;
    tokio::task::spawn_blocking(move || commit_ytdlp(&bin, &staging)).await.map_err(blocking_failed)?
}

/// The swap, with the engine held: checks the prepared files are still there, then
/// replaces the live folder, keeping the old one until the new one is in place.
fn commit_ytdlp(bin: &Path, staging: &Path) -> Result<()> {
    if !staging.join("yt-dlp.exe").is_file() {
        return Err(AppError::msg("The prepared yt-dlp files are gone, so nothing was changed. Try again."));
    }
    let target = bin.join("yt-dlp");
    swap_in(staging, &target, &backup_of(&target))?;
    // The single-file build from older versions is no longer needed.
    let _ = std::fs::remove_file(bin.join("yt-dlp.exe"));
    Ok(())
}

/// What a tool archive may contain. The archives are checksum-verified before they are
/// opened, so these limits only matter if that check were ever bypassed or a publisher's
/// release were compromised: they are far above the real sizes (yt-dlp unpacks to about
/// 40 MB in 146 files) and far below anything that could fill a disk or memory.
struct Limits {
    entries: usize,
    entry_bytes: u64,
    total_bytes: u64,
}

const YTDLP_LIMITS: Limits = Limits {
    entries: 2_000,
    entry_bytes: 256 * 1024 * 1024,
    total_bytes: 512 * 1024 * 1024,
};
const FFMPEG_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
/// deno.exe unpacks to roughly 130 MB.
const DENO_ENTRY_BYTES: u64 = 512 * 1024 * 1024;

/// Entries no tool release contains. Encrypted entries cannot be read without a password
/// and links could point out of the folder, so either means the archive is not ours.
fn check_entry(entry: &zip::read::ZipFile<'_, impl Read>) -> Result<()> {
    if entry.encrypted() {
        return Err(AppError::msg("The archive contains an encrypted entry, so it was not used."));
    }
    if entry.is_symlink() {
        return Err(AppError::msg("The archive contains a link, so it was not used."));
    }
    Ok(())
}

/// Copies at most `limit` bytes. The size an entry declares is not trusted: what counts
/// is what is actually produced, so an archive that lies about its size is caught here.
fn copy_limited(from: &mut impl Read, to: &mut impl Write, limit: u64) -> Result<u64> {
    let copied = std::io::copy(&mut from.take(limit.saturating_add(1)), to)?;
    if copied > limit {
        return Err(AppError::msg("An entry in the archive is larger than allowed, so it was not used."));
    }
    Ok(copied)
}

fn extract_all(zip_path: &Path, staging: &Path, limits: &Limits) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path)?)?;
    if archive.len() > limits.entries {
        return Err(AppError::msg("The archive has too many entries, so it was not used."));
    }
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        check_entry(&entry)?;
        // enclosed_name skips what could land outside the staging folder: absolute paths,
        // "..", Windows device names and ':' (an NTFS alternate data stream).
        let Some(relative) = entry.enclosed_name() else { continue };
        let out = staging.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let budget = limits.entry_bytes.min(limits.total_bytes - total);
        total += copy_limited(&mut entry, &mut std::fs::File::create(&out)?, budget)?;
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

fn discard(staged: &[(PathBuf, PathBuf)]) {
    for (tmp, _) in staged {
        let _ = std::fs::remove_file(tmp);
    }
}

/// Unpacks `ffmpeg.exe` and `ffprobe.exe` to temporary names in `staging` (on the same
/// volume as `bin`), returning `(temporary, final)` pairs for `replace_all`. Nothing else
/// in the archive is read, and a failure leaves no temporary files behind.
fn stage_ffmpeg(zip_path: &Path, staging: &Path, bin: &Path, entry_limit: u64) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path)?)?;
    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        // Only the two executables we need, picked by file name, never by the archive's
        // own path.
        let name = match entry.name() {
            Ok(name) => name.replace('\\', "/"),
            Err(_) => continue,
        };
        let file_name = match name.rsplit('/').next() {
            Some(n) if name.contains("/bin/") => n.to_string(),
            _ => continue,
        };
        if file_name != "ffmpeg.exe" && file_name != "ffprobe.exe" {
            continue;
        }
        let out_path = bin.join(&file_name);
        if staged.iter().any(|(_, out)| *out == out_path) {
            continue;
        }
        if let Err(e) = check_entry(&entry) {
            discard(&staged);
            return Err(e);
        }
        let tmp = staging.join(format!("{file_name}.part"));
        let copied = std::fs::File::create(&tmp)
            .map_err(AppError::from)
            .and_then(|mut f| copy_limited(&mut entry, &mut f, entry_limit));
        if let Err(e) = copied {
            let _ = std::fs::remove_file(&tmp);
            discard(&staged);
            return Err(e);
        }
        staged.push((tmp, out_path));
    }
    if staged.len() < 2 {
        discard(&staged);
        return Err(AppError::msg("The ffmpeg archive did not contain the expected files."));
    }
    Ok(staged)
}

/// Installs ffmpeg and ffprobe from the yt-dlp project's own FFmpeg builds.
pub async fn install_ffmpeg(app: &AppHandle) -> Result<()> {
    install_ffmpeg_from(app, &FFMPEG_RELEASE, TIMING).await
}

async fn install_ffmpeg_from(app: &AppHandle, release: &Release<'_>, timing: Timing) -> Result<()> {
    let state = app.state::<AppState>();
    let _install = reserve_install(&state)?;
    drop(reserve_engine(&state)?);
    let bin = state.bin_dir();
    let attempt = Attempt::create(&bin).await?;
    let archive = attempt.dir.join("archive.zip.part");

    fetch_verified(app, release, timing, "ffmpeg", &archive).await?;

    emit_progress(app, release.tool, 0, 0, "extract");
    let (archive_for_task, staging_for_task, bin_for_task) = (archive.clone(), attempt.dir.clone(), bin.clone());
    let staged = tokio::task::spawn_blocking(move || {
        stage_ffmpeg(&archive_for_task, &staging_for_task, &bin_for_task, FFMPEG_ENTRY_BYTES)
    })
    .await
    .map_err(blocking_failed)??;
    let _ = tokio::fs::remove_file(&archive).await;

    let _engine = reserve_engine(&state)?;
    tokio::task::spawn_blocking(move || commit_ffmpeg(&staged)).await.map_err(blocking_failed)?
}

/// The swap, with the engine held: checks the staged files are still there, then
/// replaces both tools as one unit.
fn commit_ffmpeg(staged: &[(PathBuf, PathBuf)]) -> Result<()> {
    if staged.iter().any(|(tmp, _)| !tmp.is_file()) {
        return Err(AppError::msg("The prepared ffmpeg files are gone, so nothing was changed. Try again."));
    }
    if let Err(e) = replace_all(staged) {
        discard(staged);
        return Err(AppError::msg(format!(
            "Could not install ffmpeg because a file is in use ({e}). The previous copy was kept."
        )));
    }
    Ok(())
}

/// Only the root `deno.exe` is read; returns the `(temporary, final)` pair for `replace_all`.
fn stage_deno(zip_path: &Path, staging: &Path, bin: &Path, entry_limit: u64) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path)?)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if !matches!(entry.name(), Ok(name) if name == "deno.exe") {
            continue;
        }
        check_entry(&entry)?;
        let tmp = staging.join("deno.exe.part");
        let copied = std::fs::File::create(&tmp)
            .map_err(AppError::from)
            .and_then(|mut f| copy_limited(&mut entry, &mut f, entry_limit));
        if let Err(e) = copied {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        return Ok(vec![(tmp, bin.join("deno.exe"))]);
    }
    Err(AppError::msg("The Deno archive did not contain deno.exe."))
}

pub async fn install_deno(app: &AppHandle) -> Result<()> {
    install_deno_from(app, &DENO_RELEASE, TIMING).await
}

async fn install_deno_from(app: &AppHandle, release: &Release<'_>, timing: Timing) -> Result<()> {
    let state = app.state::<AppState>();
    let _install = reserve_install(&state)?;
    drop(reserve_engine(&state)?);
    let bin = state.bin_dir();
    let attempt = Attempt::create(&bin).await?;
    let archive = attempt.dir.join("archive.zip.part");

    fetch_verified(app, release, timing, "Deno", &archive).await?;

    emit_progress(app, release.tool, 0, 0, "extract");
    let (archive_for_task, staging_for_task, bin_for_task) = (archive.clone(), attempt.dir.clone(), bin.clone());
    let staged = tokio::task::spawn_blocking(move || stage_deno(&archive_for_task, &staging_for_task, &bin_for_task, DENO_ENTRY_BYTES))
        .await
        .map_err(blocking_failed)??;
    let _ = tokio::fs::remove_file(&archive).await;

    // yt-dlp starts Deno itself, so the swap waits for the engine like the other tools.
    let _engine = reserve_engine(&state)?;
    tokio::task::spawn_blocking(move || {
        if staged.iter().any(|(tmp, _)| !tmp.is_file()) {
            return Err(AppError::msg("The prepared Deno file is gone, so nothing was changed. Try again."));
        }
        replace_all(&staged).map_err(|e| {
            discard(&staged);
            AppError::msg(format!("Could not install Deno because the file is in use ({e}). The previous copy was kept."))
        })
    })
    .await
    .map_err(blocking_failed)?
}

/// Startup repair after an exit mid-install: restores a tool left as `.old` and
/// removes `.part` leftovers and abandoned install-attempt folders.
pub fn recover_bin(bin: &Path) {
    for name in ["yt-dlp", "ffmpeg.exe", "ffprobe.exe", "deno.exe"] {
        let target = bin.join(name);
        let backup = backup_of(&target);
        if !target.exists() && backup.exists() {
            let _ = std::fs::rename(&backup, &target);
        }
    }
    let _ = std::fs::remove_dir_all(bin.join("yt-dlp.new"));
    if let Ok(entries) = std::fs::read_dir(bin) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".part") {
                let _ = std::fs::remove_file(entry.path());
            } else if name.starts_with(ATTEMPT_PREFIX) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

fn backup_of(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

/// Moves each `(staged, target)` into place as one unit: all new or all old. Existing
/// targets are renamed to `.old` first (Windows allows renaming a running exe, not
/// overwriting it) and put back if any step fails.
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

    #[cfg(windows)]
    #[test]
    fn swap_keeps_the_old_copy_when_its_folder_is_in_use() {
        let dir = temp_dir("swap-locked");
        let (staging, target, backup) = (dir.join("new"), dir.join("cur"), dir.join("old"));
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("yt-dlp.exe"), b"new").unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("yt-dlp.exe"), b"old").unwrap();
        // Windows refuses to rename a folder while a file inside it is open: the same thing
        // a running yt-dlp does.
        let in_use = std::fs::File::open(target.join("yt-dlp.exe")).unwrap();
        let err = swap_in(&staging, &target, &backup).unwrap_err();
        assert!(err.to_string().contains("in use"), "{err}");
        drop(in_use);
        assert_eq!(std::fs::read(target.join("yt-dlp.exe")).unwrap(), b"old");
        assert!(!backup.exists(), "no backup may be left behind");
        assert!(!staging.exists(), "the discarded staging folder must be removed");
        let _ = std::fs::remove_dir_all(dir);
    }

    fn opts() -> zip::write::SimpleFileOptions {
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)
    }

    fn make_zip(dir: &Path, build: impl FnOnce(&mut zip::ZipWriter<std::fs::File>)) -> PathBuf {
        let path = dir.join("archive.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        build(&mut writer);
        writer.finish().unwrap();
        path
    }

    fn file(writer: &mut zip::ZipWriter<std::fs::File>, name: &str, data: &[u8]) {
        writer.start_file(name, opts()).unwrap();
        writer.write_all(data).unwrap();
    }

    const SMALL: Limits = Limits { entries: 10, entry_bytes: 1024, total_bytes: 4096 };

    #[test]
    fn extracts_an_ordinary_archive() {
        let dir = temp_dir("zip-ok");
        let archive = make_zip(&dir, |w| {
            file(w, "yt-dlp.exe", b"exe");
            file(w, "lib/python.dll", b"dll");
        });
        let staging = dir.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        extract_all(&archive, &staging, &SMALL).unwrap();
        assert_eq!(std::fs::read(staging.join("yt-dlp.exe")).unwrap(), b"exe");
        assert_eq!(std::fs::read(staging.join("lib").join("python.dll")).unwrap(), b"dll");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn links_are_refused() {
        let dir = temp_dir("zip-link");
        let archive = make_zip(&dir, |w| {
            file(w, "yt-dlp.exe", b"exe");
            w.add_symlink("shortcut", "C:\\Windows\\System32", opts()).unwrap();
        });
        let staging = dir.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        assert!(extract_all(&archive, &staging, &SMALL).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn names_that_escape_the_folder_are_skipped() {
        let dir = temp_dir("zip-escape");
        let archive = make_zip(&dir, |w| {
            file(w, "../evil.txt", b"x");
            file(w, "..\\evil2.txt", b"x");
            file(w, "yt-dlp.exe", b"exe");
        });
        let staging = dir.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        extract_all(&archive, &staging, &SMALL).unwrap();
        assert!(!dir.join("evil.txt").exists() && !dir.join("evil2.txt").exists());
        assert!(!staging.join("evil.txt").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn hidden_streams_and_device_names_are_never_written() {
        let dir = temp_dir("zip-ads");
        let archive = make_zip(&dir, |w| {
            file(w, "yt-dlp.exe", b"exe");
            file(w, "yt-dlp.exe:hidden", b"x");
            file(w, "con.txt", b"x");
        });
        let staging = dir.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        extract_all(&archive, &staging, &SMALL).unwrap();
        // A stream would not show up in a directory listing, so read it by name.
        assert!(std::fs::read(staging.join("yt-dlp.exe:hidden")).is_err());
        assert_eq!(std::fs::read(staging.join("yt-dlp.exe")).unwrap(), b"exe");
        assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn limits_count_what_is_actually_written() {
        let dir = temp_dir("zip-size");
        // Compresses to a few bytes, but unpacks past the per-entry limit.
        let archive = make_zip(&dir, |w| {
            file(w, "yt-dlp.exe", &vec![0u8; 2048]);
        });
        let staging = dir.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        assert!(extract_all(&archive, &staging, &SMALL).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn total_size_and_entry_count_are_capped() {
        let dir = temp_dir("zip-total");
        let archive = make_zip(&dir, |w| {
            for i in 0..5 {
                file(w, &format!("f{i}.bin"), &[1u8; 1000]);
            }
            file(w, "yt-dlp.exe", b"exe");
        });
        let staging = dir.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        let by_total = Limits { entries: 10, entry_bytes: 1024, total_bytes: 3000 };
        assert!(extract_all(&archive, &staging, &by_total).is_err());
        let by_count = Limits { entries: 3, entry_bytes: 1024, total_bytes: 1 << 20 };
        assert!(extract_all(&archive, &staging, &by_count).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ffmpeg_stages_only_the_two_executables() {
        let dir = temp_dir("ffmpeg-ok");
        let archive = make_zip(&dir, |w| {
            file(w, "build/bin/ffmpeg.exe", b"ffmpeg");
            file(w, "build/bin/ffprobe.exe", b"ffprobe");
            file(w, "build/bin/ffplay.exe", b"ffplay");
            file(w, "build/doc/ffmpeg.exe", b"decoy");
        });
        let staged = stage_ffmpeg(&archive, &dir, &dir, 1024).unwrap();
        assert_eq!(staged.len(), 2);
        for (tmp, target) in &staged {
            assert!(tmp.is_file());
            assert!(target.ends_with("ffmpeg.exe") || target.ends_with("ffprobe.exe"));
        }
        assert!(!dir.join("ffplay.exe").exists() && !dir.join("ffplay.exe.part").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ffmpeg_oversize_entry_leaves_nothing_behind() {
        let dir = temp_dir("ffmpeg-big");
        let archive = make_zip(&dir, |w| {
            file(w, "build/bin/ffmpeg.exe", b"ffmpeg");
            file(w, "build/bin/ffprobe.exe", &vec![0u8; 4096]);
        });
        assert!(stage_ffmpeg(&archive, &dir, &dir, 1024).is_err());
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
            .collect();
        assert!(leftovers.is_empty(), "no temporary files should remain");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ffmpeg_archive_missing_a_tool_is_an_error() {
        let dir = temp_dir("ffmpeg-missing");
        let archive = make_zip(&dir, |w| file(w, "build/bin/ffmpeg.exe", b"ffmpeg"));
        assert!(stage_ffmpeg(&archive, &dir, &dir, 1024).is_err());
        assert!(!dir.join("ffmpeg.exe.part").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    // Local HTTP fixtures only; the deadlines are injected short so nothing waits for a
    // production interval.

    use std::future::Future;
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::download_tests::names;
    use crate::lifecycle_tests::TestApp;

    const MIB: u64 = 1024 * 1024;

    enum Route {
        /// A complete answer.
        Body(Vec<u8>),
        /// Accepts the request and never answers.
        Silent,
        /// Sends headers declaring this many bytes, then nothing.
        HeadersThenStall(u64),
        /// Sends `sent`, declares `declared`, then stalls.
        StallMidBody { sent: Vec<u8>, declared: u64 },
        /// No Content-Length: streams this many bytes, then closes.
        Unbounded(usize),
        /// Declares this length and sends nothing.
        Declared(u64),
        /// Declares this length and sends one byte every 50 ms.
        Trickle(u64),
        /// Waits for the gate to open, then answers with the body.
        Gated(Vec<u8>, Arc<AtomicBool>),
        /// An error status.
        Status(u16),
    }

    struct Server {
        base: String,
        stop: Arc<AtomicBool>,
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl Server {
        fn url(&self, path: &str) -> String {
            format!("{}{path}", self.base)
        }
        fn requested(&self, path: &str) -> bool {
            self.requests.lock().unwrap().iter().any(|p| p == path)
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
        }
    }

    fn serve(routes: Vec<(&str, Route)>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let routes: Arc<Vec<(String, Route)>> = Arc::new(routes.into_iter().map(|(p, r)| (p.to_string(), r)).collect());
        let (accept_stop, accept_requests) = (stop.clone(), requests.clone());
        std::thread::spawn(move || {
            while !accept_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (routes, stop, requests) = (routes.clone(), accept_stop.clone(), accept_requests.clone());
                        std::thread::spawn(move || handle(stream, &routes, &stop, &requests));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Server { base, stop, requests }
    }

    fn handle(mut stream: TcpStream, routes: &[(String, Route)], stop: &AtomicBool, requests: &Mutex<Vec<String>>) {
        let _ = stream.set_nonblocking(false);
        let mut request = Vec::new();
        let mut buf = [0u8; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => request.extend_from_slice(&buf[..n]),
            }
        }
        let text = String::from_utf8_lossy(&request).into_owned();
        let target = text.split_whitespace().nth(1).unwrap_or("/");
        let path = target.split('?').next().unwrap_or("/").to_string();
        requests.lock().unwrap().push(path.clone());
        let hold = || {
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let head = |stream: &mut TcpStream, status: &str, length: Option<u64>| {
            let length = length.map(|n| format!("Content-Length: {n}\r\n")).unwrap_or_default();
            let _ = write!(stream, "HTTP/1.1 {status}\r\n{length}Connection: close\r\n\r\n");
            let _ = stream.flush();
        };
        let Some((_, route)) = routes.iter().find(|(p, _)| *p == path) else {
            head(&mut stream, "404 Not Found", Some(0));
            return;
        };
        match route {
            Route::Body(bytes) => {
                head(&mut stream, "200 OK", Some(bytes.len() as u64));
                let _ = stream.write_all(bytes);
            }
            Route::Silent => hold(),
            Route::HeadersThenStall(n) | Route::Declared(n) => {
                head(&mut stream, "200 OK", Some(*n));
                hold();
            }
            Route::StallMidBody { sent, declared } => {
                head(&mut stream, "200 OK", Some(*declared));
                let _ = stream.write_all(sent);
                let _ = stream.flush();
                hold();
            }
            Route::Unbounded(total) => {
                head(&mut stream, "200 OK", None);
                let block = vec![b'x'; 64 * 1024];
                let mut sent = 0usize;
                while sent < *total {
                    let n = block.len().min(*total - sent);
                    if stream.write_all(&block[..n]).is_err() {
                        return;
                    }
                    sent += n;
                }
            }
            Route::Trickle(n) => {
                head(&mut stream, "200 OK", Some(*n));
                while !stop.load(Ordering::SeqCst) {
                    if stream.write_all(b"x").and_then(|_| stream.flush()).is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            Route::Gated(bytes, gate) => {
                while !gate.load(Ordering::SeqCst) && !stop.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(10));
                }
                head(&mut stream, "200 OK", Some(bytes.len() as u64));
                let _ = stream.write_all(bytes);
            }
            Route::Status(code) => head(&mut stream, &format!("{code} Error"), Some(0)),
        }
    }

    /// Fails the test, instead of hanging it, when an operation is not bounded.
    async fn within<T>(seconds: u64, what: &str, future: impl Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(seconds), future)
            .await
            .unwrap_or_else(|_| panic!("{what} was still running after {seconds}s: it is not bounded"))
    }

    fn bounds(idle_ms: u64, total_ms: u64, max_bytes: u64) -> Bounds {
        Bounds { idle: Duration::from_millis(idle_ms), total: Duration::from_millis(total_ms), max_bytes }
    }

    fn quick() -> Timing {
        Timing { idle: Duration::from_secs(1), artifact_total: Duration::from_secs(5), text_total: Duration::from_secs(5) }
    }

    fn plain_client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    async fn fetch(url: &str, dest: &Path, bounds: Bounds) -> Result<String> {
        fetch_to_file(&plain_client(), url, dest, bounds, &mut |_, _| {}).await
    }

    fn sha_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn tool_install_policy_constants_match_the_plan() {
        assert_eq!(TIMING.idle, Duration::from_secs(30));
        assert_eq!(TIMING.artifact_total, Duration::from_secs(15 * 60));
        assert_eq!(YTDLP_ZIP_MAX, 256 * MIB);
        assert_eq!(FFMPEG_ZIP_MAX, 512 * MIB);
        assert_eq!(CHECKSUM_MAX, MIB);
        assert_eq!(RELEASE_JSON_MAX, 2 * MIB);
        assert_eq!(YTDLP_RELEASE.archive_max, YTDLP_ZIP_MAX);
        assert_eq!(FFMPEG_RELEASE.archive_max, FFMPEG_ZIP_MAX);
    }

    #[test]
    fn tool_install_byte_counter_never_wraps() {
        assert_eq!(add_received(5, 5, 10).unwrap(), 10);
        assert!(add_received(5, 6, 10).is_err());
        assert!(add_received(u64::MAX - 1, 10, u64::MAX).is_err(), "an overflowing count must be refused");
    }

    #[tokio::test]
    async fn tool_install_fetch_returns_the_digest_of_what_it_wrote() {
        let server = serve(vec![("/a", Route::Body(b"hello".to_vec()))]);
        let dir = temp_dir("fetch-ok");
        let dest = dir.join("a.part");
        let digest = within(10, "fetch", fetch(&server.url("/a"), &dest, bounds(3000, 5000, MIB))).await.unwrap();
        assert_eq!(digest, sha_hex(b"hello"));
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn tool_install_fetch_stops_a_server_that_never_answers() {
        let server = serve(vec![("/a", Route::Silent)]);
        let dir = temp_dir("fetch-silent");
        let dest = dir.join("a.part");
        let started = Instant::now();
        let err = within(10, "fetch", fetch(&server.url("/a"), &dest, bounds(300, 5000, MIB))).await.unwrap_err();
        assert!(err.to_string().contains("stalled"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(!dest.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn tool_install_fetch_stops_a_stall_after_the_headers() {
        let server = serve(vec![("/a", Route::HeadersThenStall(100))]);
        let dir = temp_dir("fetch-headers");
        let dest = dir.join("a.part");
        let started = Instant::now();
        let err = within(10, "fetch", fetch(&server.url("/a"), &dest, bounds(300, 5000, MIB))).await.unwrap_err();
        assert!(err.to_string().contains("stalled"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(!dest.exists(), "the partial file must be removed");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn tool_install_fetch_stops_a_stall_between_chunks() {
        let server = serve(vec![("/a", Route::StallMidBody { sent: vec![7u8; 100], declared: 1000 })]);
        let dir = temp_dir("fetch-mid");
        let dest = dir.join("a.part");
        let started = Instant::now();
        let err = within(10, "fetch", fetch(&server.url("/a"), &dest, bounds(300, 5000, MIB))).await.unwrap_err();
        assert!(err.to_string().contains("stalled"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(!dest.exists(), "the partial file must be removed");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn tool_install_fetch_enforces_the_total_deadline_on_a_slow_trickle() {
        let server = serve(vec![("/a", Route::Trickle(1_000_000))]);
        let dir = temp_dir("fetch-total");
        let dest = dir.join("a.part");
        let started = Instant::now();
        let err = within(10, "fetch", fetch(&server.url("/a"), &dest, bounds(2000, 700, MIB))).await.unwrap_err();
        assert!(err.to_string().contains("took longer"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(4));
        assert!(!dest.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn tool_install_fetch_counts_a_stream_with_no_declared_length() {
        let server = serve(vec![("/a", Route::Unbounded(8 * MIB as usize))]);
        let dir = temp_dir("fetch-stream");
        let dest = dir.join("a.part");
        let err = within(20, "fetch", fetch(&server.url("/a"), &dest, bounds(5000, 15000, MIB))).await.unwrap_err();
        assert!(err.to_string().contains("more data than allowed"), "{err}");
        assert!(!dest.exists(), "the oversized partial file must be removed");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn tool_install_fetch_refuses_a_declared_length_over_the_cap_before_reading() {
        let server = serve(vec![("/a", Route::Declared(10 * MIB))]);
        let dir = temp_dir("fetch-declared");
        let dest = dir.join("a.part");
        let started = Instant::now();
        let err = within(10, "fetch", fetch(&server.url("/a"), &dest, bounds(5000, 15000, MIB))).await.unwrap_err();
        assert!(err.to_string().contains("more data than allowed"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(2), "the header alone is enough to refuse it");
        assert!(!dest.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn tool_install_fetch_errors_do_not_expose_the_address() {
        let server = serve(vec![("/s", Route::Status(500))]);
        let dir = temp_dir("fetch-url");
        let dest = dir.join("a.part");
        let url = server.url("/s?sig=secret-token");
        let err = within(10, "fetch", fetch(&url, &dest, bounds(300, 5000, MIB))).await.unwrap_err();
        let text = err.to_string();
        assert!(!text.contains("secret-token") && !text.contains("127.0.0.1"), "{text}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn tool_install_small_documents_are_bounded_too() {
        let server = serve(vec![
            ("/ok", Route::Body(b"fine".to_vec())),
            ("/declared", Route::Body(vec![b'x'; 4096])),
            ("/stream", Route::Unbounded(100_000)),
            ("/silent", Route::Silent),
        ]);
        let client = plain_client();
        let small = bounds(1000, 5000, 1024);
        let ok = within(10, "document", fetch_capped(&client, &server.url("/ok"), None, small)).await.unwrap();
        assert_eq!(ok, b"fine");
        for path in ["/declared", "/stream"] {
            let err = within(10, "document", fetch_capped(&client, &server.url(path), None, small)).await.unwrap_err();
            assert!(err.to_string().contains("more data than allowed"), "{path}: {err}");
        }
        let err = within(10, "document", fetch_capped(&client, &server.url("/silent"), None, small)).await.unwrap_err();
        assert!(err.to_string().contains("stalled"), "{err}");
    }

    #[tokio::test]
    async fn tool_install_release_information_is_capped_and_bounded() {
        let json = br#"{"tag_name":"2099.01.01"}"#.to_vec();
        let mut huge = br#"{"tag_name":"2099.01.01","body":""#.to_vec();
        huge.extend(std::iter::repeat_n(b'a', RELEASE_JSON_MAX as usize));
        huge.extend_from_slice(br#""}"#);
        let server = serve(vec![
            ("/ok", Route::Body(json)),
            ("/huge", Route::Body(huge)),
            ("/stream", Route::Unbounded(3 * MIB as usize)),
            ("/silent", Route::Silent),
        ]);
        let client = plain_client();
        let version = within(10, "version", latest_version_from(&client, &server.url("/ok"), quick())).await.unwrap();
        assert_eq!(version, "2099.01.01");
        for path in ["/huge", "/stream"] {
            let err = within(20, "version", latest_version_from(&client, &server.url(path), quick())).await.unwrap_err();
            assert!(err.to_string().contains("more data than allowed"), "{path}: {err}");
        }
        let err = within(10, "version", latest_version_from(&client, &server.url("/silent"), quick())).await.unwrap_err();
        assert!(err.to_string().contains("stalled"), "{err}");
    }

    #[tokio::test]
    async fn tool_install_checksum_listing_is_capped() {
        let server = serve(vec![("/sums", Route::Unbounded(3 * MIB as usize))]);
        let err = within(20, "checksum", expected_checksum(&plain_client(), &server.url("/sums"), "x.zip", quick()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("more data than allowed"), "{err}");
    }


    struct Rig {
        app: TestApp,
        bin: PathBuf,
    }

    fn rig(tag: &str) -> Rig {
        let dir = temp_dir(tag);
        let app = TestApp::with_data(dir.clone(), dir.join("out"), Vec::new());
        let bin = dir.join("bin");
        std::fs::create_dir_all(bin.join("yt-dlp")).unwrap();
        std::fs::write(bin.join("yt-dlp").join("yt-dlp.exe"), b"known-good").unwrap();
        Rig { app, bin }
    }

    fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in entries {
            writer.start_file(*name, opts()).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn ytdlp_zip(exe: &[u8]) -> Vec<u8> {
        zip_bytes(&[("yt-dlp.exe", exe), ("lib/python.dll", b"dll")])
    }

    fn ffmpeg_zip() -> Vec<u8> {
        zip_bytes(&[("build/bin/ffmpeg.exe", b"new-ffmpeg"), ("build/bin/ffprobe.exe", b"new-ffprobe")])
    }

    fn sums(name: &str, archive: &[u8]) -> Vec<u8> {
        format!("{}  {name}\n", sha_hex(archive)).into_bytes()
    }

    fn archive_server(name: &str, archive: Vec<u8>) -> Server {
        serve(vec![("/sums", Route::Body(sums(name, &archive))), ("/archive", Route::Body(archive))])
    }

    async fn ytdlp_install(app: &TestApp, server: &Server, archive_max: u64) -> Result<()> {
        let (archive, sums) = (server.url("/archive"), server.url("/sums"));
        let release = Release { tool: "ytdlp", archive_url: &archive, sums_url: &sums, archive_name: YTDLP_ZIP_NAME, archive_max };
        install_ytdlp_from(app.handle(), &release, quick()).await
    }

    async fn ffmpeg_install(app: &TestApp, server: &Server, archive_max: u64) -> Result<()> {
        let (archive, sums) = (server.url("/archive"), server.url("/sums"));
        let release = Release { tool: "ffmpeg", archive_url: &archive, sums_url: &sums, archive_name: FFMPEG_ZIP_NAME, archive_max };
        install_ffmpeg_from(app.handle(), &release, quick()).await
    }

    fn live_ytdlp(rig: &Rig) -> Vec<u8> {
        std::fs::read(rig.bin.join("yt-dlp").join("yt-dlp.exe")).unwrap()
    }

    fn only_known_good(rig: &Rig) {
        assert_eq!(live_ytdlp(rig), b"known-good", "the known-good install must be preserved");
        assert_eq!(names(&rig.bin), vec!["yt-dlp"], "no staging or partial file may remain");
    }

    #[test]
    fn tool_install_replaces_the_tool_and_leaves_nothing_behind() {
        let rig = rig("ti-ok");
        let server = archive_server(YTDLP_ZIP_NAME, ytdlp_zip(b"new-ytdlp"));
        tauri::async_runtime::block_on(within(20, "install", ytdlp_install(&rig.app, &server, MIB))).unwrap();
        assert_eq!(live_ytdlp(&rig), b"new-ytdlp");
        assert_eq!(names(&rig.bin), vec!["yt-dlp"]);
    }

    #[test]
    fn tool_install_stalled_archive_is_stopped_and_the_engine_stays_free() {
        let rig = rig("ti-stall");
        let archive = ytdlp_zip(b"new");
        let server = serve(vec![("/sums", Route::Body(sums(YTDLP_ZIP_NAME, &archive))), ("/archive", Route::HeadersThenStall(5000))]);
        let started = Instant::now();
        let err = tauri::async_runtime::block_on(within(15, "install", ytdlp_install(&rig.app, &server, MIB))).unwrap_err();
        assert!(err.to_string().contains("stalled"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(rig.app.state().engine.try_write().is_ok(), "the engine must be free again");
        only_known_good(&rig);
    }

    #[test]
    fn tool_install_oversized_archive_is_rejected_and_cleaned_up() {
        let rig = rig("ti-big");
        let server = serve(vec![
            ("/sums", Route::Body(sums(YTDLP_ZIP_NAME, b"whatever"))),
            ("/archive", Route::Unbounded(8 * MIB as usize)),
        ]);
        let err = tauri::async_runtime::block_on(within(30, "install", ytdlp_install(&rig.app, &server, MIB))).unwrap_err();
        assert!(err.to_string().contains("more data than allowed"), "{err}");
        only_known_good(&rig);
    }

    #[test]
    fn tool_install_oversized_checksum_listing_never_reaches_the_download() {
        let rig = rig("ti-bigsums");
        let server = serve(vec![
            ("/sums", Route::Unbounded(3 * MIB as usize)),
            ("/archive", Route::Body(ytdlp_zip(b"new"))),
        ]);
        let err = tauri::async_runtime::block_on(within(30, "install", ytdlp_install(&rig.app, &server, MIB))).unwrap_err();
        assert!(err.to_string().contains("more data than allowed"), "{err}");
        assert!(!server.requested("/archive"));
        only_known_good(&rig);
    }

    #[test]
    fn tool_install_bad_checksum_is_discarded_and_the_old_copy_kept() {
        let rig = rig("ti-badsum");
        let archive = ytdlp_zip(b"new");
        let server = serve(vec![("/sums", Route::Body(sums(YTDLP_ZIP_NAME, b"a different archive"))), ("/archive", Route::Body(archive))]);
        let err = tauri::async_runtime::block_on(within(20, "install", ytdlp_install(&rig.app, &server, MIB))).unwrap_err();
        assert!(err.to_string().contains("checksum"), "{err}");
        only_known_good(&rig);
    }

    #[test]
    fn tool_install_unusable_archive_keeps_the_old_copy() {
        let rig = rig("ti-badzip");
        let server = archive_server(YTDLP_ZIP_NAME, zip_bytes(&[("readme.txt", b"no exe here")]));
        let err = tauri::async_runtime::block_on(within(20, "install", ytdlp_install(&rig.app, &server, MIB))).unwrap_err();
        assert!(err.to_string().contains("yt-dlp.exe"), "{err}");
        only_known_good(&rig);
    }

    #[test]
    fn tool_install_readers_are_not_blocked_while_the_archive_downloads() {
        let rig = rig("ti-reader");
        let archive = ytdlp_zip(b"new-ytdlp");
        let gate = Arc::new(AtomicBool::new(false));
        let server = serve(vec![("/sums", Route::Body(sums(YTDLP_ZIP_NAME, &archive))), ("/archive", Route::Gated(archive, gate.clone()))]);
        tauri::async_runtime::block_on(within(30, "install", async {
            let install = ytdlp_install(&rig.app, &server, MIB);
            let driver = async {
                while !server.requested("/archive") {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                let state = rig.app.state();
                let reader = tokio::time::timeout(Duration::from_millis(500), state.engine.read())
                    .await
                    .expect("a probe or download must not wait for an install's network transfer");
                for name in names(&rig.bin) {
                    assert!(name == "yt-dlp" || name.starts_with(".install-"), "in-progress files belong in a private attempt folder, found {name}");
                }
                drop(reader);
                gate.store(true, Ordering::SeqCst);
            };
            let (result, ()) = tokio::join!(install, driver);
            result.unwrap();
        }));
        assert_eq!(live_ytdlp(&rig), b"new-ytdlp");
        assert_eq!(names(&rig.bin), vec!["yt-dlp"]);
    }

    #[test]
    fn tool_install_busy_swap_keeps_the_old_copy_and_a_retry_succeeds() {
        let rig = rig("ti-busy");
        let archive = ytdlp_zip(b"new-ytdlp");
        let gate = Arc::new(AtomicBool::new(false));
        let server = serve(vec![("/sums", Route::Body(sums(YTDLP_ZIP_NAME, &archive))), ("/archive", Route::Gated(archive, gate.clone()))]);
        tauri::async_runtime::block_on(within(30, "install", async {
            let state = rig.app.state();
            // The engine is free when the install starts and taken during the download, so
            // only the commit-time reservation can catch it.
            let install = ytdlp_install(&rig.app, &server, MIB);
            let driver = async {
                while !server.requested("/archive") {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                let reader = state.engine.read().await;
                gate.store(true, Ordering::SeqCst);
                reader
            };
            let (result, reader) = tokio::join!(install, driver);
            let err = result.unwrap_err();
            assert!(err.to_string().contains("in use"), "{err}");
            only_known_good(&rig);
            drop(reader);
            ytdlp_install(&rig.app, &server, MIB).await.unwrap();
        }));
        assert_eq!(live_ytdlp(&rig), b"new-ytdlp");
        assert_eq!(names(&rig.bin), vec!["yt-dlp"]);
    }

    #[test]
    fn tool_install_busy_engine_fails_before_any_download() {
        let rig = rig("ti-busy-early");
        let server = archive_server(YTDLP_ZIP_NAME, ytdlp_zip(b"new-ytdlp"));
        tauri::async_runtime::block_on(within(30, "install", async {
            let state = rig.app.state();
            let reader = state.engine.read().await;
            let started = Instant::now();
            let err = ytdlp_install(&rig.app, &server, MIB).await.unwrap_err();
            assert!(err.to_string().contains("in use"), "{err}");
            assert!(started.elapsed() < Duration::from_secs(2), "a busy engine must be reported at once");
            assert!(!server.requested("/sums") && !server.requested("/archive"), "nothing may be downloaded while the engine is busy");
            only_known_good(&rig);
            drop(reader);
            ytdlp_install(&rig.app, &server, MIB).await.unwrap();
        }));
        assert_eq!(live_ytdlp(&rig), b"new-ytdlp");
    }

    #[test]
    fn tool_install_a_second_install_fails_promptly_while_one_runs() {
        let rig = rig("ti-second");
        let archive = ytdlp_zip(b"new-ytdlp");
        let gate = Arc::new(AtomicBool::new(false));
        let server = serve(vec![("/sums", Route::Body(sums(YTDLP_ZIP_NAME, &archive))), ("/archive", Route::Gated(archive, gate.clone()))]);
        let other = serve(vec![("/sums", Route::Body(sums(FFMPEG_ZIP_NAME, &ffmpeg_zip()))), ("/archive", Route::Body(ffmpeg_zip()))]);
        tauri::async_runtime::block_on(within(30, "install", async {
            let install = ytdlp_install(&rig.app, &server, MIB);
            let driver = async {
                while !server.requested("/archive") {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                let started = Instant::now();
                for second in [ytdlp_install(&rig.app, &server, MIB).await, ffmpeg_install(&rig.app, &other, MIB).await] {
                    let err = second.unwrap_err();
                    assert!(err.to_string().contains("already"), "{err}");
                }
                assert!(started.elapsed() < Duration::from_secs(2), "a second install must not wait");
                assert!(!other.requested("/archive"), "the refused install must not start downloading");
                gate.store(true, Ordering::SeqCst);
            };
            let (result, ()) = tokio::join!(install, driver);
            result.unwrap();
        }));
        assert_eq!(live_ytdlp(&rig), b"new-ytdlp");
        assert_eq!(names(&rig.bin), vec!["yt-dlp"]);
    }

    #[test]
    fn tool_install_ffmpeg_replaces_both_files_and_leaves_nothing_behind() {
        let rig = rig("ti-ffmpeg");
        std::fs::write(rig.bin.join("ffmpeg.exe"), b"old-ffmpeg").unwrap();
        std::fs::write(rig.bin.join("ffprobe.exe"), b"old-ffprobe").unwrap();
        let server = archive_server(FFMPEG_ZIP_NAME, ffmpeg_zip());
        tauri::async_runtime::block_on(within(20, "install", ffmpeg_install(&rig.app, &server, MIB))).unwrap();
        assert_eq!(std::fs::read(rig.bin.join("ffmpeg.exe")).unwrap(), b"new-ffmpeg");
        assert_eq!(std::fs::read(rig.bin.join("ffprobe.exe")).unwrap(), b"new-ffprobe");
        assert_eq!(names(&rig.bin), vec!["ffmpeg.exe", "ffprobe.exe", "yt-dlp"]);
    }

    #[test]
    fn tool_install_ffmpeg_busy_swap_keeps_the_previous_files() {
        let rig = rig("ti-ffmpeg-busy");
        std::fs::write(rig.bin.join("ffmpeg.exe"), b"old-ffmpeg").unwrap();
        std::fs::write(rig.bin.join("ffprobe.exe"), b"old-ffprobe").unwrap();
        let archive = ffmpeg_zip();
        let gate = Arc::new(AtomicBool::new(false));
        let server = serve(vec![("/sums", Route::Body(sums(FFMPEG_ZIP_NAME, &archive))), ("/archive", Route::Gated(archive, gate.clone()))]);
        tauri::async_runtime::block_on(within(30, "install", async {
            let state = rig.app.state();
            let install = ffmpeg_install(&rig.app, &server, MIB);
            let driver = async {
                while !server.requested("/archive") {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                let reader = state.engine.read().await;
                gate.store(true, Ordering::SeqCst);
                reader
            };
            let (result, reader) = tokio::join!(install, driver);
            let err = result.unwrap_err();
            assert!(err.to_string().contains("in use"), "{err}");
            drop(reader);
        }));
        assert_eq!(std::fs::read(rig.bin.join("ffmpeg.exe")).unwrap(), b"old-ffmpeg");
        assert_eq!(std::fs::read(rig.bin.join("ffprobe.exe")).unwrap(), b"old-ffprobe");
        assert_eq!(names(&rig.bin), vec!["ffmpeg.exe", "ffprobe.exe", "yt-dlp"]);
    }

    #[test]
    fn tool_install_ffmpeg_busy_engine_fails_before_any_download() {
        let rig = rig("ti-ffmpeg-busy-early");
        let server = archive_server(FFMPEG_ZIP_NAME, ffmpeg_zip());
        tauri::async_runtime::block_on(within(30, "install", async {
            let state = rig.app.state();
            let reader = state.engine.read().await;
            let err = ffmpeg_install(&rig.app, &server, MIB).await.unwrap_err();
            assert!(err.to_string().contains("in use"), "{err}");
            assert!(!server.requested("/sums") && !server.requested("/archive"));
            drop(reader);
        }));
        assert_eq!(names(&rig.bin), vec!["yt-dlp"]);
    }

    #[test]
    fn tool_install_ffmpeg_stalled_archive_leaves_nothing_behind() {
        let rig = rig("ti-ffmpeg-stall");
        let archive = ffmpeg_zip();
        let server = serve(vec![("/sums", Route::Body(sums(FFMPEG_ZIP_NAME, &archive))), ("/archive", Route::StallMidBody { sent: archive[..10].to_vec(), declared: archive.len() as u64 })]);
        let err = tauri::async_runtime::block_on(within(15, "install", ffmpeg_install(&rig.app, &server, MIB))).unwrap_err();
        assert!(err.to_string().contains("stalled"), "{err}");
        assert!(rig.app.state().engine.try_write().is_ok());
        only_known_good(&rig);
    }

    fn deno_sums(archive: &[u8], path: &str) -> Vec<u8> {
        format!("\r\nAlgorithm : SHA256\r\nHash      : {}\r\nPath      : {path}\r\n\r\n", sha_hex(archive).to_uppercase()).into_bytes()
    }

    const DENO_BUILD_PATH: &str = "C:\\a\\deno\\deno\\target\\release\\deno-x86_64-pc-windows-msvc.zip";

    #[test]
    fn checksums_are_read_from_a_listing_or_from_get_filehash_output() {
        let hash = sha_hex(b"archive");
        let listing = format!("{}  other.zip\n{hash} *wanted.zip\n", sha_hex(b"other"));
        assert_eq!(checksum_in(&listing, "wanted.zip"), Some(hash.clone()));
        assert_eq!(checksum_in(&listing, "missing.zip"), None);

        let powershell = String::from_utf8(deno_sums(b"archive", DENO_BUILD_PATH)).unwrap();
        assert_eq!(checksum_in(&powershell, DENO_ZIP_NAME), Some(hash), "the published upper-case hash is compared in lower case");
        // The file it describes must be the one being installed.
        assert_eq!(checksum_in(&powershell, "deno-aarch64-pc-windows-msvc.zip"), None);
        assert_eq!(checksum_in(&powershell.replace("SHA256", "MD5"), DENO_ZIP_NAME), None);
        assert_eq!(checksum_in("Algorithm : SHA256\nHash : abc\nPath : deno-x86_64-pc-windows-msvc.zip\n", DENO_ZIP_NAME), None);
        assert_eq!(checksum_in("not-a-hash  wanted.zip\n", "wanted.zip"), None);
    }

    #[test]
    fn the_javascript_runtime_is_named_only_when_yt_dlp_cannot_find_it() {
        let managed = Path::new("C:\\Users\\me\\AppData\\Roaming\\dev.claws.downloader\\bin\\deno.exe");
        // Claws' own copy is not on PATH, so it is named by its path, whatever else exists.
        assert_eq!(js_runtime_args(Some(managed), true, true), ["--js-runtimes".to_string(), format!("deno:{}", managed.display())]);
        assert!(js_runtime_args(None, true, true).is_empty(), "yt-dlp finds a Deno on PATH unaided");
        assert_eq!(js_runtime_args(None, false, true), ["--js-runtimes", "node"]);
        assert!(js_runtime_args(None, false, false).is_empty());
    }

    async fn deno_install(app: &TestApp, server: &Server, archive_max: u64) -> Result<()> {
        let (archive, sums) = (server.url("/archive"), server.url("/sums"));
        let release = Release { tool: "deno", archive_url: &archive, sums_url: &sums, archive_name: DENO_ZIP_NAME, archive_max };
        install_deno_from(app.handle(), &release, quick()).await
    }

    fn deno_server(archive: Vec<u8>, sums: Vec<u8>) -> Server {
        serve(vec![("/sums", Route::Body(sums)), ("/archive", Route::Body(archive))])
    }

    #[test]
    fn tool_install_deno_is_verified_installed_and_then_named_to_yt_dlp() {
        let rig = rig("ti-deno");
        let before = common_args(rig.app.handle(), &Settings::default());
        assert!(!before.iter().any(|a| a.starts_with("deno:")));

        let archive = zip_bytes(&[("deno.exe", b"new-deno"), ("nested/deno.exe", b"not-this-one")]);
        let server = deno_server(archive.clone(), deno_sums(&archive, DENO_BUILD_PATH));
        tauri::async_runtime::block_on(within(20, "install", deno_install(&rig.app, &server, MIB))).unwrap();
        assert_eq!(std::fs::read(rig.bin.join("deno.exe")).unwrap(), b"new-deno");
        assert_eq!(names(&rig.bin), vec!["deno.exe", "yt-dlp"], "no staging or partial file may remain");

        let after = common_args(rig.app.handle(), &Settings::default());
        let flag = after.iter().position(|a| a == "--js-runtimes").expect("the runtime is named");
        assert_eq!(after[flag + 1], format!("deno:{}", rig.bin.join("deno.exe").display()));
    }

    #[test]
    fn tool_install_deno_bad_or_foreign_checksum_keeps_the_previous_copy() {
        let rig = rig("ti-deno-sum");
        std::fs::write(rig.bin.join("deno.exe"), b"old-deno").unwrap();
        let archive = zip_bytes(&[("deno.exe", b"new-deno")]);
        // A checksum of something else, then a correct checksum that describes another file.
        for sums in [deno_sums(b"something else", DENO_BUILD_PATH), deno_sums(&archive, "C:\\a\\deno-aarch64-pc-windows-msvc.zip")] {
            let server = deno_server(archive.clone(), sums);
            let error = tauri::async_runtime::block_on(within(20, "install", deno_install(&rig.app, &server, MIB))).unwrap_err();
            assert!(error.to_string().contains("checksum"), "{error}");
            assert_eq!(std::fs::read(rig.bin.join("deno.exe")).unwrap(), b"old-deno");
            assert_eq!(names(&rig.bin), vec!["deno.exe", "yt-dlp"]);
        }
    }

    #[test]
    fn tool_install_deno_archive_without_the_executable_changes_nothing() {
        let rig = rig("ti-deno-empty");
        let archive = zip_bytes(&[("nested/deno.exe", b"wrong place"), ("README.md", b"hello")]);
        let server = deno_server(archive.clone(), deno_sums(&archive, DENO_BUILD_PATH));
        let error = tauri::async_runtime::block_on(within(20, "install", deno_install(&rig.app, &server, MIB))).unwrap_err();
        assert!(error.to_string().contains("deno.exe"), "{error}");
        only_known_good(&rig);
    }

    #[test]
    fn recover_bin_removes_abandoned_install_attempts() {
        let dir = temp_dir("recover-attempt");
        std::fs::create_dir_all(dir.join(".install-abc").join("yt-dlp.new")).unwrap();
        std::fs::write(dir.join(".install-abc").join("archive.zip.part"), b"x").unwrap();
        std::fs::create_dir_all(dir.join("yt-dlp")).unwrap();
        recover_bin(&dir);
        assert_eq!(names(&dir), vec!["yt-dlp"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn replace_all_restores_earlier_files_when_a_backup_cannot_be_made() {
        let dir = temp_dir("set-backup");
        for name in ["ffmpeg.exe", "ffprobe.exe"] {
            std::fs::write(dir.join(name), format!("old-{name}")).unwrap();
            std::fs::write(dir.join(format!("{name}.part")), format!("new-{name}")).unwrap();
        }
        // A non-empty folder where the second backup belongs: renaming onto it fails after
        // the first file was already replaced.
        std::fs::create_dir_all(dir.join("ffprobe.exe.old")).unwrap();
        std::fs::write(dir.join("ffprobe.exe.old").join("keep"), b"x").unwrap();
        let pairs = vec![
            (dir.join("ffmpeg.exe.part"), dir.join("ffmpeg.exe")),
            (dir.join("ffprobe.exe.part"), dir.join("ffprobe.exe")),
        ];
        assert!(replace_all(&pairs).is_err());
        assert_eq!(std::fs::read(dir.join("ffmpeg.exe")).unwrap(), b"old-ffmpeg.exe");
        assert_eq!(std::fs::read(dir.join("ffprobe.exe")).unwrap(), b"old-ffprobe.exe");
        assert!(!dir.join("ffmpeg.exe.old").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}

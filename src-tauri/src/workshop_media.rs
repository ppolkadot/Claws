//! Turning a video Wallpaper Engine project into a playable MP4 or a GIF.
//!
//! Only a project whose own `project.json` declares a video is converted: scene, web,
//! application and preset projects are only ever archived, never opened or run. The
//! declared file is read from inside the checked project, with every path part verified as
//! a plain local name, and FFmpeg is given that one file as its only input.
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::process::Command;

use crate::{
    bounded_io::read_tail,
    download_files::is_link,
    downloader::kill_tree,
    encoder::Encoder,
    export_progress::{FfmpegProgress, ProgressParser},
};

/// Largest `project.json` Claws reads. Real ones are a few kilobytes.
pub const MAX_PROJECT_JSON: u64 = 64 * 1024;
/// Kept from FFmpeg's error stream for the "Show the full message" detail.
const DIAGNOSTIC_BYTES: usize = 16 * 1024;
const DIAGNOSTIC_LINES: usize = 40;
/// Containers a Wallpaper Engine video project may declare. Anything else is not converted.
const MEDIA_EXTENSIONS: [&str; 7] = ["mp4", "m4v", "mov", "webm", "mkv", "avi", "wmv"];
/// Rounds each side down to an even number, which H.264 and GIF require, without scaling the
/// picture up: a source with an odd width or height loses at most one pixel per side.
const EVEN_SIZE: &str = "scale=trunc(iw/2)*2:trunc(ih/2)*2";
/// What the user chose for a video project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaFormat {
    Mp4,
    Gif,
}

impl MediaFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Gif => "gif",
        }
    }

    /// Upper bound on the output against the source video's size: H.264 keeps roughly the
    /// same bytes, while a GIF of a long clip can grow several times over.
    pub fn space_factor(self) -> u64 {
        match self {
            Self::Mp4 => 2,
            Self::Gif => 5,
        }
    }
}

/// The project's declared video, after every path part was checked.
#[derive(Debug)]
pub struct VideoProject {
    pub path: PathBuf,
    pub bytes: u64,
}

/// Why a project cannot be converted.
#[derive(Debug, PartialEq, Eq)]
pub enum MediaError {
    /// Not a video project, or it names no file Claws can convert.
    Unsupported,
    /// `project.json` or the file it names is a link, an unsafe path, or outside the project.
    Unsafe,
}

/// Reads the project's own metadata and returns the one video file it declares.
pub fn video_file(project: &Path) -> Result<VideoProject, MediaError> {
    let manifest = project.join("project.json");
    let meta = fs::symlink_metadata(&manifest).map_err(|_| MediaError::Unsupported)?;
    if is_link(&meta) {
        return Err(MediaError::Unsafe);
    }
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_PROJECT_JSON {
        return Err(MediaError::Unsupported);
    }
    let text = read_manifest(&manifest)?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|_| MediaError::Unsupported)?;
    let kind = value.get("type").and_then(|value| value.as_str()).unwrap_or_default();
    if !kind.eq_ignore_ascii_case("video") {
        return Err(MediaError::Unsupported);
    }
    let declared = value.get("file").and_then(|value| value.as_str()).ok_or(MediaError::Unsupported)?;
    let parts = plain_parts(declared).ok_or(MediaError::Unsafe)?;
    let extension = parts.last().and_then(|name| name.rsplit_once('.')).map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    if !MEDIA_EXTENSIONS.contains(&extension.as_str()) {
        return Err(MediaError::Unsupported);
    }

    let root = project.canonicalize().map_err(|_| MediaError::Unsupported)?;
    let mut path = root.clone();
    let last = parts.len() - 1;
    for (index, part) in parts.iter().enumerate() {
        path.push(part);
        let meta = fs::symlink_metadata(&path).map_err(|_| MediaError::Unsupported)?;
        if is_link(&meta) {
            return Err(MediaError::Unsafe);
        }
        if index == last {
            if !meta.is_file() || meta.len() == 0 {
                return Err(MediaError::Unsupported);
            }
        } else if !meta.is_dir() {
            return Err(MediaError::Unsupported);
        }
    }
    let resolved = path.canonicalize().map_err(|_| MediaError::Unsupported)?;
    if !resolved.starts_with(&root) {
        return Err(MediaError::Unsafe);
    }
    let bytes = fs::metadata(&resolved).map_err(|_| MediaError::Unsupported)?.len();
    Ok(VideoProject { path: resolved, bytes })
}

/// Reads the manifest through a handle that refuses a link swapped in after the check.
fn read_manifest(path: &Path) -> Result<String, MediaError> {
    let file = crate::workshop::open_plain(path).map_err(|_| MediaError::Unsafe)?;
    let mut bytes = Vec::new();
    file.take(MAX_PROJECT_JSON + 1).read_to_end(&mut bytes).map_err(|_| MediaError::Unsupported)?;
    if bytes.len() as u64 > MAX_PROJECT_JSON {
        return Err(MediaError::Unsupported);
    }
    let text = String::from_utf8(bytes).map_err(|_| MediaError::Unsupported)?;
    Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_string())
}

/// The manifest's file name as safe path parts: plain local names only, no drive prefix,
/// rooted or UNC path, traversal, alternate data stream or trailing dot/space.
fn plain_parts(file: &str) -> Option<Vec<&str>> {
    if file.is_empty() || file.len() > 1024 {
        return None;
    }
    let mut parts = Vec::new();
    for part in file.split(['\\', '/']) {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.len() > 255
            || part.contains(':')
            || part.chars().any(char::is_control)
            || part.ends_with(['.', ' '])
            || reserved_name(part)
        {
            return None;
        }
        parts.push(part);
    }
    (!parts.is_empty()).then_some(parts)
}

/// Windows device names, which never name a file inside the project.
fn reserved_name(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).to_ascii_uppercase();
    match stem.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        _ => (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit(),
    }
}

/// The demuxer FFmpeg must use for the declared container, so a file that is really a
/// playlist or concat script cannot make FFmpeg open other local files.
fn demuxer(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "mp4" | "m4v" | "mov" => Some("mov"),
        "webm" | "mkv" => Some("matroska"),
        "avi" => Some("avi"),
        "wmv" => Some("asf"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------
// What the video is

/// What ffprobe reports about the declared video file.
#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    pub video_codec: String,
    pub pix_fmt: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// Seconds, from the container; `None` when it does not say.
    pub duration: Option<f64>,
    pub audio_codec: Option<String>,
    /// ffprobe's container name, such as `mov,mp4,m4a,3gp,3g2,mj2`.
    pub format_name: String,
}

/// ffprobe's frame rate fields are fractions such as `30000/1001`.
fn parse_rate(text: &str) -> Option<f64> {
    let (num, den) = text.split_once('/').unwrap_or((text, "1"));
    let (num, den) = (num.trim().parse::<f64>().ok()?, den.trim().parse::<f64>().ok()?);
    (den > 0.0 && num > 0.0 && (num / den).is_finite()).then_some(num / den)
}

/// Reads ffprobe's JSON (`-of json` with streams and the container's format).
pub fn parse_probe(json: &[u8]) -> Option<Probe> {
    let value: serde_json::Value = serde_json::from_slice(json).ok()?;
    let streams = value.get("streams")?.as_array()?;
    let text = |stream: &serde_json::Value, key: &str| stream.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let video = streams.iter().find(|s| s.get("codec_type").and_then(|v| v.as_str()) == Some("video"))?;
    let audio = streams.iter().find(|s| s.get("codec_type").and_then(|v| v.as_str()) == Some("audio"));
    let size = |key: &str| video.get(key).and_then(|v| v.as_u64()).and_then(|v| u32::try_from(v).ok()).filter(|v| *v > 0);
    let fps = text(video, "avg_frame_rate").and_then(|r| parse_rate(&r)).or_else(|| text(video, "r_frame_rate").and_then(|r| parse_rate(&r))).unwrap_or(30.0);
    let format = value.get("format");
    Some(Probe {
        video_codec: text(video, "codec_name")?,
        pix_fmt: text(video, "pix_fmt").unwrap_or_default(),
        width: size("width")?,
        height: size("height")?,
        fps,
        duration: format
            .and_then(|f| f.get("duration"))
            .and_then(|d| d.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| d.as_f64()))
            .filter(|d| d.is_finite() && *d > 0.0),
        audio_codec: audio.and_then(|a| text(a, "codec_name")),
        format_name: format.and_then(|f| f.get("format_name")).and_then(|v| v.as_str()).unwrap_or_default().to_string(),
    })
}

impl Probe {
    /// Whether the original file can be saved as it is: 8-bit H.264 in an MP4 or M4V that
    /// plays in every common player, with no sound or sound in AAC or MP3. Anything else
    /// (HEVC, VP9, 10-bit, another container, odd audio) is converted instead.
    pub fn can_copy(&self, path: &Path) -> bool {
        let extension = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        self.video_codec == "h264"
            && matches!(self.pix_fmt.as_str(), "yuv420p" | "yuvj420p")
            && matches!(extension.as_str(), "mp4" | "m4v")
            && self.format_name.split(',').any(|name| name == "mp4")
            && self.audio_codec.as_deref().is_none_or(|codec| matches!(codec, "aac" | "mp3"))
    }
}

/// Runs ffprobe on the declared file: one local input, its own container as the demuxer, and
/// a bounded reply. `None` when ffprobe cannot read it.
pub async fn probe(ffprobe: &Path, input: &Path) -> Option<Probe> {
    let mut command = Command::new(ffprobe);
    command.args(["-v", "error", "-hide_banner", "-of", "json", "-protocol_whitelist", "file"]);
    command.args([
        "-show_entries",
        "stream=codec_type,codec_name,pix_fmt,width,height,avg_frame_rate,r_frame_rate:format=format_name,duration",
    ]);
    if let Some(demuxer) = demuxer(input) {
        command.args(["-f", demuxer]);
    }
    command.arg("-i").arg(input).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take()?;
    let read = crate::bounded_io::read_capped(stdout, 256 * 1024);
    let (bytes, status) = match tokio::time::timeout(Duration::from_secs(30), async { tokio::join!(read, child.wait()) }).await {
        Ok((Ok(bytes), Ok(status))) => (bytes, status),
        _ => {
            let _ = child.kill().await;
            return None;
        }
    };
    status.success().then(|| parse_probe(&bytes)).flatten()
}

// ---------------------------------------------------------------------------------------
// Saving the original

/// How a copy ended.
#[derive(Debug, PartialEq, Eq)]
pub enum CopyOutcome {
    Done,
    Cancelled,
}

const COPY_CHUNK: usize = 4 * 1024 * 1024;

/// Copies the source into a fresh partial file, reporting `(written, total)` after each chunk.
/// `on_finish` runs once every byte is written, before the file is flushed to disk, which can take
/// seconds for a large file and is a different step from copying.
/// The source is opened without following a link swapped in after the check, the size is
/// verified at the end, and the partial file is removed on cancel and on every error.
pub fn copy_original(
    source: &Path,
    partial: &Path,
    mut report: impl FnMut(u64, u64),
    on_finish: impl FnOnce(),
    cancelled: impl Fn() -> bool,
) -> std::io::Result<CopyOutcome> {
    use std::io::{Read as _, Write as _};
    let result = (|| -> std::io::Result<CopyOutcome> {
        let mut input = crate::workshop::open_plain(source)?;
        let total = input.metadata()?.len();
        if total == 0 {
            return Err(std::io::Error::other("The wallpaper's video is empty."));
        }
        let mut output = fs::OpenOptions::new().write(true).create_new(true).open(partial)?;
        let mut buffer = vec![0u8; COPY_CHUNK];
        let mut written = 0u64;
        report(0, total);
        loop {
            if cancelled() {
                return Ok(CopyOutcome::Cancelled);
            }
            let read = input.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read])?;
            written += read as u64;
            report(written, total);
        }
        on_finish();
        output.flush()?;
        output.sync_all()?;
        let size = fs::metadata(partial)?.len();
        if written != total || size != total {
            return Err(std::io::Error::other("The copy came out a different size than the original, so it was not saved."));
        }
        Ok(CopyOutcome::Done)
    })();
    if !matches!(result, Ok(CopyOutcome::Done)) {
        let _ = fs::remove_file(partial);
    }
    result
}

// ---------------------------------------------------------------------------------------
// Converting

/// A GIF of a long clip is slow to make and huge: it is cut to this length by default, at most
/// the maximum, and never taller than the cap.
pub const GIF_DEFAULT_SECONDS: u32 = 30;
pub const GIF_MAX_SECONDS: u32 = 60;
pub const GIF_MAX_HEIGHT: u32 = 720;

/// The length a GIF is cut to: the request, within one second and the maximum.
pub fn gif_length(requested: Option<u32>) -> u32 {
    requested.unwrap_or(GIF_DEFAULT_SECONDS).clamp(1, GIF_MAX_SECONDS)
}

/// A palette per frame keeps GIF colours faithful, at a frame rate that stays small enough.
/// A palette for the whole clip (`stats_mode=full` or `diff`) only arrives at the end, so
/// FFmpeg would hold every decoded frame in memory until then: about 1.2 GB for 20 seconds
/// of 720p, growing with length and resolution. Per-frame palettes stream in bounded memory.
/// The picture keeps its shape and is never scaled up: taller than 720 pixels it is brought to
/// 720, and every side is rounded down to the even number the encoder needs.
fn gif_filter() -> String {
    let cap = GIF_MAX_HEIGHT;
    format!(
        "fps=15,scale=w='trunc(iw*min(ih,{cap})/ih/2)*2':h='trunc(min(ih,{cap})/2)*2':flags=lanczos,split[s0][s1];[s0]palettegen=stats_mode=single[p];[s1][p]paletteuse=new=1:dither=sierra2_4a"
    )
}

/// Everything that decides how one conversion runs.
#[derive(Debug, Clone)]
pub struct ConvertPlan {
    pub format: MediaFormat,
    /// For MP4: the encoder, chosen for the source's size.
    pub encoder: Encoder,
    /// The source's size and frame rate, which size the encoder's settings.
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Cut the output after this many seconds (GIF).
    pub length: Option<u32>,
}

impl ConvertPlan {
    /// A plan with software encoding and no cut, for a source whose size is not known.
    #[cfg(test)]
    pub fn basic(format: MediaFormat) -> Self {
        Self { format, encoder: Encoder::X264, width: 1920, height: 1080, fps: 30, length: None }
    }

    /// The seconds of source the output covers, for the progress fraction.
    pub fn output_seconds(&self, source: Option<f64>) -> Option<f64> {
        match (source, self.length) {
            (Some(duration), Some(length)) => Some(duration.min(f64::from(length))),
            (Some(duration), None) => Some(duration),
            (None, Some(length)) => Some(f64::from(length)),
            (None, None) => None,
        }
    }
}

/// FFmpeg's arguments for one export: the local file as the only input, bounded to the
/// `file` protocol and its declared container, mapped streams only, a fresh output that is
/// never overwritten, and machine-readable progress on stdout.
pub fn export_args(input: &Path, output: &Path, plan: &ConvertPlan) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostats".into(),
        "-progress".into(),
        "pipe:1".into(),
        // Refuse an existing file outright; the partial name is fresh.
        "-n".into(),
    ];
    if let Some(demuxer) = demuxer(input) {
        args.push("-f".into());
        args.push(demuxer.into());
    }
    args.extend(
        [
            // Only a local file may be read: no network protocols, playlists or extras.
            "-protocol_whitelist",
            "file",
            "-i",
            &input.to_string_lossy(),
            "-map",
            "0:v:0",
            "-sn",
            "-dn",
        ]
        .map(str::to_string),
    );
    if let Some(length) = plan.length {
        args.extend(["-t".to_string(), length.to_string()]);
    }
    match plan.format {
        MediaFormat::Mp4 => {
            // H.264 and AAC in MP4 play in every common player and editor.
            args.extend(["-map", "0:a:0?"].map(str::to_string));
            args.extend(plan.encoder.args(plan.width, plan.height, plan.fps));
            args.extend(["-vf", EVEN_SIZE, "-c:a", "aac", "-b:a", "192k", "-ac", "2", "-movflags", "+faststart", "-f", "mp4"].map(str::to_string));
        }
        MediaFormat::Gif => {
            args.extend(["-an", "-vf", &gif_filter(), "-loop", "0", "-f", "gif"].map(str::to_string));
        }
    }
    args.push(output.to_string_lossy().into_owned());
    args
}

/// A conversion that ended without producing a file.
#[derive(Debug, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    pub detail: String,
}

/// How a conversion ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Cancelled,
}

/// A conversion that reports no progress for this long is stopped.
const STALL_LIMIT: Duration = Duration::from_secs(10 * 60);
/// No conversion of a wallpaper takes longer than this, however well it progresses.
const CONVERT_LIMIT: Duration = Duration::from_secs(6 * 60 * 60);

/// Reads FFmpeg's `-progress` lines, hands each finished block on, and notes when the output
/// time last moved forward.
async fn read_progress<R: tokio::io::AsyncRead + Unpin>(stdout: R, on_progress: &(dyn Fn(&FfmpegProgress) + Sync), advanced: &Mutex<Instant>) {
    let mut reader = tokio::io::BufReader::new(stdout);
    let mut line = Vec::new();
    let mut parser = ProgressParser::default();
    let mut best = 0u64;
    while let Some(truncated) = crate::bounded_io::read_line_capped(&mut reader, &mut line, 4096).await {
        if truncated {
            continue;
        }
        let text = String::from_utf8_lossy(&line).into_owned();
        if let Some(block) = parser.push(&text) {
            if block.out_time_us.is_some_and(|time| time > best) {
                best = block.out_time_us.unwrap_or(best);
                *advanced.lock().unwrap() = Instant::now();
            }
            on_progress(&block);
        }
    }
}

/// Runs FFmpeg once. `on_spawn` records the process so Cancel and shutdown can stop it and
/// returns whether to continue; `cancelled` is checked before starting and after waiting;
/// `on_progress` gets each block of FFmpeg's progress report.
pub async fn convert(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    plan: &ConvertPlan,
    mut on_spawn: impl FnMut(Option<u32>) -> bool,
    cancelled: &(dyn Fn() -> bool + Sync),
    on_progress: &(dyn Fn(&FfmpegProgress) + Sync),
) -> Result<Outcome, Failure> {
    let failure = |message: &str, detail: String| Failure { message: message.to_string(), detail };
    if cancelled() {
        return Ok(Outcome::Cancelled);
    }
    let mut command = Command::new(ffmpeg);
    command
        .args(export_args(input, output, plan))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command
        .spawn()
        .map_err(|_| failure("Couldn't start FFmpeg to convert this wallpaper.", String::new()))?;
    let pid = child.id();
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let errors = tokio::spawn(read_tail(stderr, DIAGNOSTIC_BYTES));
    if !on_spawn(pid) {
        if let Some(pid) = pid {
            kill_tree(pid, true);
        }
        let _ = child.kill().await;
        let _ = errors.await;
        let _ = fs::remove_file(output);
        return Ok(Outcome::Cancelled);
    }
    let started = Instant::now();
    let advanced = Mutex::new(started);
    let supervise = async {
        loop {
            tokio::select! {
                status = child.wait() => return Ok(status),
                _ = tokio::time::sleep(Duration::from_secs(2)) => {
                    if advanced.lock().unwrap().elapsed() > STALL_LIMIT || started.elapsed() > CONVERT_LIMIT {
                        return Err(());
                    }
                }
            }
        }
    };
    let (_, waited) = tokio::join!(read_progress(stdout, on_progress, &advanced), supervise);
    let status = match waited {
        Ok(status) => status.map_err(|_| {
            let _ = fs::remove_file(output);
            failure("FFmpeg couldn't convert this wallpaper's video. Save it as a ZIP instead.", String::new())
        })?,
        Err(()) => {
            if let Some(pid) = pid {
                kill_tree(pid, true);
            }
            let _ = child.kill().await;
            let _ = errors.await;
            let _ = fs::remove_file(output);
            return Err(failure("The video stopped converting, so it was not saved. Try again, or save it as a ZIP.", String::new()));
        }
    };
    let detail = errors.await.map(|bytes| tail_text(&bytes)).unwrap_or_default();
    if cancelled() {
        let _ = fs::remove_file(output);
        return Ok(Outcome::Cancelled);
    }
    if !status.success() {
        let _ = fs::remove_file(output);
        return Err(failure("FFmpeg couldn't convert this wallpaper's video. Save it as a ZIP instead.", detail));
    }
    Ok(Outcome::Done)
}

/// The last few non-blank lines of FFmpeg's error output.
pub(crate) fn tail_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(DIAGNOSTIC_LINES)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn project(tag: &str) -> PathBuf {
        let dir = crate::download_tests::temp_dir(tag);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn manifest(dir: &Path, body: &str) {
        std::fs::write(dir.join("project.json"), body).unwrap();
    }

    #[test]
    fn reads_the_video_a_video_project_declares() {
        let dir = project("media-video");
        std::fs::create_dir_all(dir.join("media")).unwrap();
        std::fs::write(dir.join("media/clip.mp4"), vec![1u8; 64]).unwrap();
        manifest(&dir, r#"{"type":"video","file":"media/clip.mp4","title":"Loop"}"#);
        let found = video_file(&dir).unwrap();
        assert_eq!(found.bytes, 64);
        assert_eq!(found.path, dir.join("media/clip.mp4").canonicalize().unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn accepts_a_manifest_with_a_byte_order_mark() {
        let dir = project("media-bom");
        std::fs::write(dir.join("loop.webm"), vec![1u8; 8]).unwrap();
        manifest(&dir, "\u{feff}{\"type\":\"Video\",\"file\":\"loop.webm\"}");
        assert!(video_file(&dir).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn leaves_scene_web_application_and_preset_projects_as_archives() {
        for kind in ["scene", "web", "application", "preset", ""] {
            let dir = project("media-kind");
            std::fs::write(dir.join("payload.mp4"), vec![1u8; 8]).unwrap();
            manifest(&dir, &format!(r#"{{"type":"{kind}","file":"payload.mp4"}}"#));
            assert_eq!(video_file(&dir).unwrap_err(), MediaError::Unsupported, "type {kind:?}");
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn refuses_a_project_without_a_usable_video_file() {
        let dir = project("media-nofile");
        manifest(&dir, r#"{"type":"video","file":"missing.mp4"}"#);
        assert_eq!(video_file(&dir).unwrap_err(), MediaError::Unsupported);
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        manifest(&dir, r#"{"type":"video","file":"notes.txt"}"#);
        assert_eq!(video_file(&dir).unwrap_err(), MediaError::Unsupported);
        std::fs::write(dir.join("empty.mp4"), b"").unwrap();
        manifest(&dir, r#"{"type":"video","file":"empty.mp4"}"#);
        assert_eq!(video_file(&dir).unwrap_err(), MediaError::Unsupported);
        std::fs::write(dir.join("video.mp4"), b"x").unwrap();
        manifest(&dir, r#"{"type":"video"}"#);
        assert_eq!(video_file(&dir).unwrap_err(), MediaError::Unsupported);
        manifest(&dir, "not json");
        assert_eq!(video_file(&dir).unwrap_err(), MediaError::Unsupported);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refuses_paths_that_leave_the_project_or_name_something_else() {
        let dir = project("media-paths");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/video.mp4"), b"x").unwrap();
        std::fs::write(dir.join("outside.mp4"), b"x").unwrap();
        for bad in [
            "",
            ".",
            "..",
            "../outside.mp4",
            "sub/../outside.mp4",
            "C:\\Windows\\video.mp4",
            "C:/Windows/video.mp4",
            "\\outside.mp4",
            "/outside.mp4",
            "\\\\server\\share\\video.mp4",
            "//server/share/video.mp4",
            "sub/video.mp4:stream",
            "sub\\video.mp4:stream",
            "video.mp4.",
            "video.mp4 ",
            "con.mp4",
            "sub/COM3.mp4",
            "sub//video.mp4",
            "sub/",
        ] {
            manifest(&dir, &format!(r#"{{"type":"video","file":{}}}"#, serde_json::to_string(bad).unwrap()));
            assert_eq!(video_file(&dir).unwrap_err(), MediaError::Unsafe, "{bad:?}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn refuses_a_manifest_link_even_when_it_points_inside() {
        let dir = project("media-manifest-link");
        std::fs::write(dir.join("real.json"), r#"{"type":"video","file":"clip.mp4"}"#).unwrap();
        std::fs::write(dir.join("clip.mp4"), b"x").unwrap();
        let link = dir.join("project.json");
        if std::os::windows::fs::symlink_file(dir.join("real.json"), &link).is_err() {
            std::fs::remove_dir_all(dir).unwrap();
            eprintln!("skipped: creating a symlink needs Developer Mode or elevation");
            return;
        }
        assert_eq!(video_file(&dir).unwrap_err(), MediaError::Unsafe);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn refuses_a_junction_inside_the_declared_path() {
        let dir = project("media-junction");
        let outside = project("media-junction-target");
        std::fs::write(outside.join("video.mp4"), b"x").unwrap();
        let status = Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(dir.join("linked"))
            .arg(&outside)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        if !status.success() {
            std::fs::remove_dir_all(&dir).unwrap();
            std::fs::remove_dir_all(&outside).unwrap();
            eprintln!("skipped: the test user cannot create a junction");
            return;
        }
        manifest(&dir, r#"{"type":"video","file":"linked/video.mp4"}"#);
        assert_eq!(video_file(&dir).unwrap_err(), MediaError::Unsafe);
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn refuses_an_oversized_manifest_before_reading_it() {
        let dir = project("media-big-manifest");
        std::fs::write(dir.join("clip.mp4"), b"x").unwrap();
        manifest(&dir, &format!(r#"{{"type":"video","file":"clip.mp4","pad":"{}"}}"#, "x".repeat(MAX_PROJECT_JSON as usize)));
        assert_eq!(video_file(&dir).unwrap_err(), MediaError::Unsupported);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ffmpeg_gets_one_local_input_and_the_right_encoder() {
        let mp4 = export_args(Path::new(r"C:\p\clip.mp4"), Path::new(r"C:\out\x.partial"), &ConvertPlan::basic(MediaFormat::Mp4));
        let inputs = mp4.iter().filter(|arg| *arg == "-i").count();
        assert_eq!(inputs, 1);
        assert!(mp4.iter().any(|arg| arg == "-n"));
        assert!(mp4.windows(2).any(|pair| pair == ["-f", "mov"]));
        assert!(mp4.windows(2).any(|pair| pair == ["-protocol_whitelist", "file"]));
        assert!(mp4.windows(2).any(|pair| pair == ["-map", "0:v:0"]));
        assert!(mp4.windows(2).any(|pair| pair == ["-map", "0:a:0?"]));
        assert!(mp4.windows(2).any(|pair| pair == ["-c:v", "libx264"]));
        assert!(mp4.windows(2).any(|pair| pair == ["-pix_fmt", "yuv420p"]));
        assert!(mp4.windows(2).any(|pair| pair == ["-c:a", "aac"]));
        assert!(mp4.windows(2).any(|pair| pair == ["-f", "mp4"]));
        assert_eq!(mp4.last().unwrap(), r"C:\out\x.partial");

        let gif = export_args(Path::new(r"C:\p\clip.webm"), Path::new(r"C:\out\x.partial"), &ConvertPlan::basic(MediaFormat::Gif));
        assert!(gif.iter().any(|arg| arg == "-an"));
        assert!(gif.windows(2).any(|pair| pair == ["-f", "gif"]));
        assert!(gif.iter().any(|arg| arg.contains("palettegen") && arg.contains("paletteuse")));
        // The palette must stream per frame; a whole-clip palette buffers every frame.
        assert!(gif.iter().any(|arg| arg.contains("stats_mode=single") && arg.contains("new=1")));
        assert!(!gif.iter().any(|arg| arg.contains("stats_mode=diff") || arg.contains("stats_mode=full")));
        assert!(!gif.windows(2).any(|pair| pair == ["-c:a", "aac"]));
    }

    #[test]
    fn extension_and_space_estimate_follow_the_format() {
        assert_eq!(MediaFormat::Mp4.extension(), "mp4");
        assert_eq!(MediaFormat::Gif.extension(), "gif");
        assert_eq!(MediaFormat::Mp4.space_factor(), 2);
        assert_eq!(MediaFormat::Gif.space_factor(), 5);
        assert_eq!(demuxer(Path::new("clip.mp4")), Some("mov"));
        assert_eq!(demuxer(Path::new("clip.M4V")), Some("mov"));
        assert_eq!(demuxer(Path::new("clip.webm")), Some("matroska"));
        assert_eq!(demuxer(Path::new("clip.mkv")), Some("matroska"));
        assert_eq!(demuxer(Path::new("clip.avi")), Some("avi"));
        assert_eq!(demuxer(Path::new("clip.wmv")), Some("asf"));
        assert_eq!(demuxer(Path::new("clip.unknown")), None);
    }

    /// Live: needs ffmpeg and ffprobe (`CLAWS_TEST_FFMPEG_DIR`, or the copies Claws manages).
    #[tokio::test]
    async fn converts_a_generated_clip_and_probes_the_output() {
        let Some(tools) = crate::download_tests::ffmpeg_dir() else {
            eprintln!("skipped: needs ffmpeg and ffprobe");
            return;
        };
        let ffmpeg = tools.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
        let ffprobe = tools.join(format!("ffprobe{}", std::env::consts::EXE_SUFFIX));
        let dir = crate::download_tests::temp_dir("workshop-media-live");

        let make = |name: &str, sound: bool, size: &str| {
            let path = dir.join(name);
            let mut command = Command::new(&ffmpeg);
            command.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", &format!("testsrc2=r=10:d=1:s={size}")]);
            if sound {
                command.args(["-f", "lavfi", "-i", "sine=f=440:d=1", "-c:a", "aac", "-ac", "2"]);
            }
            command.args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-shortest"]).arg(&path);
            let output = command.output().unwrap();
            assert!(output.status.success(), "fixture: {}", String::from_utf8_lossy(&output.stderr));
            path
        };
        let with_sound = make("sound.mp4", true, "64x48");
        let silent = make("silent.mp4", false, "64x48");
        let probe = |file: &Path| -> Vec<serde_json::Value> {
            let output = Command::new(&ffprobe)
                .args(["-v", "error", "-show_entries", "stream=codec_type,codec_name,width,height", "-of", "json"])
                .arg(file)
                .output()
                .unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["streams"].as_array().unwrap().clone()
        };

        let mp4 = dir.join("out.mp4");
        let done = convert(&ffmpeg, &with_sound, &mp4, &ConvertPlan::basic(MediaFormat::Mp4), |_| true, &|| false, &|_| {}).await.unwrap();
        assert_eq!(done, Outcome::Done);
        assert!(mp4.is_file());
        let streams = probe(&mp4);
        let video = streams.iter().find(|s| s["codec_type"] == "video").expect("a video stream");
        assert_eq!(video["codec_name"], "h264");
        assert_eq!((video["width"].as_u64(), video["height"].as_u64()), (Some(64), Some(48)));
        let audio = streams.iter().find(|s| s["codec_type"] == "audio").expect("the audio is kept");
        assert_eq!(audio["codec_name"], "aac");

        // A silent source stays silent: no invented or empty audio track.
        let quiet = dir.join("quiet.mp4");
        convert(&ffmpeg, &silent, &quiet, &ConvertPlan::basic(MediaFormat::Mp4), |_| true, &|| false, &|_| {}).await.unwrap();
        assert!(!probe(&quiet).iter().any(|s| s["codec_type"] == "audio"));

        let gif = dir.join("out.gif");
        let done = convert(&ffmpeg, &with_sound, &gif, &ConvertPlan::basic(MediaFormat::Gif), |_| true, &|| false, &|_| {}).await.unwrap();
        assert_eq!(done, Outcome::Done);
        let streams = probe(&gif);
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0]["codec_name"], "gif");
        assert_eq!((streams[0]["width"].as_u64(), streams[0]["height"].as_u64()), (Some(64), Some(48)));
        assert!(fs::metadata(&gif).unwrap().len() > 0);

        // An odd-sized source stays its own resolution, rounded down to encoder-safe even
        // sides: no upscaling, and a portrait clip keeps its shape.
        let odd = make("odd.mp4", false, "65x49");
        let adjusted = dir.join("odd-out.mp4");
        convert(&ffmpeg, &odd, &adjusted, &ConvertPlan::basic(MediaFormat::Mp4), |_| true, &|| false, &|_| {}).await.unwrap();
        let streams = probe(&adjusted);
        let video = streams.iter().find(|s| s["codec_type"] == "video").expect("a video stream");
        assert_eq!((video["width"].as_u64(), video["height"].as_u64()), (Some(64), Some(48)));

        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Live: needs ffmpeg. A source FFmpeg cannot open fails with bounded, useful detail.
    #[tokio::test]
    async fn a_broken_source_fails_without_leaving_a_video() {
        let Some(tools) = crate::download_tests::ffmpeg_dir() else {
            eprintln!("skipped: needs ffmpeg");
            return;
        };
        let ffmpeg = tools.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
        let dir = crate::download_tests::temp_dir("workshop-media-broken");
        let source = dir.join("broken.mp4");
        std::fs::write(&source, b"not a video").unwrap();
        let output = dir.join("out.partial");
        let failure = convert(&ffmpeg, &source, &output, &ConvertPlan::basic(MediaFormat::Mp4), |_| true, &|| false, &|_| {}).await.unwrap_err();
        assert!(failure.message.contains("couldn't convert"), "{}", failure.message);
        assert!(failure.detail.len() <= DIAGNOSTIC_BYTES);
        assert!(!output.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_cancelled_export_starts_nothing() {
        let dir = crate::download_tests::temp_dir("workshop-media-cancel");
        let output = dir.join("out.partial");
        // No ffmpeg is needed: cancellation is checked before the process starts.
        let done = convert(Path::new("ffmpeg-not-used"), Path::new("input.mp4"), &output, &ConvertPlan::basic(MediaFormat::Gif), |_| true, &|| true, &|_| {})
            .await
            .unwrap();
        assert_eq!(done, Outcome::Cancelled);
        assert!(!output.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Live: a cancel after the process starts is reported as cancelled, not as a failure.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_running_export_stopped_by_its_process_ends_cancelled() {
        let Some(tools) = crate::download_tests::ffmpeg_dir() else {
            eprintln!("skipped: needs ffmpeg");
            return;
        };
        let ffmpeg = tools.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
        let dir = crate::download_tests::temp_dir("workshop-media-kill");
        let source = dir.join("source.mp4");
        let made = Command::new(&ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=r=10:d=30:s=64x48", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&source)
            .status()
            .unwrap();
        assert!(made.success());
        let output = dir.join("out.partial");
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let at_spawn = cancelled.clone();
        let at_check = cancelled.clone();
        let done = convert(
            &ffmpeg,
            &source,
            &output,
            &ConvertPlan::basic(MediaFormat::Mp4),
            |pid| {
                at_spawn.store(true, std::sync::atomic::Ordering::SeqCst);
                if let Some(pid) = pid {
                    kill_tree(pid, true);
                }
                true
            },
            &move || at_check.load(std::sync::atomic::Ordering::SeqCst),
            &|_| {},
        )
        .await
        .unwrap();
        assert_eq!(done, Outcome::Cancelled, "a killed run is a cancellation");
        assert!(!output.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    // ----- what the video is -----

    const H264_AAC: &str = r#"{"streams":[
        {"codec_name":"h264","codec_type":"video","width":3840,"height":2160,"pix_fmt":"yuv420p","r_frame_rate":"60/1","avg_frame_rate":"60/1"},
        {"codec_name":"aac","codec_type":"audio"}],
        "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"294.350000"}}"#;
    const H264_SILENT: &str = r#"{"streams":[
        {"codec_name":"h264","codec_type":"video","width":1920,"height":1080,"pix_fmt":"yuv420p","r_frame_rate":"30000/1001","avg_frame_rate":"30000/1001"}],
        "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"59.9"}}"#;
    const H264_10BIT: &str = r#"{"streams":[
        {"codec_name":"h264","codec_type":"video","width":1920,"height":1080,"pix_fmt":"yuv420p10le","avg_frame_rate":"30/1"},
        {"codec_name":"aac","codec_type":"audio"}],"format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"10"}}"#;
    const HEVC: &str = r#"{"streams":[
        {"codec_name":"hevc","codec_type":"video","width":1920,"height":1080,"pix_fmt":"yuv420p","avg_frame_rate":"30/1"}],
        "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"10"}}"#;
    const VP9_WEBM: &str = r#"{"streams":[
        {"codec_name":"vp9","codec_type":"video","width":1280,"height":720,"pix_fmt":"yuv420p","avg_frame_rate":"30/1"},
        {"codec_name":"opus","codec_type":"audio"}],"format":{"format_name":"matroska,webm","duration":"5.5"}}"#;

    #[test]
    fn reads_what_ffprobe_says_about_the_video() {
        let probe = parse_probe(H264_AAC.as_bytes()).unwrap();
        assert_eq!((probe.video_codec.as_str(), probe.width, probe.height, probe.fps), ("h264", 3840, 2160, 60.0));
        assert_eq!((probe.audio_codec.as_deref(), probe.duration), (Some("aac"), Some(294.35)));
        let ntsc = parse_probe(H264_SILENT.as_bytes()).unwrap();
        assert!((ntsc.fps - 29.97).abs() < 0.01, "{}", ntsc.fps);
        assert_eq!(ntsc.audio_codec, None, "a silent video has no audio stream");
        assert_eq!(parse_probe(VP9_WEBM.as_bytes()).unwrap().format_name, "matroska,webm");
        // Garbage, no video stream, a zero-sized picture and a missing duration.
        assert_eq!(parse_probe(b"not json"), None);
        assert_eq!(parse_probe(br#"{"streams":[{"codec_type":"audio","codec_name":"aac"}]}"#), None);
        assert_eq!(parse_probe(br#"{"streams":[{"codec_type":"video","codec_name":"h264","width":0,"height":0}]}"#), None);
        let no_duration = parse_probe(br#"{"streams":[{"codec_type":"video","codec_name":"h264","width":64,"height":48,"pix_fmt":"yuv420p"}],"format":{"format_name":"mov,mp4"}}"#).unwrap();
        assert_eq!((no_duration.duration, no_duration.fps), (None, 30.0));
    }

    #[test]
    fn only_eight_bit_h264_mp4_is_copied() {
        let ok = |json: &str, name: &str| parse_probe(json.as_bytes()).unwrap().can_copy(Path::new(name));
        assert!(ok(H264_AAC, "clip.mp4"));
        assert!(ok(H264_SILENT, "Mountains In The Clouds [4K] .mp4"), "a space before the extension and brackets are only a name");
        assert!(ok(H264_SILENT, "\u{5c71}\u{306e}\u{98a8}\u{666f}.MP4"));
        assert!(ok(H264_AAC, "clip.m4v"));
        assert!(!ok(H264_10BIT, "clip.mp4"), "10-bit H.264 does not play everywhere");
        assert!(!ok(HEVC, "clip.mp4"));
        assert!(!ok(VP9_WEBM, "clip.webm"));
        assert!(!ok(H264_AAC, "clip.mkv"), "another container is converted");
        assert!(!ok(H264_AAC, "clip.mov"));
        let opus = H264_AAC.replace("\"aac\"", "\"opus\"");
        assert!(!ok(&opus, "clip.mp4"), "sound that players may not handle is converted");
    }

    // ----- saving the original -----

    #[test]
    fn copies_the_original_byte_for_byte_and_reports_every_step() {
        let dir = project("media-copy");
        let source = dir.join("Mountains In The Clouds [4K] .mp4");
        let bytes: Vec<u8> = (0..(COPY_CHUNK * 2 + 123)).map(|i| (i % 251) as u8).collect();
        std::fs::write(&source, &bytes).unwrap();
        let partial = dir.join("out.partial");
        let mut steps = Vec::new();
        let done = copy_original(&source, &partial, |written, total| steps.push((written, total)), || {}, || false).unwrap();
        assert_eq!(done, CopyOutcome::Done);
        assert_eq!(std::fs::read(&partial).unwrap(), bytes);
        assert_eq!(steps.first(), Some(&(0, bytes.len() as u64)), "starts at zero");
        assert_eq!(steps.last(), Some(&(bytes.len() as u64, bytes.len() as u64)), "ends at the full size");
        assert!(steps.windows(2).all(|pair| pair[0].0 < pair[1].0), "never backwards");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_cancelled_or_failed_copy_leaves_no_partial_file() {
        let dir = project("media-copy-cancel");
        let source = dir.join("clip.mp4");
        std::fs::write(&source, vec![7u8; COPY_CHUNK + 5]).unwrap();
        let partial = dir.join("out.partial");
        // Cancelled after the first chunk.
        let calls = std::cell::Cell::new(0);
        let done = copy_original(&source, &partial, |_, _| {}, || {}, || {
            calls.set(calls.get() + 1);
            calls.get() > 1
        })
        .unwrap();
        assert_eq!(done, CopyOutcome::Cancelled);
        assert!(!partial.exists());
        // An empty source, a missing source and an existing destination all fail cleanly.
        let empty = dir.join("empty.mp4");
        std::fs::write(&empty, b"").unwrap();
        assert!(copy_original(&empty, &partial, |_, _| {}, || {}, || false).is_err());
        assert!(!partial.exists());
        assert!(copy_original(&dir.join("missing.mp4"), &partial, |_, _| {}, || {}, || false).is_err());
        std::fs::write(&partial, b"someone else's file").unwrap();
        assert!(copy_original(&source, &partial, |_, _| {}, || {}, || false).is_err(), "never overwrites");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_linked_source_is_not_copied() {
        let dir = project("media-copy-link");
        std::fs::write(dir.join("real.mp4"), b"x").unwrap();
        let link = dir.join("link.mp4");
        if std::os::windows::fs::symlink_file(dir.join("real.mp4"), &link).is_err() {
            std::fs::remove_dir_all(dir).unwrap();
            eprintln!("skipped: creating a symlink needs Developer Mode or elevation");
            return;
        }
        let partial = dir.join("out.partial");
        assert!(copy_original(&link, &partial, |_, _| {}, || {}, || false).is_err());
        assert!(!partial.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    // ----- GIF limits and the encoder plan -----

    #[test]
    fn a_gif_is_cut_to_thirty_seconds_unless_asked_for_up_to_sixty() {
        assert_eq!((gif_length(None), gif_length(Some(10)), gif_length(Some(60))), (30, 10, 60));
        assert_eq!((gif_length(Some(0)), gif_length(Some(61)), gif_length(Some(u32::MAX))), (1, 60, 60));
        let mut plan = ConvertPlan::basic(MediaFormat::Gif);
        plan.length = Some(gif_length(None));
        let args = export_args(Path::new("clip.mp4"), Path::new("x.partial"), &plan);
        let at = args.iter().position(|arg| arg == "-t").expect("the cut");
        assert_eq!(args[at + 1], "30");
        assert!(at > args.iter().position(|arg| arg == "-i").unwrap(), "an output option, after the input");
        let mp4 = export_args(Path::new("clip.mp4"), Path::new("x.partial"), &ConvertPlan::basic(MediaFormat::Mp4));
        assert!(!mp4.iter().any(|arg| arg == "-t"), "an MP4 conversion keeps the whole video");
        assert_eq!(plan.output_seconds(Some(200.0)), Some(30.0));
        assert_eq!(plan.output_seconds(Some(12.0)), Some(12.0));
        assert_eq!(ConvertPlan::basic(MediaFormat::Mp4).output_seconds(Some(200.0)), Some(200.0));
    }

    #[test]
    fn a_gif_is_capped_at_720_high_and_never_scaled_up() {
        let filter = gif_filter();
        assert!(filter.contains("min(ih,720)"), "{filter}");
        assert!(!filter.contains("scale=trunc(iw/2)*2"), "the old uncapped scale is gone");
        // The expression, worked by hand: width follows height, both rounded down to even.
        let scaled = |iw: f64, ih: f64| {
            let h = ih.min(720.0);
            (((iw * h / ih) / 2.0).floor() * 2.0, (h / 2.0).floor() * 2.0)
        };
        assert_eq!(scaled(3840.0, 2160.0), (1280.0, 720.0));
        assert_eq!(scaled(1920.0, 1080.0), (1280.0, 720.0));
        assert_eq!(scaled(1280.0, 720.0), (1280.0, 720.0));
        assert_eq!(scaled(640.0, 360.0), (640.0, 360.0), "smaller sources stay as they are");
        assert_eq!(scaled(65.0, 49.0), (64.0, 48.0), "odd sides lose a pixel");
        assert_eq!(scaled(3440.0, 1440.0), (1720.0, 720.0));
    }

    #[test]
    fn an_mp4_conversion_asks_the_chosen_encoder_and_reports_progress_on_stdout() {
        let mut plan = ConvertPlan::basic(MediaFormat::Mp4);
        plan.encoder = Encoder::Nvenc;
        plan.width = 3840;
        plan.height = 2160;
        plan.fps = 60;
        let args = export_args(Path::new(r"C:\p\clip.mkv"), Path::new(r"C:\out\x.partial"), &plan);
        assert!(args.windows(2).any(|pair| pair == ["-c:v", "h264_nvenc"]));
        assert!(args.windows(2).any(|pair| pair == ["-level", "5.2"]));
        assert!(args.windows(2).any(|pair| pair == ["-progress", "pipe:1"]));
        assert!(args.iter().any(|arg| arg == "-nostats"));
        assert!(args.windows(2).any(|pair| pair == ["-vf", EVEN_SIZE]), "source resolution is kept, only made even");
        // The default plan is what it always was.
        let basic = export_args(Path::new("a.mp4"), Path::new("b.partial"), &ConvertPlan::basic(MediaFormat::Mp4));
        assert!(basic.windows(2).any(|pair| pair == ["-crf", "20"]));
    }

    // ----- live: copy and convert the owner's kinds of file -----

    /// Live: needs ffmpeg and ffprobe. Generates the kinds of file a library holds (a name with
    /// brackets and a space before the extension, CJK, a silent video, a 10-bit one, HEVC), then
    /// checks which are copied and that the copy is identical.
    #[tokio::test]
    async fn copies_what_can_be_copied_and_converts_the_rest_from_generated_fixtures() {
        let Some(tools) = crate::download_tests::ffmpeg_dir() else {
            eprintln!("skipped: needs ffmpeg and ffprobe");
            return;
        };
        let ffmpeg = tools.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
        let ffprobe = tools.join(format!("ffprobe{}", std::env::consts::EXE_SUFFIX));
        let dir = crate::download_tests::temp_dir("workshop-media-copy-live");
        let make = |name: &str, codec: &[&str], pix: &str, sound: bool| {
            let path = dir.join(name);
            let mut command = Command::new(&ffmpeg);
            command.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=r=10:d=1:s=128x96"]);
            if sound {
                command.args(["-f", "lavfi", "-i", "sine=f=440:d=1", "-c:a", "aac", "-ac", "2"]);
            }
            command.args(codec).args(["-pix_fmt", pix, "-shortest"]).arg(&path);
            let output = command.output().unwrap();
            assert!(output.status.success(), "fixture {name}: {}", String::from_utf8_lossy(&output.stderr));
            path
        };
        let cases = [
            (make("Mountains In The Clouds [4K] .mp4", &["-c:v", "libx264"], "yuv420p", true), true),
            (make("\u{5c71}\u{306e}\u{98a8}\u{666f} silent.mp4", &["-c:v", "libx264"], "yuv420p", false), true),
            (make("ten bit.mp4", &["-c:v", "libx264", "-profile:v", "high10"], "yuv420p10le", false), false),
            (make("hevc.mp4", &["-c:v", "libx265", "-x265-params", "log-level=none"], "yuv420p", false), false),
        ];
        for (path, copyable) in cases {
            let probe = probe(&ffprobe, &path).await.unwrap_or_else(|| panic!("ffprobe reads {}", path.display()));
            assert_eq!(probe.can_copy(&path), copyable, "{}: {probe:?}", path.display());
            assert!((probe.duration.unwrap() - 1.0).abs() < 0.2);
            if copyable {
                let partial = dir.join("copy.partial");
                assert_eq!(copy_original(&path, &partial, |_, _| {}, || {}, || false).unwrap(), CopyOutcome::Done);
                assert_eq!(std::fs::read(&partial).unwrap(), std::fs::read(&path).unwrap(), "{}", path.display());
                std::fs::remove_file(&partial).unwrap();
            } else {
                let out = dir.join("converted.partial");
                let seen = std::sync::Mutex::new(Vec::new());
                let done = convert(&ffmpeg, &path, &out, &ConvertPlan::basic(MediaFormat::Mp4), |_| true, &|| false, &|block| seen.lock().unwrap().push(block.clone())).await;
                assert_eq!(done, Ok(Outcome::Done), "{}", path.display());
                let converted = probe_of(&ffprobe, &out).await;
                assert_eq!((converted.video_codec.as_str(), converted.pix_fmt.as_str()), ("h264", "yuv420p"));
                assert!(seen.lock().unwrap().last().is_some_and(|block| block.end), "FFmpeg reported the end of its work");
                std::fs::remove_file(&out).unwrap();
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    async fn probe_of(ffprobe: &Path, path: &Path) -> Probe {
        // A converted file is a plain `.partial` name: probe it as MP4 by giving ffprobe a copy
        // under the right extension.
        let named = path.with_extension("mp4");
        std::fs::copy(path, &named).unwrap();
        let probe = probe(ffprobe, &named).await.expect("the converted file is readable");
        std::fs::remove_file(&named).unwrap();
        probe
    }

    /// Live: a GIF made with the cut and the cap, from a source taller than 720.
    #[tokio::test]
    async fn a_gif_of_a_tall_clip_is_cut_and_capped() {
        let Some(tools) = crate::download_tests::ffmpeg_dir() else {
            eprintln!("skipped: needs ffmpeg and ffprobe");
            return;
        };
        let ffmpeg = tools.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
        let ffprobe = tools.join(format!("ffprobe{}", std::env::consts::EXE_SUFFIX));
        let dir = crate::download_tests::temp_dir("workshop-media-gif-live");
        let source = dir.join("tall.mp4");
        let made = Command::new(&ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=r=10:d=6:s=1920x1080", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&source)
            .status()
            .unwrap();
        assert!(made.success());
        let mut plan = ConvertPlan::basic(MediaFormat::Gif);
        plan.length = Some(2);
        let out = dir.join("out.gif");
        let done = convert(&ffmpeg, &source, &out, &plan, |_| true, &|| false, &|_| {}).await;
        assert_eq!(done, Ok(Outcome::Done));
        let output = Command::new(&ffprobe)
            .args(["-v", "error", "-show_entries", "stream=width,height,nb_read_frames:format=duration", "-count_frames", "-of", "json"])
            .arg(&out)
            .output()
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let stream = &json["streams"][0];
        assert_eq!((stream["width"].as_u64(), stream["height"].as_u64()), (Some(1280), Some(720)), "capped, same shape");
        let frames: u64 = stream["nb_read_frames"].as_str().unwrap().parse().unwrap();
        assert!((28..=32).contains(&frames), "2 seconds at 15 fps, not the whole 6: {frames}");
        std::fs::remove_dir_all(dir).unwrap();
    }

}

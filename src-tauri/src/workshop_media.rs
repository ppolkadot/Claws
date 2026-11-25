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
    time::Duration,
};
use tokio::process::Command;

use crate::{
    bounded_io::read_tail,
    download_files::is_link,
    downloader::kill_tree,
};

/// Largest `project.json` Claws reads. Real ones are a few kilobytes.
pub const MAX_PROJECT_JSON: u64 = 64 * 1024;
/// A wallpaper video is short; a conversion that runs this long is stopped.
const CONVERT_LIMIT: Duration = Duration::from_secs(30 * 60);
/// Kept from FFmpeg's error stream for the "Show the full message" detail.
const DIAGNOSTIC_BYTES: usize = 16 * 1024;
const DIAGNOSTIC_LINES: usize = 40;
/// Containers a Wallpaper Engine video project may declare. Anything else is not converted.
const MEDIA_EXTENSIONS: [&str; 7] = ["mp4", "m4v", "mov", "webm", "mkv", "avi", "wmv"];
/// Rounds each side down to an even number, which H.264 and GIF require, without scaling the
/// picture up: a source with an odd width or height loses at most one pixel per side.
const EVEN_SIZE: &str = "scale=trunc(iw/2)*2:trunc(ih/2)*2";
/// A palette per frame keeps GIF colours faithful, at a frame rate that stays small enough.
/// A palette for the whole clip (`stats_mode=full` or `diff`) only arrives at the end, so
/// FFmpeg would hold every decoded frame in memory until then: about 1.2 GB for 20 seconds
/// of 720p, growing with length and resolution. Per-frame palettes stream in bounded memory.
const GIF_FILTER: &str = "fps=15,scale=trunc(iw/2)*2:trunc(ih/2)*2:flags=lanczos,split[s0][s1];[s0]palettegen=stats_mode=single[p];[s1][p]paletteuse=new=1:dither=sierra2_4a";

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

/// FFmpeg's arguments for one export: the local file as the only input, bounded to the
/// `file` protocol and its declared container, mapped streams only, and a fresh output that
/// is never overwritten.
pub fn export_args(input: &Path, output: &Path, format: MediaFormat) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-nostats".into(),
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
    match format {
        MediaFormat::Mp4 => {
            // H.264 and AAC in MP4 play in every common player and editor.
            args.extend(
                [
                    "-map", "0:a:0?", "-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p", "-vf", EVEN_SIZE, "-c:a",
                    "aac", "-b:a", "192k", "-ac", "2", "-movflags", "+faststart", "-f", "mp4",
                ]
                .map(str::to_string),
            );
        }
        MediaFormat::Gif => {
            args.extend(["-an", "-vf", GIF_FILTER, "-loop", "0", "-f", "gif"].map(str::to_string));
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

/// Runs FFmpeg once. `on_spawn` records the process so Cancel and shutdown can stop it and
/// returns whether to continue; `cancelled` is checked before starting and after waiting.
pub async fn convert(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    format: MediaFormat,
    mut on_spawn: impl FnMut(Option<u32>) -> bool,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Outcome, Failure> {
    let failure = |message: &str, detail: String| Failure { message: message.to_string(), detail };
    if cancelled() {
        return Ok(Outcome::Cancelled);
    }
    let mut command = Command::new(ffmpeg);
    command
        .args(export_args(input, output, format))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command
        .spawn()
        .map_err(|_| failure("Couldn't start FFmpeg to convert this wallpaper.", String::new()))?;
    let pid = child.id();
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
    let status = match tokio::time::timeout(CONVERT_LIMIT, child.wait()).await {
        Ok(status) => status.map_err(|_| {
            let _ = fs::remove_file(output);
            failure("FFmpeg couldn't convert this wallpaper's video. Save it as a ZIP instead.", String::new())
        })?,
        Err(_) => {
            if let Some(pid) = pid {
                kill_tree(pid, true);
            }
            let _ = child.kill().await;
            let _ = errors.await;
            let _ = fs::remove_file(output);
            return Err(failure("The video took too long to convert. Try ZIP instead, or a different wallpaper.", String::new()));
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
fn tail_text(bytes: &[u8]) -> String {
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
        let mp4 = export_args(Path::new(r"C:\p\clip.mp4"), Path::new(r"C:\out\x.partial"), MediaFormat::Mp4);
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

        let gif = export_args(Path::new(r"C:\p\clip.webm"), Path::new(r"C:\out\x.partial"), MediaFormat::Gif);
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
        let done = convert(&ffmpeg, &with_sound, &mp4, MediaFormat::Mp4, |_| true, &|| false).await.unwrap();
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
        convert(&ffmpeg, &silent, &quiet, MediaFormat::Mp4, |_| true, &|| false).await.unwrap();
        assert!(!probe(&quiet).iter().any(|s| s["codec_type"] == "audio"));

        let gif = dir.join("out.gif");
        let done = convert(&ffmpeg, &with_sound, &gif, MediaFormat::Gif, |_| true, &|| false).await.unwrap();
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
        convert(&ffmpeg, &odd, &adjusted, MediaFormat::Mp4, |_| true, &|| false).await.unwrap();
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
        let failure = convert(&ffmpeg, &source, &output, MediaFormat::Mp4, |_| true, &|| false).await.unwrap_err();
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
        let done = convert(Path::new("ffmpeg-not-used"), Path::new("input.mp4"), &output, MediaFormat::Gif, |_| true, &|| true)
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
            MediaFormat::Mp4,
            |pid| {
                at_spawn.store(true, std::sync::atomic::Ordering::SeqCst);
                if let Some(pid) = pid {
                    kill_tree(pid, true);
                }
                true
            },
            &move || at_check.load(std::sync::atomic::Ordering::SeqCst),
        )
        .await
        .unwrap();
        assert_eq!(done, Outcome::Cancelled, "a killed run is a cancellation");
        assert!(!output.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

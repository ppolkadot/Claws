//! Process and file-system tests for the download pipeline. Unit tests of pure logic live
//! next to that logic; these exercise real files and real child processes.
//!
//! Process tests need the yt-dlp stand-in, built with
//! `cargo build --features test-tools --bin claws-fixture-tool`. Without it they skip.
//!
//! Policy tests run the real yt-dlp against `scripts/dev/download-fixtures.py` on
//! localhost, with the exact arguments `download_policy` builds. They need Python and
//! yt-dlp (`CLAWS_TEST_YTDLP`, or the copy Claws manages in %APPDATA%); without them they
//! skip. No live site is contacted.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::download_policy::{self, classify, FailureKind, JobExecution, Source};
use crate::downloader;
use crate::settings::Settings;

pub(crate) fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("claws-{tag}-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub(crate) fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The stand-in tool, if it has been built next to the test binary.
pub(crate) fn fixture_tool() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("CLAWS_FIXTURE_TOOL") {
        return Some(PathBuf::from(path));
    }
    let exe = std::env::current_exe().ok()?;
    // target/debug/deps/<test>.exe -> target/debug/claws-fixture-tool.exe
    let candidate = exe.parent()?.parent()?.join(format!("claws-fixture-tool{}", std::env::consts::EXE_SUFFIX));
    candidate.is_file().then_some(candidate)
}

#[test]
fn legacy_cleanup_leaves_unrelated_files() {
    let dir = temp_dir("prefix");
    for name in ["clip.mp4.part", "clip.mp4.partnership", "clip.mp4.part-Frag2", "clip.mp4.ytdl", "clip.mp4"] {
        std::fs::write(dir.join(name), b"x").unwrap();
    }
    downloader::remove_partials_now(&[dir.join("clip.mp4").to_string_lossy().into_owned()]);
    assert_eq!(names(&dir), vec!["clip.mp4", "clip.mp4.partnership"]);
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(windows)]
#[test]
fn kill_tree_stops_the_whole_process_tree() {
    use std::io::{BufRead, BufReader};
    let Some(tool) = fixture_tool() else {
        eprintln!("skipped: build claws-fixture-tool with --features test-tools");
        return;
    };
    let dir = temp_dir("tree");
    let mut child = std::process::Command::new(&tool)
        .args(["-P", &dir.to_string_lossy(), "--scenario", "tree"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let grandchild: u32 = lines
        .find_map(|l| l.ok()?.strip_prefix("CHILD|").and_then(|p| p.parse().ok()))
        .expect("the tool reports its child");
    downloader::kill_tree(child.id(), true);
    let _ = child.wait();
    let alive = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {grandchild}"), "/NH"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&grandchild.to_string()))
        .unwrap_or(false);
    assert!(!alive, "the grandchild {grandchild} survived");
    let _ = std::fs::remove_dir_all(dir);
}

pub(crate) fn ytdlp() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("CLAWS_TEST_YTDLP") {
        return Some(PathBuf::from(path));
    }
    let appdata = std::env::var_os("APPDATA")?;
    let managed = Path::new(&appdata).join("dev.claws.downloader").join("bin").join("yt-dlp").join("yt-dlp.exe");
    managed.is_file().then_some(managed)
}

struct Fixtures {
    child: Child,
    base: String,
}

impl Drop for Fixtures {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn fixtures(extra: &[&str]) -> Option<Fixtures> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("scripts").join("dev").join("download-fixtures.py");
    let mut child = Command::new("python")
        .arg(script)
        .args(["--size-mib", "2", "--segment-kib", "128"])
        .args(extra)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut line = String::new();
    BufReader::new(child.stdout.take()?).read_line(&mut line).ok()?;
    let value: serde_json::Value = serde_json::from_str(&line).ok()?;
    Some(Fixtures { base: value["baseUrl"].as_str()?.to_string(), child })
}

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    elapsed: Duration,
}

/// How a test points yt-dlp at a fixture.
enum Media<'a> {
    /// Through yt-dlp's generic extractor, which fetches the URL once to inspect it.
    Url(&'a str),
    /// From an info file naming the media URL, the way Claws starts a download from its
    /// cached probe, so the first request already is the media transfer.
    Direct(&'a str, &'a str),
}

/// Runs yt-dlp with the policy's arguments for a plain video download into `workspace`.
/// `overrides` go after the policy's own options (yt-dlp takes the last value), for
/// tests that must finish faster than the real retry budget allows.
fn run_policy(ytdlp: &Path, workspace: &Path, media: Media<'_>, overrides: &[&str], kill_after: Option<Duration>) -> Run {
    let execution = JobExecution {
        filename_template: "payload.%(ext)s".into(),
        embed_metadata: false,
        ..JobExecution::freeze(&Settings { output_dir: "unused".into(), ..Settings::default() }, false)
    };
    // Stand-in for tools::common_args; no ffmpeg is assumed here, so no fixups.
    let common = vec!["--no-colors".into(), "--ignore-config".into(), "--fixup".into(), "never".into()];
    let options = crate::downloader::DownloadOptions::default();
    let info = workspace.join("info.json");
    let source = match media {
        Media::Url(url) => Source::Url(url),
        Media::Direct(url, ext) => {
            let value = serde_json::json!({
                "id": "fixture", "title": "fixture", "ext": ext, "url": url, "protocol": "http",
                "format_id": "0", "extractor": "generic", "extractor_key": "Generic", "webpage_url": url
            });
            std::fs::write(&info, value.to_string()).unwrap();
            Source::InfoJson(&info)
        }
    };
    let mut args = download_policy::download_args(common, &options, &execution, workspace, source);
    let tail = args.split_off(args.len() - 2);
    args.extend(overrides.iter().map(|s| s.to_string()));
    args.extend(tail);
    let started = Instant::now();
    let mut child = Command::new(ytdlp).args(&args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || std::io::read_to_string(stdout).unwrap_or_default());
    let err = std::thread::spawn(move || std::io::read_to_string(stderr).unwrap_or_default());
    if let Some(after) = kill_after {
        std::thread::sleep(after);
        downloader::kill_tree(child.id(), true);
    }
    let status = child.wait().unwrap();
    let _ = std::fs::remove_file(&info);
    Run { code: status.code(), stdout: out.join().unwrap(), stderr: err.join().unwrap(), elapsed: started.elapsed() }
}

fn sha256_file(path: &Path) -> String {
    let digest = Sha256::digest(std::fs::read(path).unwrap());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn expected_digest(base: &str, route: &str) -> String {
    // A tiny HTTP GET without another dependency: the fixture answers with plain text.
    use std::io::{Read, Write};
    let address = base.trim_start_matches("http://");
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    write!(stream, "GET /sha256/{route} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response.rsplit("\r\n\r\n").next().unwrap().trim().to_string()
}

macro_rules! require {
    ($e:expr) => {
        match $e {
            Some(v) => v,
            None => {
                eprintln!("skipped: needs Python and yt-dlp");
                return;
            }
        }
    };
}

#[test]
fn a_missing_hls_fragment_fails_and_publishes_nothing() {
    let ytdlp = require!(ytdlp());
    let fx = require!(fixtures(&[]));
    let work = temp_dir("missing-fragment");
    // The policy's 10 fragment retries with backoff take minutes against a permanent 404;
    // the abort-on-missing behaviour under test is the same with one.
    let url = format!("{}/hls/list.m3u8", fx.base);
    let run = run_policy(&ytdlp, &work, Media::Url(&url), &["--fragment-retries", "1"], None);
    assert_ne!(run.code, Some(0), "a hole in the stream must not count as success");
    assert!(!run.stdout.contains("CLAWSDONE|"), "no finished file may be reported");
    assert!(!work.join("payload.mp4").exists());
    assert_eq!(classify(&run.stderr, false), FailureKind::MissingFragment, "{}", run.stderr);
    let _ = std::fs::remove_dir_all(work);
}

#[test]
fn a_complete_hls_stream_still_downloads() {
    let ytdlp = require!(ytdlp());
    let fx = require!(fixtures(&[]));
    let work = temp_dir("hls-ok");
    let url = format!("{}/hls-ok/list.m3u8", fx.base);
    let run = run_policy(&ytdlp, &work, Media::Url(&url), &[], None);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(run.stdout.contains("CLAWSDONE|"));
    let _ = std::fs::remove_dir_all(work);
}

#[test]
fn transient_503s_recover_with_backoff_and_intact_bytes() {
    let ytdlp = require!(ytdlp());
    let fx = require!(fixtures(&["--unstable-failures", "2"]));
    let work = temp_dir("unstable");
    let url = format!("{}/unstable/file.bin", fx.base);
    let run = run_policy(&ytdlp, &work, Media::Direct(&url, "bin"), &[], None);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(sha256_file(&work.join("payload.bin")), expected_digest(&fx.base, "range"));
    // exp=1:30 sleeps about 1 s, then 2 s, before the third attempt succeeds.
    assert!(run.elapsed >= Duration::from_secs(2), "backoff should have waited, took {:?}", run.elapsed);
    let _ = std::fs::remove_dir_all(work);
}

#[test]
fn a_dropped_connection_resumes_without_corrupting_the_file() {
    let ytdlp = require!(ytdlp());
    let fx = require!(fixtures(&[]));
    let work = temp_dir("reset");
    let url = format!("{}/reset/file.bin", fx.base);
    let run = run_policy(&ytdlp, &work, Media::Direct(&url, "bin"), &[], None);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(sha256_file(&work.join("payload.bin")), expected_digest(&fx.base, "range"));
    let _ = std::fs::remove_dir_all(work);
}

#[test]
fn an_expired_link_is_classified_for_one_refresh() {
    let ytdlp = require!(ytdlp());
    let fx = require!(fixtures(&[]));
    let work = temp_dir("expired");
    let url = format!("{}/expired/file.bin", fx.base);
    let run = run_policy(&ytdlp, &work, Media::Direct(&url, "bin"), &["--retries", "1"], None);
    assert_ne!(run.code, Some(0));
    let kind = classify(&run.stderr, false);
    assert!(kind.refresh_helps(), "{kind:?}: {}", run.stderr);
    let _ = std::fs::remove_dir_all(work);
}

#[test]
fn cancel_interrupts_a_retry_backoff_promptly() {
    let ytdlp = require!(ytdlp());
    // Fails far longer than the test waits, so yt-dlp is sleeping between retries.
    let fx = require!(fixtures(&["--unstable-failures", "1000"]));
    let work = temp_dir("backoff-cancel");
    let url = format!("{}/unstable/file.bin", fx.base);
    let run = run_policy(&ytdlp, &work, Media::Direct(&url, "bin"), &[], Some(Duration::from_secs(5)));
    assert_ne!(run.code, Some(0));
    assert!(run.elapsed < Duration::from_secs(9), "the kill should end the backoff, took {:?}", run.elapsed);
    let _ = std::fs::remove_dir_all(work);
}

#[test]
fn progress_output_is_capped_near_five_lines_a_second() {
    let ytdlp = require!(ytdlp());
    let fx = require!(fixtures(&["--size-mib", "48"]));
    let url = format!("{}/range/file.bin", fx.base);
    let count = |overrides: &[&str]| {
        let work = temp_dir("cadence");
        let run = run_policy(&ytdlp, &work, Media::Direct(&url, "bin"), overrides, None);
        let _ = std::fs::remove_dir_all(work);
        assert_eq!(run.code, Some(0), "{}", run.stderr);
        let lines: Vec<String> = run.stdout.lines().filter(|l| l.starts_with("CLAWS|")).map(String::from).collect();
        assert!(lines.last().is_some_and(|l| l.starts_with("CLAWS|finished")), "the final line must come through");
        (lines.len(), run.elapsed)
    };
    let (capped, elapsed) = count(&[]);
    let (uncapped, _) = count(&["--progress-delta", "0"]);
    // At most five a second over the run, plus the first and final lines.
    let ceiling = (elapsed.as_secs_f64() * 5.0).ceil() as usize + 2;
    assert!(capped <= ceiling, "{capped} lines in {elapsed:?}");
    assert!(uncapped > capped, "without the delta yt-dlp printed {uncapped}, with it {capped}");
}

#[test]
fn real_progress_output_is_weighted_by_bytes() {
    use crate::transfer_progress::TransferProgress;
    let ytdlp = require!(ytdlp());
    let fx = require!(fixtures(&["--size-mib", "6"]));
    let work = temp_dir("weighted");
    let url = format!("{}/range/file.bin", fx.base);
    let run = run_policy(&ytdlp, &work, Media::Direct(&url, "bin"), &[], None);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let mut progress = TransferProgress::new("video");
    let mut last = None;
    for line in run.stdout.lines() {
        if let Some(rest) = line.strip_prefix("CLAWSFMT|") {
            progress.set_plan(rest);
        } else if let Some(rest) = line.strip_prefix("CLAWS|") {
            let fields: Vec<&str> = rest.splitn(10, '|').collect();
            if let Some(snapshot) = progress.update(&fields) {
                last = Some(snapshot);
            }
        }
    }
    let last = last.expect("progress lines were parsed");
    assert_eq!(last.downloaded, 6 * 1024 * 1024);
    assert_eq!(last.total, Some(6 * 1024 * 1024));
    assert_eq!(last.fraction, Some(0.99), "full only after publication");
    let _ = std::fs::remove_dir_all(work);
}

/// The format yt-dlp selects from `formats` with `selector`, without downloading.
fn selected_format(ytdlp: &Path, formats: serde_json::Value, selector: &str) -> String {
    let dir = temp_dir("select");
    let info = dir.join("info.json");
    let value = serde_json::json!({
        "id": "x", "title": "x", "ext": "mp4", "extractor": "generic", "extractor_key": "Generic",
        "webpage_url": "https://example.com/v", "formats": formats
    });
    std::fs::write(&info, value.to_string()).unwrap();
    let output = Command::new(ytdlp)
        .args(["--ignore-config", "--no-warnings", "--simulate", "--load-info-json"])
        .arg(&info)
        .args(["-f", selector, "--print", "%(format_id)s"])
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(dir);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn yt_dlp_picks_the_matching_audio_stream() {
    use crate::download_policy::{audio_selector, AudioSelection::*};
    let ytdlp = require!(ytdlp());
    let audio = |id: &str, ext: &str, acodec: &str, abr: f64| {
        serde_json::json!({"format_id": id, "ext": ext, "acodec": acodec, "vcodec": "none", "abr": abr, "url": format!("https://example.com/{id}"), "protocol": "https"})
    };
    let combined = serde_json::json!({"format_id": "18", "ext": "mp4", "acodec": "mp4a.40.2", "vcodec": "avc1.42001E", "abr": 96, "height": 360, "url": "https://example.com/18", "protocol": "https"});
    let youtube_like = serde_json::json!([audio("140", "m4a", "mp4a.40.2", 128.0), audio("251", "webm", "opus", 160.0), combined.clone()]);
    let pick = |formats: &serde_json::Value, format: &str, selection| selected_format(&ytdlp, formats.clone(), audio_selector(format, selection));

    // Best keeps yt-dlp's own choice, whatever the output format.
    assert_eq!(pick(&youtube_like, "m4a", Best), "251");
    assert_eq!(pick(&youtube_like, "m4a", PreferMatching), "140");
    assert_eq!(pick(&youtube_like, "opus", PreferMatching), "251");
    // No matching stream (no MP3 or FLAC here): the same as Best, still audio only.
    assert_eq!(pick(&youtube_like, "mp3", PreferMatching), "251");
    assert_eq!(pick(&youtube_like, "flac", PreferMatching), "251");
    // AAC in another container is still preferred for M4A.
    let aac_in_ts = serde_json::json!([audio("hls-aac", "mp4", "aac", 96.0), audio("251", "webm", "opus", 160.0)]);
    assert_eq!(pick(&aac_in_ts, "m4a", PreferMatching), "hls-aac");
    let with_mp3 = serde_json::json!([audio("mp3-128", "mp3", "mp3", 128.0), audio("251", "webm", "opus", 160.0)]);
    assert_eq!(pick(&with_mp3, "mp3", PreferMatching), "mp3-128");
    // A stream with no codec information is never taken for a codec match.
    let unknown_codec = serde_json::json!([{"format_id": "a1", "ext": "m4a", "vcodec": "none", "url": "https://example.com/a1", "protocol": "https"}, audio("251", "webm", "opus", 160.0)]);
    assert_eq!(pick(&unknown_codec, "opus", PreferMatching), "251");
    let only_combined = serde_json::json!([combined]);
    assert_eq!(pick(&only_combined, "m4a", PreferMatching), pick(&only_combined, "m4a", Best));
}

// Short synthetic clips made with the installed FFmpeg, described in a made-up info file
// the way a site describes its streams, read through file links, saved by the real yt-dlp
// with the exact arguments the policy builds, and inspected with ffprobe. They need
// yt-dlp, ffmpeg and ffprobe (the copies Claws manages, or CLAWS_TEST_YTDLP and
// CLAWS_TEST_FFMPEG_DIR); without them they skip.

/// The folder holding ffmpeg and ffprobe: CLAWS_TEST_FFMPEG_DIR, or the one Claws manages.
fn ffmpeg_dir() -> Option<PathBuf> {
    let dir = match std::env::var_os("CLAWS_TEST_FFMPEG_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => Path::new(&std::env::var_os("APPDATA")?).join("dev.claws.downloader").join("bin"),
    };
    let exe = |name: &str| dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    (exe("ffmpeg").is_file() && exe("ffprobe").is_file()).then_some(dir)
}

struct MediaKit {
    ytdlp: PathBuf,
    tools: PathBuf,
    dir: PathBuf,
}

impl Drop for MediaKit {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

macro_rules! require_media {
    ($tag:expr) => {
        match (ytdlp(), ffmpeg_dir()) {
            (Some(ytdlp), Some(tools)) => MediaKit { ytdlp, tools, dir: temp_dir($tag) },
            _ => {
                eprintln!("skipped: needs yt-dlp, ffmpeg and ffprobe");
                return;
            }
        }
    };
}

/// 1 second of a flat picture: tiny files, so even 8K encodes in a moment.
const CLIP: &str = "r=10:d=1";

impl MediaKit {
    fn tool(&self, name: &str) -> PathBuf {
        self.tools.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
    }

    /// Encodes `name` in the kit's folder from FFmpeg's built-in test sources.
    fn make(&self, name: &str, args: &[&str]) -> PathBuf {
        let path = self.dir.join(name);
        let output = Command::new(self.tool("ffmpeg")).args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).arg(&path).output().unwrap();
        assert!(output.status.success(), "ffmpeg could not make {name}: {}", String::from_utf8_lossy(&output.stderr));
        path
    }

    fn picture(&self, name: &str, size: &str, encode: &[&str]) -> PathBuf {
        let source = format!("color=c=gray:s={size}:{CLIP}");
        let mut args = vec!["-f", "lavfi", "-i", &source, "-an"];
        args.extend_from_slice(encode);
        self.make(name, &args)
    }

    fn h264(&self, name: &str, size: &str) -> PathBuf {
        self.picture(name, size, &["-c:v", "libx264", "-preset", "ultrafast", "-crf", "51", "-pix_fmt", "yuv420p"])
    }

    fn sound(&self, name: &str, encode: &[&str]) -> PathBuf {
        let mut args = vec!["-f", "lavfi", "-i", "sine=f=440:d=1", "-ac", "2", "-vn"];
        args.extend_from_slice(encode);
        self.make(name, &args)
    }

    /// A stream entry for `path` as a site would list it, read through a file link.
    fn format(path: &Path, id: &str, fields: serde_json::Value) -> serde_json::Value {
        let mut format = serde_json::json!({
            "format_id": id,
            "url": url::Url::from_file_path(path).unwrap().to_string(),
            "protocol": "file",
            "ext": path.extension().unwrap().to_string_lossy(),
        });
        format.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
        format
    }

    /// Saves `formats` with the policy's arguments for `options` into a fresh work folder and
    /// returns the file yt-dlp reported as finished.
    fn download(&self, formats: serde_json::Value, options: &crate::downloader::DownloadOptions, embed_metadata: bool) -> PathBuf {
        let work = self.dir.join(format!("work-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&work).unwrap();
        let info = self.dir.join(format!("info-{}.json", uuid::Uuid::new_v4().simple()));
        let value = serde_json::json!({
            "id": "fixture", "title": "fixture", "ext": "mp4", "extractor": "generic", "extractor_key": "Generic",
            "webpage_url": "https://example.invalid/v", "duration": 1.0, "formats": formats
        });
        std::fs::write(&info, value.to_string()).unwrap();
        let execution = JobExecution {
            filename_template: "payload.%(ext)s".into(),
            embed_metadata,
            ..JobExecution::freeze(&Settings { output_dir: "unused".into(), ..Settings::default() }, false)
        };
        // As tools::common_args gives them, with this FFmpeg.
        let common = vec!["--no-colors".into(), "--ignore-config".into(), "--ffmpeg-location".into(), self.tools.to_string_lossy().into_owned()];
        let mut args = download_policy::download_args(common, options, &execution, &work, Source::InfoJson(&info));
        // Only the test reads file links; the source stays last.
        let tail = args.split_off(args.len() - 2);
        args.push("--enable-file-urls".into());
        args.extend(tail);
        let output = Command::new(&self.ytdlp).args(&args).output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{}\n{stdout}", String::from_utf8_lossy(&output.stderr));
        let done = stdout.lines().find_map(|l| l.trim_end().strip_prefix("CLAWSDONE|")).expect("yt-dlp reported the finished file");
        PathBuf::from(done)
    }

    /// ffprobe's view of each stream: codec, picture size, pixel format, colour and channels.
    fn streams(&self, file: &Path) -> Vec<serde_json::Value> {
        let output = Command::new(self.tool("ffprobe"))
            .args(["-v", "error", "-show_entries", "stream=codec_type,codec_name,width,height,pix_fmt,color_space,color_transfer,color_primaries,channels", "-of", "json"])
            .arg(file)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        value["streams"].as_array().unwrap().clone()
    }

    fn stream<'a>(streams: &'a [serde_json::Value], kind: &str) -> &'a serde_json::Value {
        let mut matching = streams.iter().filter(|s| s["codec_type"] == kind);
        let first = matching.next().unwrap_or_else(|| panic!("no {kind} stream in {streams:?}"));
        assert!(matching.next().is_none(), "one {kind} stream expected: {streams:?}");
        first
    }

    /// A hash of the first `kind` ("v" or "a") stream's packets: equal only when the stream
    /// was copied, whatever container now holds it.
    fn packets(&self, file: &Path, kind: &str) -> String {
        let output = Command::new(self.tool("ffmpeg"))
            .args(["-v", "error", "-i"])
            .arg(file)
            .args(["-map", &format!("0:{kind}:0"), "-c", "copy", "-f", "streamhash", "-hash", "sha256", "-"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let text = String::from_utf8_lossy(&output.stdout);
        text.lines().rev().find(|l| !l.starts_with('#')).and_then(|l| l.rsplit(',').next()).unwrap().trim().to_string()
    }
}

fn video_options(max_height: Option<u32>, video_preset: crate::download_policy::VideoPreset) -> crate::downloader::DownloadOptions {
    crate::downloader::DownloadOptions { max_height, video_preset, ..crate::downloader::DownloadOptions::default() }
}

#[test]
fn original_keeps_hdr_vp9_and_opus_untouched_while_compatible_still_prefers_h264() {
    use crate::download_policy::VideoPreset::{Compatible, Original};
    let kit = require_media!("media-hdr");
    // A YouTube-like 720p: VP9 Profile 2 HDR10 or H.264 SDR, and Opus or AAC.
    let hdr = kit.picture(
        "v-hdr.webm",
        "1280x720",
        &["-vf", "setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc:range=tv,format=yuv420p10le", "-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8", "-b:v", "50k"],
    );
    let sdr = kit.h264("v-sdr.mp4", "1280x720");
    let aac = kit.sound("a-aac.m4a", &["-c:a", "aac", "-b:a", "32k"]);
    let opus = kit.sound("a-opus.webm", &["-c:a", "libopus", "-b:a", "24k"]);
    let formats = serde_json::json!([
        MediaKit::format(&hdr, "vp9-hdr", serde_json::json!({"vcodec": "vp09.02.31.10.01.09.16.09.00", "acodec": "none", "width": 1280, "height": 720, "fps": 10, "dynamic_range": "HDR10", "tbr": 50})),
        MediaKit::format(&sdr, "avc1", serde_json::json!({"vcodec": "avc1.64001f", "acodec": "none", "width": 1280, "height": 720, "fps": 10, "tbr": 40})),
        MediaKit::format(&aac, "aac", serde_json::json!({"vcodec": "none", "acodec": "mp4a.40.2", "abr": 32, "tbr": 32})),
        MediaKit::format(&opus, "opus", serde_json::json!({"vcodec": "none", "acodec": "opus", "abr": 24, "tbr": 24})),
    ]);

    // Original: the site's own best streams, copied (also through the metadata pass) into a
    // container that holds them. Nothing promises an editor opens VP9 in MKV.
    let saved = kit.download(formats.clone(), &video_options(None, Original), true);
    assert_eq!(saved.extension().unwrap(), "mkv", "VP9 and Opus are not MP4 by yt-dlp's rules");
    let streams = kit.streams(&saved);
    let video = MediaKit::stream(&streams, "video");
    assert_eq!((video["codec_name"].as_str(), video["width"].as_u64(), video["height"].as_u64()), (Some("vp9"), Some(1280), Some(720)));
    assert_eq!(video["pix_fmt"], "yuv420p10le", "10-bit kept");
    assert_eq!((video["color_primaries"].as_str(), video["color_transfer"].as_str(), video["color_space"].as_str()), (Some("bt2020"), Some("smpte2084"), Some("bt2020nc")), "HDR colour kept");
    let audio = MediaKit::stream(&streams, "audio");
    assert_eq!((audio["codec_name"].as_str(), audio["channels"].as_u64()), (Some("opus"), Some(2)));
    assert_eq!(kit.packets(&saved, "v"), kit.packets(&hdr, "v"), "the video was copied, not re-encoded");
    assert_eq!(kit.packets(&saved, "a"), kit.packets(&opus, "a"), "the audio was copied, not re-encoded");

    // Compatible, as before presets: H.264 and AAC at the same size, in MP4, also copied.
    let saved = kit.download(formats, &video_options(None, Compatible), true);
    assert_eq!(saved.extension().unwrap(), "mp4");
    let streams = kit.streams(&saved);
    let video = MediaKit::stream(&streams, "video");
    assert_eq!((video["codec_name"].as_str(), video["width"].as_u64(), video["height"].as_u64()), (Some("h264"), Some(1280), Some(720)));
    assert_eq!(MediaKit::stream(&streams, "audio")["codec_name"], "aac");
    assert_eq!(kit.packets(&saved, "v"), kit.packets(&sdr, "v"));
    assert_eq!(kit.packets(&saved, "a"), kit.packets(&aac, "a"));
}

#[test]
fn an_8k_source_is_saved_at_8k_only_when_asked_and_never_upscaled() {
    use crate::download_policy::VideoPreset::{Compatible, Original};
    let kit = require_media!("media-8k");
    let eight_k = kit.h264("v-4320.mp4", "7680x4320");
    let hd = kit.h264("v-720.mp4", "1280x720");
    let aac = kit.sound("a.m4a", &["-c:a", "aac", "-b:a", "32k"]);
    let formats = serde_json::json!([
        MediaKit::format(&hd, "720", serde_json::json!({"vcodec": "avc1.64001f", "acodec": "none", "width": 1280, "height": 720, "tbr": 40})),
        MediaKit::format(&eight_k, "4320", serde_json::json!({"vcodec": "avc1.640033", "acodec": "none", "width": 7680, "height": 4320, "tbr": 400})),
        MediaKit::format(&aac, "aac", serde_json::json!({"vcodec": "none", "acodec": "mp4a.40.2", "abr": 32, "tbr": 32})),
    ]);
    let size = |file: &Path| {
        let streams = kit.streams(file);
        let video = MediaKit::stream(&streams, "video").clone();
        (video["codec_name"].as_str().unwrap().to_string(), video["width"].as_u64().unwrap(), video["height"].as_u64().unwrap())
    };

    let saved = kit.download(formats.clone(), &video_options(Some(4320), Original), false);
    assert_eq!(saved.extension().unwrap(), "mp4", "H.264 and AAC fit MP4, so Original keeps MP4 here");
    assert_eq!(size(&saved), ("h264".to_string(), 7680, 4320));
    assert_eq!(kit.packets(&saved, "v"), kit.packets(&eight_k, "v"));

    let saved = kit.download(formats.clone(), &video_options(Some(4320), Compatible), false);
    assert_eq!(size(&saved), ("h264".to_string(), 7680, 4320));
    // A cap between the offered sizes takes the one below; nothing is scaled up or down.
    let saved = kit.download(formats, &video_options(Some(2160), Compatible), false);
    assert_eq!(size(&saved), ("h264".to_string(), 1280, 720));
}

#[test]
fn portrait_video_is_capped_by_its_shorter_side() {
    use crate::download_policy::VideoPreset::{Compatible, Original};
    let kit = require_media!("media-portrait");
    let small = kit.h264("p-640.mp4", "360x640");
    let large = kit.h264("p-1280.mp4", "720x1280");
    let aac = kit.sound("a.m4a", &["-c:a", "aac", "-b:a", "32k"]);
    let formats = serde_json::json!([
        MediaKit::format(&small, "p360", serde_json::json!({"vcodec": "avc1.64001e", "acodec": "none", "width": 360, "height": 640, "tbr": 30})),
        MediaKit::format(&large, "p720", serde_json::json!({"vcodec": "avc1.64001f", "acodec": "none", "width": 720, "height": 1280, "tbr": 60})),
        MediaKit::format(&aac, "aac", serde_json::json!({"vcodec": "none", "acodec": "mp4a.40.2", "abr": 32, "tbr": 32})),
    ]);
    let dims = |file: &Path| {
        let streams = kit.streams(file);
        let video = MediaKit::stream(&streams, "video").clone();
        (video["width"].as_u64().unwrap(), video["height"].as_u64().unwrap())
    };
    // The "360p" choice of a portrait video is the 360×640 stream, as the probe offers it.
    assert_eq!(dims(&kit.download(formats.clone(), &video_options(Some(360), Compatible), false)), (360, 640));
    assert_eq!(dims(&kit.download(formats, &video_options(None, Original), false)), (720, 1280));
}

#[test]
fn a_stream_without_codec_fields_is_kept_whole() {
    use crate::download_policy::VideoPreset::{Compatible, Original};
    let kit = require_media!("media-nocodec");
    // One file with picture and sound, listed with its size but no codec names.
    let muxed = kit.make(
        "muxed.mp4",
        &["-f", "lavfi", "-i", &format!("color=c=gray:s=640x360:{CLIP}"), "-f", "lavfi", "-i", "sine=f=440:d=1", "-shortest", "-c:v", "libx264", "-preset", "ultrafast", "-crf", "51", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "32k"],
    );
    let formats = serde_json::json!([MediaKit::format(&muxed, "only", serde_json::json!({"width": 640, "height": 360}))]);
    for preset in [Original, Compatible] {
        let saved = kit.download(formats.clone(), &video_options(Some(360), preset), false);
        assert_eq!(saved.extension().unwrap(), "mp4", "{preset:?}");
        let streams = kit.streams(&saved);
        assert_eq!(MediaKit::stream(&streams, "video")["codec_name"], "h264");
        assert_eq!(MediaKit::stream(&streams, "audio")["codec_name"], "aac");
        assert_eq!(kit.packets(&saved, "v"), kit.packets(&muxed, "v"), "{preset:?}");
        assert_eq!(kit.packets(&saved, "a"), kit.packets(&muxed, "a"), "{preset:?}");
    }
}

// The captions are local files read through file links; the only "media" format points at a
// file that does not exist, so any attempt to download media would fail the run.

/// Runs a transcript download of `info` with the policy's arguments and returns the files
/// yt-dlp named on its CLAWSCAPTIONS line, plus everything left in the work folder.
fn transcript_run(ytdlp: &Path, ffmpeg: Option<&Path>, info: &Path, work: &Path, options: &crate::downloader::DownloadOptions) -> (Vec<PathBuf>, Vec<String>) {
    std::fs::create_dir_all(work).unwrap();
    let execution = JobExecution { embed_metadata: true, ..JobExecution::freeze(&Settings { output_dir: "unused".into(), ..Settings::default() }, false) };
    let mut common: Vec<String> = vec!["--no-colors".into(), "--ignore-config".into()];
    if let Some(dir) = ffmpeg {
        common.extend(["--ffmpeg-location".into(), dir.to_string_lossy().into_owned()]);
    }
    let mut args = download_policy::download_args(common, options, &execution, work, Source::InfoJson(info));
    let tail = args.split_off(args.len() - 2);
    args.push("--enable-file-urls".into());
    args.extend(tail);
    let output = Command::new(ytdlp).args(&args).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{}\n{stdout}", String::from_utf8_lossy(&output.stderr));
    let named = stdout.lines().find_map(|l| l.trim_end().strip_prefix("CLAWSCAPTIONS|")).expect("yt-dlp named the captions files");
    let files: Vec<PathBuf> = serde_json::from_str::<Vec<String>>(named).unwrap().into_iter().map(PathBuf::from).collect();
    (files, names(work))
}

#[test]
fn a_transcript_fetches_only_the_chosen_caption_track_through_the_real_yt_dlp() {
    use crate::transcript::{self, TranscriptFormat};
    let Some(ytdlp) = ytdlp() else {
        eprintln!("skipped: needs yt-dlp");
        return;
    };
    let dir = temp_dir("transcript-ytdlp");
    let link = |name: &str| url::Url::from_file_path(dir.join(name)).unwrap().to_string();
    std::fs::write(dir.join("human.vtt"), "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\n<v Ana>Hello &amp; welcome\n\n00:00:02.000 --> 00:00:03.000\nこれは字幕です\n").unwrap();
    std::fs::write(dir.join("auto.vtt"), "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\n \nhello<00:00:00.500><c> there</c>\n\n00:00:02.000 --> 00:00:02.010\nhello there\n \n").unwrap();
    std::fs::write(dir.join("orig.srt"), "1\n00:00:01,000 --> 00:00:02,000\nGuten Tag\n").unwrap();
    std::fs::write(dir.join("only.srv3"), "<timedtext><body><p t=\"0\" d=\"1000\">salut</p></body></timedtext>").unwrap();
    let info = dir.join("info.json");
    let value = serde_json::json!({
        "id": "fixture", "title": "Talk: one", "ext": "mp4", "extractor": "generic", "extractor_key": "Generic",
        "webpage_url": "https://example.invalid/v", "duration": 3.0,
        "formats": [{ "format_id": "0", "url": link("no-such-media.mp4"), "protocol": "file", "ext": "mp4", "vcodec": "avc1", "acodec": "mp4a.40.2" }],
        "subtitles": { "en": [{ "ext": "vtt", "url": link("human.vtt"), "name": "English" }] },
        "automatic_captions": {
            "en": [{ "ext": "vtt", "url": link("auto.vtt"), "name": "English" }],
            "de-orig": [{ "ext": "srt", "url": link("orig.srt"), "name": "German (Original)" }],
            "fr": [{ "ext": "srv3", "url": link("only.srv3"), "name": "French" }]
        }
    });
    std::fs::write(&info, value.to_string()).unwrap();
    let options = |code: &str, auto: bool| crate::downloader::DownloadOptions {
        mode: "transcript".into(),
        subtitle_langs: vec![code.into()],
        auto_subs: auto,
        transcript_format: TranscriptFormat::Txt,
        ..crate::downloader::DownloadOptions::default()
    };

    // Human captions: exactly that track, and no media file at all.
    let work = dir.join("work-human");
    let (files, left) = transcript_run(&ytdlp, None, &info, &work, &options("en", false));
    assert_eq!(left, ["Talk： one [fixture].en.vtt"], "only the captions file was written");
    assert_eq!(files, [work.join("Talk： one [fixture].en.vtt")]);
    let caption = crate::download_files::owned_relative(&work, &files[0]).expect("inside the work folder");
    let made = transcript::write_artifact(&work, &caption, &options("en", false)).unwrap();
    assert_eq!(made.relative, Path::new("Talk： one [fixture] [en].txt"));
    assert_eq!(std::fs::read_to_string(work.join(&made.relative)).unwrap(), "Ana: Hello & welcome\r\nこれは字幕です\r\n");

    // Automatic captions of the same language: the automatic track, not the human one.
    let work = dir.join("work-auto");
    let (files, _) = transcript_run(&ytdlp, None, &info, &work, &options("en", true));
    assert_eq!(files.len(), 1);
    let caption = crate::download_files::owned_relative(&work, &files[0]).unwrap();
    let made = transcript::write_artifact(&work, &caption, &options("en", true)).unwrap();
    assert_eq!(made.relative, Path::new("Talk： one [fixture] [en auto].txt"));
    assert_eq!(std::fs::read_to_string(work.join(&made.relative)).unwrap(), "hello there\r\n");

    // A language the video does not have: no file, and nothing named.
    let work = dir.join("work-none");
    let (files, left) = transcript_run(&ytdlp, None, &info, &work, &options("fr", false));
    assert!(files.is_empty() && left.is_empty(), "{files:?} {left:?}");

    // Without FFmpeg, SubRip captions arrive as they are and Claws reads them itself; a code
    // with a hyphen is matched literally.
    let work = dir.join("work-srt");
    let (files, left) = transcript_run(&ytdlp, None, &info, &work, &options("de-orig", true));
    assert_eq!(left, ["Talk： one [fixture].de-orig.srt"]);
    let caption = crate::download_files::owned_relative(&work, &files[0]).unwrap();
    let made = transcript::write_artifact(&work, &caption, &options("de-orig", true)).unwrap();
    assert_eq!(made.relative, Path::new("Talk： one [fixture] [de-orig auto].txt"));
    assert_eq!(std::fs::read_to_string(work.join(&made.relative)).unwrap(), "Guten Tag\r\n");

    // Without FFmpeg, a format Claws cannot read (srv3) arrives unconverted: a clear message.
    let work = dir.join("work-srv3");
    let (files, _) = transcript_run(&ytdlp, None, &info, &work, &options("fr", true));
    let caption = crate::download_files::owned_relative(&work, &files[0]).unwrap();
    let err = transcript::write_artifact(&work, &caption, &options("fr", true)).unwrap_err();
    assert!(err.contains("FFmpeg"), "{err}");

    // With FFmpeg, a site format other than WebVTT is converted by yt-dlp's FFmpeg step.
    if let Some(tools) = ffmpeg_dir() {
        let work = dir.join("work-convert");
        let (files, left) = transcript_run(&ytdlp, Some(&tools), &info, &work, &options("de-orig", true));
        assert_eq!(left, ["Talk： one [fixture].de-orig.vtt"], "converted to WebVTT, the SubRip original removed");
        let caption = crate::download_files::owned_relative(&work, &files[0]).unwrap();
        let made = transcript::write_artifact(&work, &caption, &options("de-orig", true)).unwrap();
        assert_eq!(made.relative, Path::new("Talk： one [fixture] [de-orig auto].txt"));
        assert_eq!(std::fs::read_to_string(work.join(&made.relative)).unwrap(), "Guten Tag\r\n");
    } else {
        eprintln!("conversion check skipped: needs ffmpeg");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Exercise the actual Windows yt-dlp executable and production command/arguments.
/// Environment variables alone do not control the frozen executable's print encoding.
#[cfg(windows)]
#[test]
fn unicode_finished_path_survives_ytdlp_output_and_publishes() {
    let Some(ytdlp) = ytdlp() else {
        eprintln!("skipped: real yt-dlp is required");
        return;
    };
    let dir = temp_dir("unicode-finished-path");
    let out = dir.join("Vídeos 日本語");
    std::fs::create_dir_all(&out).unwrap();
    let app = crate::lifecycle_tests::TestApp::with_data(dir.clone(), out.clone(), vec![]);
    let (workspace, mut manifest) = crate::download_files::open_workspace(&out, &uuid::Uuid::new_v4().to_string(), 0).unwrap();
    let name = "𝗺𝗶𝗻𝗶 𝘆𝘂𝗿𝗶 │ AVigília 日本語 [fixture].mp4";
    let media = workspace.path.join(name);
    // An already completed transfer: no network or media processing is needed on retry.
    std::fs::write(&media, b"completed transfer").unwrap();
    let info = dir.join("info.json");
    std::fs::write(&info, serde_json::json!({
        "id": "fixture", "title": "Unicode fixture", "ext": "mp4", "extractor": "generic",
        "webpage_url": "http://127.0.0.1:1/unused.mp4", "url": "http://127.0.0.1:1/unused.mp4"
    }).to_string()).unwrap();
    let settings = Settings { output_dir: out.to_string_lossy().into_owned(), embed_metadata: false, ..Settings::default() };
    let mut execution = JobExecution::freeze(&settings, false);
    execution.filename_template = name.into();
    let args = download_policy::download_args(crate::tools::common_args(app.handle(), &settings), &downloader::DownloadOptions::default(), &execution, &workspace.path, Source::InfoJson(&info));
    let output = tauri::async_runtime::block_on(async {
        tokio::time::timeout(Duration::from_secs(20), crate::tools::command(&ytdlp).args(args).kill_on_drop(true).output()).await.unwrap().unwrap()
    });
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    // Match the production reader: diagnostic/progress lines may use a legacy code
    // page, but the printed finished path must arrive losslessly as UTF-8.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let reported = stdout.lines().find_map(|line| line.strip_prefix("CLAWSDONE|")).expect("finished path");
    assert_eq!(Path::new(reported), media);
    let relative = crate::download_files::owned_relative(&workspace.path, Path::new(reported)).expect("finished file remains owned");
    manifest.outputs = vec![relative.to_string_lossy().into_owned()];
    manifest.state = crate::download_files::Publication::Ready;
    crate::download_files::write_manifest(&manifest).unwrap();
    let published = crate::download_files::publish(&mut manifest, &mut |_| true).unwrap();
    assert_eq!(published.media, out.join(name));
    assert_eq!(std::fs::read(published.media).unwrap(), b"completed transfer");
}

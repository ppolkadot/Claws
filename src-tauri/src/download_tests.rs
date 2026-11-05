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

// ---------------------------------------------------------------------------------------
// Real yt-dlp against the localhost fixtures.

fn ytdlp() -> Option<PathBuf> {
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

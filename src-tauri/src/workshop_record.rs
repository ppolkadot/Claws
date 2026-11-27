//! Recording a scene or web wallpaper: Wallpaper Engine plays it in a window of its own, and
//! Claws' managed FFmpeg captures that one window.
//!
//! Wallpaper Engine is found the way Steam records it and run only after Windows verifies that
//! its signature is `Skutta Software GmbH`. The window is opened with Wallpaper Engine's
//! documented command line, made borderless, hidden off-screen, kept out of the taskbar and
//! never activated, with keyboard focus handed straight back, so the user keeps working. It is
//! never minimised (a minimised window stops being captured) and it is closed on every way out,
//! success, failure, cancel and shutdown, by a guard. Claws only reads Steam and Wallpaper
//! Engine's folders and never changes Wallpaper Engine's settings.
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use serde::Serialize;
use tokio::process::Command;

use crate::{
    bounded_io::read_tail,
    downloader::kill_tree,
    encoder::{self, Encoder},
    export_progress::{FfmpegProgress, ProgressParser, Stage, Tracker, Unit},
    scene_pkg::{Authored, SceneInfo},
    win_window::{self, Display},
    workshop_media::Probe,
};

/// The only publisher whose Wallpaper Engine Claws will run.
pub const PUBLISHER: &str = "Skutta Software GmbH";
/// Sizes offered, by height, besides a scene's own authored height.
pub const STANDARD_HEIGHTS: [u32; 4] = [720, 1080, 1440, 2160];
pub const DEFAULT_LENGTH: u32 = 30;
pub const MAX_LENGTH: u32 = 600;
/// Seconds captured and thrown away while the scene loads, and the longer wait for a big package.
pub const PRE_ROLL: u32 = 2;
pub const PRE_ROLL_LARGE: u32 = 4;
const LARGE_PACKAGE_BYTES: u64 = 500 * 1024 * 1024;
const WINDOW_WAIT: Duration = Duration::from_secs(15);
/// What the interface tells the user when a size cannot be had on this display.
pub const NEEDS_BIGGER_DISPLAY: &str = "4K needs a display set to 3840×2160 or larger, such as a 4K monitor or NVIDIA DSR / AMD VSR";
pub const FROZEN: &str = "The recording didn't change; Wallpaper Engine may have paused. Keep its window visible and try again.";

// ---------------------------------------------------------------------------------------
// Wallpaper Engine: where it is, and whether it is the real one

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineProblem {
    NotInstalled,
    NotGenuine,
}

impl EngineProblem {
    pub fn message(&self) -> &'static str {
        match self {
            Self::NotInstalled => "Recording needs Wallpaper Engine installed in your Steam library.",
            Self::NotGenuine => "Claws couldn't verify Wallpaper Engine's signature, so it won't run it. Reinstall Wallpaper Engine from Steam.",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Engine {
    pub exe: PathBuf,
}

/// The engine, when there is a file and `genuine` accepts it.
pub fn check_engine(exe: Option<PathBuf>, genuine: impl Fn(&Path) -> bool) -> Result<Engine, EngineProblem> {
    let exe = exe.ok_or(EngineProblem::NotInstalled)?;
    if genuine(&exe) {
        Ok(Engine { exe })
    } else {
        Err(EngineProblem::NotGenuine)
    }
}

/// Check the publisher without caching a verdict for a replaceable path.
fn signed_by_the_publisher(exe: &Path) -> bool {
    crate::signature::signed_by(exe, PUBLISHER)
}

/// Finds Wallpaper Engine and checks it. Blocking: reads Steam's files and verifies a signature.
pub fn find_engine() -> Result<Engine, EngineProblem> {
    let found = crate::steam::steam_root().and_then(|root| {
        let libraries = crate::steam::library_folders(&root);
        crate::steam::wallpaper_engine_exe(&libraries, &root)
    });
    check_engine(found, signed_by_the_publisher)
}

// ---------------------------------------------------------------------------------------
// Sizes

/// One size a recording can be made at.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SizeOption {
    pub width: u32,
    pub height: u32,
    pub label: String,
    pub available: bool,
    /// Why it cannot be chosen (shown beside the disabled choice).
    pub reason: Option<String>,
    /// Larger than the display reports a window can be. Claws still tries, because a borderless
    /// window is not held to that limit, and says so plainly when it does not fit.
    pub beyond_display: bool,
    /// The frame rates this size can be recorded at.
    pub fps: Vec<u32>,
}

fn even(value: f64) -> u32 {
    ((value / 2.0).round() as u32) * 2
}

fn label_of(height: u32) -> String {
    if height == 2160 {
        "4K".into()
    } else {
        format!("{height}p")
    }
}

/// The sizes a scene can be recorded at: in its own aspect ratio, never larger than it was
/// authored (nothing is upscaled), and only where an encoder can keep up. `display` is what the
/// display reports; sizes beyond its limit are flagged, not removed. `works` says which encoders
/// really work on this PC.
pub fn size_options(authored: Authored, display: Option<Display>, works: &dyn Fn(Encoder) -> bool) -> Vec<SizeOption> {
    let (ratio, cap) = match authored {
        Authored::Fixed { width, height } => (f64::from(width) / f64::from(height), Some(height)),
        Authored::Auto | Authored::Perspective => {
            let (w, h) = display.map_or((16, 9), |d| d.primary);
            (f64::from(w) / f64::from(h), None)
        }
    };
    let mut heights: Vec<u32> = STANDARD_HEIGHTS.to_vec();
    if let Some(cap) = cap {
        // A scene authored at an odd size can be recorded at that size too.
        let own = cap - cap % 2;
        if own >= 2 && !heights.contains(&own) {
            heights.push(own);
        }
    }
    heights.sort_unstable();
    heights
        .into_iter()
        .filter(|height| cap.is_none_or(|cap| *height <= cap))
        .map(|height| {
            let width = even(f64::from(height) * ratio);
            let beyond_display = display.is_some_and(|d| width > d.max_client.0 || height > d.max_client.1);
            let steady = encoder::pick_for(works, height, 30).is_some();
            let fast = encoder::pick_for(works, height, 60).is_some();
            let (available, reason) = if steady {
                (true, None)
            } else {
                (false, Some("Needs a hardware video encoder (NVIDIA, AMD, Intel or Windows' own), which this PC doesn't have".to_string()))
            };
            SizeOption { width, height, label: label_of(height), available, reason, beyond_display, fps: if !steady { vec![] } else if fast { vec![30, 60] } else { vec![30] } }
        })
        .collect()
}

/// The size chosen by default: 1080p, or the largest available when the scene is smaller.
pub fn default_height(options: &[SizeOption]) -> Option<u32> {
    let available: Vec<&SizeOption> = options.iter().filter(|option| option.available).collect();
    available.iter().find(|option| option.height == 1080).or_else(|| available.last()).map(|option| option.height)
}

/// The size to record at, or why the request cannot be met.
pub fn choose_size(options: &[SizeOption], requested: Option<u32>) -> Result<&SizeOption, String> {
    let height = requested.or_else(|| default_height(options)).ok_or("This wallpaper can't be recorded at any size on this PC.")?;
    let option = options.iter().find(|option| option.height == height).ok_or("This wallpaper wasn't made that large, so Claws won't stretch it. Choose a smaller size.")?;
    if option.available {
        Ok(option)
    } else {
        Err(option.reason.clone().unwrap_or_else(|| "That size isn't available.".into()))
    }
}

/// Seconds to capture and throw away while the scene loads: longer for a big package.
pub fn pre_roll_seconds(package_bytes: u64) -> u32 {
    if package_bytes > LARGE_PACKAGE_BYTES {
        PRE_ROLL_LARGE
    } else {
        PRE_ROLL
    }
}

/// How long a recording takes, from the length chosen: the window, the pre-roll and the time to
/// finish the file (measured: 62.9 s for a 60 s recording with a 2 s pre-roll).
pub fn expected_seconds(length: u32, pre_roll: u32) -> f64 {
    f64::from(length) + f64::from(pre_roll) + 1.0
}

/// What a video recording is expected to weigh: the typical bitrate for its size times its length.
pub fn expected_video_bytes(width_height: (u32, u32), fps: u32, length: u32) -> u64 {
    u64::from(encoder::typical_bitrate_kbps(width_height.1, fps)) * 1000 / 8 * u64::from(length)
}

/// What a still picture is expected to weigh.
pub fn expected_picture_bytes(width: u32, height: u32) -> u64 {
    u64::from(width) * u64::from(height) * 12 / 10
}

// ---------------------------------------------------------------------------------------
// FFmpeg's capture

/// What one run captures.
#[derive(Debug, Clone)]
pub enum CaptureKind {
    Video { length: u32, encoder: Encoder },
    Picture,
}

#[derive(Debug, Clone)]
pub struct Capture {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub kind: CaptureKind,
}

/// FFmpeg's arguments: capture one window, throw away the pre-roll, keep `length` seconds (or one
/// frame) and write a fresh file that is never overwritten. Progress goes to stdout.
pub fn capture_args(hwnd: i64, capture: &Capture, pre_roll: u32, output: &Path) -> Vec<String> {
    let fps = capture.fps;
    let tail = match capture.kind {
        CaptureKind::Video { .. } => "format=yuv420p",
        CaptureKind::Picture => "format=rgb24",
    };
    let filter = format!("gfxcapture=hwnd={hwnd}:capture_cursor=0:max_framerate={fps},hwdownload,format=bgra,fps={fps},{tail}");
    let mut args: Vec<String> = [
        "-nostdin", "-hide_banner", "-loglevel", "error", "-nostats", "-stats_period", "0.25", "-progress", "pipe:1", "-n", "-filter_complex", &filter,
        "-ss", &pre_roll.to_string(),
    ]
    .map(str::to_string)
    .into();
    match &capture.kind {
        CaptureKind::Video { length, encoder } => {
            args.extend(["-t".to_string(), length.to_string()]);
            args.extend(encoder.args(capture.width, capture.height, fps));
            args.extend(["-movflags", "+faststart", "-f", "mp4"].map(str::to_string));
        }
        CaptureKind::Picture => {
            args.extend(["-frames:v", "1", "-c:v", "png", "-f", "image2"].map(str::to_string));
        }
    }
    args.push(output.to_string_lossy().into_owned());
    args
}

/// A window title no other program is likely to use: the token is random.
pub fn window_title(token: &str) -> String {
    format!("Claws {token}")
}

// ---------------------------------------------------------------------------------------
// Checking what was recorded

/// The frames a recording must hold: its length times its rate, within 1% (and two frames).
pub fn check_probe(probe: &Probe, capture: &Capture) -> Result<(), String> {
    if (probe.width, probe.height) != (capture.width, capture.height) {
        return Err(format!(
            "The recording came out {}×{} instead of {}×{}, so it was not saved.",
            probe.width, probe.height, capture.width, capture.height
        ));
    }
    if let (CaptureKind::Video { length, .. }, Some(frames)) = (&capture.kind, probe.frames) {
        let expected = f64::from(*length) * f64::from(capture.fps);
        let allowed = (expected * 0.01).max(2.0);
        if (frames as f64 - expected).abs() > allowed {
            return Err(format!("The recording has {frames} frames instead of about {expected:.0}, so it was not saved."));
        }
    }
    Ok(())
}

/// The mean luma of each frame difference, from FFmpeg's `signalstats` metadata print.
pub fn parse_yavg(text: &str) -> Vec<f64> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("lavfi.signalstats.YAVG="))
        .filter_map(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .collect()
}

/// Whether a set of frame differences says nothing moved: enough samples, and none above a
/// whisper. The difference of two frames of the same picture is zero.
pub fn looks_frozen(differences: &[f64]) -> bool {
    differences.len() >= 10 && differences.iter().all(|difference| *difference < 0.01)
}

/// Looks at three short stretches of the recording for any change between frames. `None` when it
/// could not be checked: a check that fails is never a reason to throw a recording away.
async fn frozen_capture(ffmpeg: &Path, file: &Path, length: u32) -> Option<bool> {
    let mut differences = Vec::new();
    for share in [0.25, 0.5, 0.75] {
        let start = (f64::from(length) * share - 1.0).max(0.0);
        let mut command = Command::new(ffmpeg);
        command
            .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-ss"])
            .arg(format!("{start:.2}"))
            .args(["-t", "2", "-i"])
            .arg(file)
            .args(["-vf", "scale=160:-2,tblend=all_mode=difference,signalstats,metadata=print:key=lavfi.signalstats.YAVG:file=-", "-f", "null", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let output = tokio::time::timeout(Duration::from_secs(60), command.output()).await.ok()?.ok()?;
        if !output.status.success() {
            return None;
        }
        // The first difference is against nothing; the rest are frame against frame.
        let mut values = parse_yavg(&String::from_utf8_lossy(&output.stdout));
        if !values.is_empty() {
            values.remove(0);
        }
        differences.extend(values);
    }
    (differences.len() >= 10).then(|| looks_frozen(&differences))
}

// ---------------------------------------------------------------------------------------
// Wallpaper Engine's window

/// Hold every path component without delete sharing, and the executable without write
/// sharing. This keeps the verified image and its path stable until process creation.
#[cfg(windows)]
pub(crate) fn lock_engine_path(exe: &Path) -> std::io::Result<Vec<std::fs::File>> {
    use std::os::windows::fs::OpenOptionsExt;
    let mut path = PathBuf::new();
    let mut held = Vec::new();
    for component in exe.components() {
        path.push(component.as_os_str());
        if matches!(component, std::path::Component::Prefix(_)) { continue; }
        let final_component = path == exe;
        let mut options = std::fs::OpenOptions::new();
        options.read(true).share_mode(if final_component { 1 } else { 3 })
            .custom_flags(0x0020_0000 | if final_component { 0 } else { 0x0200_0000 });
        let file = options.open(&path)?;
        let meta = file.metadata()?;
        if crate::download_files::is_link(&meta) || if final_component { !meta.is_file() } else { !meta.is_dir() } {
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Unsafe Wallpaper Engine path"));
        }
        held.push(file);
    }
    if held.is_empty() { return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Empty engine path")); }
    Ok(held)
}

/// Runs Wallpaper Engine's command line. It returns as soon as it has passed the order on (or
/// has become the running Wallpaper Engine itself, which is why it is never waited for).
fn engine_command(exe: &Path, args: &[&str]) -> std::io::Result<()> {
    #[cfg(windows)]
    let held = lock_engine_path(exe)?;
    #[cfg(windows)]
    if !crate::signature::signer_organization_with_file(exe, held.last()).is_some_and(|name| crate::signature::is_publisher(&name, PUBLISHER)) {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Wallpaper Engine signature verification failed"));
    }
    let mut command = std::process::Command::new(exe);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command.spawn().map(drop)
}

fn find_with_grace<T>(grace: Duration, mut find: impl FnMut() -> Option<T>, elapsed: impl Fn() -> Duration, mut sleep: impl FnMut()) -> Option<T> {
    loop {
        if let Some(found) = find() { return Some(found); }
        if elapsed() >= grace { return None; }
        sleep();
    }
}

/// Closes the window with this title, the way Wallpaper Engine documents (`closeWallpaper`) and
/// then, if it is still there, the way a close button does. Only a window that exists is asked:
/// asking Wallpaper Engine when it is not running would start it. Blocking, about a second (plus
/// `grace`, which waits for a window that has not appeared yet).
pub fn close_window(engine: &Path, title: &str, grace: Duration) -> bool {
    // The window can still be on its way when the order to open it was only just given: wait for
    // it for `grace`, so a cancel that comes early does not leave it to appear afterwards.
    let waiting = Instant::now();
    let found = find_with_grace(grace, || win_window::find_window(title, engine), || waiting.elapsed(), || std::thread::sleep(Duration::from_millis(50)));
    let Some(_found) = found else { return true };
    let gone = |wait_ms: u64| {
        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        while Instant::now() < deadline {
            if win_window::find_window(title, engine).is_none() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        win_window::find_window(title, engine).is_none()
    };
    let _ = engine_command(engine, &["-control", "closeWallpaper", "-playInWindow", title]);
    if gone(3000) {
        return true;
    }
    // The original handle may have been destroyed and reused while waiting.
    if let Some(current) = win_window::find_window(title, engine) {
        win_window::post_close(current);
    }
    gone(2000)
}

/// Keeps Wallpaper Engine's recording window from outliving the export: whatever way the work
/// ends, dropping the guard closes it. `announce` tells the rest of Claws which window is open
/// (so a shutdown can close it too) and, with `None`, that it is gone.
pub struct WindowGuard<'a> {
    engine: PathBuf,
    title: String,
    announce: &'a (dyn Fn(Option<&str>) + Sync),
    closed: bool,
    /// When the order to open the window was given, once it has been.
    opened: Option<Instant>,
}

impl<'a> WindowGuard<'a> {
    pub fn new(engine: &Path, title: &str, announce: &'a (dyn Fn(Option<&str>) + Sync)) -> Self {
        announce(Some(title));
        Self { engine: engine.to_path_buf(), title: title.to_string(), announce, closed: false, opened: None }
    }

    /// The order to open the window has been given: from now on, closing waits briefly for a
    /// window that has not appeared yet.
    pub fn opened(&mut self) {
        self.opened = Some(Instant::now());
    }

    /// Closes the window now. Returns whether it is gone.
    pub fn close_now(&mut self) -> bool {
        self.close_now_with(|engine, title, grace| close_window(engine, title, grace))
    }

    fn close_now_with(&mut self, close: impl FnOnce(&Path, &str, Duration) -> bool) -> bool {
        if self.closed { return true; }
        let grace = self.opened.map_or(Duration::ZERO, |opened| WINDOW_WAIT.saturating_sub(opened.elapsed()));
        let gone = close(&self.engine, &self.title, grace);
        if gone {
            self.closed = true;
            (self.announce)(None);
        }
        gone
    }

}

impl Drop for WindowGuard<'_> {
    fn drop(&mut self) {
        self.close_now();
    }
}

// ---------------------------------------------------------------------------------------
// The recording

pub struct RecordRequest<'a> {
    pub ffmpeg: &'a Path,
    pub ffprobe: Option<&'a Path>,
    pub engine: &'a Engine,
    pub project_json: &'a Path,
    pub partial: &'a Path,
    pub capture: Capture,
    pub pre_roll: u32,
    /// The scene was classified as moving: a recording that never changes is a failure.
    pub moves: bool,
    pub tracker: &'a Tracker,
    pub cancelled: &'a (dyn Fn() -> bool + Sync),
    /// Records FFmpeg's process id so Cancel and shutdown can stop it; false means stop now.
    pub on_spawn: &'a (dyn Fn(Option<u32>) -> bool + Sync),
    /// Whether the drive is nearly full; polled every few seconds while recording.
    pub low_space: &'a (dyn Fn() -> bool + Sync),
    /// Which Wallpaper Engine window is open, for shutdown (`None` once it is closed).
    pub announce: &'a (dyn Fn(Option<&str>) + Sync),
}

#[derive(Debug, PartialEq)]
pub enum Recorded {
    Done {
        /// Frames FFmpeg had to repeat because the capture could not keep up, as a percentage.
        repeated_percent: f64,
    },
    Cancelled,
    LowSpace,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RecordFailure {
    pub message: String,
    pub detail: String,
}

fn fail(message: impl Into<String>, detail: impl Into<String>) -> RecordFailure {
    RecordFailure { message: message.into(), detail: detail.into() }
}

/// How often the poll loops look: window, cancel and free space.
const POLL: Duration = Duration::from_millis(100);
/// FFmpeg reporting nothing for this long, once it has started, means it is stuck.
const STALL: Duration = Duration::from_secs(45);
/// How long after the first kept frame was captured FFmpeg first reports output past the pre-roll:
/// measured at 0.52 s with the hardware encoder on the owner's PC (see the study).
const FIRST_OUTPUT_LAG: Duration = Duration::from_millis(500);

/// Records the wallpaper into `request.partial`. The partial file is removed on every way out
/// except `Done`; the caller publishes it.
pub async fn record(request: &RecordRequest<'_>) -> Result<Recorded, RecordFailure> {
    let result = record_inner(request).await;
    if !matches!(result, Ok(Recorded::Done { .. })) {
        let _ = std::fs::remove_file(request.partial);
    }
    result
}

async fn record_inner(request: &RecordRequest<'_>) -> Result<Recorded, RecordFailure> {
    let capture = &request.capture;
    let tracker = request.tracker;
    let engine = &request.engine.exe;
    if (request.cancelled)() {
        return Ok(Recorded::Cancelled);
    }
    tracker.stage(Stage::Opening, None);
    let title = window_title(&uuid::Uuid::new_v4().simple().to_string());
    let before = win_window::foreground();
    // From here on the window is closed however this function ends.
    let mut guard = WindowGuard::new(engine, &title, request.announce);
    let project = dunce::canonicalize(request.project_json).map_err(|_| fail("Claws couldn't read this wallpaper's project.", ""))?;
    let (width, height) = (capture.width.to_string(), capture.height.to_string());
    engine_command(
        engine,
        &["-control", "openWallpaper", "-file", &project.to_string_lossy(), "-playInWindow", &title, "-width", &width, "-height", &height],
    )
    .map_err(|_| fail("Claws couldn't start Wallpaper Engine.", ""))?;
    guard.opened();

    // Wait for its window.
    let waiting = Instant::now();
    let window = loop {
        if (request.cancelled)() {
            return Ok(Recorded::Cancelled);
        }
        if let Some(window) = win_window::find_window(&title, engine) {
            break window;
        }
        if waiting.elapsed() > WINDOW_WAIT {
            return Err(fail("Wallpaper Engine didn't open the wallpaper. Check that it plays in Wallpaper Engine, then try again.", ""));
        }
        tokio::time::sleep(POLL).await;
    };

    // Out of sight and out of the way: borderless, beyond every monitor, no taskbar button, at the
    // bottom, not activated. Then the keyboard goes back to whoever had it.
    let right_edge = win_window::display().map_or(5000, |display| display.right_edge.saturating_add(100));
    win_window::make_background(window, capture.width, capture.height, right_edge).map_err(|e| fail("Claws couldn't set up Wallpaper Engine's window.", e))?;
    if let Some(before) = before {
        win_window::restore_foreground(before);
    }
    // The window gets its size a moment after it is changed.
    let mut client = win_window::client_size(window);
    for _ in 0..30 {
        if client == Some((capture.width, capture.height)) {
            break;
        }
        tokio::time::sleep(POLL).await;
        client = win_window::client_size(window);
    }
    if client != Some((capture.width, capture.height)) {
        let reached = client.map_or_else(|| "an unknown size".to_string(), |(w, h)| format!("{w}×{h}"));
        let hint = if capture.height >= 2160 { format!(" {NEEDS_BIGGER_DISPLAY}.") } else { String::new() };
        return Err(fail(
            format!(
                "Wallpaper Engine's window came out {reached} instead of {}×{}, so nothing was recorded. Claws never stretches a recording to fake a size.{hint}",
                capture.width, capture.height
            ),
            "",
        ));
    }
    if (request.cancelled)() {
        return Ok(Recorded::Cancelled);
    }

    // Capture it.
    tracker.stage(Stage::Loading, None);
    let mut command = Command::new(request.ffmpeg);
    command
        .args(capture_args(window as i64, capture, request.pre_roll, request.partial))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command.spawn().map_err(|_| fail("Couldn't start FFmpeg to record this wallpaper.", ""))?;
    let pid = child.id();
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let errors = tokio::spawn(read_tail(stderr, 16 * 1024));
    if !(request.on_spawn)(pid) {
        if let Some(pid) = pid {
            kill_tree(pid, true);
        }
        let _ = child.kill().await;
        let _ = errors.await;
        return Ok(Recorded::Cancelled);
    }

    let length = match capture.kind {
        CaptureKind::Video { length, .. } => Some(f64::from(length)),
        CaptureKind::Picture => None,
    };
    let started = Instant::now();
    let last_report = Mutex::new(started);
    let totals = Mutex::new((0u64, 0u64)); // frames, repeated frames
    let low = std::sync::atomic::AtomicBool::new(false);
    let reading = async {
        use tokio::io::AsyncBufReadExt;
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        let mut parser = ProgressParser::default();
        let mut recording_since: Option<Instant> = None;
        while let Ok(Some(line)) = lines.next_line().await {
            let Some(block) = parser.push(&line) else { continue };
            report(&block, length, &mut recording_since, tracker, &last_report, &totals);
        }
    };
    let supervising = async {
        let mut last_space = Instant::now();
        loop {
            tokio::select! {
                status = child.wait() => return Some(status),
                _ = tokio::time::sleep(POLL) => {}
            }
            let mut stop = (request.cancelled)()
                || last_report.lock().unwrap().elapsed() > STALL
                || started.elapsed() > Duration::from_secs(u64::from(request.pre_roll) + length.unwrap_or(0.0) as u64 + 180);
            if !stop && last_space.elapsed() >= Duration::from_secs(5) {
                last_space = Instant::now();
                if (request.low_space)() {
                    low.store(true, std::sync::atomic::Ordering::SeqCst);
                    stop = true;
                }
            }
            if stop {
                // Stop FFmpeg here, so that its output ends and the reader below can finish.
                if let Some(pid) = pid {
                    kill_tree(pid, true);
                }
                let _ = child.kill().await;
                return None;
            }
        }
    };
    let (_, status) = tokio::join!(reading, supervising);
    let Some(status) = status else {
        let _ = errors.await;
        guard.close_now();
        if low.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(Recorded::LowSpace);
        }
        if (request.cancelled)() {
            return Ok(Recorded::Cancelled);
        }
        return Err(fail("The recording stopped making progress, so it was not saved. Try again.", ""));
    };
    let detail = errors.await.map(|bytes| crate::workshop_media::tail_text(&bytes)).unwrap_or_default();
    if (request.cancelled)() {
        guard.close_now();
        return Ok(Recorded::Cancelled);
    }
    let status = status.map_err(|_| fail("FFmpeg stopped unexpectedly while recording.", detail.clone()))?;

    // The capture is over: close the window, then look at what came out.
    tracker.stage(Stage::Finishing, None);
    let closed = guard.close_now();
    if !status.success() {
        return Err(fail("FFmpeg couldn't record this wallpaper.", detail));
    }
    if !closed {
        // Not fatal: the recording is complete. The guard tries again when it is dropped.
    }
    if let Some(ffprobe) = request.ffprobe {
        if let Some(probe) = crate::workshop_media::probe_file(ffprobe, request.partial).await {
            check_probe(&probe, capture).map_err(|message| fail(message, ""))?;
        }
    }
    if request.moves {
        if let CaptureKind::Video { length, .. } = capture.kind {
            if frozen_capture(request.ffmpeg, request.partial, length).await == Some(true) {
                return Err(fail(FROZEN, ""));
            }
        }
    }
    let (frames, repeated) = *totals.lock().unwrap();
    let repeated_percent = if frames == 0 { 0.0 } else { repeated as f64 * 100.0 / frames as f64 };
    Ok(Recorded::Done { repeated_percent })
}

/// One block of FFmpeg's progress. The recording starts being counted when FFmpeg first has
/// output past the pre-roll; from then on the seconds are the clock's, because FFmpeg's own
/// output time trails the capture (the encoder holds frames back).
fn report(
    block: &FfmpegProgress,
    length: Option<f64>,
    recording_since: &mut Option<Instant>,
    tracker: &Tracker,
    last_report: &Mutex<Instant>,
    totals: &Mutex<(u64, u64)>,
) {
    *last_report.lock().unwrap() = Instant::now();
    if let (Some(frame), dup) = (block.frame, block.dup_frames.unwrap_or(0)) {
        *totals.lock().unwrap() = (frame, dup);
    }
    let Some(length) = length else { return };
    if recording_since.is_none() {
        if let Some(out_us) = block.out_time_us.filter(|us| *us > 0) {
            // The first output arrives about half a second after the first kept frame was
            // captured (the encoder and the capture pipeline hold a few frames), and the output
            // time is what has been kept so far: the recording began that long ago.
            let ago = Duration::from_micros(out_us) + FIRST_OUTPUT_LAG;
            *recording_since = Some(Instant::now().checked_sub(ago).unwrap_or_else(Instant::now));
            tracker.stage(Stage::Recording, Some((0.0, length, Unit::Seconds)));
        }
    }
    if let Some(since) = recording_since {
        tracker.recording(since.elapsed().as_secs_f64(), length, block.total_size);
    }
}

// ---------------------------------------------------------------------------------------
// What the interface asks before it offers recording

/// What the Download panel needs to offer recording for a scene or web project.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecordInfo {
    /// "scene" or "web".
    pub kind: &'static str,
    /// Recording can start now.
    pub available: bool,
    /// One line on why not, when it can't.
    pub reason: Option<String>,
    /// The scene moves by itself; `None` for a web page, which Claws cannot judge.
    pub moves: Option<bool>,
    pub has_audio: bool,
    pub sizes: Vec<SizeOption>,
    pub default_height: Option<u32>,
    pub pre_roll_seconds: u32,
    /// The encoder a recording will use, by name; `None` when only software works.
    pub encoder: Option<&'static str>,
    /// Wallpaper Engine is not running now and will be started.
    pub engine_will_start: bool,
    /// The largest window a framed window can have; sizes beyond it may not fit.
    pub display_limit: Option<(u32, u32)>,
}

/// Everything Claws knows about recording this project, from what is on this PC.
pub async fn record_info(kind: &'static str, scene: Option<SceneInfo>, ffmpeg: Option<&Path>) -> RecordInfo {
    let engine = tokio::task::spawn_blocking(find_engine).await.unwrap_or(Err(EngineProblem::NotInstalled));
    let capturer = match ffmpeg {
        Some(path) => has_gfxcapture(path).await,
        None => false,
    };
    let working = match ffmpeg {
        Some(path) if capturer => encoder::available(path).await,
        _ => vec![Encoder::X264],
    };
    let works = |candidate: Encoder| working.contains(&candidate);
    let display = win_window::display();
    let authored = scene.as_ref().map_or(Authored::Perspective, |scene| scene.authored);
    let sizes = size_options(authored, display, &works);
    let reason = match (&engine, ffmpeg.is_some(), capturer) {
        (Err(problem), _, _) => Some(problem.message().to_string()),
        (_, false, _) => Some("Recording needs FFmpeg. Install it in Settings, then try again.".to_string()),
        (_, _, false) => Some("This FFmpeg can't capture a window. Update FFmpeg in Settings → Tools.".to_string()),
        _ if default_height(&sizes).is_none() => Some("This PC has no video encoder that can record this wallpaper.".to_string()),
        _ => None,
    };
    let chosen = default_height(&sizes);
    let encoder = chosen.and_then(|height| encoder::pick_for(&works, height, 30)).filter(|encoder| encoder.hardware()).map(Encoder::name);
    RecordInfo {
        kind,
        available: reason.is_none(),
        reason,
        moves: scene.as_ref().map(|scene| scene.moves),
        has_audio: scene.as_ref().is_some_and(|scene| scene.has_audio),
        pre_roll_seconds: pre_roll_seconds(scene.as_ref().map_or(0, |scene| scene.package_bytes)),
        default_height: chosen,
        sizes,
        encoder,
        engine_will_start: engine.is_ok() && !win_window::process_running("wallpaper64.exe"),
        display_limit: display.map(|display| display.max_client),
    }
}

/// Whether FFmpeg can capture a window (`gfxcapture`). Asked once per FFmpeg.
pub async fn has_gfxcapture(ffmpeg: &Path) -> bool {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, bool>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(known) = cache.lock().unwrap().get(ffmpeg) {
        return *known;
    }
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-filters"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let found = match tokio::time::timeout(Duration::from_secs(20), command.output()).await {
        Ok(Ok(output)) => String::from_utf8_lossy(&output.stdout[..output.stdout.len().min(512 * 1024)]).contains("gfxcapture"),
        _ => false,
    };
    cache.lock().unwrap().insert(ffmpeg.to_path_buf(), found);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nothing(_: Encoder) -> bool {
        false
    }
    fn nvenc(e: Encoder) -> bool {
        e == Encoder::Nvenc
    }
    fn display(max: (u32, u32), primary: (u32, u32)) -> Option<Display> {
        Some(Display { max_client: max, primary, right_edge: 1920 })
    }
    fn heights(options: &[SizeOption]) -> Vec<u32> {
        options.iter().map(|option| option.height).collect()
    }
    fn fixed(width: u32, height: u32) -> Authored {
        Authored::Fixed { width, height }
    }

    // ----- finding Wallpaper Engine -----

    #[test]
    fn wallpaper_engine_must_exist_and_be_signed_by_the_publisher() {
        let exe = PathBuf::from(r"C:\Steam\steamapps\common\wallpaper_engine\wallpaper64.exe");
        assert_eq!(check_engine(None, |_| true).unwrap_err(), EngineProblem::NotInstalled);
        assert_eq!(check_engine(Some(exe.clone()), |_| false).unwrap_err(), EngineProblem::NotGenuine);
        assert_eq!(check_engine(Some(exe.clone()), |_| true).unwrap().exe, exe);
        assert!(EngineProblem::NotInstalled.message().contains("Steam library"));
        assert!(EngineProblem::NotGenuine.message().contains("signature"));
    }

    #[test]
    fn cleanup_finds_a_window_appearing_near_the_open_deadline() {
        use std::cell::Cell;
        let elapsed = Cell::new(Duration::ZERO);
        let found = find_with_grace(Duration::from_secs(15),
            || (elapsed.get() >= Duration::from_secs(14)).then_some(42),
            || elapsed.get(),
            || elapsed.set(elapsed.get() + Duration::from_secs(1)));
        assert_eq!(found, Some(42));
    }

    #[test]
    fn failed_close_keeps_the_window_announced_and_retries() {
        let announcements = Mutex::new(Vec::new());
        let announce = |title: Option<&str>| announcements.lock().unwrap().push(title.map(str::to_owned));
        let mut guard = WindowGuard::new(Path::new("engine"), "title", &announce);
        guard.opened();
        assert!(!guard.close_now_with(|_, _, grace| { assert!(grace > Duration::from_secs(14)); false }));
        assert_eq!(announcements.lock().unwrap().as_slice(), &[Some("title".to_owned())]);
        assert!(guard.close_now_with(|_, _, _| true));
        assert_eq!(announcements.lock().unwrap().as_slice(), &[Some("title".to_owned()), None]);
    }

    #[cfg(windows)]
    #[test]
    fn engine_path_lock_denies_replacement_and_write() {
        let dir = crate::download_tests::temp_dir("engine-lock");
        let nested = dir.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let exe = nested.join("wallpaper64.exe");
        std::fs::write(&exe, b"test").unwrap();
        let held = lock_engine_path(&exe).unwrap();
        assert!(std::fs::write(&exe, b"replace").is_err());
        assert!(std::fs::rename(&nested, dir.join("renamed")).is_err());
        drop(held);
        std::fs::write(&exe, b"replace").unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_the_exact_publisher_name_passes_and_lookalikes_do_not() {
        assert_eq!(PUBLISHER, "Skutta Software GmbH");
        for lookalike in ["Skutta Software GmbH.", "Skutta Software", "skutta software gmbh", "Skutta Software GmbH (x)", "Skutta  Software GmbH", " Skutta Software GmbH", "Skutta Softvvare GmbH", "Skutta Software GmbH, Inc"] {
            assert!(!crate::signature::is_publisher(lookalike, PUBLISHER), "{lookalike:?}");
        }
        assert!(crate::signature::is_publisher("Skutta Software GmbH", PUBLISHER));
        // A file that is not signed at all never passes.
        let me = std::env::current_exe().unwrap();
        assert!(!signed_by_the_publisher(&me));
    }

    // ----- sizes -----

    #[test]
    fn sizes_follow_the_scene_up_to_its_authored_size_and_never_beyond() {
        let all = |e: Encoder| e != Encoder::Amf && e != Encoder::Qsv;
        let sizes = size_options(fixed(1920, 1080), display((3860, 1106), (1920, 1080)), &all);
        assert_eq!(heights(&sizes), [720, 1080]);
        assert_eq!(sizes.iter().map(|s| s.width).collect::<Vec<_>>(), [1280, 1920]);
        let sizes = size_options(fixed(3840, 2160), display((3860, 1106), (1920, 1080)), &all);
        assert_eq!(heights(&sizes), [720, 1080, 1440, 2160]);
        assert_eq!(sizes.last().unwrap().width, 3840);
        // Authored at 4K but in 21:9: the shape is kept.
        let wide = size_options(fixed(3440, 1440), None, &all);
        assert_eq!(heights(&wide), [720, 1080, 1440]);
        assert_eq!(wide.iter().map(|s| s.width).collect::<Vec<_>>(), [1720, 2580, 3440]);
        // Authored smaller than any standard size: its own size is the one choice.
        let small = size_options(fixed(1200, 696), None, &all);
        assert_eq!(heights(&small), [696]);
        assert_eq!(small[0].label, "696p");
        // An odd authored size sits beside the standard ones, never above the authored height.
        let tall = size_options(fixed(1920, 1200), None, &all);
        assert_eq!(heights(&tall), [720, 1080, 1200]);
        assert_eq!(size_options(fixed(1873, 927), None, &all).last().unwrap().height, 926, "even sides only");
    }

    #[test]
    fn a_scene_with_no_fixed_size_takes_the_displays_shape_and_has_no_cap() {
        let all = |_: Encoder| true;
        let auto = size_options(Authored::Auto, display((3860, 1106), (2560, 1440)), &all);
        assert_eq!(heights(&auto), [720, 1080, 1440, 2160]);
        assert_eq!(auto[1].width, 1920, "16:9 from a 2560x1440 display");
        let ultra = size_options(Authored::Perspective, display((8000, 4000), (3440, 1440)), &all);
        assert_eq!(ultra[1].width, 2580, "21:9 from an ultrawide display");
        let unknown = size_options(Authored::Perspective, None, &all);
        assert_eq!(unknown[1].width, 1920, "16:9 when the display is not known");
    }

    #[test]
    fn sizes_beyond_what_a_display_reports_are_flagged_not_removed() {
        let all = |_: Encoder| true;
        let sizes = size_options(fixed(3840, 2160), display((3860, 1106), (1920, 1080)), &all);
        let flags: Vec<(u32, bool)> = sizes.iter().map(|s| (s.height, s.beyond_display)).collect();
        assert_eq!(flags, [(720, false), (1080, false), (1440, true), (2160, true)]);
        assert!(sizes.iter().all(|s| s.available), "a borderless window is not held to the framed limit");
        let big = size_options(fixed(3840, 2160), display((3860, 2200), (3840, 2160)), &all);
        assert!(big.iter().all(|s| !s.beyond_display), "a 4K display holds all of them");
        assert!(size_options(fixed(3840, 2160), None, &all).iter().all(|s| !s.beyond_display));
    }

    #[test]
    fn software_alone_records_up_to_1440p_at_30_and_never_more() {
        let sizes = size_options(fixed(3840, 2160), None, &nothing);
        let by = |h: u32| sizes.iter().find(|s| s.height == h).unwrap();
        assert!(by(720).available && by(1080).available && by(1440).available);
        assert_eq!(by(1440).fps, [30], "60 fps needs a hardware encoder");
        assert!(!by(2160).available);
        assert!(by(2160).reason.as_deref().unwrap().contains("hardware video encoder"));
        assert!(by(2160).fps.is_empty());
        // With a hardware encoder everything is offered at both rates.
        let hw = size_options(fixed(3840, 2160), None, &nvenc);
        assert!(hw.iter().all(|s| s.available && s.fps == [30, 60]), "{hw:?}");
    }

    #[test]
    fn the_default_is_1080p_or_the_largest_when_smaller_and_a_request_is_checked() {
        let all = |_: Encoder| true;
        let big = size_options(fixed(3840, 2160), None, &all);
        assert_eq!(default_height(&big), Some(1080));
        let small = size_options(fixed(1280, 720), None, &all);
        assert_eq!(default_height(&small), Some(720));
        let odd = size_options(fixed(1200, 696), None, &all);
        assert_eq!(default_height(&odd), Some(696));
        assert_eq!(default_height(&[]), None);
        // No hardware: 1080p is still the default; 4K is refused with its reason.
        let soft = size_options(fixed(3840, 2160), None, &nothing);
        assert_eq!(default_height(&soft), Some(1080));
        assert_eq!(choose_size(&big, None).unwrap().height, 1080);
        assert_eq!(choose_size(&big, Some(2160)).unwrap().width, 3840);
        assert!(choose_size(&small, Some(2160)).unwrap_err().contains("stretch"), "never upscaled");
        assert!(choose_size(&soft, Some(2160)).unwrap_err().contains("hardware video encoder"));
        assert!(choose_size(&[], None).is_err());
    }

    #[test]
    fn the_pre_roll_is_longer_for_a_big_package() {
        assert_eq!(pre_roll_seconds(0), 2);
        assert_eq!(pre_roll_seconds(500 * 1024 * 1024), 2, "500 MB exactly is not over 500 MB");
        assert_eq!(pre_roll_seconds(500 * 1024 * 1024 + 1), 4);
        assert_eq!(pre_roll_seconds(1_268_000_000), 4, "the 1.2 GB scene");
    }

    #[test]
    fn the_time_and_size_are_estimated_from_the_length() {
        assert_eq!(expected_seconds(60, 2), 63.0);
        assert_eq!(expected_seconds(30, 4), 35.0);
        // 60 s at 1080p30: about 120 MB at the typical bitrate.
        let bytes = expected_video_bytes((1920, 1080), 30, 60);
        assert!((100_000_000..=200_000_000).contains(&bytes), "{bytes}");
        assert!(expected_video_bytes((3840, 2160), 30, 60) > bytes);
        assert_eq!(expected_picture_bytes(1920, 1080), 2_488_320);
    }

    // ----- FFmpeg's capture -----

    #[test]
    fn the_capture_names_one_window_discards_the_pre_roll_and_never_overwrites() {
        let capture = Capture { width: 1920, height: 1080, fps: 30, kind: CaptureKind::Video { length: 60, encoder: Encoder::Nvenc } };
        let args = capture_args(131_458, &capture, 2, Path::new(r"C:\out\x.partial"));
        let filter = args.iter().find(|a| a.starts_with("gfxcapture")).expect("the capture filter");
        assert_eq!(filter, "gfxcapture=hwnd=131458:capture_cursor=0:max_framerate=30,hwdownload,format=bgra,fps=30,format=yuv420p");
        assert!(args.windows(2).any(|p| p == ["-filter_complex", filter.as_str()]));
        assert!(args.windows(2).any(|p| p == ["-ss", "2"]));
        assert!(args.windows(2).any(|p| p == ["-t", "60"]));
        assert!(args.windows(2).any(|p| p == ["-c:v", "h264_nvenc"]));
        assert!(args.windows(2).any(|p| p == ["-progress", "pipe:1"]));
        assert!(args.windows(2).any(|p| p == ["-movflags", "+faststart"]));
        assert!(args.iter().any(|a| a == "-n"), "an existing file is never replaced");
        assert!(args.iter().any(|a| a == "-nostdin"));
        assert_eq!(args.last().unwrap(), r"C:\out\x.partial");
        // The order matters: the cut is an output option, after the filter graph.
        let at = |needle: &str| args.iter().position(|a| a == needle).unwrap();
        assert!(at("-filter_complex") < at("-ss") && at("-ss") < at("-t") && at("-t") < at("-c:v"));
        assert_eq!(args.iter().filter(|a| a.starts_with("gfxcapture")).count(), 1, "one window");
    }

    #[test]
    fn a_picture_is_one_frame_after_the_pre_roll() {
        let capture = Capture { width: 1920, height: 1080, fps: 30, kind: CaptureKind::Picture };
        let args = capture_args(77, &capture, 2, Path::new("x.partial"));
        assert!(args.windows(2).any(|p| p == ["-frames:v", "1"]));
        assert!(args.windows(2).any(|p| p == ["-c:v", "png"]));
        assert!(args.windows(2).any(|p| p == ["-f", "image2"]));
        assert!(args.iter().any(|a| a.ends_with("format=rgb24")));
        assert!(!args.iter().any(|a| a == "-t" || a == "-movflags"));
    }

    #[test]
    fn the_window_title_carries_a_random_token() {
        let a = window_title(&uuid::Uuid::new_v4().simple().to_string());
        let b = window_title(&uuid::Uuid::new_v4().simple().to_string());
        assert!(a.starts_with("Claws ") && a.len() == "Claws ".len() + 32);
        assert_ne!(a, b);
    }

    // ----- checking the result -----

    fn probe(width: u32, height: u32, frames: Option<u64>) -> Probe {
        Probe {
            video_codec: "h264".into(),
            pix_fmt: "yuv420p".into(),
            width,
            height,
            fps: 30.0,
            duration: Some(60.0),
            audio_codec: None,
            format_name: "mov,mp4,m4a,3gp,3g2,mj2".into(),
            frames,
        }
    }

    #[test]
    fn a_recording_must_be_the_size_asked_for_with_about_the_right_number_of_frames() {
        let capture = Capture { width: 1920, height: 1080, fps: 30, kind: CaptureKind::Video { length: 60, encoder: Encoder::X264 } };
        assert!(check_probe(&probe(1920, 1080, Some(1800)), &capture).is_ok());
        assert!(check_probe(&probe(1920, 1080, Some(1782)), &capture).is_ok(), "exactly 1%");
        assert!(check_probe(&probe(1920, 1080, Some(1818)), &capture).is_ok());
        assert!(check_probe(&probe(1920, 1080, Some(1781)), &capture).unwrap_err().contains("1781 frames"));
        assert!(check_probe(&probe(1920, 1080, Some(900)), &capture).is_err(), "half the frames: the capture stalled");
        assert!(check_probe(&probe(1920, 1067, Some(1800)), &capture).unwrap_err().contains("1920×1067 instead of 1920×1080"));
        assert!(check_probe(&probe(1920, 1080, None), &capture).is_ok(), "no frame count: only the size is checked");
        // A short recording keeps a two-frame allowance.
        let short = Capture { width: 1280, height: 720, fps: 30, kind: CaptureKind::Video { length: 3, encoder: Encoder::X264 } };
        assert!(check_probe(&probe(1280, 720, Some(88)), &short).is_ok());
        assert!(check_probe(&probe(1280, 720, Some(87)), &short).is_err());
        let picture = Capture { width: 1920, height: 1080, fps: 30, kind: CaptureKind::Picture };
        assert!(check_probe(&probe(1920, 1080, Some(1)), &picture).is_ok(), "a picture has no frame count to check");
    }

    #[test]
    fn a_capture_that_never_changes_is_frozen_and_one_that_moves_is_not() {
        let metadata = "frame:0 pts:0 pts_time:0\nlavfi.signalstats.YAVG=0.000000\nframe:1 pts:1\nlavfi.signalstats.YAVG=1.455000\n";
        assert_eq!(parse_yavg(metadata), [0.0, 1.455]);
        assert!(parse_yavg("nothing useful").is_empty());
        assert!(parse_yavg("lavfi.signalstats.YAVG=abc\nlavfi.signalstats.YAVG=nan").is_empty());
        assert!(looks_frozen(&[0.0; 12]));
        assert!(looks_frozen(&[0.0, 0.002, 0.0, 0.001, 0.0, 0.0, 0.003, 0.0, 0.0, 0.0]));
        assert!(!looks_frozen(&[0.0, 0.0, 0.0, 1.4, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]), "one changing frame is movement");
        assert!(!looks_frozen(&[1.0; 30]));
        assert!(!looks_frozen(&[0.0; 5]), "too few samples to say");
        assert!(!looks_frozen(&[]));
        assert_eq!(FROZEN, "The recording didn't change; Wallpaper Engine may have paused. Keep its window visible and try again.");
    }

    // ----- the window guard -----

    #[test]
    fn the_window_is_announced_and_closed_when_the_guard_is_dropped_on_any_path() {
        let events = Mutex::new(Vec::<Option<String>>::new());
        let announce = |title: Option<&str>| events.lock().unwrap().push(title.map(str::to_string));
        // No Wallpaper Engine here: closing a window that does not exist is a no-op, and must
        // not start anything.
        let engine = Path::new(r"C:\no such folder\wallpaper64.exe");
        {
            let _guard = WindowGuard::new(engine, "Claws a", &announce);
            // The scope ends as an early return, a failure or a cancel would end it.
        }
        assert_eq!(*events.lock().unwrap(), [Some("Claws a".to_string()), None]);
        events.lock().unwrap().clear();
        {
            let mut guard = WindowGuard::new(engine, "Claws b", &announce);
            assert!(guard.close_now(), "nothing to close counts as closed");
            assert!(guard.close_now(), "closing again does nothing more");
        }
        assert_eq!(*events.lock().unwrap(), [Some("Claws b".to_string()), None], "announced gone once");
        // A panic unwinding through the scope still closes it.
        events.lock().unwrap().clear();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = WindowGuard::new(engine, "Claws c", &announce);
            panic!("the recording blew up");
        }));
        assert!(result.is_err());
        assert_eq!(*events.lock().unwrap(), [Some("Claws c".to_string()), None]);
    }

    #[test]
    fn the_interface_reads_the_record_info_in_camel_case() {
        let info = RecordInfo {
            kind: "scene",
            available: true,
            reason: None,
            moves: Some(true),
            has_audio: false,
            sizes: size_options(fixed(1920, 1080), None, &nvenc),
            default_height: Some(1080),
            pre_roll_seconds: 2,
            encoder: Some("h264_nvenc"),
            engine_will_start: false,
            display_limit: Some((3860, 1106)),
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["kind"], "scene");
        assert_eq!(json["preRollSeconds"], 2);
        assert_eq!(json["sizes"][1]["height"], 1080);
        assert_eq!(json["sizes"][1]["beyondDisplay"], false);
        assert_eq!(json["sizes"][1]["fps"], serde_json::json!([30, 60]));
        assert_eq!(json["engineWillStart"], false);
    }

    // ----- live: the real Wallpaper Engine -----
    //
    // These need the owner's PC (Wallpaper Engine installed and signed, Claws' FFmpeg with
    // gfxcapture). They open and close one pop-out window through Wallpaper Engine's own command
    // line, record into a temporary folder and leave nothing behind. They are skipped unless the
    // variable below names a subscribed item.

    struct LiveRun {
        events: Vec<crate::export_progress::ProgressEvent>,
        probe: Probe,
        title: Option<String>,
        engine: PathBuf,
        outcome: Recorded,
        elapsed: Duration,
        bytes: u64,
        dir: PathBuf,
        file: PathBuf,
    }

    /// Records `length` seconds (or one picture) of a library item at `height` and `fps`.
    async fn live_record(item: &str, height: u32, fps: u32, length: u32, picture: bool, encoder_name: Option<Encoder>) -> Result<LiveRun, RecordFailure> {
        live_record_with(item, height, fps, length, picture, encoder_name, false).await
    }

    async fn live_record_with(item: &str, height: u32, fps: u32, length: u32, picture: bool, encoder_name: Option<Encoder>, drive_full: bool) -> Result<LiveRun, RecordFailure> {
        let tools = crate::download_tests::ffmpeg_dir().expect("needs Claws' ffmpeg and ffprobe");
        let ffmpeg = tools.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
        let ffprobe = tools.join(format!("ffprobe{}", std::env::consts::EXE_SUFFIX));
        let engine = find_engine().unwrap_or_else(|problem| panic!("{}", problem.message()));
        let root = crate::steam::steam_root().expect("Steam is installed");
        let libraries = crate::steam::library_folders(&root);
        let id: u64 = item.parse().expect("an item id");
        let project = crate::steam::locate_item(&libraries, id, None).project.expect("the item is in the library");
        let scene = crate::scene_pkg::analyze_file(&project.join("scene.pkg")).expect("a scene package");
        let working = encoder::available(&ffmpeg).await;
        let works = |candidate: Encoder| working.contains(&candidate);
        let sizes = size_options(scene.authored, win_window::display(), &works);
        let size = match choose_size(&sizes, Some(height)) {
            Ok(size) => size,
            Err(message) => return Err(RecordFailure { message, detail: String::new() }),
        };
        let chosen = encoder_name.or_else(|| encoder::pick_for(&works, size.height, fps)).expect("an encoder");
        let capture = Capture {
            width: size.width,
            height: size.height,
            fps,
            kind: if picture { CaptureKind::Picture } else { CaptureKind::Video { length, encoder: chosen } },
        };
        let dir = crate::download_tests::temp_dir("live-record");
        let partial = dir.join("recording.partial");
        let events = std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let tracker = Tracker::new("live", move |event| sink.lock().unwrap().push(event));
        let titles = Mutex::new(Vec::<String>::new());
        let announce = |title: Option<&str>| {
            if let Some(title) = title {
                titles.lock().unwrap().push(title.to_string());
            }
        };
        let cancelled = || false;
        let on_spawn = |_pid: Option<u32>| true;
        let low_space = || drive_full;
        let request = RecordRequest {
            ffmpeg: &ffmpeg,
            ffprobe: Some(&ffprobe),
            engine: &engine,
            project_json: &project.join("project.json"),
            partial: &partial,
            capture,
            pre_roll: pre_roll_seconds(scene.package_bytes),
            moves: scene.moves,
            tracker: &tracker,
            cancelled: &cancelled,
            on_spawn: &on_spawn,
            low_space: &low_space,
            announce: &announce,
        };
        let started = Instant::now();
        let outcome = record(&request).await?;
        let elapsed = started.elapsed();
        let title = titles.lock().unwrap().first().cloned();
        let file = dir.join(if picture { "recording.png" } else { "recording.mp4" });
        let (probe, bytes) = if matches!(outcome, Recorded::Done { .. }) {
            let probe = crate::workshop_media::probe_file(&ffprobe, &partial).await.expect("ffprobe reads the recording");
            let bytes = std::fs::metadata(&partial).unwrap().len();
            std::fs::rename(&partial, &file).unwrap();
            (probe, bytes)
        } else {
            let nothing = Probe { video_codec: String::new(), pix_fmt: String::new(), width: 0, height: 0, fps: 0.0, duration: None, audio_codec: None, format_name: String::new(), frames: None };
            (nothing, 0)
        };
        let events = events.lock().unwrap().clone();
        Ok(LiveRun { events, probe, title, engine: engine.exe, outcome, elapsed, bytes, dir, file })
    }

    fn assert_window_gone(run: &LiveRun) {
        let title = run.title.as_deref().expect("a window was opened");
        assert_eq!(win_window::find_window(title, &run.engine), None, "Wallpaper Engine's window for this recording is gone");
    }

    /// Live: `CLAWS_TEST_WALLPAPER_ENGINE=<itemId>` records 3 s of that scene at 720p30 and checks
    /// the size, the frames, the motion, the stages and that the window is closed.
    #[tokio::test]
    async fn records_three_seconds_of_a_real_scene_when_given() {
        let Some(item) = std::env::var_os("CLAWS_TEST_WALLPAPER_ENGINE") else { return };
        let item = item.to_string_lossy().into_owned();
        let run = live_record(&item, 720, 30, 3, false, None).await.unwrap_or_else(|f| panic!("{} {}", f.message, f.detail));
        assert!(matches!(run.outcome, Recorded::Done { .. }));
        assert_eq!((run.probe.width, run.probe.height), (1280, 720), "exactly the size asked for");
        let frames = run.probe.frames.expect("the container counts frames");
        assert!((88..=92).contains(&frames), "3 s at 30 fps is 90 frames: {frames}");
        assert_window_gone(&run);
        // The stages the tracker announced, in order, ending with the file being finished.
        let stages: Vec<Stage> = run.events.iter().map(|event| event.stage).fold(Vec::new(), |mut seen, stage| {
            if seen.last() != Some(&stage) {
                seen.push(stage);
            }
            seen
        });
        assert_eq!(stages, [Stage::Checking, Stage::Opening, Stage::Loading, Stage::Recording, Stage::Finishing], "{stages:?}");
        let recorded: Vec<f64> = run.events.iter().filter(|e| e.stage == Stage::Recording).filter_map(|e| e.done).collect();
        assert!(!recorded.is_empty() && recorded.windows(2).all(|pair| pair[0] <= pair[1]), "recording seconds never go back: {recorded:?}");
        assert!(recorded.iter().all(|done| *done <= 2.8 + 1e-9), "held 0.2 s short of the end until FFmpeg is done: {recorded:?}");
        // A moving scene: the motion check inside record() has already accepted it; prove it here too.
        let tools = crate::download_tests::ffmpeg_dir().unwrap();
        let ffmpeg = tools.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
        assert_eq!(frozen_capture(&ffmpeg, &run.file, 3).await, Some(false), "a moving scene is not frozen");
        eprintln!("recorded {item}: {} frames, {} bytes, {:.1}s total, events {}", frames, run.bytes, run.elapsed.as_secs_f64(), run.events.len());
        for event in &run.events {
            eprintln!("  {:>6} ms  {:<9}  done {:?} / {:?}  eta {:?}  bytes {:?}", event.elapsed_ms, event.stage.name(), event.done, event.total, event.eta_ms, event.bytes);
        }
        std::fs::remove_dir_all(&run.dir).unwrap();
    }

    /// Live: `CLAWS_TEST_WALLPAPER_ENGINE_PNG=<itemId>` takes one picture of that scene at 1080p.
    #[tokio::test]
    async fn takes_a_picture_of_a_real_scene_when_given() {
        let Some(item) = std::env::var_os("CLAWS_TEST_WALLPAPER_ENGINE_PNG") else { return };
        let item = item.to_string_lossy().into_owned();
        let run = live_record(&item, 1080, 30, 1, true, None).await.unwrap_or_else(|f| panic!("{} {}", f.message, f.detail));
        assert!(matches!(run.outcome, Recorded::Done { .. }));
        assert_eq!((run.probe.width, run.probe.height), (1920, 1080));
        let head = std::fs::read(&run.file).unwrap();
        assert_eq!(&head[..8], b"\x89PNG\r\n\x1a\n", "a PNG file");
        assert_window_gone(&run);
        eprintln!("picture of {item}: {} bytes in {:.1}s", run.bytes, run.elapsed.as_secs_f64());
        std::fs::remove_dir_all(&run.dir).unwrap();
    }

    /// Live: `CLAWS_TEST_WALLPAPER_ENGINE_LOW_SPACE=<itemId>` starts a 60 s recording on a drive that
    /// reports itself full: it must stop within seconds, say so, and leave nothing behind.
    #[tokio::test]
    async fn a_recording_stops_when_the_drive_fills_when_given() {
        let Some(item) = std::env::var_os("CLAWS_TEST_WALLPAPER_ENGINE_LOW_SPACE") else { return };
        let item = item.to_string_lossy().into_owned();
        let started = Instant::now();
        let run = live_record_with(&item, 720, 30, 60, false, None, true).await.unwrap_or_else(|f| panic!("{} {}", f.message, f.detail));
        assert_eq!(run.outcome, Recorded::LowSpace);
        assert!(started.elapsed() < Duration::from_secs(25), "{:?}", started.elapsed());
        assert_window_gone(&run);
        std::fs::remove_dir_all(&run.dir).ok();
    }

    /// Live: `CLAWS_TEST_WALLPAPER_ENGINE_4K=<itemId>` records 3 s at 3840×2160. Where the display
    /// lets a borderless window reach that size the recording must be exactly 3840×2160; where it
    /// does not, the failure must say so plainly and nothing is saved.
    #[tokio::test]
    async fn records_four_k_when_the_window_can_reach_it() {
        let Some(item) = std::env::var_os("CLAWS_TEST_WALLPAPER_ENGINE_4K") else { return };
        let item = item.to_string_lossy().into_owned();
        match live_record(&item, 2160, 30, 3, false, None).await {
            Ok(run) => {
                assert_eq!((run.probe.width, run.probe.height), (3840, 2160), "never a stretched picture");
                assert!(run.probe.frames.is_some_and(|frames| (88..=92).contains(&frames)));
                assert_window_gone(&run);
                eprintln!("4K reached: {} frames, {} bytes, {:.1}s", run.probe.frames.unwrap_or(0), run.bytes, run.elapsed.as_secs_f64());
                std::fs::remove_dir_all(&run.dir).unwrap();
            }
            Err(failure) => {
                if failure.message.contains("stretch") {
                    eprintln!("skipped: this scene wasn't made at 4K ({})", failure.message);
                    return;
                }
                assert!(failure.message.contains("4K needs a display set to 3840×2160"), "{}", failure.message);
                eprintln!("4K not reachable on this display: {}", failure.message);
            }
        }
    }


    /// What a cancelled recording must leave behind: nothing.
    struct Cancelled {
        outcome: Result<Recorded, RecordFailure>,
        title: Option<String>,
        engine: PathBuf,
        partial_exists: bool,
        ffmpeg_pid: Option<u32>,
        reached: Vec<Stage>,
    }

    fn pid_alive(pid: u32) -> bool {
        let output = std::process::Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"]).output().unwrap();
        String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
    }

    /// Records 20 s of a scene at 720p and cancels once `stage` has been reached and `after`
    /// has passed. With `like_shutdown` the way out is the one Claws' shutdown takes: FFmpeg's tree
    /// is killed and the window is closed from outside, while the recording is still running.
    async fn live_cancel(item: &str, stage: Stage, after: Duration, like_shutdown: bool) -> Cancelled {
        let tools = crate::download_tests::ffmpeg_dir().expect("needs Claws' ffmpeg and ffprobe");
        let ffmpeg = tools.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
        let ffprobe = tools.join(format!("ffprobe{}", std::env::consts::EXE_SUFFIX));
        let engine = find_engine().unwrap_or_else(|problem| panic!("{}", problem.message()));
        let root = crate::steam::steam_root().expect("Steam is installed");
        let libraries = crate::steam::library_folders(&root);
        let project = crate::steam::locate_item(&libraries, item.parse().unwrap(), None).project.expect("the item is in the library");
        let dir = crate::download_tests::temp_dir("live-cancel");
        let partial = dir.join("recording.partial");
        let events = std::sync::Arc::new(Mutex::new(Vec::<crate::export_progress::ProgressEvent>::new()));
        let sink = events.clone();
        let tracker = Tracker::new("live", move |event| sink.lock().unwrap().push(event));
        let reached_at = std::sync::Arc::new(Mutex::new(None::<Instant>));
        let watch_events = events.clone();
        let watch_reached = reached_at.clone();
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let titles = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
        let pids = std::sync::Arc::new(Mutex::new(Vec::<u32>::new()));
        let engine_exe = engine.exe.clone();
        let task = {
            let (flag, titles, pids, tracker) = (flag.clone(), titles.clone(), pids.clone(), tracker.clone());
            let (ffmpeg, ffprobe, partial, project) = (ffmpeg.clone(), ffprobe.clone(), partial.clone(), project.clone());
            tokio::spawn(async move {
                let cancelled = || flag.load(std::sync::atomic::Ordering::SeqCst);
                let on_spawn = |pid: Option<u32>| {
                    if let Some(pid) = pid {
                        pids.lock().unwrap().push(pid);
                    }
                    true
                };
                let announce = |title: Option<&str>| {
                    if let Some(title) = title {
                        titles.lock().unwrap().push(title.to_string());
                    }
                };
                let low_space = || false;
                let request = RecordRequest {
                    ffmpeg: &ffmpeg,
                    ffprobe: Some(&ffprobe),
                    engine: &engine,
                    project_json: &project.join("project.json"),
                    partial: &partial,
                    capture: Capture { width: 1280, height: 720, fps: 30, kind: CaptureKind::Video { length: 20, encoder: Encoder::X264 } },
                    pre_roll: 2,
                    moves: false,
                    tracker: &tracker,
                    cancelled: &cancelled,
                    on_spawn: &on_spawn,
                    low_space: &low_space,
                    announce: &announce,
                };
                record(&request).await
            })
        };
        // Wait for the stage, then the extra time, then cancel.
        let deadline = Instant::now() + Duration::from_secs(40);
        while Instant::now() < deadline && !task.is_finished() {
            let at_stage = watch_events.lock().unwrap().last().is_some_and(|event| event.stage == stage);
            if at_stage {
                let mut when = watch_reached.lock().unwrap();
                let since = *when.get_or_insert_with(Instant::now);
                if since.elapsed() >= after {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if like_shutdown {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            for pid in pids.lock().unwrap().iter() {
                kill_tree(*pid, true);
            }
            if let Some(title) = titles.lock().unwrap().first() {
                close_window(&engine_exe, title, Duration::ZERO);
            }
        } else {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let outcome = task.await.unwrap();
        let reached = events.lock().unwrap().iter().map(|event| event.stage).fold(Vec::new(), |mut seen, stage| {
            if seen.last() != Some(&stage) {
                seen.push(stage);
            }
            seen
        });
        let result = Cancelled {
            outcome,
            title: titles.lock().unwrap().first().cloned(),
            engine: engine_exe,
            partial_exists: partial.exists(),
            ffmpeg_pid: pids.lock().unwrap().first().copied(),
            reached,
        };
        std::fs::remove_dir_all(&dir).unwrap();
        result
    }

    /// Live: `CLAWS_TEST_WALLPAPER_ENGINE_CANCEL=<itemId>` cancels a recording while Wallpaper
    /// Engine opens, while the scene loads and while it records, and once the way Claws' shutdown
    /// ends one. Each leaves no window, no process and no partial file.
    #[tokio::test]
    async fn a_cancelled_recording_leaves_nothing_behind_when_given() {
        let Some(item) = std::env::var_os("CLAWS_TEST_WALLPAPER_ENGINE_CANCEL") else { return };
        let item = item.to_string_lossy().into_owned();
        for (name, stage, after, shutdown) in [
            ("while opening", Stage::Opening, Duration::ZERO, false),
            ("while loading", Stage::Loading, Duration::from_millis(300), false),
            ("while recording", Stage::Recording, Duration::from_millis(1500), false),
            ("like a shutdown while recording", Stage::Recording, Duration::from_millis(1500), true),
        ] {
            let run = live_cancel(&item, stage, after, shutdown).await;
            assert_eq!(run.outcome, Ok(Recorded::Cancelled), "{name}: {:?}", run.reached);
            assert!(run.reached.contains(&stage), "{name}: reached {:?}", run.reached);
            assert!(!run.partial_exists, "{name}: no partial file");
            if let Some(title) = &run.title {
                assert_eq!(win_window::find_window(title, &run.engine), None, "{name}: Wallpaper Engine's window is gone");
            }
            if let Some(pid) = run.ffmpeg_pid {
                assert!(!pid_alive(pid), "{name}: FFmpeg (pid {pid}) is gone");
            }
            eprintln!("cancel {name}: ok, stages {:?}", run.reached);
        }
    }

}

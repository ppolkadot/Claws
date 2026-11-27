//! Which H.264 encoder FFmpeg uses for a conversion or a recording. A hardware encoder is
//! preferred, because libx264 at its `veryfast` preset encodes real 4K at about 0.84 times real
//! time. Availability is decided by running a tiny encode once per FFmpeg build, never by
//! reading a version string, and the first working encoder in a fixed order wins.
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Encoder {
    Nvenc,
    Amf,
    Qsv,
    MediaFoundation,
    X264,
}

/// The probe order: NVIDIA, AMD, Intel, then Windows' own hardware encoder, then software.
pub const ORDER: [Encoder; 5] = [Encoder::Nvenc, Encoder::Amf, Encoder::Qsv, Encoder::MediaFoundation, Encoder::X264];

/// libx264 keeps up with a live capture only up to this size and rate (real 4K ran at 0.84x).
pub const SOFTWARE_MAX_HEIGHT: u32 = 1440;
pub const SOFTWARE_MAX_FPS: u32 = 30;

impl Encoder {
    pub fn name(self) -> &'static str {
        match self {
            Self::Nvenc => "h264_nvenc",
            Self::Amf => "h264_amf",
            Self::Qsv => "h264_qsv",
            Self::MediaFoundation => "h264_mf",
            Self::X264 => "libx264",
        }
    }

    pub fn hardware(self) -> bool {
        self != Self::X264
    }

    /// Whether this encoder may be used for a picture this size at this rate. Software is
    /// limited to what it can encode in real time.
    pub fn handles(self, height: u32, fps: u32) -> bool {
        self.hardware() || (height <= SOFTWARE_MAX_HEIGHT && fps <= SOFTWARE_MAX_FPS)
    }

    /// The codec and quality arguments: about 20 CRF-equivalent, `yuv420p`, and an H.264 level
    /// that fits the picture. Every argument is its own element; nothing goes through a shell.
    pub fn args(self, width: u32, height: u32, fps: u32) -> Vec<String> {
        let level = h264_level(width, height, fps);
        let mut args: Vec<String> = ["-c:v", self.name()].map(str::to_string).into();
        let more: Vec<&str> = match self {
            Self::X264 => vec!["-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p"],
            Self::Nvenc => vec!["-preset", "p5", "-tune", "hq", "-rc", "vbr", "-cq", "21", "-b:v", "0", "-profile:v", "high", "-level", level, "-pix_fmt", "yuv420p"],
            Self::Amf => vec![
                "-quality", "quality", "-rc", "cqp", "-qp_i", "20", "-qp_p", "22", "-qp_b", "22", "-profile:v", "high", "-level", level, "-pix_fmt", "yuv420p",
            ],
            Self::Qsv => vec!["-preset", "veryfast", "-global_quality", "21", "-profile:v", "high", "-level", level, "-pix_fmt", "nv12"],
            Self::MediaFoundation => {
                let bitrate = format!("{}k", typical_bitrate_kbps(height, fps));
                args.extend(["-rate_control", "pc_vbr", "-b:v", &bitrate].map(str::to_string));
                // Media Foundation takes no `-profile:v high`: it rejects the name.
                vec!["-pix_fmt", "yuv420p"]
            }
        };
        args.extend(more.into_iter().map(str::to_string));
        args
    }
}

/// The lowest H.264 level whose frame size and macroblock rate hold the picture.
pub fn h264_level(width: u32, height: u32, fps: u32) -> &'static str {
    // (level, max macroblocks per second, max macroblocks per frame), from the standard.
    const LEVELS: [(&str, u64, u64); 10] = [
        ("3.1", 108_000, 3_600),
        ("3.2", 216_000, 5_120),
        ("4.0", 245_760, 8_192),
        ("4.2", 522_240, 8_704),
        ("5.0", 589_824, 22_080),
        ("5.1", 983_040, 36_864),
        ("5.2", 2_073_600, 36_864),
        ("6.0", 4_177_920, 139_264),
        ("6.1", 8_355_840, 139_264),
        ("6.2", 16_711_680, 139_264),
    ];
    let per_frame = u64::from(width.div_ceil(16)) * u64::from(height.div_ceil(16));
    let per_second = per_frame * u64::from(fps.max(1));
    LEVELS
        .iter()
        .find(|(_, rate, frame)| per_second <= *rate && per_frame <= *frame)
        .map_or("6.2", |(level, _, _)| level)
}

/// A typical output bitrate for a live capture, in kbit/s. It sets Media Foundation's target and
/// the size estimate shown before a recording. Real use varies with the scene.
pub fn typical_bitrate_kbps(height: u32, fps: u32) -> u32 {
    let base: u32 = match height {
        0..=720 => 8_000,
        721..=1080 => 16_000,
        1081..=1440 => 28_000,
        _ => 50_000,
    };
    if fps > 30 {
        base * 3 / 2
    } else {
        base
    }
}

/// The best encoder that can handle this picture, from those that work. `None` when only
/// software works and the picture is too large or too fast for it.
pub fn pick_for(works: &dyn Fn(Encoder) -> bool, height: u32, fps: u32) -> Option<Encoder> {
    ORDER.into_iter().find(|encoder| encoder.handles(height, fps) && (*encoder == Encoder::X264 || works(*encoder)))
}

fn cache() -> &'static Mutex<HashMap<PathBuf, Vec<Encoder>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Vec<Encoder>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The arguments of the check encode: a short black clip into the null muxer.
fn probe_args(encoder: Encoder) -> Vec<String> {
    let mut args: Vec<String> = ["-nostdin", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=black:s=1280x720:r=30:d=0.4", "-frames:v", "6"]
        .map(str::to_string)
        .into();
    args.extend(encoder.args(1280, 720, 30));
    args.extend(["-f", "null", "-"].map(str::to_string));
    args
}

/// Probe candidates at the requested capture dimensions, trying the next one on failure.
#[cfg(test)]
async fn pick_tested_with<F, Fut>(working: &[Encoder], height: u32, fps: u32, mut test: F) -> Option<Encoder>
where F: FnMut(Encoder) -> Fut, Fut: std::future::Future<Output = bool> {
    for encoder in ORDER {
        if encoder.handles(height, fps) && (encoder == Encoder::X264 || working.contains(&encoder)) && test(encoder).await {
            return Some(encoder);
        }
    }
    None
}

pub async fn pick_tested(
    ffmpeg: &Path,
    working: &[Encoder],
    width: u32,
    height: u32,
    fps: u32,
    on_spawn: &(dyn Fn(Option<u32>) -> bool + Sync),
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Option<Encoder> {
    for encoder in ORDER {
        if cancelled() {
            return None;
        }
        if !encoder.handles(height, fps) || (encoder != Encoder::X264 && !working.contains(&encoder)) {
            continue;
        }
        let source = format!("color=c=black:s={width}x{height}:r={fps}");
        let mut args: Vec<String> = ["-nostdin", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", &source, "-frames:v", "2"].map(str::to_string).into();
        args.extend(encoder.args(width, height, fps));
        args.extend(["-f", "null", "-"].map(str::to_string));
        let mut command = Command::new(ffmpeg);
        command.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        let Ok(mut child) = command.spawn() else { continue };
        let pid = child.id();
        if !on_spawn(pid) {
            if let Some(pid) = pid { crate::downloader::kill_tree(pid, true); }
            let _ = child.kill().await;
            return None;
        }
        let result = tokio::time::timeout(Duration::from_secs(20), child.wait()).await;
        if result.is_err() {
            if let Some(pid) = pid { crate::downloader::kill_tree(pid, true); }
            let _ = child.kill().await;
        }
        on_spawn(None);
        if cancelled() {
            return None;
        }
        if matches!(result, Ok(Ok(status)) if status.success()) {
            return Some(encoder);
        }
    }
    None
}

/// Which encoders this FFmpeg can really use on this PC, in probe order. Each is tried with a
/// short encode; the answer is kept for the life of the app.
pub async fn available(ffmpeg: &Path) -> Vec<Encoder> {
    available_tracked(ffmpeg, &|_| true, &|| false).await.unwrap_or_default()
}

/// Same as `available`, but registers each child PID and stops between probes on cancel.
/// Returns `None` when cancelled so partial results are never cached.
pub async fn available_tracked(
    ffmpeg: &Path,
    on_spawn: &(dyn Fn(Option<u32>) -> bool + Sync),
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Option<Vec<Encoder>> {
    if cancelled() {
        return None;
    }
    if let Some(known) = cache().lock().unwrap().get(ffmpeg) {
        return Some(known.clone());
    }
    let mut working = Vec::new();
    for encoder in ORDER {
        if cancelled() {
            return None;
        }
        if encoder == Encoder::X264 {
            working.push(encoder);
            continue;
        }
        let mut command = Command::new(ffmpeg);
        command.args(probe_args(encoder)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let Ok(mut child) = command.spawn() else { continue };
        let pid = child.id();
        if !on_spawn(pid) {
            if let Some(pid) = pid { crate::downloader::kill_tree(pid, true); }
            let _ = child.kill().await;
            return None;
        }
        let result = tokio::time::timeout(Duration::from_secs(20), child.wait()).await;
        if result.is_err() {
            if let Some(pid) = pid { crate::downloader::kill_tree(pid, true); }
            let _ = child.kill().await;
        }
        on_spawn(None);
        if cancelled() {
            return None;
        }
        if matches!(result, Ok(Ok(status)) if status.success()) {
            working.push(encoder);
        }
    }
    cache().lock().unwrap().insert(ffmpeg.to_path_buf(), working.clone());
    Some(working)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_working_encoder_in_order_wins_and_software_is_the_floor() {
        let first = |works: &dyn Fn(Encoder) -> bool| pick_for(works, 720, 30);
        assert_eq!(first(&|_| true), Some(Encoder::Nvenc));
        assert_eq!(first(&|e| e != Encoder::Nvenc), Some(Encoder::Amf));
        assert_eq!(first(&|e| matches!(e, Encoder::Qsv | Encoder::MediaFoundation)), Some(Encoder::Qsv));
        assert_eq!(first(&|e| e == Encoder::MediaFoundation), Some(Encoder::MediaFoundation));
        assert_eq!(first(&|_| false), Some(Encoder::X264), "software is always there");
        assert_eq!(ORDER.map(Encoder::name), ["h264_nvenc", "h264_amf", "h264_qsv", "h264_mf", "libx264"]);
    }

    #[tokio::test]
    async fn tested_encoder_falls_back_when_initialization_fails() {
        let tried = Mutex::new(Vec::new());
        let picked = pick_tested_with(&ORDER, 2160, 60, |candidate| {
            tried.lock().unwrap().push(candidate);
            async move { candidate == Encoder::Qsv }
        }).await;
        assert_eq!(picked, Some(Encoder::Qsv));
        assert_eq!(*tried.lock().unwrap(), [Encoder::Nvenc, Encoder::Amf, Encoder::Qsv]);
    }

    #[tokio::test]
    async fn actual_size_probe_starts_and_releases_its_child() {
        let Some(tools) = crate::download_tests::ffmpeg_dir() else { return };
        let ffmpeg = tools.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
        let events = Mutex::new(Vec::new());
        let chosen = pick_tested(&ffmpeg, &[], 640, 360, 30, &|pid| { events.lock().unwrap().push(pid); true }, &|| false).await;
        assert_eq!(chosen, Some(Encoder::X264));
        let events = events.lock().unwrap();
        assert!(events.first().is_some_and(Option::is_some));
        assert_eq!(events.last(), Some(&None));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn cancelling_during_tested_encoder_probe_does_not_spawn_next_candidate() {
        use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
        let dir = crate::download_tests::temp_dir("pick-tested-cancel");
        let script = dir.join("slow-ffmpeg.cmd");
        std::fs::write(&script, "@echo off\r\nping -n 30 127.0.0.1 >nul\r\n").unwrap();

        let spawned = Arc::new(Mutex::new(Vec::<u32>::new()));
        let flag = Arc::new(AtomicBool::new(false));
        let (spawned_task, flag_task, script_task) = (spawned.clone(), flag.clone(), script.clone());
        let task = tokio::spawn(async move {
            pick_tested(
                &script_task,
                &[Encoder::Nvenc, Encoder::Amf],
                1920,
                1080,
                30,
                &|pid| {
                    if let Some(pid) = pid {
                        spawned_task.lock().unwrap().push(pid);
                    }
                    true
                },
                &|| flag_task.load(Ordering::SeqCst),
            ).await
        });

        let wait_start = std::time::Instant::now();
        while spawned.lock().unwrap().is_empty() && wait_start.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        let first_pid = *spawned.lock().unwrap().first().expect("candidate 1 spawned");
        flag.store(true, Ordering::SeqCst);
        crate::downloader::kill_tree(first_pid, true);

        let chosen = tokio::time::timeout(Duration::from_secs(5), task).await.expect("probe stopped promptly").unwrap();
        assert_eq!(chosen, None);
        assert_eq!(spawned.lock().unwrap().len(), 1, "candidate 2 must not be spawned after cancel");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn software_is_only_used_where_it_keeps_up() {
        let nothing = |_: Encoder| false;
        assert_eq!(pick_for(&nothing, 1080, 30), Some(Encoder::X264));
        assert_eq!(pick_for(&nothing, 1440, 30), Some(Encoder::X264));
        assert_eq!(pick_for(&nothing, 2160, 30), None, "4K needs a hardware encoder");
        assert_eq!(pick_for(&nothing, 1080, 60), None, "60 fps needs a hardware encoder");
        let nvenc = |e: Encoder| e == Encoder::Nvenc;
        assert_eq!(pick_for(&nvenc, 2160, 60), Some(Encoder::Nvenc));
        assert_eq!(pick_for(&nvenc, 720, 30), Some(Encoder::Nvenc), "hardware is preferred even for small pictures");
        let mf = |e: Encoder| e == Encoder::MediaFoundation;
        assert_eq!(pick_for(&mf, 2160, 30), Some(Encoder::MediaFoundation));
        assert!(Encoder::X264.handles(1440, 30) && !Encoder::X264.handles(1441, 30) && !Encoder::X264.handles(720, 31));
    }

    #[test]
    fn the_level_fits_the_picture_and_the_rate() {
        assert_eq!(h264_level(1280, 720, 30), "3.1");
        assert_eq!(h264_level(1920, 1080, 30), "4.0");
        assert_eq!(h264_level(1920, 1080, 60), "4.2");
        assert_eq!(h264_level(2560, 1440, 30), "5.0");
        assert_eq!(h264_level(2560, 1440, 60), "5.1");
        assert_eq!(h264_level(3840, 2160, 30), "5.1");
        assert_eq!(h264_level(3840, 2160, 60), "5.2");
        assert_eq!(h264_level(7680, 4320, 30), "6.0");
        // A one-pixel overhang still needs its whole macroblock.
        assert_eq!(h264_level(1281, 721, 30), h264_level(1296, 736, 30));
    }

    #[test]
    fn every_encoder_asks_for_yuv420_h264_with_its_own_quality_control() {
        for encoder in ORDER {
            let args = encoder.args(1920, 1080, 30);
            assert_eq!(&args[..2], ["-c:v", encoder.name()]);
            assert!(args.windows(2).any(|pair| pair[0] == "-pix_fmt" && (pair[1] == "yuv420p" || (encoder == Encoder::Qsv && pair[1] == "nv12"))), "{encoder:?}");
        }
        let x264 = Encoder::X264.args(1920, 1080, 30);
        assert!(x264.windows(2).any(|p| p == ["-crf", "20"]));
        assert!(x264.windows(2).any(|p| p == ["-preset", "veryfast"]));
        let nvenc = Encoder::Nvenc.args(3840, 2160, 60);
        assert!(nvenc.windows(2).any(|p| p == ["-cq", "21"]));
        assert!(nvenc.windows(2).any(|p| p == ["-level", "5.2"]));
        let mf = Encoder::MediaFoundation.args(1920, 1080, 30);
        assert!(mf.windows(2).any(|p| p == ["-b:v", "16000k"]));
    }

    #[test]
    fn the_check_encode_is_a_short_run_into_nothing() {
        let args = probe_args(Encoder::Nvenc);
        assert!(args.windows(2).any(|p| p == ["-c:v", "h264_nvenc"]));
        assert_eq!(args[args.len() - 3..], ["-f", "null", "-"]);
        assert!(args.iter().any(|a| a.starts_with("color=")));
    }

    #[test]
    fn bitrates_grow_with_size_and_rate() {
        assert!(typical_bitrate_kbps(720, 30) < typical_bitrate_kbps(1080, 30));
        assert!(typical_bitrate_kbps(1080, 30) < typical_bitrate_kbps(2160, 30));
        assert!(typical_bitrate_kbps(1080, 60) > typical_bitrate_kbps(1080, 30));
    }

    /// Live: runs the real probe with Claws' FFmpeg (or `CLAWS_TEST_FFMPEG_DIR`) and prints
    /// which encoders work on this PC. Software always does.
    #[tokio::test]
    async fn probes_the_encoders_this_pc_really_has() {
        let Some(tools) = crate::download_tests::ffmpeg_dir() else {
            eprintln!("skipped: needs ffmpeg");
            return;
        };
        let ffmpeg = tools.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
        let working = available(&ffmpeg).await;
        eprintln!("working encoders: {:?}", working.iter().map(|e| e.name()).collect::<Vec<_>>());
        assert!(working.contains(&Encoder::X264));
        // A second call answers from the cache, to the same list.
        assert_eq!(available(&ffmpeg).await, working);
    }
}

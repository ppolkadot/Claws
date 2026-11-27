//! The numbers behind an export's progress: FFmpeg's `-progress` output, the rate over a
//! sliding window, and the time left. Pure code, so every rule that decides what the bar and
//! the status line may claim is tested without running anything.
//!
//! The rule is the product's honest-progress rule: a number is reported only when it can be
//! backed. Nothing here invents a fraction, and the time left is `None` until there is enough
//! data to stand behind it.
#![allow(dead_code)] // some of this is used only by the export tracker, which lands in a later commit
use std::{collections::VecDeque, time::Duration};

// ---------------------------------------------------------------------------------------
// FFmpeg `-progress pipe:1`

/// One block of FFmpeg's `-progress` output: the values it reported, each `None` while FFmpeg
/// says `N/A` (as it does before the first frame is encoded).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FfmpegProgress {
    pub frame: Option<u64>,
    /// Output time reached, in microseconds. Never negative: FFmpeg prints a huge negative
    /// number or `N/A` before it has one.
    pub out_time_us: Option<u64>,
    pub total_size: Option<u64>,
    /// Encoding speed as a multiple of real time (`speed=1.23x`).
    pub speed: Option<f64>,
    pub dup_frames: Option<u64>,
    pub drop_frames: Option<u64>,
    /// `progress=end` was reported: FFmpeg is finished.
    pub end: bool,
}

/// Collects `key=value` lines into blocks. A block ends at its `progress=` line.
#[derive(Debug, Default)]
pub struct ProgressParser {
    current: FfmpegProgress,
}

impl ProgressParser {
    /// Feeds one line. Returns the finished block when the line is its `progress=` line.
    pub fn push(&mut self, line: &str) -> Option<FfmpegProgress> {
        let (key, value) = line.trim().split_once('=')?;
        let value = value.trim();
        let number = || value.parse::<u64>().ok();
        match key.trim() {
            "frame" => self.current.frame = number(),
            "out_time_us" => {
                self.current.out_time_us = value.parse::<i64>().ok().and_then(|v| u64::try_from(v).ok());
            }
            "total_size" => self.current.total_size = number(),
            "dup_frames" => self.current.dup_frames = number(),
            "drop_frames" => self.current.drop_frames = number(),
            "speed" => {
                self.current.speed = value.strip_suffix('x').unwrap_or(value).trim().parse::<f64>().ok().filter(|s| s.is_finite() && *s > 0.0);
            }
            "progress" => {
                let mut block = std::mem::take(&mut self.current);
                block.end = value == "end";
                return Some(block);
            }
            _ => {}
        }
        None
    }
}

// ---------------------------------------------------------------------------------------
// Rate and time left

/// Bytes (or any count) over time, for a rate over the last few seconds.
#[derive(Debug)]
pub struct RateWindow {
    window: Duration,
    samples: VecDeque<(Duration, u64)>,
}

impl RateWindow {
    pub fn new(window: Duration) -> Self {
        Self { window, samples: VecDeque::new() }
    }

    /// Records `done` at `at` (time since the start). Samples older than the window go.
    pub fn push(&mut self, at: Duration, done: u64) {
        if self.samples.back().is_some_and(|(last, _)| at < *last) {
            return;
        }
        self.samples.push_back((at, done));
        while self.samples.len() > 2 && self.samples.front().is_some_and(|(first, _)| at.saturating_sub(*first) > self.window) {
            self.samples.pop_front();
        }
    }

    /// Units per second over the window, or `None` with fewer than two samples or no time passed.
    pub fn rate(&self) -> Option<f64> {
        let (first_at, first) = *self.samples.front()?;
        let (last_at, last) = *self.samples.back()?;
        let seconds = last_at.checked_sub(first_at)?.as_secs_f64();
        (seconds > 0.0 && last >= first).then(|| (last - first) as f64 / seconds)
    }
}

/// Seconds left for a byte transfer, from the rate over the last window. Needs at least
/// `min_elapsed` of data, and a rate above zero. `None` is "no estimate yet".
pub fn eta_from_rate(done: u64, total: u64, rate: Option<f64>, elapsed: Duration, min_elapsed: Duration) -> Option<f64> {
    if elapsed < min_elapsed || done > total {
        return None;
    }
    let rate = rate.filter(|rate| *rate > 0.0)?;
    Some((total - done) as f64 / rate)
}

/// Seconds left for a conversion: the media left to encode divided by FFmpeg's speed. Needs
/// `min_elapsed` of data so the first slow seconds do not make a wild guess.
pub fn eta_from_speed(done_seconds: f64, total_seconds: f64, speed: Option<f64>, elapsed: Duration, min_elapsed: Duration) -> Option<f64> {
    if elapsed < min_elapsed || done_seconds < 0.0 || total_seconds <= 0.0 {
        return None;
    }
    let speed = speed.filter(|speed| *speed > 0.0)?;
    Some((total_seconds - done_seconds).max(0.0) / speed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(parser: &mut ProgressParser, text: &str) -> Vec<FfmpegProgress> {
        text.lines().filter_map(|line| parser.push(line)).collect()
    }

    /// A transcript recorded from FFmpeg 8 (`-progress pipe:1 -nostats`), shortened. The first
    /// block is what it prints before the first frame is encoded.
    const TRANSCRIPT: &str = "frame=0
fps=0.00
stream_0_0_q=0.0
bitrate=N/A
total_size=48
out_time_us=N/A
out_time_ms=N/A
out_time=N/A
dup_frames=0
drop_frames=0
speed=N/A
progress=continue
frame=45
fps=0.00
stream_0_0_q=-1.0
bitrate=1021.4kbits/s
total_size=262144
out_time_us=1500000
out_time_ms=1500000
out_time=00:00:01.500000
dup_frames=2
drop_frames=1
speed=1.52x
progress=continue
frame=90
fps=45.00
stream_0_0_q=-1.0
bitrate=1100.0kbits/s
total_size=524288
out_time_us=3000000
out_time_ms=3000000
out_time=00:00:03.000000
dup_frames=2
drop_frames=1
speed=   2x
progress=end
";

    #[test]
    fn reads_every_block_of_a_recorded_transcript_including_the_not_available_start() {
        let mut parser = ProgressParser::default();
        let blocks = feed(&mut parser, TRANSCRIPT);
        assert_eq!(blocks.len(), 3);
        let first = &blocks[0];
        assert_eq!((first.frame, first.out_time_us, first.speed, first.end), (Some(0), None, None, false), "N/A before the first frame");
        assert_eq!(first.total_size, Some(48));
        let second = &blocks[1];
        assert_eq!((second.frame, second.out_time_us, second.total_size), (Some(45), Some(1_500_000), Some(262_144)));
        assert_eq!(second.speed, Some(1.52));
        assert_eq!((second.dup_frames, second.drop_frames), (Some(2), Some(1)));
        let last = &blocks[2];
        assert!(last.end);
        assert_eq!(last.speed, Some(2.0), "padding around the value is ignored");
    }

    #[test]
    fn a_negative_or_garbled_time_is_not_a_time() {
        let mut parser = ProgressParser::default();
        let blocks = feed(&mut parser, "out_time_us=-9223372036854775807\nspeed=0.0x\nframe=abc\ntotal_size=N/A\nprogress=continue\n");
        assert_eq!(blocks[0], FfmpegProgress::default());
        let blocks = feed(&mut parser, "speed=inf\nprogress=continue\n");
        assert_eq!(blocks[0].speed, None);
    }

    #[test]
    fn lines_without_an_equals_sign_and_unknown_keys_are_ignored() {
        let mut parser = ProgressParser::default();
        assert_eq!(parser.push("not a pair"), None);
        assert_eq!(parser.push(""), None);
        assert_eq!(parser.push("future_key=whatever"), None);
        assert_eq!(parser.push("progress=continue").map(|block| block.end), Some(false));
    }

    #[test]
    fn values_do_not_leak_from_one_block_into_the_next() {
        let mut parser = ProgressParser::default();
        let blocks = feed(&mut parser, "frame=10\nprogress=continue\nprogress=continue\n");
        assert_eq!(blocks[0].frame, Some(10));
        assert_eq!(blocks[1].frame, None);
    }

    #[test]
    fn the_rate_is_taken_over_the_window_only() {
        let mut window = RateWindow::new(Duration::from_secs(3));
        assert_eq!(window.rate(), None, "no samples");
        window.push(Duration::from_secs(0), 0);
        assert_eq!(window.rate(), None, "one sample");
        window.push(Duration::from_secs(1), 100);
        assert_eq!(window.rate(), Some(100.0));
        window.push(Duration::from_secs(2), 200);
        window.push(Duration::from_secs(3), 300);
        // The copy then stalls: after more than the window, the old fast samples are gone.
        window.push(Duration::from_secs(7), 310);
        window.push(Duration::from_secs(8), 320);
        let rate = window.rate().unwrap();
        assert!((rate - 10.0).abs() < 1e-9, "{rate}");
        // Time going backwards is ignored; so is a counter going backwards.
        window.push(Duration::from_secs(1), 9999);
        assert!((window.rate().unwrap() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn the_time_left_waits_for_enough_data() {
        let s = Duration::from_secs;
        assert_eq!(eta_from_rate(100, 1000, Some(100.0), s(0), s(1)), None, "less than a second of data");
        assert_eq!(eta_from_rate(100, 1000, Some(100.0), s(1), s(1)), Some(9.0));
        assert_eq!(eta_from_rate(100, 1000, None, s(5), s(1)), None, "no rate");
        assert_eq!(eta_from_rate(100, 1000, Some(0.0), s(5), s(1)), None, "a stalled copy has no time left");
        assert_eq!(eta_from_rate(1000, 1000, Some(5.0), s(5), s(1)), Some(0.0));
        assert_eq!(eta_from_rate(2000, 1000, Some(5.0), s(5), s(1)), None, "more than the total is not an estimate");

        assert_eq!(eta_from_speed(10.0, 100.0, Some(2.0), s(2), s(3)), None, "under three seconds of data");
        assert_eq!(eta_from_speed(10.0, 100.0, Some(2.0), s(3), s(3)), Some(45.0));
        assert_eq!(eta_from_speed(10.0, 100.0, None, s(9), s(3)), None, "no speed yet");
        assert_eq!(eta_from_speed(120.0, 100.0, Some(2.0), s(9), s(3)), Some(0.0), "past the end never goes negative");
        assert_eq!(eta_from_speed(10.0, 0.0, Some(2.0), s(9), s(3)), None, "no known length");
    }
}

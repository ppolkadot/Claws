//! The numbers behind an export's progress: FFmpeg's `-progress` output, the rate over a
//! sliding window, and the time left. Pure code, so every rule that decides what the bar and
//! the status line may claim is tested without running anything.
//!
//! The rule is the product's honest-progress rule: a number is reported only when it can be
//! backed. Nothing here invents a fraction, and the time left is `None` until there is enough
//! data to stand behind it.
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::Serialize;

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

/// The rate over the last few seconds, held between half and twice the average rate of the whole
/// stage. A disk that stalls for a moment (or bursts from a cache) would otherwise promise two hours,
/// or one second, where a copy of a 5.6 GB file took 42 s: measured, the held rate halved the
/// median error of the time left. With no recent rate the average stands in.
pub fn steadied_rate(window: Option<f64>, done: f64, since: Duration) -> Option<f64> {
    let seconds = since.as_secs_f64();
    let average = (seconds > 0.0 && done > 0.0).then(|| done / seconds)?;
    Some(window.map_or(average, |rate| rate.clamp(average * 0.5, average * 2.0)))
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

// ---------------------------------------------------------------------------------------
// The tracker: what the interface is told

/// Where an export is. The stages an export passes through depend on what it does:
/// checking, then copying, converting or zipping (or opening, loading and recording), then
/// finishing.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Checking,
    Opening,
    Loading,
    Recording,
    Finishing,
    Copying,
    Converting,
    Zipping,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Self::Checking => "checking",
            Self::Opening => "opening",
            Self::Loading => "loading",
            Self::Recording => "recording",
            Self::Finishing => "finishing",
            Self::Copying => "copying",
            Self::Converting => "converting",
            Self::Zipping => "zipping",
        }
    }
}

/// What `done` and `total` count.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Unit {
    Seconds,
    Bytes,
}

/// The `workshop-progress` event. A number is present only when it can be backed.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub url: String,
    pub stage: Stage,
    pub done: Option<f64>,
    pub total: Option<f64>,
    pub unit: Option<Unit>,
    /// Milliseconds left, or `None` when there is no estimate to stand behind.
    pub eta_ms: Option<u64>,
    /// The time left rests on a measured rate, not on a known length.
    pub approximate: bool,
    pub elapsed_ms: u64,
    /// Recording: the size of the video written so far (FFmpeg's own count, which trails a little).
    pub bytes: Option<u64>,
}

/// How long an export spent in one stage.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StageTime {
    pub name: &'static str,
    pub ms: u64,
}

/// What a finished export reports: the total time and the time in each stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub elapsed_ms: u64,
    pub stages: Vec<StageTime>,
}

fn whole_ms(time: Duration) -> Duration {
    Duration::from_millis(time.as_millis() as u64)
}

/// At most this many events per second, plus one at every change of stage.
const MIN_EVENT_GAP: Duration = Duration::from_millis(250);
/// A byte rate is taken over this window, and trusted after this much data.
const RATE_WINDOW: Duration = Duration::from_secs(3);
const BYTES_MIN_ELAPSED: Duration = Duration::from_secs(1);
/// A conversion's speed settles after a few seconds.
const SPEED_MIN_ELAPSED: Duration = Duration::from_secs(3);
/// While recording, the bar waits this long below the end until FFmpeg has really finished.
const RECORDING_HOLD_BACK: f64 = 0.2;

struct Inner {
    stage: Stage,
    stage_started: Duration,
    stages: Vec<StageTime>,
    last_emit: Option<Duration>,
    /// The largest `done` sent in this stage: it never goes back.
    done: f64,
    rate: RateWindow,
}

/// Turns what an export knows into events: throttled, never backwards, and carrying a time
/// left only when it can be backed. Cheap to clone; every clone is the same export.
#[derive(Clone)]
pub struct Tracker {
    url: Arc<str>,
    inner: Arc<Mutex<Inner>>,
    emit: Arc<dyn Fn(ProgressEvent) + Send + Sync>,
    clock: Arc<dyn Fn() -> Duration + Send + Sync>,
}

impl Tracker {
    /// A tracker with its own clock, starting now.
    pub fn new(url: &str, emit: impl Fn(ProgressEvent) + Send + Sync + 'static) -> Self {
        let start = Instant::now();
        Self::with_clock(url, emit, move || start.elapsed())
    }

    /// A tracker on a given clock, which reports the time since the export began.
    pub fn with_clock(url: &str, emit: impl Fn(ProgressEvent) + Send + Sync + 'static, clock: impl Fn() -> Duration + Send + Sync + 'static) -> Self {
        let tracker = Self {
            url: url.into(),
            inner: Arc::new(Mutex::new(Inner {
                stage: Stage::Checking,
                stage_started: Duration::ZERO,
                stages: Vec::new(),
                last_emit: None,
                done: 0.0,
                rate: RateWindow::new(RATE_WINDOW),
            })),
            emit: Arc::new(emit),
            clock: Arc::new(clock),
        };
        tracker.send(&mut tracker.inner.lock().unwrap(), None, None, None, None, false, None);
        tracker
    }

    pub fn elapsed(&self) -> Duration {
        (self.clock)()
    }

    #[allow(clippy::too_many_arguments)]
    fn send(&self, inner: &mut Inner, done: Option<f64>, total: Option<f64>, unit: Option<Unit>, eta: Option<f64>, approximate: bool, bytes: Option<u64>) {
        let now = (self.clock)();
        inner.last_emit = Some(now);
        (self.emit)(ProgressEvent {
            url: self.url.to_string(),
            stage: inner.stage,
            done,
            total,
            unit,
            eta_ms: eta.filter(|eta| eta.is_finite()).map(|eta| (eta.max(0.0) * 1000.0).round() as u64),
            approximate: approximate && eta.is_some(),
            elapsed_ms: now.as_millis() as u64,
            bytes,
        });
    }

    /// Moves to a new stage and says so at once. The numbers are those the stage starts with
    /// (`None` for a stage with nothing to count yet).
    pub fn stage(&self, stage: Stage, start: Option<(f64, f64, Unit)>) {
        let mut inner = self.inner.lock().unwrap();
        // Whole milliseconds, so the stages always add up to the total.
        let now = whole_ms((self.clock)());
        if inner.stage != stage {
            let spent = now.saturating_sub(inner.stage_started);
            let name = inner.stage.name();
            inner.stages.push(StageTime { name, ms: spent.as_millis() as u64 });
            inner.stage = stage;
            inner.stage_started = now;
            inner.done = start.map_or(0.0, |(done, _, _)| done);
            inner.rate = RateWindow::new(RATE_WINDOW);
        }
        let (done, total, unit) = match start {
            Some((done, total, unit)) => (Some(done), Some(total), Some(unit)),
            None => (None, None, None),
        };
        self.send(&mut inner, done, total, unit, None, false, None);
    }

    /// Whether an update may be sent now (not more than four a second).
    fn due(&self, inner: &Inner) -> bool {
        inner.last_emit.is_none_or(|last| (self.clock)().saturating_sub(last) >= MIN_EVENT_GAP)
    }

    /// Copying or zipping: bytes written of bytes to write. The time left comes from the rate
    /// over the last three seconds once a second of data exists, and is marked approximate.
    pub fn bytes(&self, done: u64, total: u64) {
        let mut inner = self.inner.lock().unwrap();
        let since = (self.clock)().saturating_sub(inner.stage_started);
        inner.rate.push(since, done);
        // The last byte is always reported, so the bar reaches the end of the count.
        if !self.due(&inner) && done < total {
            return;
        }
        let shown = (done as f64).max(inner.done).min(total as f64);
        inner.done = shown;
        let eta = eta_from_rate(shown as u64, total, steadied_rate(inner.rate.rate(), shown, since), since, BYTES_MIN_ELAPSED);
        self.send(&mut inner, Some(shown), Some(total as f64), Some(Unit::Bytes), eta, true, None);
    }

    /// Converting: seconds of media encoded of the seconds to encode, with the time left from
    /// FFmpeg's speed once it has run for three seconds.
    pub fn converting(&self, done_seconds: f64, total_seconds: f64, speed: Option<f64>) {
        let mut inner = self.inner.lock().unwrap();
        if !self.due(&inner) {
            return;
        }
        let since = (self.clock)().saturating_sub(inner.stage_started);
        let shown = done_seconds.max(inner.done).min(total_seconds);
        inner.done = shown;
        let eta = eta_from_speed(shown, total_seconds, speed, since, SPEED_MIN_ELAPSED);
        self.send(&mut inner, Some(shown), Some(total_seconds), Some(Unit::Seconds), eta, true, None);
    }

    /// Recording: seconds since FFmpeg's estimated first captured frame, of the length chosen.
    /// The time left is approximate. It stops 0.2 s short of the end until FFmpeg itself has finished.
    pub fn recording(&self, seconds_since_first_frame: f64, length: f64, bytes: Option<u64>) {
        let mut inner = self.inner.lock().unwrap();
        if !self.due(&inner) {
            return;
        }
        let cap = (length - RECORDING_HOLD_BACK).max(0.0);
        let shown = seconds_since_first_frame.clamp(0.0, cap).max(inner.done.min(cap));
        inner.done = shown;
        self.send(&mut inner, Some(shown), Some(length), Some(Unit::Seconds), Some(length - shown), true, bytes);
    }

    /// Ends the export: the time in each stage, and the whole of it.
    pub fn finish(&self) -> Summary {
        let mut inner = self.inner.lock().unwrap();
        let now = whole_ms((self.clock)());
        let spent = now.saturating_sub(inner.stage_started);
        let name = inner.stage.name();
        inner.stages.push(StageTime { name, ms: spent.as_millis() as u64 });
        inner.stage_started = now;
        Summary { elapsed_ms: now.as_millis() as u64, stages: std::mem::take(&mut inner.stages) }
    }
}

#[cfg(test)]
mod tracker_tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    struct Rig {
        now: Arc<StdMutex<Duration>>,
        events: Arc<StdMutex<Vec<ProgressEvent>>>,
        tracker: Tracker,
    }

    impl Rig {
        fn new() -> Self {
            let now = Arc::new(StdMutex::new(Duration::ZERO));
            let events = Arc::new(StdMutex::new(Vec::new()));
            let (sink, clock) = (events.clone(), now.clone());
            let tracker = Tracker::with_clock("https://example/?id=1", move |event| sink.lock().unwrap().push(event), move || *clock.lock().unwrap());
            Self { now, events, tracker }
        }
        fn wait(&self, ms: u64) {
            *self.now.lock().unwrap() += Duration::from_millis(ms);
        }
        fn last(&self) -> ProgressEvent {
            self.events.lock().unwrap().last().cloned().unwrap()
        }
        fn count(&self) -> usize {
            self.events.lock().unwrap().len()
        }
    }

    #[test]
    fn starts_checking_with_nothing_to_count() {
        let rig = Rig::new();
        let first = rig.last();
        assert_eq!((first.stage, first.done, first.total, first.unit, first.eta_ms), (Stage::Checking, None, None, None, None));
        assert!(!first.approximate);
    }

    #[test]
    fn sends_at_most_four_updates_a_second_and_every_change_of_stage() {
        let rig = Rig::new();
        rig.tracker.stage(Stage::Copying, Some((0.0, 1000.0, Unit::Bytes)));
        let before = rig.count();
        // A thousand progress calls inside one second: no more than four events.
        for step in 0..1000u64 {
            rig.wait(1);
            rig.tracker.bytes(step, 1000);
        }
        assert!(rig.count() - before <= 4, "{} events", rig.count() - before);
        // A stage change is never throttled.
        rig.tracker.stage(Stage::Finishing, None);
        let after = rig.count();
        rig.tracker.stage(Stage::Copying, Some((0.0, 1.0, Unit::Bytes)));
        assert_eq!(rig.count(), after + 1);
    }

    #[test]
    fn done_never_goes_backwards_within_a_stage_and_never_passes_the_total() {
        let rig = Rig::new();
        rig.tracker.stage(Stage::Copying, Some((0.0, 100.0, Unit::Bytes)));
        rig.wait(300);
        rig.tracker.bytes(60, 100);
        rig.wait(300);
        rig.tracker.bytes(40, 100);
        assert_eq!(rig.last().done, Some(60.0));
        rig.wait(300);
        rig.tracker.bytes(500, 100);
        assert_eq!(rig.last().done, Some(100.0));
    }

    #[test]
    fn a_copy_has_a_time_left_only_after_a_second_and_marks_it_approximate() {
        let rig = Rig::new();
        rig.tracker.stage(Stage::Copying, Some((0.0, 1000.0, Unit::Bytes)));
        rig.wait(300);
        rig.tracker.bytes(100, 1000);
        assert_eq!(rig.last().eta_ms, None, "under a second of data");
        rig.wait(1000);
        rig.tracker.bytes(500, 1000);
        let event = rig.last();
        // 400 bytes in 1.0 s through the window: 500 left at 400/s is 1.25 s.
        assert_eq!(event.eta_ms, Some(1250));
        assert!(event.approximate);
        assert_eq!(event.unit, Some(Unit::Bytes));
    }

    #[test]
    fn a_stall_or_a_burst_cannot_make_the_time_left_absurd() {
        let s = Duration::from_secs;
        // A stall: the recent rate is almost nothing, but the stage has averaged 100 per second.
        assert_eq!(steadied_rate(Some(0.001), 1000.0, s(10)), Some(50.0), "no less than half the average");
        // A burst out of a cache: ten times the average counts as twice.
        assert_eq!(steadied_rate(Some(1000.0), 1000.0, s(10)), Some(200.0));
        // An ordinary rate is left alone.
        assert_eq!(steadied_rate(Some(120.0), 1000.0, s(10)), Some(120.0));
        // No recent rate: the average stands in. Nothing done or no time: nothing to say.
        assert_eq!(steadied_rate(None, 1000.0, s(10)), Some(100.0));
        assert_eq!(steadied_rate(Some(5.0), 0.0, s(10)), None);
        assert_eq!(steadied_rate(Some(5.0), 10.0, s(0)), None);

        // Through the tracker: 100 bytes a second for ten seconds, then nothing for three.
        let rig = Rig::new();
        rig.tracker.stage(Stage::Copying, Some((0.0, 10_000.0, Unit::Bytes)));
        for second in 1..=10u64 {
            rig.wait(1000);
            rig.tracker.bytes(second * 100, 10_000);
        }
        for _ in 0..3 {
            rig.wait(1000);
            rig.tracker.bytes(1000, 10_000);
        }
        // 9,000 left. The last 3 s moved nothing, but the average is 1000/13 = 77 per second, and half
        // of that is the slowest the estimate will believe: 9000 / 38.5 = 234 s, not hours.
        let left = rig.last().eta_ms.expect("a time left") as f64 / 1000.0;
        assert!((left - 234.0).abs() < 1.0, "{left}");
        assert!(rig.last().approximate);
    }

    #[test]
    fn a_conversion_has_a_time_left_only_after_three_seconds_of_speed() {
        let rig = Rig::new();
        rig.tracker.stage(Stage::Converting, Some((0.0, 100.0, Unit::Seconds)));
        rig.wait(2900);
        rig.tracker.converting(10.0, 100.0, Some(2.0));
        assert_eq!(rig.last().eta_ms, None);
        rig.wait(300);
        rig.tracker.converting(20.0, 100.0, Some(2.0));
        assert_eq!(rig.last().eta_ms, Some(40_000), "80 seconds left at 2x");
        assert!(rig.last().approximate);
        rig.wait(300);
        rig.tracker.converting(30.0, 100.0, None);
        assert_eq!(rig.last().eta_ms, None, "no speed, no estimate");
        assert!(!rig.last().approximate, "nothing to be approximate about");
    }

    #[test]
    fn recording_counts_seconds_with_an_approximate_time_left_and_holds_back_until_it_is_over() {
        let rig = Rig::new();
        rig.tracker.stage(Stage::Recording, Some((0.0, 60.0, Unit::Seconds)));
        rig.wait(1000);
        rig.tracker.recording(42.0, 60.0, Some(163_577_856));
        let event = rig.last();
        assert_eq!((event.done, event.total, event.eta_ms), (Some(42.0), Some(60.0), Some(18_000)));
        assert_eq!(event.bytes, Some(163_577_856));
        assert!(event.approximate, "capture start time is estimated");
        rig.wait(1000);
        rig.tracker.recording(61.0, 60.0, None);
        assert!((rig.last().done.unwrap() - 59.8).abs() < 1e-9, "0.2 s short of the end until FFmpeg exits");
        rig.wait(1000);
        rig.tracker.recording(10.0, 60.0, None);
        assert!((rig.last().done.unwrap() - 59.8).abs() < 1e-9, "never back");
    }

    #[test]
    fn the_total_and_each_stage_are_timed() {
        let rig = Rig::new();
        rig.wait(400);
        rig.tracker.stage(Stage::Copying, Some((0.0, 10.0, Unit::Bytes)));
        rig.wait(9_600);
        rig.tracker.stage(Stage::Finishing, None);
        rig.wait(2_300);
        let summary = rig.tracker.finish();
        assert_eq!(summary.elapsed_ms, 12_300);
        let named: Vec<_> = summary.stages.iter().map(|s| (s.name, s.ms)).collect();
        assert_eq!(named, [("checking", 400), ("copying", 9_600), ("finishing", 2_300)]);
        assert_eq!(summary.stages.iter().map(|s| s.ms).sum::<u64>(), summary.elapsed_ms);
        assert_eq!(rig.last().elapsed_ms, 10_000, "events carry the time since the start");
    }

    #[test]
    fn the_event_has_the_shape_the_interface_reads() {
        let event = ProgressEvent {
            url: "u".into(),
            stage: Stage::Recording,
            done: Some(1.5),
            total: Some(60.0),
            unit: Some(Unit::Seconds),
            eta_ms: Some(58_500),
            approximate: false,
            elapsed_ms: 3_000,
            bytes: Some(1024),
        };
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::json!({"url":"u","stage":"recording","done":1.5,"total":60.0,"unit":"seconds","etaMs":58500,"approximate":false,"elapsedMs":3000,"bytes":1024})
        );
        let idle = ProgressEvent { stage: Stage::Checking, done: None, total: None, unit: None, eta_ms: None, ..event };
        let json = serde_json::to_value(&idle).unwrap();
        assert_eq!((&json["done"], &json["total"], &json["unit"], &json["etaMs"]), (&serde_json::Value::Null, &serde_json::Value::Null, &serde_json::Value::Null, &serde_json::Value::Null));
        for (stage, name) in [(Stage::Opening, "opening"), (Stage::Loading, "loading"), (Stage::Finishing, "finishing"), (Stage::Copying, "copying"), (Stage::Converting, "converting"), (Stage::Zipping, "zipping")] {
            assert_eq!(serde_json::to_value(stage).unwrap(), name);
            assert_eq!(stage.name(), name);
        }
    }
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

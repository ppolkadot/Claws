//! Overall progress across the streams of one download, weighted by bytes.
//!
//! yt-dlp reports each stream on its own (video, then audio). A 9 GiB video and a
//! 100 MiB audio track are not two equal halves, so the overall fraction is
//! `sum(downloaded) / sum(expected)` over the selected streams. When any expected size is
//! unknown there is no honest overall percentage: the job reports bytes and speed, and the
//! interface shows an indeterminate bar. Sizes that are estimates are marked as such, and
//! an estimate that changes moves the bar with it (it can step back slightly).
//!
//! Streams are identified by their format id (from `%(info.format_id)s`), not by noticing
//! that the file name changed, so repeated or resumed progress for one stream never counts
//! as another stream.

/// Expected size of one stream and where it came from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub bytes: u64,
    pub exact: bool,
}

#[derive(Clone, Debug, Default)]
struct Stream {
    format_id: String,
    /// From the format selection (CLAWSFMT), before any byte arrives.
    planned: Option<Size>,
    /// From the progress lines, which win over the plan.
    reported: Option<Size>,
    downloaded: u64,
    finished: bool,
}

impl Stream {
    fn expected(&self) -> Option<Size> {
        match (self.reported, self.planned) {
            (Some(r), _) if r.exact => Some(r),
            (_, Some(p)) if p.exact => Some(p),
            (Some(r), _) => Some(r),
            (_, p) => p,
        }
    }
}

/// What one progress line amounts to for the whole job.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    /// 0.0..=0.99 while transferring; `None` when a total is unknown.
    pub fraction: Option<f64>,
    /// Some expected size is an estimate.
    pub estimated: bool,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub speed: Option<f64>,
    /// Seconds of transfer left, only when the total and a usable speed are known.
    pub eta: Option<u64>,
}

/// Progress shown before the file is published never reaches 100%.
const CEILING: f64 = 0.99;
/// Below this speed (bytes/s) a remaining time would be fiction.
const MIN_SPEED: f64 = 1.0;
const MAX_ETA: f64 = 7.0 * 24.0 * 3600.0;

#[derive(Debug, Default)]
pub struct TransferProgress {
    streams: Vec<Stream>,
    /// Streams expected before the selection is known: 2 for video, 1 for audio.
    expected_streams: usize,
    planned: bool,
}

fn number(field: &str) -> Option<f64> {
    field.trim().parse().ok().filter(|v: &f64| v.is_finite() && *v >= 0.0)
}

fn known_id(field: &str) -> Option<&str> {
    let id = field.trim();
    (!id.is_empty() && id != "NA").then_some(id)
}

/// Subtitles, thumbnails and info files report progress too, but are not media streams.
pub fn is_side_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let ext = lower.trim_end_matches(".part").rsplit('.').next().unwrap_or("");
    matches!(
        ext,
        "vtt" | "srt" | "ass" | "ssa" | "ttml" | "srv1" | "srv2" | "srv3" | "json3" | "lrc" | "sbv"
            | "dfxp" | "jpg" | "jpeg" | "png" | "webp" | "json" | "description"
    )
}

impl TransferProgress {
    /// Expected size of each selected stream (None where unknown), once the selection is
    /// known; `None` before that.
    pub fn planned_sizes(&self) -> Option<Vec<Option<u64>>> {
        self.planned.then(|| self.streams.iter().map(|s| s.expected().map(|e| e.bytes)).collect())
    }

    pub fn new(mode: &str) -> Self {
        Self {
            streams: Vec::new(),
            expected_streams: if mode == "audio" { 1 } else { 2 },
            planned: false,
        }
    }

    /// The format selection, from the CLAWSFMT line:
    /// `id0|size0|approx0|id1|size1|approx1|id|size|approx`, where the first two groups
    /// are the separate streams of a merged download (NA otherwise) and the last group is
    /// the single format of an unmerged one.
    pub fn set_plan(&mut self, line: &str) {
        let fields: Vec<&str> = line.split('|').collect();
        let group = |i: usize| -> Option<(String, Option<Size>)> {
            let id = known_id(fields.get(i)?)?;
            let exact = fields.get(i + 1).and_then(|f| number(f)).map(|b| Size { bytes: b as u64, exact: true });
            let approx = fields.get(i + 2).and_then(|f| number(f)).map(|b| Size { bytes: b as u64, exact: false });
            Some((id.to_string(), exact.or(approx)))
        };
        let mut plan: Vec<(String, Option<Size>)> = [group(0), group(3)].into_iter().flatten().collect();
        if plan.is_empty() {
            plan.extend(group(6));
        }
        if plan.is_empty() {
            return;
        }
        self.planned = true;
        self.expected_streams = plan.len();
        for (id, size) in plan {
            match self.streams.iter_mut().find(|s| s.format_id == id) {
                Some(stream) => stream.planned = size,
                None => self.streams.push(Stream { format_id: id, planned: size, ..Stream::default() }),
            }
        }
    }

    /// One progress line: `status|downloaded|total|estimate|speed|eta|fragment|fragments|format_id|filename`.
    /// `None` for side files (subtitles, thumbnails).
    pub fn update(&mut self, fields: &[&str]) -> Option<Snapshot> {
        if fields.len() < 10 {
            return None;
        }
        let filename = fields[9].trim();
        if is_side_file(filename) {
            return None;
        }
        // Without a format id (some extractors), the file name stands in for it.
        let key = known_id(fields[8]).map(str::to_string).unwrap_or_else(|| filename.trim_end_matches(".part").to_string());
        let index = match self.streams.iter().position(|s| s.format_id == key) {
            Some(i) => i,
            None => {
                self.streams.push(Stream { format_id: key, ..Stream::default() });
                self.streams.len() - 1
            }
        };
        let stream = &mut self.streams[index];
        let finished = fields[0].trim() == "finished";
        let downloaded = number(fields[1]).map(|b| b as u64);
        let total = number(fields[2]).map(|b| Size { bytes: b as u64, exact: true });
        let estimate = number(fields[3]).map(|b| Size { bytes: b as u64, exact: false });
        // Fragmented streams may only report fragments: extrapolate, as an estimate.
        let by_fragments = match (downloaded, number(fields[6]), number(fields[7])) {
            (Some(done), Some(index), Some(count)) if index >= 1.0 && count >= index && done > 0 => {
                Some(Size { bytes: (done as f64 * count / index) as u64, exact: false })
            }
            _ => None,
        };
        if let Some(size) = total.or(estimate).or(by_fragments) {
            stream.reported = Some(size);
        }
        if let Some(done) = downloaded {
            stream.downloaded = done;
        }
        if finished {
            stream.finished = true;
            stream.reported = Some(Size { bytes: stream.downloaded.max(stream.expected().map_or(0, |s| s.bytes)), exact: true });
            stream.downloaded = stream.reported.map_or(stream.downloaded, |s| s.bytes);
        }
        Some(self.snapshot(number(fields[4])))
    }

    fn snapshot(&self, speed: Option<f64>) -> Snapshot {
        let downloaded: u64 = self.streams.iter().map(|s| s.downloaded).sum();
        let all_seen = self.planned || self.streams.len() >= self.expected_streams;
        let sizes: Option<Vec<Size>> = self.streams.iter().map(Stream::expected).collect();
        let (fraction, total, estimated) = match sizes {
            Some(sizes) if all_seen && !sizes.is_empty() => {
                let total: u64 = sizes.iter().map(|s| s.bytes).sum();
                let counted: u64 = self.streams.iter().zip(&sizes).map(|(s, size)| s.downloaded.min(size.bytes)).sum();
                let fraction = if total > 0 { (counted as f64 / total as f64).min(CEILING) } else { 0.0 };
                (Some(fraction), Some(total), sizes.iter().any(|s| !s.exact))
            }
            _ => (None, None, self.streams.iter().any(|s| s.expected().is_some_and(|e| !e.exact))),
        };
        let eta = match (total, speed) {
            (Some(total), Some(speed)) if speed >= MIN_SPEED => {
                let seconds = total.saturating_sub(downloaded) as f64 / speed;
                (seconds <= MAX_ETA).then_some(seconds.round() as u64)
            }
            _ => None,
        };
        Snapshot { fraction, estimated, downloaded, total, speed: speed.filter(|s| *s > 0.0), eta }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;
    const MIB: u64 = 1 << 20;

    fn line(status: &str, done: u64, total: &str, estimate: &str, speed: &str, id: &str, file: &str) -> Vec<String> {
        vec![status.into(), done.to_string(), total.into(), estimate.into(), speed.into(), "NA".into(), "NA".into(), "NA".into(), id.into(), file.into()]
    }

    fn update(p: &mut TransferProgress, fields: Vec<String>) -> Option<Snapshot> {
        let refs: Vec<&str> = fields.iter().map(String::as_str).collect();
        p.update(&refs)
    }

    #[test]
    fn unequal_streams_are_weighted_by_bytes() {
        let mut p = TransferProgress::new("video");
        p.set_plan(&format!("137|{}|NA|140|{}|NA|137+140|NA|NA", 9 * GIB, 100 * MIB));
        let half_video = update(&mut p, line("downloading", 9 * GIB / 2, &(9 * GIB).to_string(), "NA", "NA", "137", "v.f137.mp4")).unwrap();
        let expected = (9 * GIB / 2) as f64 / (9 * GIB + 100 * MIB) as f64;
        assert!((half_video.fraction.unwrap() - expected).abs() < 1e-9, "{half_video:?}");
        assert!(half_video.fraction.unwrap() < 0.5, "not two equal halves");
        let video_done = update(&mut p, line("finished", 9 * GIB, &(9 * GIB).to_string(), "NA", "NA", "137", "v.f137.mp4")).unwrap();
        assert!(video_done.fraction.unwrap() > 0.98);
        assert!(!video_done.estimated);
    }

    #[test]
    fn audio_retry_does_not_double_count_the_finished_video() {
        let mut p = TransferProgress::new("video");
        p.set_plan("137|1000|NA|140|100|NA|137+140|NA|NA");
        update(&mut p, line("finished", 1000, "1000", "NA", "NA", "137", "v.f137.mp4"));
        update(&mut p, line("downloading", 80, "100", "NA", "10", "140", "v.f140.m4a"));
        // The audio restarts from a smaller offset (a retry).
        let again = update(&mut p, line("downloading", 20, "100", "NA", "10", "140", "v.f140.m4a")).unwrap();
        assert_eq!(again.downloaded, 1020);
        assert!((again.fraction.unwrap() - 1020.0 / 1100.0).abs() < 1e-9);
    }

    #[test]
    fn unknown_sizes_give_no_percentage_or_eta() {
        let mut p = TransferProgress::new("video");
        p.set_plan("137|NA|NA|140|100|NA|137+140|NA|NA");
        let s = update(&mut p, line("downloading", 500, "NA", "NA", "1000", "137", "v.f137.mp4")).unwrap();
        assert_eq!(s.fraction, None);
        assert_eq!(s.eta, None);
        assert_eq!(s.downloaded, 500);
        assert_eq!(s.speed, Some(1000.0));
    }

    #[test]
    fn a_missing_plan_waits_for_every_stream() {
        let mut p = TransferProgress::new("video");
        // No CLAWSFMT line: one stream seen out of the two a video normally has.
        let s = update(&mut p, line("downloading", 50, "100", "NA", "NA", "137", "v.f137.mp4")).unwrap();
        assert_eq!(s.fraction, None);
        let audio = TransferProgress::new("audio");
        assert_eq!(audio.expected_streams, 1);
    }

    #[test]
    fn a_single_format_uses_the_last_plan_group() {
        let mut p = TransferProgress::new("video");
        p.set_plan("NA|NA|NA|NA|NA|NA|18|NA|2000");
        let s = update(&mut p, line("downloading", 500, "NA", "NA", "100", "18", "v.mp4")).unwrap();
        assert_eq!(s.fraction, Some(0.25));
        assert!(s.estimated, "filesize_approx is an estimate");
        assert_eq!(s.eta, Some(15));
    }

    #[test]
    fn changing_estimates_are_followed_and_marked() {
        let mut p = TransferProgress::new("audio");
        p.set_plan("NA|NA|NA|NA|NA|NA|251|NA|NA");
        let first = update(&mut p, line("downloading", 400, "NA", "1000", "NA", "251", "a.webm")).unwrap();
        assert_eq!(first.fraction, Some(0.4));
        assert!(first.estimated);
        let rebased = update(&mut p, line("downloading", 500, "NA", "2000", "NA", "251", "a.webm")).unwrap();
        assert_eq!(rebased.fraction, Some(0.25));
        let exact = update(&mut p, line("downloading", 600, "1000", "NA", "NA", "251", "a.webm")).unwrap();
        assert!(!exact.estimated);
    }

    #[test]
    fn side_files_are_not_streams() {
        let mut p = TransferProgress::new("video");
        p.set_plan("137|100|NA|140|100|NA|137+140|NA|NA");
        assert_eq!(update(&mut p, line("finished", 10, "10", "NA", "NA", "NA", "v.en.vtt")), None);
        assert_eq!(update(&mut p, line("downloading", 10, "10", "NA", "NA", "NA", "v.en.vtt.part")), None);
        let s = update(&mut p, line("downloading", 50, "100", "NA", "NA", "137", "v.f137.mp4")).unwrap();
        assert_eq!(s.fraction, Some(0.25));
    }

    #[test]
    fn progress_never_shows_complete_before_publication() {
        let mut p = TransferProgress::new("audio");
        p.set_plan("NA|NA|NA|NA|NA|NA|140|100|NA");
        let s = update(&mut p, line("finished", 100, "100", "NA", "NA", "140", "a.m4a")).unwrap();
        assert_eq!(s.fraction, Some(CEILING));
    }

    #[test]
    fn a_stalled_transfer_has_no_eta() {
        let mut p = TransferProgress::new("audio");
        p.set_plan("NA|NA|NA|NA|NA|NA|140|1000000|NA");
        assert_eq!(update(&mut p, line("downloading", 10, "1000000", "NA", "0.001", "140", "a.m4a")).unwrap().eta, None);
        assert_eq!(update(&mut p, line("downloading", 10, "1000000", "NA", "NA", "140", "a.m4a")).unwrap().eta, None);
        assert_eq!(update(&mut p, line("downloading", 10, "1000000", "NA", "1000", "140", "a.m4a")).unwrap().eta, Some(1000));
    }

    #[test]
    fn fragments_without_byte_totals_are_an_estimate() {
        let mut p = TransferProgress::new("video");
        p.set_plan("NA|NA|NA|NA|NA|NA|hls-1|NA|NA");
        let fields: Vec<String> = ["downloading", "300", "NA", "NA", "NA", "NA", "3", "10", "hls-1", "v.mp4"].map(String::from).to_vec();
        let s = update(&mut p, fields).unwrap();
        assert_eq!(s.fraction, Some(0.3));
        assert!(s.estimated);
    }

    #[test]
    fn missing_format_ids_fall_back_to_the_file_name() {
        let mut p = TransferProgress::new("audio");
        update(&mut p, line("downloading", 10, "100", "NA", "NA", "NA", "a.webm.part"));
        let s = update(&mut p, line("downloading", 60, "100", "NA", "NA", "NA", "a.webm")).unwrap();
        assert_eq!(s.downloaded, 60, "one stream, not two");
        assert_eq!(s.fraction, Some(0.6));
    }
}

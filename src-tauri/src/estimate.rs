//! Size and temporary-space previews for a download that has not started.
//!
//! The probe's own size fields describe yt-dlp's default format choice, not the one Claws
//! downloads with, so a preview re-runs Claws' exact selection (`selection_args`, shared
//! with the download) against the cached lookup, offline (`--simulate --load-info-json`),
//! and sizes the streams it picks. The temporary-space figure comes from the same
//! formula admission reserves (`disk_budget::RunPlan`), without other jobs' reservations.
//!
//! A preview is display only. Admission still reads the actual free space when the
//! download starts (downloader.rs `Budget`), whatever the preview said.
//!
//! Extension points for later options: a clipped range scales `download_bytes` and the
//! plan's inputs by the kept fraction (yt-dlp's sizes ignore sections), and a processing
//! output (an editing transcode) adds its term to `RunPlan`. Both belong in `build`.

#[cfg(test)]
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::Manager;

use crate::disk_budget::{self, RunPlan};
use crate::download_policy::{self, AudioSelection, JobExecution, Source};
use crate::downloader::DownloadOptions;
use crate::error::{AppError, Result};
use crate::probe;
use crate::state::AppState;
use crate::tools;
use crate::AppHandle;

/// The selected format as a whole (for a merge: the joined result, with the duration).
const TAG_TOP: &str = "CLAWSEST|";
/// The separate streams of a merge; an empty list for a single format.
const TAG_PARTS: &str = "CLAWSESTPARTS|";
const TOP_PRINT: &str = "CLAWSEST|%(.{format_id,ext,filesize,filesize_approx,vcodec,acodec,width,height,duration})j";
const PARTS_PRINT: &str = "CLAWSESTPARTS|%(requested_formats.:.{format_id,ext,filesize,filesize_approx,vcodec,acodec,width,height})j";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    /// Every selected stream has a size the site states.
    Exact,
    /// At least one size is yt-dlp's estimate (usually bitrate times duration).
    Approximate,
    /// Some selected stream has no size at all. Never shown as zero.
    #[default]
    Unknown,
}

/// One selected stream.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct EstimatePart {
    pub format_id: String,
    /// `None` when the stream has no video (yt-dlp's "none") or does not say.
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub ext: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bytes: Option<u64>,
    /// `bytes` is the size the site states, not an estimate.
    pub exact: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct DownloadEstimate {
    /// Bytes transferred: the sum of the selected streams, or `None` if any is unknown.
    pub download_bytes: Option<u64>,
    pub confidence: Confidence,
    pub parts: Vec<EstimatePart>,
    /// The saved file's container, when it is known before saving.
    pub container: Option<String>,
    /// Peak bytes the work folder reaches (streams, merge, conversion, rewrite), before
    /// admission's safety margin.
    pub peak_temp_bytes: Option<u64>,
    /// Free space admission asks for when this download starts on an otherwise idle volume:
    /// the peak plus its safety margin (`disk_budget::required`). This is the figure shown;
    /// other jobs' reservations are not counted, since they change by the time it starts.
    pub required_free_bytes: Option<u64>,
    /// Size of the file that is kept.
    pub output_bytes: Option<u64>,
    /// The kept file is a conversion (an audio target), so `output_bytes` is not the
    /// downloaded size and `container` is the target's, not the downloaded stream's.
    pub converted: bool,
    /// Why a figure above is missing.
    pub unknown_reason: Option<String>,
}

/// What yt-dlp printed about its selection.
#[derive(Debug)]
struct Selection {
    top: EstimatePart,
    duration: Option<f64>,
    /// The streams downloaded: those of a merge, or the single format.
    parts: Vec<EstimatePart>,
}

fn text(value: &Value) -> Option<String> {
    value.as_str().filter(|s| !s.is_empty() && *s != "none").map(str::to_string)
}

fn number(value: &Value) -> Option<u64> {
    value.as_f64().filter(|n| n.is_finite() && *n > 0.0).map(|n| n.round() as u64)
}

fn part(value: &Value) -> EstimatePart {
    // A stated size of zero is no size at all.
    let exact = number(&value["filesize"]);
    EstimatePart {
        format_id: value["format_id"].as_str().unwrap_or_default().to_string(),
        vcodec: text(&value["vcodec"]),
        acodec: text(&value["acodec"]),
        ext: text(&value["ext"]),
        width: number(&value["width"]).and_then(|n| u32::try_from(n).ok()),
        height: number(&value["height"]).and_then(|n| u32::try_from(n).ok()),
        bytes: exact.or_else(|| number(&value["filesize_approx"])),
        exact: exact.is_some(),
    }
}

/// Reads the two print lines. `None` when the selection line is missing or unreadable:
/// an unknown selection, never an empty (zero-byte) one.
fn parse_selection(output: &str) -> Option<Selection> {
    let mut top: Option<Value> = None;
    let mut streams = Vec::new();
    for line in output.lines().map(str::trim_end) {
        if let Some(rest) = line.strip_prefix(TAG_PARTS) {
            if rest.trim() != "NA" {
                let list: Value = serde_json::from_str(rest).ok()?;
                streams = list.as_array()?.iter().map(part).collect();
            }
        } else if let Some(rest) = line.strip_prefix(TAG_TOP) {
            let value: Value = serde_json::from_str(rest).ok()?;
            if !value.is_object() {
                return None;
            }
            top = Some(value);
        }
    }
    let top = top?;
    let duration = top["duration"].as_f64().filter(|d| *d > 0.0);
    let top = part(&top);
    let parts = if streams.is_empty() { vec![top.clone()] } else { streams };
    Some(Selection { top, duration, parts })
}

pub fn unknown(reason: &str) -> DownloadEstimate {
    DownloadEstimate { unknown_reason: Some(reason.to_string()), ..DownloadEstimate::default() }
}

/// Sizes a selection for these options. `rewrite` is whether metadata or subtitles are
/// written into the finished file, as admission counts it.
fn build(selection: Selection, options: &DownloadOptions, rewrite: bool) -> DownloadEstimate {
    let converted = options.mode == "audio" && options.audio_format != "best";
    let container = if options.mode == "audio" {
        // A converted file has the target's extension; "Original" keeps whatever comes.
        converted.then(|| options.audio_format.clone())
    } else {
        selection.top.ext.clone()
    };
    let parts = selection.parts;
    let sizes: Vec<Option<u64>> = parts.iter().map(|p| p.bytes).collect();
    let download_bytes: Option<u64> = sizes.iter().copied().sum();
    let confidence = match download_bytes {
        None => Confidence::Unknown,
        Some(_) if parts.iter().all(|p| p.exact) => Confidence::Exact,
        Some(_) => Confidence::Approximate,
    };
    let plan = RunPlan { mode: &options.mode, audio_format: &options.audio_format, rewrite, duration: selection.duration };
    let peak_temp_bytes = plan.peak(&sizes);
    let output_bytes = plan.output(&sizes);
    let unknown_reason = if download_bytes.is_none() {
        Some(if parts.len() > 1 {
            "The site does not say how large one of the selected streams is."
        } else {
            "The site does not say how large this download is."
        })
    } else if peak_temp_bytes.is_none() {
        Some("The converted file's size depends on the length, which the site does not give.")
    } else {
        None
    };
    DownloadEstimate {
        download_bytes,
        confidence,
        parts,
        container,
        peak_temp_bytes,
        required_free_bytes: peak_temp_bytes.map(disk_budget::required),
        output_bytes,
        converted,
        unknown_reason: unknown_reason.map(String::from),
    }
}

/// yt-dlp arguments that print the selection for `source` without downloading anything.
fn preview_args(common: Vec<String>, options: &DownloadOptions, audio_selection: AudioSelection, source: Source<'_>) -> Vec<String> {
    let mut args = common;
    args.extend(["--simulate", "--no-warnings", "--socket-timeout", "20"].map(String::from));
    match &source {
        Source::PlaylistItem { index, .. } => args.extend(["--yes-playlist".into(), "--playlist-items".into(), index.to_string()]),
        _ => args.push("--no-playlist".into()),
    }
    args.extend(download_policy::selection_args(options, audio_selection));
    args.extend(["--print", TOP_PRINT, "--print", PARTS_PRINT].map(String::from));
    match source {
        Source::InfoJson(path) => args.extend(["--load-info-json".into(), path.to_string_lossy().into_owned()]),
        Source::Url(url) | Source::PlaylistItem { url, .. } => args.extend(["--".into(), url.to_string()]),
    }
    args
}

/// The offline preview: the download's selection over the probe's cached info.
#[cfg(test)]
fn estimate_args(common: Vec<String>, options: &DownloadOptions, audio_selection: AudioSelection, info: &Path) -> Vec<String> {
    preview_args(common, options, audio_selection, Source::InfoJson(info))
}

/// Estimates one item with the options as they would be frozen if it started now. The
/// cached lookup is used when there is one (no network); otherwise the link is read again
/// through the same bounded lookup path as a probe. A size that cannot be told is an
/// `unknown` estimate with a reason, not an error.
pub async fn estimate(app: &AppHandle, raw_url: &str, playlist_index: Option<u32>, mut options: DownloadOptions) -> Result<DownloadEstimate> {
    let url = probe::validate_url(raw_url)?;
    options.validate()?;
    let playlist_index = playlist_index.filter(|i| *i > 0);
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();
    // What the download would bind (JobExecution::freeze) if it started now.
    let execution = JobExecution::freeze(&settings, options.fast_save);
    let rewrite = execution.embed_metadata || options.embeds_subtitles();
    // Waits while yt-dlp is being replaced, and keeps it from being replaced meanwhile.
    let _engine = state.engine.read().await;
    let ytdlp = tools::find_ytdlp(app).ok_or_else(|| AppError::msg("yt-dlp is not installed yet."))?;
    // Playlist items picked by position are never cached. The lease keeps the cached file
    // from being pruned while yt-dlp reads it.
    let cached = playlist_index.is_none().then(|| probe::cached_info(app, &url)).flatten();
    let source = match (&cached, playlist_index) {
        (Some(info), _) => Source::InfoJson(info.path()),
        (None, Some(index)) => Source::PlaylistItem { url: &url, index },
        (None, None) => Source::Url(&url),
    };
    let args = preview_args(tools::common_args(app, &settings), &options, execution.audio_selection, source);
    let output = probe::run_lookup(app, &ytdlp, &args).await;
    drop(cached);
    Ok(match output {
        Ok(stdout) => match parse_selection(&String::from_utf8_lossy(&stdout)) {
            Some(selection) => build(selection, &options, rewrite),
            None => unknown("yt-dlp did not say which streams it would download."),
        },
        Err(e) => unknown(&e.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk_budget::RunPlan;
    use crate::download_policy::AudioSelection::{self, Best, PreferMatching};
    use serde_json::json;

    fn options(mode: &str) -> DownloadOptions {
        DownloadOptions { mode: mode.into(), ..DownloadOptions::default() }
    }

    fn top(fields: serde_json::Value) -> String {
        format!("{TAG_TOP}{fields}")
    }

    fn parts(list: serde_json::Value) -> String {
        format!("{TAG_PARTS}{list}")
    }

    #[test]
    fn a_merge_with_one_approximate_part_is_approximate() {
        let out = [
            "[info] something yt-dlp says".to_string(),
            top(json!({"format_id": "v1080+a140", "ext": "mp4", "filesize_approx": 309_600_000, "vcodec": "avc1.640028", "acodec": "mp4a.40.2", "width": 1920, "height": 1080, "duration": 600})),
            parts(json!([
                {"format_id": "v1080", "ext": "mp4", "filesize": 300_000_000, "vcodec": "avc1.640028", "acodec": "none", "width": 1920, "height": 1080},
                {"format_id": "a140", "ext": "m4a", "filesize_approx": 9_600_000, "vcodec": "none", "acodec": "mp4a.40.2"}
            ])),
        ]
        .join("\n");
        let selection = parse_selection(&out).expect("a selection");
        let estimate = build(selection, &options("video"), false);
        assert_eq!(estimate.confidence, Confidence::Approximate);
        assert_eq!(estimate.download_bytes, Some(309_600_000));
        assert_eq!(estimate.parts.len(), 2);
        assert_eq!(estimate.parts[0].format_id, "v1080");
        assert_eq!((estimate.parts[0].bytes, estimate.parts[0].exact), (Some(300_000_000), true));
        assert_eq!(estimate.parts[0].vcodec.as_deref(), Some("avc1.640028"));
        assert_eq!(estimate.parts[0].acodec, None, "\"none\" is no codec");
        assert_eq!((estimate.parts[0].width, estimate.parts[0].height), (Some(1920), Some(1080)));
        assert_eq!((estimate.parts[1].bytes, estimate.parts[1].exact), (Some(9_600_000), false));
        assert_eq!(estimate.container.as_deref(), Some("mp4"));
        // The merge writes the joined file while both streams exist.
        assert_eq!(estimate.peak_temp_bytes, Some(2 * 309_600_000));
        assert_eq!(estimate.required_free_bytes, Some(disk_budget::required(2 * 309_600_000)));
        assert_eq!(estimate.output_bytes, Some(309_600_000));
        assert!(!estimate.converted);
        assert_eq!(estimate.unknown_reason, None);
    }

    #[test]
    fn every_part_exact_is_exact() {
        let out = [
            top(json!({"format_id": "v720+a1", "ext": "mp4", "duration": 60})),
            parts(json!([{"format_id": "v720", "filesize": 1000}, {"format_id": "a1", "filesize": 24}])),
        ]
        .join("\n");
        let estimate = build(parse_selection(&out).unwrap(), &options("video"), false);
        assert_eq!(estimate.confidence, Confidence::Exact);
        assert_eq!(estimate.download_bytes, Some(1024));
    }

    #[test]
    fn a_missing_size_is_unknown_with_a_reason_never_zero() {
        let out = [
            top(json!({"format_id": "v+a", "ext": "mp4"})),
            parts(json!([{"format_id": "v", "filesize": 5000}, {"format_id": "a"}])),
        ]
        .join("\n");
        let estimate = build(parse_selection(&out).unwrap(), &options("video"), true);
        assert_eq!(estimate.confidence, Confidence::Unknown);
        assert_eq!(estimate.download_bytes, None);
        assert_eq!(estimate.peak_temp_bytes, None);
        assert_eq!(estimate.required_free_bytes, None);
        assert_eq!(estimate.output_bytes, None);
        assert_eq!(estimate.parts[0].bytes, Some(5000), "what is known stays visible per part");
        assert_eq!(estimate.parts[1].bytes, None);
        assert!(estimate.unknown_reason.is_some());
        // Unparseable or missing output is unknown too, not an empty (zero) selection.
        assert!(parse_selection("").is_none());
        assert!(parse_selection("CLAWSEST|not json").is_none());
        let none = unknown("why");
        assert_eq!((none.download_bytes, none.peak_temp_bytes, none.confidence), (None, None, Confidence::Unknown));
        assert_eq!(none.unknown_reason.as_deref(), Some("why"));
    }

    #[test]
    fn a_single_combined_format_is_one_part() {
        let out = [
            top(json!({"format_id": "18", "ext": "mp4", "filesize": 7000, "vcodec": "avc1.42001E", "acodec": "mp4a.40.2", "width": 640, "height": 360})),
            parts(json!([])),
        ]
        .join("\n");
        let estimate = build(parse_selection(&out).unwrap(), &options("video"), true);
        assert_eq!(estimate.parts.len(), 1);
        assert_eq!(estimate.parts[0].acodec.as_deref(), Some("mp4a.40.2"));
        assert_eq!(estimate.download_bytes, Some(7000));
        // Not merged, but metadata is written into it: one more whole copy.
        assert_eq!(estimate.peak_temp_bytes, Some(14_000));
        // Older yt-dlp prints NA for a missing list.
        let na = [top(json!({"format_id": "18", "ext": "mp4", "filesize": 7000})), format!("{TAG_PARTS}NA")].join("\n");
        assert_eq!(parse_selection(&na).unwrap().parts.len(), 1);
    }

    #[test]
    fn the_peak_is_the_one_admission_reserves() {
        // Audio to WAV: the source stays until the output (sized from the duration) exists.
        let out = top(json!({"format_id": "a251", "ext": "webm", "filesize_approx": 12_000_000, "acodec": "opus", "vcodec": "none", "duration": 600}));
        let wav = DownloadOptions { audio_format: "wav".into(), ..options("audio") };
        let estimate = build(parse_selection(&out).unwrap(), &wav, false);
        let plan = RunPlan { mode: "audio", audio_format: "wav", rewrite: false, duration: Some(600.0) };
        assert_eq!(estimate.peak_temp_bytes, plan.peak(&[Some(12_000_000)]));
        assert_eq!(estimate.peak_temp_bytes, Some(12_000_000 + 600 * 192_000));
        assert_eq!(estimate.output_bytes, Some(600 * 192_000));
        assert!(estimate.converted, "the kept WAV is not the downloaded stream");
        assert_eq!(estimate.container.as_deref(), Some("wav"), "the converted file's container");
        // Without a duration the converted size, and so the peak, is unknown.
        let no_duration = top(json!({"format_id": "a251", "ext": "webm", "filesize_approx": 12_000_000}));
        let estimate = build(parse_selection(&no_duration).unwrap(), &wav, false);
        assert_eq!(estimate.download_bytes, Some(12_000_000));
        assert_eq!(estimate.peak_temp_bytes, None);
        assert!(estimate.unknown_reason.is_some());
        // "Original" is not converted, so its container is only known once it is saved.
        let original = DownloadOptions { audio_format: "best".into(), ..options("audio") };
        let estimate = build(parse_selection(&out).unwrap(), &original, false);
        assert_eq!(estimate.container, None);
        assert!(!estimate.converted);
        assert_eq!(estimate.peak_temp_bytes, Some(12_000_000));
    }

    #[test]
    fn the_free_space_shown_is_what_admission_asks_for() {
        // One merged video of 2 GB: the peak is 4 GB, and admission adds its margin.
        let out = [
            top(json!({"format_id": "v+a", "ext": "mp4", "duration": 600})),
            parts(json!([{"format_id": "v", "filesize": 1_900_000_000u64}, {"format_id": "a", "filesize": 100_000_000u64}])),
        ]
        .join("
");
        let estimate = build(parse_selection(&out).unwrap(), &options("video"), false);
        let peak = estimate.peak_temp_bytes.unwrap();
        let required = estimate.required_free_bytes.unwrap();
        assert!(required > peak, "the margin is part of what is needed");
        // With nothing else running, admission refuses one byte less and accepts exactly it.
        assert_eq!(disk_budget::check(peak, required - 1, 0).unwrap_err().required, required);
        assert!(disk_budget::check(peak, required, 0).is_ok());
        // Small downloads still need the minimum margin.
        let small = build(parse_selection(&top(json!({"format_id": "18", "filesize": 1000}))).unwrap(), &options("video"), false);
        assert_eq!(small.required_free_bytes, Some(1000 + disk_budget::margin(1000)));
    }

    #[test]
    fn the_estimate_is_sent_in_camel_case_and_loads_without_new_fields() {
        let estimate = build(parse_selection(&top(json!({"format_id": "18", "filesize": 10}))).unwrap(), &options("video"), false);
        let value = serde_json::to_value(&estimate).unwrap();
        assert_eq!(value["downloadBytes"], 10);
        assert_eq!(value["confidence"], "exact");
        assert_eq!(value["peakTempBytes"], 10);
        assert_eq!(value["requiredFreeBytes"], 10 + disk_budget::margin(10));
        assert_eq!(value["converted"], false);
        assert_eq!(value["parts"][0]["formatId"], "18");
        let empty: DownloadEstimate = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.confidence, Confidence::Unknown);
        assert_eq!(empty.download_bytes, None);
    }

    #[test]
    fn the_preview_command_reads_the_cached_info_with_the_download_selection_and_nothing_else() {
        let info = Path::new("C:/cache/abc.json");
        let video = DownloadOptions { max_height: Some(1080), subtitle_langs: vec!["en".into()], ..options("video") };
        let args = estimate_args(vec!["--no-colors".into()], &video, Best, info);
        let shared = download_policy::selection_args(&video, Best);
        assert!(args.windows(shared.len()).any(|w| w == shared.as_slice()));
        assert_eq!(&args[args.len() - 2..], ["--load-info-json", "C:/cache/abc.json"]);
        assert_eq!(args[0], "--no-colors");
        for flag in ["--simulate", "--no-playlist", "--no-warnings"] {
            assert!(args.iter().any(|a| a == flag), "{flag}");
        }
        for flag in ["--no-simulate", "--write-subs", "--embed-metadata", "-P", "-o", "--"] {
            assert!(!args.iter().any(|a| a == flag), "{flag} has no place in a preview");
        }
    }

    // ---- The installed yt-dlp, offline, on made-up info ----------------------------------

    fn audio(id: &str, ext: &str, acodec: &str, abr: f64) -> serde_json::Value {
        json!({"format_id": id, "ext": ext, "acodec": acodec, "vcodec": "none", "abr": abr, "tbr": abr, "url": format!("https://example.invalid/{id}"), "protocol": "https"})
    }

    fn video(id: &str, ext: &str, vcodec: &str, width: u32, height: u32, size: serde_json::Value) -> serde_json::Value {
        let mut format = json!({"format_id": id, "ext": ext, "vcodec": vcodec, "acodec": "none", "width": width, "height": height, "tbr": height as f64 * 4.0, "url": format!("https://example.invalid/{id}"), "protocol": "https"});
        if let serde_json::Value::Object(extra) = size {
            format.as_object_mut().unwrap().extend(extra);
        }
        format
    }

    /// A YouTube-like set: exact sizes at 720p and 1080p, a 4K VP9 stream sized only from
    /// its bitrate, AAC audio with an approximate size and a larger Opus stream.
    fn sample_formats() -> serde_json::Value {
        let mut a140 = audio("a140", "m4a", "mp4a.40.2", 128.0);
        a140["filesize_approx"] = json!(9_600_000);
        json!([
            audio("a251", "webm", "opus", 160.0),
            a140,
            video("v720", "mp4", "avc1.4d401f", 1280, 720, json!({"filesize": 150_000_000})),
            video("v1080", "mp4", "avc1.640028", 1920, 1080, json!({"filesize": 300_000_000})),
            video("v2160", "webm", "vp09.00.50.08", 3840, 2160, json!({})),
        ])
    }

    fn run(ytdlp: &Path, formats: serde_json::Value, duration: Option<f64>, options: &DownloadOptions, selection: AudioSelection) -> DownloadEstimate {
        let dir = crate::download_tests::temp_dir("estimate");
        let info = dir.join("info.json");
        let mut value = json!({
            "id": "x", "title": "x", "ext": "mp4", "extractor": "generic", "extractor_key": "Generic",
            "webpage_url": "https://example.invalid/v", "formats": formats
        });
        if let Some(d) = duration {
            value["duration"] = json!(d);
        }
        std::fs::write(&info, value.to_string()).unwrap();
        let args = estimate_args(vec!["--no-colors".into(), "--ignore-config".into()], options, selection, &info);
        let output = std::process::Command::new(ytdlp).args(&args).output().unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let selected = parse_selection(&String::from_utf8_lossy(&output.stdout)).expect("yt-dlp printed the selection");
        build(selected, options, false)
    }

    fn ids(estimate: &DownloadEstimate) -> Vec<&str> {
        estimate.parts.iter().map(|p| p.format_id.as_str()).collect()
    }

    macro_rules! require_ytdlp {
        () => {
            match crate::download_tests::ytdlp() {
                Some(path) => path,
                None => {
                    eprintln!("skipped: needs yt-dlp");
                    return;
                }
            }
        };
    }

    #[test]
    fn changing_the_quality_changes_the_selected_streams_and_their_size() {
        let ytdlp = require_ytdlp!();
        let at = |height: Option<u32>| run(&ytdlp, sample_formats(), Some(600.0), &DownloadOptions { max_height: height, ..options("video") }, Best);

        let hd = at(Some(1080));
        assert_eq!(ids(&hd), ["v1080", "a140"], "H.264 and AAC at the capped height");
        assert_eq!(hd.download_bytes, Some(300_000_000 + 9_600_000));
        assert_eq!(hd.confidence, Confidence::Approximate, "the audio size is approximate");
        assert_eq!(hd.container.as_deref(), Some("mp4"));
        assert_eq!(hd.peak_temp_bytes, Some(2 * (300_000_000 + 9_600_000)));

        let sd = at(Some(720));
        assert_eq!(ids(&sd), ["v720", "a140"]);
        assert_eq!(sd.download_bytes, Some(150_000_000 + 9_600_000));

        // Best takes the 4K stream, sized by yt-dlp from its bitrate and the duration: not
        // the largest format in the list by chance, the one the download would take.
        let best = at(None);
        assert_eq!(ids(&best), ["v2160", "a140"]);
        assert_eq!(best.parts[0].bytes, Some((2160.0 * 4.0 * 1000.0 / 8.0 * 600.0) as u64));
        assert!(!best.parts[0].exact);
        assert_eq!(best.parts[0].height, Some(2160));
    }

    #[test]
    fn the_audio_selection_setting_changes_the_estimated_stream() {
        let ytdlp = require_ytdlp!();
        let m4a = DownloadOptions { audio_format: "m4a".into(), ..options("audio") };
        let best = run(&ytdlp, sample_formats(), Some(600.0), &m4a, Best);
        assert_eq!(ids(&best), ["a251"], "yt-dlp's own pick of the best audio");
        assert_eq!(best.download_bytes, Some(160 * 1000 / 8 * 600));
        let matching = run(&ytdlp, sample_formats(), Some(600.0), &m4a, PreferMatching);
        assert_eq!(ids(&matching), ["a140"], "the stream already in AAC");
        assert_eq!(matching.download_bytes, Some(9_600_000));
        assert_eq!(matching.container.as_deref(), Some("m4a"));
        // A lossy target is about the source's size: the source and the output coexist.
        assert_eq!(matching.peak_temp_bytes, Some(2 * 9_600_000));
    }

    #[test]
    fn sizes_the_site_does_not_give_stay_unknown() {
        let ytdlp = require_ytdlp!();
        // No size, no bitrate: yt-dlp cannot estimate it, and neither does Claws.
        let formats = json!([{"format_id": "18", "ext": "mp4", "vcodec": "avc1.42001E", "acodec": "mp4a.40.2", "width": 640, "height": 360, "url": "https://example.invalid/18", "protocol": "https"}]);
        let estimate = run(&ytdlp, formats, None, &options("video"), Best);
        assert_eq!(ids(&estimate), ["18"]);
        assert_eq!(estimate.download_bytes, None);
        assert_eq!(estimate.confidence, Confidence::Unknown);
        assert_eq!(estimate.peak_temp_bytes, None);
        assert!(estimate.unknown_reason.is_some());

        // One exact and one unknown part: unknown overall, never the known part alone.
        let mixed = json!([
            {"format_id": "a1", "ext": "m4a", "acodec": "mp4a.40.2", "vcodec": "none", "url": "https://example.invalid/a1", "protocol": "https"},
            video("v1", "mp4", "avc1.640028", 1920, 1080, json!({"filesize": 300_000_000})),
        ]);
        let estimate = run(&ytdlp, mixed, None, &options("video"), Best);
        assert_eq!(ids(&estimate), ["v1", "a1"]);
        assert_eq!(estimate.download_bytes, None);
        assert_eq!(estimate.parts[0].bytes, Some(300_000_000));
    }
}

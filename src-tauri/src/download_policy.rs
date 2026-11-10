//! Download policy: what a job was started with (frozen, so later settings changes cannot
//! move or reshape a download that already has bytes on disk), the exact yt-dlp command
//! line built from it, and how failures are categorised.

use std::path::{Path, PathBuf};

use crate::disk_budget::RunPlan;
use crate::downloader::DownloadOptions;

use serde::{Deserialize, Serialize};

use crate::settings::Settings;

pub const EXECUTION_SCHEMA: u16 = 1;

pub const FRAGMENT_CONCURRENCY: u8 = 8;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum AudioSelection {
    /// yt-dlp's own pick of the best audio, converted as needed.
    #[default]
    Best,
    /// Prefer an audio stream already in the requested codec, so it can be copied.
    PreferMatching,
}

/// What a video download keeps: which streams are chosen and what the saved file is. Part
/// of the job's options, so it is fixed at enqueue and a paused or retried job never
/// changes it. A missing value (jobs saved before presets existed) is `Compatible`, the
/// behaviour those jobs were queued with.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum VideoPreset {
    /// Prefers H.264 video and AAC audio at the chosen resolution and saves MP4. Where a
    /// resolution exists only in another codec, that codec is taken (still in MP4).
    #[default]
    Compatible,
    /// The site's best streams at the chosen resolution, with no codec preference, merged
    /// without re-encoding into MP4 when the codecs allow it and MKV otherwise.
    Original,
    /// An editing transcode. Reserved: not offered or accepted until processing exists.
    EditingProRes,
}

impl VideoPreset {
    pub fn available(self) -> bool {
        self != VideoPreset::EditingProRes
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Transport {
    #[default]
    Native,
    Aria2c,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobExecution {
    pub schema_version: u16,
    pub output_dir: PathBuf,
    pub filename_template: String,
    pub embed_metadata: bool,
    pub audio_selection: AudioSelection,
    pub fragment_concurrency: u8,
    pub transport: Transport,
}

impl JobExecution {
    /// Binds the settings in force when the job first starts. `fast_save` turns off the
    /// optional whole-file metadata pass for this job only.
    pub fn freeze(settings: &Settings, fast_save: bool) -> Self {
        Self {
            schema_version: EXECUTION_SCHEMA,
            output_dir: PathBuf::from(&settings.output_dir),
            filename_template: settings.filename_template.clone(),
            embed_metadata: settings.embed_metadata && !fast_save,
            audio_selection: settings.audio_selection,
            fragment_concurrency: FRAGMENT_CONCURRENCY,
            transport: Transport::Native,
        }
    }
}

// Post-transfer processing. One decision, read by the command line below and by disk sizing
// (`disk_budget::RunPlan`, used by both admission and the size preview), so what is run and
// what is reserved cannot drift apart.

/// Whether an audio download is converted: any target but "Original". A conversion is
/// necessary work, so Fast Save never changes it.
pub fn converts_audio(mode: &str, audio_format: &str) -> bool {
    mode == "audio" && audio_format != "best"
}

/// What a download does to its streams after transferring them.
///
/// Contract: Fast Save skips the optional passes (metadata, embedded subtitles and embedded
/// cover art), because each can rewrite the finished file. Necessary work stays: audio
/// conversion, a video merge and any container repair yt-dlp itself decides on. A transcript
/// downloads no media, so nothing here applies to it.
///
/// The Fast Save hint in the interface (`src/processing-options.ts`) restates this contract
/// by hand: change both together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessingPolicy {
    pub convert_audio: bool,
    pub embed_metadata: bool,
    pub embed_subtitles: bool,
    /// MP3 and M4A only. A missing thumbnail makes yt-dlp skip the step, which cannot be
    /// known in advance, so sizing assumes the pass happens.
    pub embed_thumbnail: bool,
}

impl ProcessingPolicy {
    pub fn new(options: &DownloadOptions, execution: &JobExecution) -> Self {
        let optional = options.mode != "transcript" && !options.fast_save;
        Self {
            convert_audio: converts_audio(&options.mode, &options.audio_format),
            embed_metadata: optional && execution.embed_metadata,
            embed_subtitles: optional && options.mode == "video" && !options.subtitle_langs.is_empty() && options.embeds_subtitles(),
            embed_thumbnail: optional && options.mode == "audio" && matches!(options.audio_format.as_str(), "mp3" | "m4a"),
        }
    }

    /// Whether a pass may write the finished file again (a temporary copy of it), which
    /// disk sizing must leave room for. Thumbnail embedding is a rewrite for MP3 (an ffmpeg
    /// temporary output); for M4A it depends on the backend yt-dlp finds, so it is counted
    /// as one.
    pub fn rewrites_output(&self) -> bool {
        self.embed_metadata || self.embed_subtitles || self.embed_thumbnail
    }

    /// The sizing plan for this job, as admission and the preview both use it.
    pub fn run_plan<'a>(&self, options: &'a DownloadOptions, duration: Option<f64>) -> RunPlan<'a> {
        RunPlan { mode: &options.mode, audio_format: &options.audio_format, rewrite: self.rewrites_output(), duration }
    }
}

// yt-dlp arguments. Pure, so the exact command line can be tested without a running app.

/// Where the media comes from. The source always goes last, after `--` when it is a URL,
/// so it can never be read as an option.
pub enum Source<'a> {
    Url(&'a str),
    /// An entry with no link of its own: item `index` of the playlist at `url`.
    PlaylistItem { url: &'a str, index: u32 },
    /// The probe's cached info, which skips a second extraction.
    InfoJson(&'a Path),
}

/// Finite retries and no wall-clock limit, so a healthy multi-hour transfer is never cut
/// short; a cancel kills the process tree even while it sleeps between retries.
pub const RETRY_ARGS: &[&str] = &[
    "--retries",
    "10",
    "--fragment-retries",
    "10",
    "--retry-sleep",
    "http:exp=1:30",
    "--retry-sleep",
    "fragment:exp=1:30",
    "--socket-timeout",
    "30",
    // A missing fragment fails the download instead of leaving a hole in the file.
    "--abort-on-unavailable-fragments",
];

/// Without it yt-dlp prints a progress line for every block read; the interface shows a
/// few updates a second. First, final and status-change lines still come through.
pub const PROGRESS_DELTA: &str = "0.2";

/// Progress per stream. The format id identifies the stream; the file name goes last
/// because it may contain the separator.
pub const PROGRESS_TEMPLATE: &str = "download:CLAWS|%(progress.status)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s|%(progress.fragment_index)s|%(progress.fragment_count)s|%(info.format_id)s|%(progress.filename)s";
pub const POSTPROCESS_TEMPLATE: &str = "postprocess:CLAWSPP|%(progress.status)s|%(progress.postprocessor)s";
/// The selected streams and their sizes (exact, then approximate), before any byte:
/// two groups for a merged download, then the single format of an unmerged one.
pub const FORMATS_PRINT: &str = "before_dl:CLAWSFMT|%(requested_formats.0.format_id)s|%(requested_formats.0.filesize)s|%(requested_formats.0.filesize_approx)s|%(requested_formats.1.format_id)s|%(requested_formats.1.filesize)s|%(requested_formats.1.filesize_approx)s|%(format_id)s|%(filesize)s|%(filesize_approx)s";
pub const DONE_PRINT: &str = "after_move:CLAWSDONE|%(filepath)s";
/// A transcript run downloads no media, so nothing reaches `after_move`: the caption files
/// written are named once the video is processed, as a JSON list of paths.
pub const CAPTIONS_PRINT: &str = "after_video:CLAWSCAPTIONS|%(requested_subtitles.:.filepath)j";

/// The format selection for an audio-only download. Codecs are matched on the stream's
/// codec field, not its extension (except M4A, whose container implies AAC). Every
/// selector ends in `ba/b`, so a site without a matching stream behaves like `Best`.
pub fn audio_selector(audio_format: &str, selection: AudioSelection) -> &'static str {
    match (selection, audio_format) {
        (AudioSelection::Best, _) | (_, "best") => "ba/b",
        (_, "m4a") => "ba[ext=m4a]/ba[acodec^=aac]/ba[acodec^=mp4a.40.]/ba/b",
        (_, "opus") => "ba[acodec=opus]/ba/b",
        (_, "mp3") => "ba[acodec=mp3]/ba/b",
        (_, "flac") => "ba[acodec=flac]/ba/b",
        (_, "wav") => "ba[acodec^=pcm]/ba/b",
        _ => "ba/b",
    }
}

/// Merge containers for `Original`, in order of preference. yt-dlp takes the first its own
/// rules accept for the selected codecs as they are: MP4 for H.264 with AAC (checked with
/// yt-dlp 2026.10.26), MKV, which holds any of them, otherwise (VP9 or Opus, and also HEVC
/// labelled `hvc1`). The streams are copied, never re-encoded.
pub const ORIGINAL_MERGE: &str = "mp4/mkv";

/// The arguments that decide which streams are downloaded and what they become: format
/// selection, sort, merge container and audio conversion. The download and the size
/// preview (estimate.rs) both take them from here, so a preview sizes the streams the
/// download will actually fetch.
///
/// `res:N` caps the shorter side of the picture (a portrait 1080×1920 stream is 1080) and
/// is a preference, not a filter: nothing is ever scaled. Where nothing is that small,
/// yt-dlp takes the smallest stream above it.
pub fn selection_args(options: &DownloadOptions, audio_selection: AudioSelection) -> Vec<String> {
    if options.mode == "audio" {
        let format = options.audio_format.as_str();
        return ["-f", audio_selector(format, audio_selection), "-x", "--audio-format", format, "--audio-quality", "0"]
            .map(String::from)
            .to_vec();
    }
    let res = match options.max_height {
        Some(h) => format!("res:{h}"),
        None => "res".to_string(),
    };
    let (sort, merge) = match options.video_preset {
        // H.264 and AAC win only between streams of the same resolution.
        VideoPreset::Compatible => (format!("{res},vcodec:h264,acodec:aac"), "mp4"),
        // EditingProRes is never accepted by this build (`DownloadOptions::validate`, and
        // `execute` refuses a saved one); its source selection would be Original's.
        VideoPreset::Original | VideoPreset::EditingProRes => (res, ORIGINAL_MERGE),
    };
    ["-f", "bv*+ba/b", "-S", &sort, "--merge-output-format", merge].map(String::from).to_vec()
}

/// Caption formats in order of preference. yt-dlp only picks among what the site offers
/// (`best` is the last listed, which can be srv3 or json3); with FFmpeg, `--convert-subs vtt`
/// then turns the pick into WebVTT, so the transcript is made from WebVTT.
pub const CAPTION_FORMATS: &str = "vtt/srt/ttml/best";
/// Without FFmpeg nothing can be converted (yt-dlp refuses to start a conversion at all),
/// so the formats Claws reads itself come first. Anything else arrives as it is and the
/// job explains that FFmpeg is needed (transcript.rs `write_artifact`).
pub const CAPTION_FORMATS_NO_FFMPEG: &str = "vtt/srt/best";

pub fn download_args(
    common: Vec<String>,
    options: &DownloadOptions,
    execution: &JobExecution,
    workspace: &Path,
    source: Source<'_>,
) -> Vec<String> {
    // tools::common_args names FFmpeg's folder whenever Claws found FFmpeg.
    let ffmpeg = common.iter().any(|a| a == "--ffmpeg-location");
    let mut args = common;
    let mut push = |items: &[&str]| args.extend(items.iter().map(|s| s.to_string()));
    let transcript = options.mode == "transcript";

    push(&["--newline", "--no-warnings", "--progress", "--no-simulate", "--windows-filenames"]);
    push(RETRY_ARGS);
    push(&["--progress-delta", PROGRESS_DELTA]);
    if !transcript {
        let fragments = execution.fragment_concurrency.max(1).to_string();
        push(&["--concurrent-fragments", &fragments]);
    }
    match &source {
        Source::PlaylistItem { index, .. } => push(&["--yes-playlist", "--playlist-items", &index.to_string()]),
        _ => push(&["--no-playlist"]),
    }
    push(&["--progress-template", PROGRESS_TEMPLATE, "--progress-template", POSTPROCESS_TEMPLATE]);

    if transcript {
        // One caption track and no media: no format is downloaded (a video whose formats
        // are unavailable still has its captions), nothing is embedded or rewritten.
        let code = options.subtitle_langs.first().map(String::as_str).unwrap_or_default();
        let kind = if options.auto_subs { "--write-auto-subs" } else { "--write-subs" };
        push(&[
            "--print",
            CAPTIONS_PRINT,
            "--skip-download",
            "--ignore-no-formats-error",
            kind,
            "--sub-langs",
            &crate::transcript::sub_langs(code),
        ]);
        if ffmpeg {
            push(&["--sub-format", CAPTION_FORMATS, "--convert-subs", "vtt"]);
        } else {
            push(&["--sub-format", CAPTION_FORMATS_NO_FFMPEG]);
        }
    } else {
        push(&["--print", FORMATS_PRINT, "--print", DONE_PRINT]);
        media_args(&mut push, options, execution);
    }

    // Everything, including the finished file, is written inside the work folder.
    let workspace = workspace.to_string_lossy();
    push(&["-P", &workspace, "-o", &execution.filename_template]);
    match source {
        Source::InfoJson(path) => push(&["--load-info-json", &path.to_string_lossy()]),
        Source::Url(url) | Source::PlaylistItem { url, .. } => push(&["--", url]),
    }
    args
}

/// The streams, conversions, subtitles and metadata of a video or audio download.
fn media_args(push: &mut impl FnMut(&[&str]), options: &DownloadOptions, execution: &JobExecution) {
    // Which streams, and what they become: shared with the size preview.
    let selection = selection_args(options, execution.audio_selection);
    push(&selection.iter().map(String::as_str).collect::<Vec<_>>());
    let policy = ProcessingPolicy::new(options, execution);
    if options.mode == "audio" {
        if policy.embed_thumbnail {
            push(&["--embed-thumbnail"]);
        }
    } else if !options.subtitle_langs.is_empty() {
        let langs = options.subtitle_langs.join(",");
        push(&["--write-subs", "--sub-langs", &langs]);
        if options.auto_subs {
            push(&["--write-auto-subs"]);
        }
        if policy.embed_subtitles {
            push(&["--embed-subs"]);
        } else {
            push(&["--convert-subs", "srt"]);
        }
    }
    if policy.embed_metadata {
        push(&["--embed-metadata"]);
    }
}

// Failure categories, so a cached extraction is refreshed only when the evidence says the
// source links expired, never for a full disk or a failed merge.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    Network,
    ExpiredMedia,
    MissingFragment,
    LocalIo,
    Processing,
    MissingTool,
    Unknown,
}

impl FailureKind {
    /// Whether reading the link again (fresh media URLs) can fix it.
    pub fn refresh_helps(self) -> bool {
        self == FailureKind::ExpiredMedia
    }

    /// A short lead-in naming the failed stage, shown before yt-dlp's own message.
    pub fn summary(self) -> Option<&'static str> {
        match self {
            FailureKind::MissingFragment => Some("Part of the stream could not be downloaded, so nothing was saved."),
            FailureKind::LocalIo => Some("The file could not be written."),
            FailureKind::Processing => Some("The download finished, but processing the file failed."),
            FailureKind::MissingTool => Some("A required tool is missing. Check Settings › Tools."),
            FailureKind::ExpiredMedia => Some("The site refused the media link."),
            FailureKind::Network | FailureKind::Unknown => None,
        }
    }
}

/// Classifies a failed run from its error output. `processing` is true once a
/// post-processor (merge, conversion) had started.
pub fn classify(stderr: &str, processing: bool) -> FailureKind {
    let text = stderr.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| text.contains(n));
    if has(&["ffmpeg not found", "ffprobe and ffmpeg not found", "ffmpeg is not installed"]) {
        FailureKind::MissingTool
    } else if has(&["no space left", "errno 28", "winerror 112", "disk full", "permission denied", "winerror 32"]) {
        FailureKind::LocalIo
    } else if text.contains("fragment") && has(&["not found", "unable to continue", "unavailable"]) {
        FailureKind::MissingFragment
    } else if has(&["http error 403", "http error 410", "requested format is not available", "url has expired"]) {
        FailureKind::ExpiredMedia
    } else if processing || has(&["postprocessing:", "conversion failed", "error opening output", "invalid data found when processing input"]) {
        FailureKind::Processing
    } else if has(&["timed out", "connection reset", "connection aborted", "getaddrinfo", "name resolution", "http error 5", "remote end closed"]) {
        FailureKind::Network
    } else {
        FailureKind::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn options(mode: &str) -> DownloadOptions {
        DownloadOptions { mode: mode.into(), ..DownloadOptions::default() }
    }

    pub(crate) fn execution() -> JobExecution {
        JobExecution::freeze(&Settings { output_dir: "D:/Out".into(), ..Settings::default() }, false)
    }

    pub(crate) fn value_after<'a>(args: &'a [String], flag: &str) -> Vec<&'a str> {
        args.windows(2).filter(|w| w[0] == flag).map(|w| w[1].as_str()).collect()
    }

    #[test]
    fn baseline_has_strict_fragments_backoff_and_quiet_progress() {
        let args = download_args(vec![], &options("video"), &execution(), Path::new("D:/Out/.claws-work/abc"), Source::Url("https://example.com/v"));
        assert!(args.contains(&"--abort-on-unavailable-fragments".to_string()));
        assert_eq!(value_after(&args, "--retries"), ["10"]);
        assert_eq!(value_after(&args, "--fragment-retries"), ["10"]);
        assert_eq!(value_after(&args, "--retry-sleep"), ["http:exp=1:30", "fragment:exp=1:30"]);
        assert_eq!(value_after(&args, "--socket-timeout"), ["30"]);
        assert_eq!(value_after(&args, "--progress-delta"), ["0.2"]);
        assert_eq!(value_after(&args, "--concurrent-fragments"), ["8"]);
        assert_eq!(value_after(&args, "-P"), ["D:/Out/.claws-work/abc"]);
    }

    #[test]
    fn the_source_is_always_last_and_never_an_option() {
        let url = "https://example.com/-o";
        let args = download_args(vec!["--cookies".into(), "x".into()], &options("video"), &execution(), Path::new("W"), Source::Url(url));
        assert_eq!(&args[args.len() - 2..], ["--", url]);
        assert_eq!(&args[..2], ["--cookies", "x"]);
        let item = download_args(vec![], &options("video"), &execution(), Path::new("W"), Source::PlaylistItem { url, index: 7 });
        assert_eq!(value_after(&item, "--playlist-items"), ["7"]);
        assert!(item.contains(&"--yes-playlist".to_string()));
        let cached = download_args(vec![], &options("video"), &execution(), Path::new("W"), Source::InfoJson(Path::new("C:/cache/a.json")));
        assert_eq!(&cached[cached.len() - 2..], ["--load-info-json", "C:/cache/a.json"]);
        assert!(!cached.contains(&"--".to_string()));
    }

    #[test]
    fn frozen_choices_shape_the_command() {
        let mut execution = execution();
        execution.embed_metadata = false;
        execution.filename_template = "%(uploader)s/%(title)s.%(ext)s".into();
        let args = download_args(vec![], &options("video"), &execution, Path::new("W"), Source::Url("https://e.com/v"));
        assert!(!args.contains(&"--embed-metadata".to_string()));
        assert_eq!(value_after(&args, "-o"), ["%(uploader)s/%(title)s.%(ext)s"]);
    }

    #[test]
    fn fast_save_skips_the_whole_file_passes_but_not_the_streams() {
        let settings = Settings { output_dir: "D:/Out".into(), embed_metadata: true, ..Settings::default() };
        let options = DownloadOptions {
            mode: "video".into(),
            max_height: Some(1080),
            subtitle_langs: vec!["en".into()],
            embed_subs: true,
            fast_save: true,
            ..DownloadOptions::default()
        };
        let fast = JobExecution::freeze(&settings, options.fast_save);
        assert!(!fast.embed_metadata);
        let args = download_args(vec![], &options, &fast, Path::new("W"), Source::Url("https://e.com/v"));
        assert!(!args.contains(&"--embed-metadata".to_string()));
        assert!(!args.contains(&"--embed-subs".to_string()));
        assert_eq!(value_after(&args, "--convert-subs"), ["srt"], "subtitles are kept, as files");
        assert_eq!(value_after(&args, "--sub-langs"), ["en"]);
        // The same streams as without Fast Save.
        let normal_options = DownloadOptions { fast_save: false, ..options.clone() };
        let normal = download_args(vec![], &normal_options, &JobExecution::freeze(&settings, false), Path::new("W"), Source::Url("https://e.com/v"));
        assert_eq!(value_after(&args, "-f"), value_after(&normal, "-f"));
        assert_eq!(value_after(&args, "-S"), value_after(&normal, "-S"));
        assert!(normal.contains(&"--embed-subs".to_string()) && normal.contains(&"--embed-metadata".to_string()));
    }

    const AUDIO_TARGETS: [&str; 6] = ["mp3", "m4a", "opus", "flac", "wav", "best"];

    fn audio_command(format: &str, metadata: bool, fast_save: bool, selection: AudioSelection) -> Vec<String> {
        let settings = Settings { output_dir: "D:/Out".into(), embed_metadata: metadata, audio_selection: selection, ..Settings::default() };
        let options = DownloadOptions { audio_format: format.into(), fast_save, ..options("audio") };
        download_args(vec![], &options, &JobExecution::freeze(&settings, fast_save), Path::new("W"), Source::Url("https://e.com/a"))
    }

    #[test]
    fn the_audio_matrix_emits_conversion_always_and_optional_passes_unless_fast_save() {
        for format in AUDIO_TARGETS {
            for metadata in [false, true] {
                for fast_save in [false, true] {
                    for selection in [AudioSelection::Best, AudioSelection::PreferMatching] {
                        let args = audio_command(format, metadata, fast_save, selection);
                        let case = format!("{format} metadata={metadata} fast={fast_save} {selection:?}");
                        // The conversion is necessary work: Fast Save never turns it into a copy
                        // of some other format, and never changes the streams.
                        assert_eq!(value_after(&args, "--audio-format"), [format], "{case}");
                        assert!(args.contains(&"-x".to_string()), "{case}");
                        assert_eq!(value_after(&args, "-f"), [audio_selector(format, selection)], "{case}");
                        assert_eq!(args.contains(&"--embed-metadata".to_string()), metadata && !fast_save, "{case}");
                        let thumbnail = matches!(format, "mp3" | "m4a") && !fast_save;
                        assert_eq!(args.contains(&"--embed-thumbnail".to_string()), thumbnail, "{case}");
                        assert!(!args.contains(&"--embed-subs".to_string()) && !args.contains(&"--write-subs".to_string()), "{case}");
                    }
                }
            }
        }
    }

    #[test]
    fn fast_save_leaves_necessary_video_work_alone() {
        // Merging, the container and the chosen streams stay; only the optional passes go.
        let settings = Settings { output_dir: "D:/Out".into(), embed_metadata: true, ..Settings::default() };
        let options = DownloadOptions { max_height: Some(720), fast_save: true, ..options("video") };
        let args = download_args(vec![], &options, &JobExecution::freeze(&settings, true), Path::new("W"), Source::Url("https://e.com/v"));
        assert_eq!(value_after(&args, "--merge-output-format"), ["mp4"]);
        assert_eq!(value_after(&args, "-f"), ["bv*+ba/b"]);
        for flag in ["--embed-metadata", "--embed-subs", "--embed-thumbnail"] {
            assert!(!args.contains(&flag.to_string()), "{flag}");
        }
    }

    fn frozen(format: &str, metadata: bool, fast_save: bool) -> (DownloadOptions, JobExecution) {
        let settings = Settings { output_dir: "D:/Out".into(), embed_metadata: metadata, ..Settings::default() };
        let options = DownloadOptions { audio_format: format.into(), fast_save, ..options("audio") };
        let execution = JobExecution::freeze(&settings, fast_save);
        (options, execution)
    }

    #[test]
    fn the_policy_names_exactly_the_passes_the_command_runs() {
        for format in AUDIO_TARGETS {
            for metadata in [false, true] {
                for fast_save in [false, true] {
                    let (options, execution) = frozen(format, metadata, fast_save);
                    let policy = ProcessingPolicy::new(&options, &execution);
                    let args = download_args(vec![], &options, &execution, Path::new("W"), Source::Url("https://e.com/a"));
                    let has = |flag: &str| args.contains(&flag.to_string());
                    let case = format!("{format} metadata={metadata} fast={fast_save}");
                    assert_eq!(policy.embed_metadata, has("--embed-metadata"), "{case}");
                    assert_eq!(policy.embed_thumbnail, has("--embed-thumbnail"), "{case}");
                    assert_eq!(policy.embed_subtitles, has("--embed-subs"), "{case}");
                    assert_eq!(policy.convert_audio, format != "best", "{case}");
                    assert_eq!(policy.rewrites_output(), policy.embed_metadata || policy.embed_thumbnail, "{case}");
                }
            }
        }
        // Video: subtitles are embedded only when asked for, with languages, and not in Fast Save.
        let settings = Settings { output_dir: "D:/Out".into(), embed_metadata: false, ..Settings::default() };
        let en = || vec!["en".to_string()];
        for (langs, embed, fast_save) in [(vec![], true, false), (en(), false, false), (en(), true, false), (en(), true, true)] {
            let options = DownloadOptions { subtitle_langs: langs, embed_subs: embed, fast_save, ..options("video") };
            let execution = JobExecution::freeze(&settings, fast_save);
            let policy = ProcessingPolicy::new(&options, &execution);
            let args = download_args(vec![], &options, &execution, Path::new("W"), Source::Url("https://e.com/v"));
            assert_eq!(policy.embed_subtitles, args.contains(&"--embed-subs".to_string()));
            assert!(!policy.embed_thumbnail && !policy.convert_audio);
        }
        // A transcript downloads no media: nothing is converted, embedded or rewritten.
        let loud = JobExecution::freeze(&Settings { embed_metadata: true, ..Settings::default() }, false);
        let policy = ProcessingPolicy::new(&transcript("en", false), &loud);
        assert!(!policy.rewrites_output() && !policy.convert_audio);
    }

    #[test]
    fn audio_sizing_follows_the_policy_for_every_format_and_size_relation() {
        const MIB: u64 = 1 << 20;
        // A 10-minute source of 12 MiB: WAV and FLAC come out larger (sized from the duration),
        // a lossy target about the same. A 600 MiB source with a 30-second duration shrinks to
        // WAV and FLAC. A missing thumbnail does not change these figures: yt-dlp then skips
        // the pass, so they are an upper bound.
        let cases: [(u64, f64); 2] = [(12 * MIB, 600.0), (600 * MIB, 30.0)];
        for (source, duration) in cases {
            for format in AUDIO_TARGETS {
                for metadata in [false, true] {
                    for fast_save in [false, true] {
                        let (options, execution) = frozen(format, metadata, fast_save);
                        let policy = ProcessingPolicy::new(&options, &execution);
                        let plan = policy.run_plan(&options, Some(duration));
                        let case = format!("{format} metadata={metadata} fast={fast_save} source={source} duration={duration}");
                        let peak = plan.peak(&[Some(source)]).unwrap_or_else(|| panic!("{case}: unsized"));
                        let output = plan.output(&[Some(source)]).unwrap();
                        let thumbnail = matches!(format, "mp3" | "m4a") && !fast_save;
                        let rewrite = (metadata && !fast_save) || thumbnail;
                        assert_eq!(policy.rewrites_output(), rewrite, "{case}");
                        // Every audio target goes through `-x`. A conversion writes its output while the
                        // source exists; "Original" is sized the same way with an output as large as the
                        // source, because a changed container (webm Opus to .opus) is a whole copy.
                        // A rewrite adds a second output-sized copy.
                        let copies = if rewrite { 2 } else { 1 };
                        if format == "best" {
                            assert_eq!(output, source, "{case}");
                        }
                        assert_eq!(peak, source + copies * output, "{case}");
                        assert!(peak > source && peak >= output + if rewrite { output } else { 0 }, "{case}");
                    }
                }
            }
        }
    }

    #[test]
    fn options_saved_by_1_0_4_have_fast_save_off() {
        let old = r#"{"mode":"video","maxHeight":null,"audioFormat":"mp3","subtitleLangs":["en"],"autoSubs":false,"embedSubs":true}"#;
        let options: DownloadOptions = serde_json::from_str(old).unwrap();
        assert!(!options.fast_save);
        assert!(options.embeds_subtitles());
    }

    #[test]
    fn audio_selection_prefers_a_copyable_stream_only_when_asked() {
        for format in ["mp3", "m4a", "opus", "flac", "wav", "best"] {
            assert_eq!(audio_selector(format, AudioSelection::Best), "ba/b", "{format}");
        }
        assert_eq!(audio_selector("best", AudioSelection::PreferMatching), "ba/b");
        assert_eq!(audio_selector("m4a", AudioSelection::PreferMatching), "ba[ext=m4a]/ba[acodec^=aac]/ba[acodec^=mp4a.40.]/ba/b");
        assert_eq!(audio_selector("opus", AudioSelection::PreferMatching), "ba[acodec=opus]/ba/b");
        for format in ["mp3", "m4a", "opus", "flac", "wav"] {
            assert!(audio_selector(format, AudioSelection::PreferMatching).ends_with("/ba/b"), "{format} must fall back");
        }
        // The frozen choice is what reaches the command line, with conversion kept.
        let mut execution = execution();
        execution.audio_selection = AudioSelection::PreferMatching;
        let options = DownloadOptions { mode: "audio".into(), audio_format: "opus".into(), ..DownloadOptions::default() };
        let args = download_args(vec![], &options, &execution, Path::new("W"), Source::Url("https://e.com/a"));
        assert_eq!(value_after(&args, "-f"), ["ba[acodec=opus]/ba/b"]);
        assert_eq!(value_after(&args, "--audio-format"), ["opus"]);
        assert!(args.contains(&"-x".to_string()));
    }

    #[test]
    fn the_download_uses_exactly_the_shared_selection() {
        // The size preview re-runs this selection, so the download must not add its own.
        let cases = [
            DownloadOptions { max_height: Some(1080), ..options("video") },
            options("video"),
            DownloadOptions { subtitle_langs: vec!["en".into()], embed_subs: true, ..options("video") },
            DownloadOptions { audio_format: "m4a".into(), ..options("audio") },
            DownloadOptions { audio_format: "best".into(), ..options("audio") },
            DownloadOptions { max_height: Some(4320), video_preset: VideoPreset::Original, ..options("video") },
            DownloadOptions { video_preset: VideoPreset::Original, subtitle_langs: vec!["en".into()], ..options("video") },
        ];
        for selection in [AudioSelection::Best, AudioSelection::PreferMatching] {
            let mut execution = execution();
            execution.audio_selection = selection;
            for options in &cases {
                let shared = selection_args(options, selection);
                let args = download_args(vec![], options, &execution, Path::new("W"), Source::Url("https://e.com/v"));
                assert!(args.windows(shared.len()).any(|w| w == shared.as_slice()), "{options:?} {selection:?}: {args:?}");
                for flag in ["-f", "-S", "--merge-output-format", "--audio-format"] {
                    assert_eq!(value_after(&args, flag), value_after(&shared, flag), "{flag} only from the shared selection");
                }
            }
        }
        assert_eq!(
            selection_args(&DownloadOptions { max_height: Some(720), ..options("video") }, AudioSelection::Best),
            ["-f", "bv*+ba/b", "-S", "res:720,vcodec:h264,acodec:aac", "--merge-output-format", "mp4"]
        );
        assert_eq!(
            selection_args(&DownloadOptions { audio_format: "opus".into(), ..options("audio") }, AudioSelection::PreferMatching),
            ["-f", "ba[acodec=opus]/ba/b", "-x", "--audio-format", "opus", "--audio-quality", "0"]
        );
    }

    #[test]
    fn compatible_is_exactly_the_selection_jobs_had_before_presets() {
        // Missing from every job saved before presets: they keep what they were queued with.
        let old: DownloadOptions = serde_json::from_str(r#"{"mode":"video","maxHeight":1080,"audioFormat":"mp3","subtitleLangs":[],"autoSubs":false,"embedSubs":false,"fastSave":false}"#).unwrap();
        assert_eq!(old.video_preset, VideoPreset::Compatible);
        assert_eq!(
            selection_args(&old, AudioSelection::Best),
            ["-f", "bv*+ba/b", "-S", "res:1080,vcodec:h264,acodec:aac", "--merge-output-format", "mp4"]
        );
        assert_eq!(
            selection_args(&DownloadOptions { video_preset: VideoPreset::Compatible, ..options("video") }, AudioSelection::Best),
            ["-f", "bv*+ba/b", "-S", "res,vcodec:h264,acodec:aac", "--merge-output-format", "mp4"]
        );
    }

    #[test]
    fn original_keeps_the_best_streams_without_a_codec_preference_or_re_encoding() {
        let original = |max_height| DownloadOptions { max_height, video_preset: VideoPreset::Original, ..options("video") };
        assert_eq!(
            selection_args(&original(Some(4320)), AudioSelection::Best),
            ["-f", "bv*+ba/b", "-S", "res:4320", "--merge-output-format", "mp4/mkv"]
        );
        assert_eq!(selection_args(&original(None), AudioSelection::Best), ["-f", "bv*+ba/b", "-S", "res", "--merge-output-format", "mp4/mkv"]);
        let args = download_args(vec![], &original(Some(2160)), &execution(), Path::new("W"), Source::Url("https://e.com/v"));
        for flag in ["--recode-video", "--remux-video", "--postprocessor-args", "--ppa", "--exec"] {
            assert!(!args.iter().any(|a| a == flag), "{flag}: Original never re-encodes");
        }
        assert!(!args.iter().any(|a| a.contains("vcodec") || a.contains("acodec")), "no codec preference: {args:?}");
        // Subtitles and metadata are unchanged by the preset.
        let with_subs = DownloadOptions { subtitle_langs: vec!["en".into()], embed_subs: true, ..original(None) };
        let args = download_args(vec![], &with_subs, &execution(), Path::new("W"), Source::Url("https://e.com/v"));
        assert!(args.contains(&"--embed-subs".to_string()) && args.contains(&"--embed-metadata".to_string()));
    }

    #[test]
    fn the_preset_does_not_touch_audio_downloads() {
        for preset in [VideoPreset::Compatible, VideoPreset::Original] {
            let audio = DownloadOptions { audio_format: "m4a".into(), video_preset: preset, ..options("audio") };
            assert_eq!(selection_args(&audio, AudioSelection::Best), ["-f", "ba/b", "-x", "--audio-format", "m4a", "--audio-quality", "0"]);
        }
    }

    #[test]
    fn a_frozen_job_keeps_its_preset_whatever_the_settings_say_later() {
        // A paused or failed job starts again from its saved options and frozen execution;
        // nothing in the settings (there is no preset setting) reaches its selection.
        let options = DownloadOptions { max_height: Some(2880), video_preset: VideoPreset::Original, ..options("video") };
        let first = JobExecution::freeze(&Settings { output_dir: "D:/Out".into(), ..Settings::default() }, false);
        let first_args = download_args(vec![], &options, &first, Path::new("W"), Source::Url("https://e.com/v"));
        let saved: DownloadOptions = serde_json::from_str(&serde_json::to_string(&options).unwrap()).unwrap();
        let frozen: JobExecution = serde_json::from_str(&serde_json::to_string(&first).unwrap()).unwrap();
        let retry_args = download_args(vec![], &saved, &frozen, Path::new("W"), Source::Url("https://e.com/v"));
        assert_eq!(first_args, retry_args);
        assert_eq!(value_after(&retry_args, "-S"), ["res:2880"]);
        assert_eq!(value_after(&retry_args, "--merge-output-format"), ["mp4/mkv"]);
    }

    fn transcript(code: &str, auto: bool) -> DownloadOptions {
        DownloadOptions { mode: "transcript".into(), subtitle_langs: vec![code.into()], auto_subs: auto, ..DownloadOptions::default() }
    }

    #[test]
    fn a_transcript_fetches_one_caption_track_and_no_media() {
        let mut execution = execution();
        execution.embed_metadata = true;
        let common = vec!["--cookies".into(), "c".into(), "--ffmpeg-location".into(), "C:/ff".into()];
        let args = download_args(common, &transcript("en", false), &execution, Path::new("W"), Source::Url("https://e.com/v"));
        for flag in ["--skip-download", "--write-subs", "--ignore-no-formats-error"] {
            assert!(args.iter().any(|a| a == flag), "{flag}: {args:?}");
        }
        assert!(!args.contains(&"--write-auto-subs".to_string()), "only the kind chosen");
        assert_eq!(value_after(&args, "--sub-langs"), ["en"]);
        assert_eq!(value_after(&args, "--sub-format"), ["vtt/srt/ttml/best"]);
        assert_eq!(value_after(&args, "--convert-subs"), ["vtt"]);
        for flag in ["-f", "-S", "--merge-output-format", "-x", "--embed-subs", "--embed-metadata", "--embed-thumbnail", "--concurrent-fragments"] {
            assert!(!args.iter().any(|a| a == flag), "{flag} has nothing to do in a transcript: {args:?}");
        }
        // No media plan or media file is announced; the captions file is.
        assert_eq!(value_after(&args, "--print"), [CAPTIONS_PRINT]);
        assert!(!args.iter().any(|a| a == FORMATS_PRINT || a == DONE_PRINT));
        assert_eq!(value_after(&args, "-o"), [execution.filename_template.as_str()]);
        assert_eq!(value_after(&args, "-P"), ["W"]);
        assert_eq!(&args[args.len() - 2..], ["--", "https://e.com/v"]);
        assert_eq!(&args[..2], ["--cookies", "c"]);
        assert!(args.contains(&"--no-playlist".to_string()));

        let auto = download_args(vec![], &transcript("zh-Hans", true), &execution, Path::new("W"), Source::InfoJson(Path::new("C:/cache/a.json")));
        assert!(auto.contains(&"--write-auto-subs".to_string()));
        assert!(!auto.contains(&"--write-subs".to_string()), "human captions of the same language are not taken instead");
        assert_eq!(value_after(&auto, "--sub-langs"), ["zh\\-Hans"], "matched literally, not as a pattern");
        assert_eq!(&auto[auto.len() - 2..], ["--load-info-json", "C:/cache/a.json"]);
        let item = download_args(vec![], &transcript("en", false), &execution, Path::new("W"), Source::PlaylistItem { url: "https://e.com/l", index: 3 });
        assert_eq!(value_after(&item, "--playlist-items"), ["3"]);
    }

    #[test]
    fn without_ffmpeg_a_transcript_asks_for_formats_claws_reads_and_converts_nothing() {
        // yt-dlp stops before writing anything when asked to convert without FFmpeg.
        let args = download_args(vec!["--no-colors".into()], &transcript("de", false), &execution(), Path::new("W"), Source::Url("https://e.com/v"));
        assert_eq!(value_after(&args, "--sub-format"), ["vtt/srt/best"]);
        assert!(!args.iter().any(|a| a == "--convert-subs"), "{args:?}");
        assert!(args.iter().any(|a| a == "--skip-download"));
    }

    #[test]
    fn failures_are_classified_from_evidence() {
        assert_eq!(classify("ERROR: fragment 2 not found, unable to continue", false), FailureKind::MissingFragment);
        assert_eq!(classify("ERROR: unable to download video data: HTTP Error 403: Forbidden", false), FailureKind::ExpiredMedia);
        assert_eq!(classify("ERROR: [Errno 28] No space left on device", false), FailureKind::LocalIo);
        assert_eq!(classify("ERROR: Postprocessing: Conversion failed!", true), FailureKind::Processing);
        assert_eq!(classify("ERROR: something odd", true), FailureKind::Processing);
        assert_eq!(classify("ERROR: Read timed out.", false), FailureKind::Network);
        assert_eq!(classify("ERROR: something odd", false), FailureKind::Unknown);
        assert!(FailureKind::ExpiredMedia.refresh_helps());
        for kind in [FailureKind::Network, FailureKind::MissingFragment, FailureKind::LocalIo, FailureKind::Processing, FailureKind::Unknown] {
            assert!(!kind.refresh_helps(), "{kind:?}");
        }
    }
}

//! Download policy: what a job was started with (frozen, so later settings changes cannot
//! move or reshape a download that already has bytes on disk), the exact yt-dlp command
//! line built from it, and how failures are categorised.

use std::path::{Path, PathBuf};

use crate::downloader::DownloadOptions;

use serde::{Deserialize, Serialize};

use crate::settings::Settings;

pub const EXECUTION_SCHEMA: u16 = 1;

/// The fragment concurrency used until benchmarks justify another value (plan task 13).
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

// ---------------------------------------------------------------------------------------
// yt-dlp arguments. Pure: everything comes in as parameters, so the exact command line can
// be tested without a running app.

/// Where the media comes from. The source always goes last, after `--` when it is a URL,
/// so it can never be read as an option.
pub enum Source<'a> {
    Url(&'a str),
    /// An entry with no link of its own: item `index` of the playlist at `url`.
    PlaylistItem { url: &'a str, index: u32 },
    /// The probe's cached info, which skips a second extraction.
    InfoJson(&'a Path),
}

/// Resilience defaults for ordinary media downloads. Finite on purpose, and no wall-clock
/// limit is added, so a healthy multi-hour transfer is never cut short; a cancel kills the
/// process tree whether it is transferring or sleeping between retries.
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

/// The format selection for an audio-only download.
pub fn audio_selector(audio_format: &str, selection: AudioSelection) -> &'static str {
    let _ = (audio_format, selection);
    "ba/b"
}

pub fn download_args(
    common: Vec<String>,
    options: &DownloadOptions,
    execution: &JobExecution,
    workspace: &Path,
    source: Source<'_>,
) -> Vec<String> {
    let mut args = common;
    let mut push = |items: &[&str]| args.extend(items.iter().map(|s| s.to_string()));

    push(&["--newline", "--no-warnings", "--progress", "--no-simulate", "--windows-filenames"]);
    push(RETRY_ARGS);
    push(&["--progress-delta", PROGRESS_DELTA]);
    let fragments = execution.fragment_concurrency.max(1).to_string();
    push(&["--concurrent-fragments", &fragments]);
    match &source {
        Source::PlaylistItem { index, .. } => push(&["--yes-playlist", "--playlist-items", &index.to_string()]),
        _ => push(&["--no-playlist"]),
    }
    push(&[
        "--progress-template",
        PROGRESS_TEMPLATE,
        "--progress-template",
        POSTPROCESS_TEMPLATE,
        "--print",
        FORMATS_PRINT,
        "--print",
        DONE_PRINT,
    ]);

    if options.mode == "audio" {
        push(&[
            "-f",
            audio_selector(&options.audio_format, execution.audio_selection),
            "-x",
            "--audio-format",
            &options.audio_format,
            "--audio-quality",
            "0",
        ]);
        if matches!(options.audio_format.as_str(), "mp3" | "m4a") {
            push(&["--embed-thumbnail"]);
        }
    } else {
        let sort = match options.max_height {
            Some(h) => format!("res:{h},vcodec:h264,acodec:aac"),
            None => "res,vcodec:h264,acodec:aac".to_string(),
        };
        push(&["-f", "bv*+ba/b", "-S", &sort, "--merge-output-format", "mp4"]);
        if !options.subtitle_langs.is_empty() {
            let langs = options.subtitle_langs.join(",");
            push(&["--write-subs", "--sub-langs", &langs]);
            if options.auto_subs {
                push(&["--write-auto-subs"]);
            }
            if options.embed_subs {
                push(&["--embed-subs"]);
            } else {
                push(&["--convert-subs", "srt"]);
            }
        }
    }
    if execution.embed_metadata {
        push(&["--embed-metadata"]);
    }

    // Everything, including the finished file, is written inside the job's work folder;
    // Claws publishes it to the output folder afterwards.
    let workspace = workspace.to_string_lossy();
    push(&["-P", &workspace, "-o", &execution.filename_template]);
    match source {
        Source::InfoJson(path) => push(&["--load-info-json", &path.to_string_lossy()]),
        Source::Url(url) | Source::PlaylistItem { url, .. } => push(&["--", url]),
    }
    args
}

// ---------------------------------------------------------------------------------------
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

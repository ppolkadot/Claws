//! A stand-in for yt-dlp in process tests. Built only with
//! `cargo build --features test-tools --bin claws-fixture-tool`; never bundled.
//!
//! It speaks the same marker protocol Claws asks yt-dlp for (CLAWSFMT, CLAWS progress,
//! CLAWSPP, CLAWSDONE), writes its files under `-P <dir>`, and can hold a child process
//! open so process-tree shutdown can be tested.
//!
//!   claws-fixture-tool -P <dir> [--scenario ok|fail|hang|tree] [--bytes N] [--name clip]
//!
//! When Claws itself runs it as yt-dlp, the scenario and name come from the job's link,
//! `https://fixture.invalid/<scenario>[/<name>[/<planned bytes>[/<merged extension>]]]`, and
//! every start is appended to `fixture-starts.log` two folders above `-P` (the output folder
//! of a work folder), with the selection it was given (`-S` and `--merge-output-format`).
//!
//! ok    two streams (video, audio) with progress, a merge, then CLAWSDONE; exit 0
//! ok-fast  like ok, but the merge does not pause (for races with a late notification)
//! ok-subs  like ok-fast, and it leaves caption files `<name>.en.srt` and `<name>.es.srt` next to
//!          the merged file, as yt-dlp does for `--write-subs`
//! ok-preconvert  like ok, with a postprocessor that yt-dlp runs before the download first
//! late-progress  like ok, but the merge is announced on stderr before the last stream's
//!                progress reaches stdout (pipe reordering); `fixture-late.log` marks it sent
//! fail  one stream that stops halfway with an ERROR line; exit 1
//! fail-preconvert  a pre-download postprocessor, then a stream that stops with a network error
//! hang  starts a stream and then waits forever (for pause/cancel tests)
//! tree  like hang, but first starts a copy of itself as a child, which also waits
//!
//! Transcript runs (`--skip-download` with `--write-subs` or `--write-auto-subs`):
//! captions        writes `<name>.<lang>.vtt` (rolling captions for automatic ones) and
//!                 names it on a CLAWSCAPTIONS line, as yt-dlp's `after_video` print does
//! captions-hang   writes the captions file, then waits forever (for cancel tests)
//! captions-empty  writes a captions file with no cues
//! captions-none   names no file: the site has no captions in that language
//! A captions scenario started without `--skip-download` fails: no media may be fetched.
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut dir = PathBuf::from(".");
    let mut scenario = "ok".to_string();
    let mut bytes: u64 = 1 << 20;
    let mut name = "clip".to_string();
    let mut merged_ext = "mp4".to_string();
    let (mut sort, mut merge) = (String::from("-"), String::from("-"));
    let (mut skip_download, mut human_subs, mut auto_subs) = (false, false, false);
    let mut sub_langs = String::from("-");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-P" => dir = PathBuf::from(args.next().expect("-P needs a directory")),
            "-S" => sort = args.next().expect("-S needs a value"),
            "--merge-output-format" => merge = args.next().expect("--merge-output-format needs a value"),
            "--skip-download" => skip_download = true,
            "--write-subs" => human_subs = true,
            "--write-auto-subs" => auto_subs = true,
            // A literal language code arrives escaped as a pattern (`zh\-Hans`).
            "--sub-langs" => sub_langs = args.next().expect("--sub-langs needs a value").replace('\\', ""),
            "--scenario" => scenario = args.next().expect("--scenario needs a value"),
            "--bytes" => bytes = args.next().and_then(|b| b.parse().ok()).expect("--bytes needs a number"),
            "--name" => name = args.next().expect("--name needs a value"),
            link if link.starts_with("https://fixture.invalid/") => {
                let mut parts = link["https://fixture.invalid/".len()..].split('/').filter(|p| !p.is_empty());
                if let Some(s) = parts.next() {
                    scenario = s.to_string();
                }
                if let Some(n) = parts.next() {
                    name = n.to_string();
                }
                if let Some(b) = parts.next().and_then(|b| b.parse().ok()) {
                    bytes = b;
                }
                if let Some(ext) = parts.next() {
                    merged_ext = ext.to_string();
                }
            }
            _ => {}
        }
    }
    std::fs::create_dir_all(&dir).expect("cannot create the output directory");
    if let Some(output) = dir.parent().and_then(|p| p.parent()) {
        use std::io::Write as _;
        if let Ok(mut log) = std::fs::OpenOptions::new().create(true).append(true).open(output.join("fixture-starts.log")) {
            let kind = match (human_subs, auto_subs) {
                (true, false) => "human",
                (false, true) => "auto",
                (true, true) => "both",
                (false, false) => "-",
            };
            let skip = if skip_download { "yes" } else { "no" };
            let _ = writeln!(log, "{scenario} {name} {} skip={skip} subs={sub_langs} kind={kind} sort={sort} merge={merge}", dir.display());
        }
    }
    let out = std::io::stdout();
    let mut out = out.lock();

    let stream = |out: &mut std::io::StdoutLock<'_>, format: &str, ext: &str, total: u64, stop_at: Option<u64>| {
        let file = dir.join(format!("{name}.f{format}.{ext}"));
        let part = dir.join(format!("{name}.f{format}.{ext}.part"));
        let step = (total / 4).max(1);
        let mut done = 0;
        while done < total {
            if stop_at.is_some_and(|s| done >= s) {
                return None;
            }
            done = (done + step).min(total);
            std::fs::write(&part, vec![0u8; done as usize]).expect("cannot write");
            writeln!(out, "CLAWS|downloading|{done}|{total}|NA|1000000|1|NA|NA|{format}|{}", file.display()).unwrap();
            out.flush().unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::rename(&part, &file).expect("cannot finish the stream");
        writeln!(out, "CLAWS|finished|{total}|{total}|NA|NA|NA|NA|NA|{format}|{}", file.display()).unwrap();
        Some(file)
    };

    match scenario.as_str() {
        "ok" | "ok-fast" | "ok-subs" | "ok-preconvert" | "late-progress" => {
            writeln!(out, "CLAWSFMT|137|{bytes}|NA|140|{}|NA|137+140|NA|NA", bytes / 10).unwrap();
            if scenario == "ok-preconvert" {
                // A postprocessor that yt-dlp runs before the download (a subtitle
                // conversion): the same signal as the merge, but not the final stage.
                eprintln!("CLAWSPP|started|FFmpegSubtitlesConvertor");
                std::thread::sleep(Duration::from_millis(200));
                eprintln!("CLAWSPP|finished|FFmpegSubtitlesConvertor");
            }
            let video = stream(&mut out, "137", "mp4", bytes, None).unwrap();
            let audio = if scenario == "late-progress" {
                // The merge announces itself on stderr; the last stream's transfer lines
                // then reach stdout after Claws has read that announcement, as buffered
                // lines of the other pipe can.
                eprintln!("CLAWSPP|started|Merger");
                std::thread::sleep(Duration::from_millis(300));
                let audio = stream(&mut out, "140", "m4a", bytes / 10, None).unwrap();
                if let Some(output) = dir.parent().and_then(|p| p.parent()) {
                    let _ = std::fs::write(output.join("fixture-late.log"), b"late transfer lines written");
                }
                audio
            } else {
                let audio = stream(&mut out, "140", "m4a", bytes / 10, None).unwrap();
                eprintln!("CLAWSPP|started|Merger");
                audio
            };
            // A merge takes a while (ffmpeg reads and writes the whole file).
            if scenario != "ok-fast" && scenario != "ok-subs" {
                std::thread::sleep(Duration::from_millis(1500));
            }
            let merged = dir.join(format!("{name}.{merged_ext}"));
            std::fs::write(&merged, b"merged").unwrap();
            let _ = std::fs::remove_file(video);
            let _ = std::fs::remove_file(audio);
            if scenario == "ok-subs" {
                for lang in ["en", "es"] {
                    std::fs::write(dir.join(format!("{name}.{lang}.srt")), format!("1
00:00:01,000 --> 00:00:02,000
fixture {lang}
")).unwrap();
                }
            }
            eprintln!("CLAWSPP|finished|Merger");
            writeln!(out, "CLAWSDONE|{}", merged.display()).unwrap();
        }
        "fail" => {
            writeln!(out, "CLAWSFMT|18|{bytes}|NA|NA|NA|NA|18|{bytes}|NA").unwrap();
            stream(&mut out, "18", "mp4", bytes, Some(bytes / 2));
            eprintln!("ERROR: fixture: unable to download video data: HTTP Error 403: Forbidden");
            std::process::exit(1);
        }
        "fail-preconvert" => {
            writeln!(out, "CLAWSFMT|18|{bytes}|NA|NA|NA|NA|18|{bytes}|NA").unwrap();
            eprintln!("CLAWSPP|started|FFmpegSubtitlesConvertor");
            eprintln!("CLAWSPP|finished|FFmpegSubtitlesConvertor");
            stream(&mut out, "18", "mp4", bytes, Some(bytes / 2));
            eprintln!("ERROR: fixture: unable to download video data: The read operation timed out");
            std::process::exit(1);
        }
        "hang" | "tree" => {
            if scenario == "tree" {
                // Intentionally left alive: the test verifies whole-tree termination.
                #[allow(clippy::zombie_processes)]
                let child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["-P", &dir.to_string_lossy(), "--scenario", "hang", "--name", &format!("{name}-child")])
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .expect("cannot start the child");
                writeln!(out, "CHILD|{}", child.id()).unwrap();
            }
            writeln!(out, "CLAWSFMT|18|{bytes}|NA|NA|NA|NA|18|{bytes}|NA").unwrap();
            // Resumes like yt-dlp: an existing partial file is continued, not replaced.
            let part = dir.join(format!("{name}.f18.mp4.part"));
            let mut previous = std::fs::read(&part).unwrap_or_default();
            previous.extend_from_slice(b"partial");
            std::fs::write(&part, &previous).unwrap();
            writeln!(out, "CLAWS|downloading|{}|{bytes}|NA|1000|NA|NA|NA|18|{}", previous.len(), dir.join(format!("{name}.f18.mp4")).display()).unwrap();
            out.flush().unwrap();
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }
        "captions" | "captions-hang" | "captions-empty" | "captions-none" => {
            if !skip_download {
                eprintln!("ERROR: fixture: a transcript run asked for the media");
                std::process::exit(3);
            }
            if scenario == "captions-none" {
                writeln!(out, "CLAWSCAPTIONS|[]").unwrap();
                return;
            }
            // `--convert-subs vtt` is the only postprocessor of a transcript run.
            eprintln!("CLAWSPP|started|FFmpegSubtitlesConvertor");
            eprintln!("CLAWSPP|finished|FFmpegSubtitlesConvertor");
            let file = dir.join(format!("{name}.{sub_langs}.vtt"));
            let body = if scenario == "captions-empty" {
                "WEBVTT\n\n"
            } else if auto_subs {
                // YouTube's rolling automatic captions.
                concat!(
                    "WEBVTT\nKind: captions\nLanguage: en\n\n",
                    "00:00:00.320 --> 00:00:02.389 align:start position:0%\n \nhello<00:00:00.640><c> everyone</c>\n\n",
                    "00:00:02.389 --> 00:00:02.399 align:start position:0%\nhello everyone\n \n\n",
                    "00:00:02.399 --> 00:00:05.030 align:start position:0%\nhello everyone\nand<00:00:02.800><c> welcome</c><00:00:03.100><c> back</c>\n\n",
                    "00:00:05.030 --> 00:00:05.040 align:start position:0%\nand welcome back\n \n",
                )
            } else {
                "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\n<v Ana>Hello &amp; welcome</v>\n\n00:00:02.000 --> 00:00:03.500\nこれは字幕です 🎬\n"
            };
            std::fs::write(&file, body.as_bytes()).unwrap();
            let size = body.len();
            writeln!(out, "CLAWS|finished|{size}|{size}|NA|NA|NA|NA|NA|NA|{}", file.display()).unwrap();
            if scenario == "captions-hang" {
                out.flush().unwrap();
                loop {
                    std::thread::sleep(Duration::from_secs(3600));
                }
            }
            let escaped = file.display().to_string().replace('\\', "\\\\").replace('"', "\\\"");
            writeln!(out, "CLAWSCAPTIONS|[\"{escaped}\"]").unwrap();
        }
        other => {
            eprintln!("ERROR: unknown scenario {other}");
            std::process::exit(2);
        }
    }
}

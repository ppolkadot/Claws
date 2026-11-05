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
//! `https://fixture.invalid/<scenario>[/<name>[/<planned bytes>]]`, and every start is appended to
//! `fixture-starts.log` two folders above `-P` (the output folder of a work folder).
//!
//! ok    two streams (video, audio) with progress, a merge, then CLAWSDONE; exit 0
//! ok-fast  like ok, but the merge does not pause (for races with a late notification)
//! fail  one stream that stops halfway with an ERROR line; exit 1
//! hang  starts a stream and then waits forever (for pause/cancel tests)
//! tree  like hang, but first starts a copy of itself as a child, which also waits
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut dir = PathBuf::from(".");
    let mut scenario = "ok".to_string();
    let mut bytes: u64 = 1 << 20;
    let mut name = "clip".to_string();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-P" => dir = PathBuf::from(args.next().expect("-P needs a directory")),
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
            }
            _ => {}
        }
    }
    std::fs::create_dir_all(&dir).expect("cannot create the output directory");
    if let Some(output) = dir.parent().and_then(|p| p.parent()) {
        use std::io::Write as _;
        if let Ok(mut log) = std::fs::OpenOptions::new().create(true).append(true).open(output.join("fixture-starts.log")) {
            let _ = writeln!(log, "{scenario} {name} {}", dir.display());
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
        "ok" | "ok-fast" => {
            writeln!(out, "CLAWSFMT|137|{bytes}|NA|140|{}|NA|137+140|NA|NA", bytes / 10).unwrap();
            let video = stream(&mut out, "137", "mp4", bytes, None).unwrap();
            let audio = stream(&mut out, "140", "m4a", bytes / 10, None).unwrap();
            eprintln!("CLAWSPP|started|Merger");
            // A merge takes a while (ffmpeg reads and writes the whole file).
            if scenario != "ok-fast" {
                std::thread::sleep(Duration::from_millis(1500));
            }
            let merged = dir.join(format!("{name}.mp4"));
            std::fs::write(&merged, b"merged").unwrap();
            let _ = std::fs::remove_file(video);
            let _ = std::fs::remove_file(audio);
            eprintln!("CLAWSPP|finished|Merger");
            writeln!(out, "CLAWSDONE|{}", merged.display()).unwrap();
        }
        "fail" => {
            writeln!(out, "CLAWSFMT|18|{bytes}|NA|NA|NA|NA|18|{bytes}|NA").unwrap();
            stream(&mut out, "18", "mp4", bytes, Some(bytes / 2));
            eprintln!("ERROR: fixture: unable to download video data: HTTP Error 403: Forbidden");
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
        other => {
            eprintln!("ERROR: unknown scenario {other}");
            std::process::exit(2);
        }
    }
}

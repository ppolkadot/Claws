//! A stand-in for yt-dlp in process tests. Built only with
//! `cargo build --features test-tools --bin claws-fixture-tool`; never bundled.
//!
//! It speaks the same marker protocol Claws asks yt-dlp for (CLAWSFMT, CLAWS progress,
//! CLAWSPP, CLAWSDONE), writes its files under `-P <dir>`, and can hold a child process
//! open so process-tree shutdown can be tested.
//!
//!   claws-fixture-tool -P <dir> [--scenario ok|fail|hang|tree] [--bytes N] [--name clip]
//!
//! ok    two streams (video, audio) with progress, a merge, then CLAWSDONE; exit 0
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
            _ => {}
        }
    }
    std::fs::create_dir_all(&dir).expect("cannot create the output directory");
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
        "ok" => {
            writeln!(out, "CLAWSFMT|137|{bytes}|NA|140|{}|NA|137+140|NA|NA", bytes / 10).unwrap();
            let video = stream(&mut out, "137", "mp4", bytes, None).unwrap();
            let audio = stream(&mut out, "140", "m4a", bytes / 10, None).unwrap();
            eprintln!("CLAWSPP|started|Merger");
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
                let child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["-P", &dir.to_string_lossy(), "--scenario", "hang", "--name", &format!("{name}-child")])
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .expect("cannot start the child");
                writeln!(out, "CHILD|{}", child.id()).unwrap();
            }
            writeln!(out, "CLAWSFMT|18|{bytes}|NA|NA|NA|NA|18|{bytes}|NA").unwrap();
            let part = dir.join(format!("{name}.f18.mp4.part"));
            std::fs::write(&part, b"partial").unwrap();
            writeln!(out, "CLAWS|downloading|7|{bytes}|NA|1000|NA|NA|NA|18|{}", dir.join(format!("{name}.f18.mp4")).display()).unwrap();
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

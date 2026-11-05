//! Process and file-system tests for the download pipeline. Unit tests of pure logic live
//! next to that logic; these exercise real files and real child processes.
//!
//! Process tests need the yt-dlp stand-in, built with
//! `cargo build --features test-tools --bin claws-fixture-tool`. Without it they skip.

use std::path::{Path, PathBuf};

use crate::downloader;

pub(crate) fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("claws-{tag}-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub(crate) fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The stand-in tool, if it has been built next to the test binary.
pub(crate) fn fixture_tool() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("CLAWS_FIXTURE_TOOL") {
        return Some(PathBuf::from(path));
    }
    let exe = std::env::current_exe().ok()?;
    // target/debug/deps/<test>.exe -> target/debug/claws-fixture-tool.exe
    let candidate = exe.parent()?.parent()?.join(format!("claws-fixture-tool{}", std::env::consts::EXE_SUFFIX));
    candidate.is_file().then_some(candidate)
}

#[test]
fn legacy_cleanup_leaves_unrelated_files() {
    let dir = temp_dir("prefix");
    for name in ["clip.mp4.part", "clip.mp4.partnership", "clip.mp4.part-Frag2", "clip.mp4.ytdl", "clip.mp4"] {
        std::fs::write(dir.join(name), b"x").unwrap();
    }
    downloader::remove_partials_now(&[dir.join("clip.mp4").to_string_lossy().into_owned()]);
    assert_eq!(names(&dir), vec!["clip.mp4", "clip.mp4.partnership"]);
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(windows)]
#[test]
fn kill_tree_stops_the_whole_process_tree() {
    use std::io::{BufRead, BufReader};
    let Some(tool) = fixture_tool() else {
        eprintln!("skipped: build claws-fixture-tool with --features test-tools");
        return;
    };
    let dir = temp_dir("tree");
    let mut child = std::process::Command::new(&tool)
        .args(["-P", &dir.to_string_lossy(), "--scenario", "tree"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let grandchild: u32 = lines
        .find_map(|l| l.ok()?.strip_prefix("CHILD|").and_then(|p| p.parse().ok()))
        .expect("the tool reports its child");
    downloader::kill_tree(child.id(), true);
    let _ = child.wait();
    let alive = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {grandchild}"), "/NH"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&grandchild.to_string()))
        .unwrap_or(false);
    assert!(!alive, "the grandchild {grandchild} survived");
    let _ = std::fs::remove_dir_all(dir);
}

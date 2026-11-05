//! Each job's files live in its own work folder, `<output>/.claws-work/<short id>/`, on the
//! destination volume. yt-dlp writes only there; Claws publishes the finished files to the
//! output folder with a no-replace move, and cancelling deletes the verified work folder,
//! never a file chosen by matching names in the user's folder.
//!
//! Ownership is decided by layout plus a marker: the folder must be named after the job,
//! sit directly in a `.claws-work` folder, not be a link or junction, and hold a marker
//! naming the same job. Paths reported by tools are untrusted and are only accepted when
//! they resolve inside the work folder.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const WORK_DIR: &str = ".claws-work";
pub const MARKER: &str = ".claws-job.json";
pub const MANIFEST_SCHEMA: u16 = 1;

/// Side files published next to the media when they share its name.
const SIDECAR_EXTENSIONS: &[&str] = &["srt", "vtt", "ass", "ssa", "lrc", "ttml", "sbv", "dfxp", "srv1", "srv2", "srv3", "json3"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRef {
    pub schema_version: u16,
    pub path: PathBuf,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Publication {
    /// yt-dlp may be writing here.
    Active,
    /// Finished and verified; `outputs` are waiting to be published.
    Ready,
    /// Some of `outputs` may already be in the output folder (see `published`).
    Publishing,
    Published,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u16,
    pub job_id: String,
    pub workspace: PathBuf,
    /// Where finished files go: the frozen output folder.
    pub destination_root: PathBuf,
    pub state: Publication,
    /// Files to publish, relative to the work folder; the media file first.
    #[serde(default)]
    pub outputs: Vec<String>,
    /// Destinations this job has already published, in the order of `outputs`.
    #[serde(default)]
    pub published: Vec<PathBuf>,
}

/// The work folder's name: the first 12 hex digits of the job id. Short, because every
/// path yt-dlp builds inside it counts against Windows' 260-character limit; the marker
/// inside holds the full id, and is what proves ownership.
pub fn folder_name(job_id: &str) -> String {
    job_id.chars().filter(|c| c.is_ascii_hexdigit()).take(12).collect::<String>().to_ascii_lowercase()
}

pub fn workspace_path(output_dir: &Path, job_id: &str) -> PathBuf {
    output_dir.join(WORK_DIR).join(folder_name(job_id))
}

fn marker_path(workspace: &Path) -> PathBuf {
    workspace.join(MARKER)
}

#[cfg(windows)]
fn is_link(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    meta.file_type().is_symlink() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_link(meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

fn plain_dir(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || is_link(&meta) {
        return Err(io::Error::other(format!("{} is not a plain folder", path.display())));
    }
    Ok(())
}

#[cfg(windows)]
fn hide(path: &Path) {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN, FILE_FLAGS_AND_ATTRIBUTES};
    let wide = HSTRING::from(path.as_os_str());
    unsafe {
        let current = GetFileAttributesW(&wide);
        if current != u32::MAX {
            let _ = SetFileAttributesW(&wide, FILE_FLAGS_AND_ATTRIBUTES(current | FILE_ATTRIBUTE_HIDDEN.0));
        }
    }
}

#[cfg(not(windows))]
fn hide(_path: &Path) {}

/// Writes through a temp file, flushed to disk before it replaces the old manifest.
fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&serde_json::to_vec_pretty(value).map_err(io::Error::other)?)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

pub fn read_manifest(workspace: &Path) -> io::Result<Manifest> {
    let text = fs::read(marker_path(workspace))?;
    serde_json::from_slice(&text).map_err(io::Error::other)
}

pub fn write_manifest(manifest: &Manifest) -> io::Result<()> {
    write_json(&marker_path(&manifest.workspace), manifest)
}

/// Opens the job's work folder, creating it (with its marker) the first time. A folder
/// that exists but is not provably this job's is refused, never adopted.
pub fn open_workspace(output_dir: &Path, job_id: &str) -> io::Result<(WorkspaceRef, Manifest)> {
    uuid::Uuid::parse_str(job_id).map_err(|_| io::Error::other("invalid job id"))?;
    fs::create_dir_all(output_dir)?;
    let root = output_dir.join(WORK_DIR);
    if !root.exists() {
        fs::create_dir(&root).or_else(|e| if root.is_dir() { Ok(()) } else { Err(e) })?;
        hide(&root);
    }
    plain_dir(&root)?;
    let workspace = workspace_path(output_dir, job_id);
    let reference = WorkspaceRef {
        schema_version: MANIFEST_SCHEMA,
        path: workspace.clone(),
    };
    if workspace.exists() {
        let manifest = verify(&reference, job_id)?;
        return Ok((reference, manifest));
    }
    fs::create_dir(&workspace)?;
    let manifest = Manifest {
        schema_version: MANIFEST_SCHEMA,
        job_id: job_id.to_string(),
        workspace: workspace.clone(),
        destination_root: output_dir.to_path_buf(),
        state: Publication::Active,
        outputs: Vec::new(),
        published: Vec::new(),
    };
    write_manifest(&manifest)?;
    Ok((reference, manifest))
}

/// Proves `reference` is this job's work folder and returns its manifest.
pub fn verify(reference: &WorkspaceRef, job_id: &str) -> io::Result<Manifest> {
    let path = &reference.path;
    let named_for_job = path.file_name().and_then(|n| n.to_str()) == Some(folder_name(job_id).as_str());
    let parent = path.parent().ok_or_else(|| io::Error::other("work folder has no parent"))?;
    let in_work_dir = parent.file_name().and_then(|n| n.to_str()) == Some(WORK_DIR);
    if !named_for_job || !in_work_dir {
        return Err(io::Error::other(format!("{} is not a Claws work folder", path.display())));
    }
    plain_dir(parent)?;
    plain_dir(path)?;
    let manifest = read_manifest(path)?;
    if manifest.job_id != job_id || manifest.schema_version > MANIFEST_SCHEMA {
        return Err(io::Error::other(format!("{} belongs to another download", path.display())));
    }
    Ok(manifest)
}

/// The path of a tool-reported file relative to the work folder, if it is really inside.
pub fn owned_relative(workspace: &Path, reported: &Path) -> Option<PathBuf> {
    let base = fs::canonicalize(workspace).ok()?;
    let resolved = fs::canonicalize(reported).ok()?;
    let relative = resolved.strip_prefix(&base).ok()?.to_path_buf();
    let normal = relative.components().all(|c| matches!(c, Component::Normal(_)));
    let marker = relative.as_os_str() == MARKER;
    (normal && !marker && !relative.as_os_str().is_empty()).then_some(relative)
}

/// Side files (subtitles) yt-dlp left next to the media, sharing its file stem.
pub fn sidecars(workspace: &Path, media: &Path) -> Vec<PathBuf> {
    let Some(stem) = media.file_stem().and_then(|s| s.to_str()) else {
        return Vec::new();
    };
    let folder = workspace.join(media.parent().unwrap_or(Path::new("")));
    let Ok(entries) = fs::read_dir(&folder) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            let ext = Path::new(&name).extension()?.to_str()?.to_ascii_lowercase();
            let rest = name.strip_prefix(stem)?;
            (rest.starts_with('.') && SIDECAR_EXTENSIONS.contains(&ext.as_str()))
                .then(|| media.parent().unwrap_or(Path::new("")).join(&name))
        })
        .collect();
    found.sort();
    found
}

/// Moves `from` to `to` only if nothing exists at `to`. On Windows this is a single
/// `MoveFileExW` without the replace flag, which refuses an existing destination and a
/// different volume atomically; a plain rename would silently replace.
pub fn move_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
        unsafe { MoveFileExW(&HSTRING::from(from.as_os_str()), &HSTRING::from(to.as_os_str()), MOVEFILE_WRITE_THROUGH) }
            .map_err(|e| {
                // ERROR_FILE_EXISTS / ERROR_ALREADY_EXISTS
                if matches!(e.code().0 as u32 & 0xFFFF, 80 | 183) {
                    io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", to.display()))
                } else {
                    io::Error::other(e.message())
                }
            })
    }
    #[cfg(not(windows))]
    {
        fs::hard_link(from, to)?;
        fs::remove_file(from)
    }
}

/// Destination for a work-folder relative path, refusing anything that is not a plain
/// relative path below the output folder.
pub fn destination(root: &Path, relative: &Path) -> io::Result<PathBuf> {
    if relative.as_os_str().is_empty() || !relative.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(io::Error::other(format!("{} is not a valid file name", relative.display())));
    }
    Ok(root.join(relative))
}

#[derive(Debug)]
pub struct Published {
    pub media: PathBuf,
    /// Side files that could not be published (a name already taken); still in the work
    /// folder, which is then kept.
    pub skipped: Vec<PathBuf>,
}

/// Publishes the manifest's outputs, recording each move so an interrupted publication
/// can continue where it stopped. The media file must publish; a side file whose name is
/// taken is kept back and reported. `reserve` claims a destination against other jobs
/// for the duration of the move and returns false when another job holds it.
pub fn publish(manifest: &mut Manifest, reserve: &mut dyn FnMut(&Path) -> bool) -> io::Result<Published> {
    let mut skipped = Vec::new();
    let mut media = None;
    if manifest.state == Publication::Ready {
        manifest.state = Publication::Publishing;
        write_manifest(manifest)?;
    }
    for (index, relative) in manifest.outputs.clone().iter().enumerate() {
        let relative = Path::new(relative);
        let target = destination(&manifest.destination_root, relative)?;
        if manifest.published.contains(&target) {
            if index == 0 {
                media = Some(target);
            }
            continue;
        }
        let source = manifest.workspace.join(relative);
        let result = if reserve(&target) {
            move_no_replace(&source, &target)
        } else {
            Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("another download is saving {}", target.display())))
        };
        match result {
            Ok(()) => {
                manifest.published.push(target.clone());
                write_manifest(manifest)?;
                if index == 0 {
                    media = Some(target);
                }
            }
            Err(e) if index == 0 => return Err(e),
            Err(_) => skipped.push(target),
        }
    }
    let media = media.ok_or_else(|| io::Error::other("nothing to publish"))?;
    manifest.state = Publication::Published;
    write_manifest(manifest)?;
    Ok(Published { media, skipped })
}

/// Deletes a verified work folder. Retries while Windows still holds handles of a killed
/// process; an error means the folder is still there and cleanup must be retried later.
pub fn remove_workspace(reference: &WorkspaceRef, job_id: &str) -> io::Result<()> {
    if !reference.path.exists() {
        return Ok(());
    }
    verify(reference, job_id)?;
    let mut last = None;
    for attempt in 0..8 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(250));
        }
        // remove_dir_all does not follow links or junctions inside the folder.
        match fs::remove_dir_all(&reference.path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("could not remove the work folder")))
}

/// Partial-file names of downloads started before work folders existed: exactly
/// `<name>.part`, `<name>.ytdl` and `<name>.part-Frag<n>` (optionally `.part`).
pub fn is_legacy_partial(name: &str, entry: &str) -> bool {
    let Some(rest) = entry.strip_prefix(name) else { return false };
    if rest == ".part" || rest == ".ytdl" {
        return true;
    }
    let Some(frag) = rest.strip_prefix(".part-Frag") else { return false };
    let digits = frag.strip_suffix(".part").unwrap_or(frag);
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download_tests::{names, temp_dir};

    fn id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    #[test]
    fn workspace_is_created_once_and_reopened() {
        let out = temp_dir("ws");
        let job = id();
        let (reference, manifest) = open_workspace(&out, &job).unwrap();
        assert_eq!(reference.path, out.join(WORK_DIR).join(folder_name(&job)));
        assert_eq!(folder_name(&job).len(), 12);
        assert_eq!(manifest.state, Publication::Active);
        fs::write(reference.path.join("clip.f137.mp4.part"), b"12345").unwrap();
        let (again, _) = open_workspace(&out, &job).unwrap();
        assert_eq!(again, reference);
        assert_eq!(fs::read(again.path.join("clip.f137.mp4.part")).unwrap(), b"12345");
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn a_folder_of_another_job_is_refused() {
        let out = temp_dir("foreign");
        let (a, _) = open_workspace(&out, &id()).unwrap();
        let other = id();
        let forged = WorkspaceRef { schema_version: 1, path: a.path.clone() };
        assert!(verify(&forged, &other).is_err());
        assert!(remove_workspace(&forged, &other).is_err());
        assert!(a.path.exists());
        // A folder named for the job but without its marker is not adopted either.
        let plain = workspace_path(&out, &other);
        fs::create_dir_all(&plain).unwrap();
        assert!(open_workspace(&out, &other).is_err());
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn cancel_removes_only_the_work_folder() {
        let out = temp_dir("cancel");
        let job = id();
        let (reference, _) = open_workspace(&out, &job).unwrap();
        for name in ["clip.f137.mp4", "clip.f140.m4a.part", "clip.temp.mp4", "clip.en.vtt"] {
            fs::write(reference.path.join(name), b"x").unwrap();
        }
        // Unrelated files in the user's folder, including look-alikes.
        for name in ["clip.mp4", "clip.mp4.partnership", "clip.temp.mp4", "clip.f137.mp4"] {
            fs::write(out.join(name), b"keep").unwrap();
        }
        let (neighbour, _) = open_workspace(&out, &id()).unwrap();
        fs::write(neighbour.path.join("other.part"), b"keep").unwrap();
        remove_workspace(&reference, &job).unwrap();
        assert!(!reference.path.exists());
        assert!(neighbour.path.join("other.part").exists());
        assert_eq!(names(&out), vec![".claws-work", "clip.f137.mp4", "clip.mp4", "clip.mp4.partnership", "clip.temp.mp4"]);
        let _ = fs::remove_dir_all(out);
    }

    #[cfg(windows)]
    #[test]
    fn removal_does_not_follow_a_junction_out_of_the_work_folder() {
        let out = temp_dir("junction");
        let outside = temp_dir("outside");
        fs::write(outside.join("precious.mp4"), b"keep").unwrap();
        let job = id();
        let (reference, _) = open_workspace(&out, &job).unwrap();
        let link = reference.path.join("escape");
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J", &link.to_string_lossy(), &outside.to_string_lossy()])
            .output()
            .is_ok_and(|o| o.status.success());
        if made {
            remove_workspace(&reference, &job).unwrap();
            assert!(outside.join("precious.mp4").exists(), "the junction target must survive");
            // A reported path through the junction is not owned either.
            assert!(owned_relative(&out, &outside.join("precious.mp4")).is_none());
        }
        let _ = fs::remove_dir_all(out);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn reported_paths_must_resolve_inside() {
        let out = temp_dir("owned");
        let (reference, _) = open_workspace(&out, &id()).unwrap();
        fs::create_dir_all(reference.path.join("Channel")).unwrap();
        fs::write(reference.path.join("Channel").join("a.mp4"), b"x").unwrap();
        fs::write(out.join("b.mp4"), b"x").unwrap();
        assert_eq!(owned_relative(&reference.path, &reference.path.join("Channel").join("a.mp4")), Some(PathBuf::from("Channel").join("a.mp4")));
        assert_eq!(owned_relative(&reference.path, &reference.path.join("..").join("..").join("b.mp4")), None);
        assert_eq!(owned_relative(&reference.path, &reference.path.join(MARKER)), None);
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn publication_never_replaces_and_can_resume() {
        let out = temp_dir("publish");
        let job = id();
        let (reference, mut manifest) = open_workspace(&out, &job).unwrap();
        fs::write(reference.path.join("clip.mp4"), b"media").unwrap();
        fs::write(reference.path.join("clip.en.srt"), b"subs").unwrap();
        fs::write(out.join("clip.en.srt"), b"user's own").unwrap();
        manifest.outputs = vec!["clip.mp4".into(), "clip.en.srt".into()];
        manifest.state = Publication::Ready;
        let done = publish(&mut manifest, &mut |_| true).unwrap();
        assert_eq!(done.media, out.join("clip.mp4"));
        assert_eq!(done.skipped, vec![out.join("clip.en.srt")]);
        assert_eq!(fs::read(out.join("clip.en.srt")).unwrap(), b"user's own");
        assert_eq!(read_manifest(&reference.path).unwrap().state, Publication::Published);
        // Publishing again (after a crash) finds the media already published.
        assert_eq!(publish(&mut manifest, &mut |_| true).unwrap().media, out.join("clip.mp4"));
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn an_existing_destination_fails_publication_and_keeps_the_file() {
        let out = temp_dir("conflict");
        let (reference, mut manifest) = open_workspace(&out, &id()).unwrap();
        fs::write(reference.path.join("clip.mp4"), b"new").unwrap();
        fs::write(out.join("clip.mp4"), b"old").unwrap();
        manifest.outputs = vec!["clip.mp4".into()];
        manifest.state = Publication::Ready;
        let err = publish(&mut manifest, &mut |_| true).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(out.join("clip.mp4")).unwrap(), b"old");
        assert_eq!(fs::read(reference.path.join("clip.mp4")).unwrap(), b"new");
        // Another job holding the name blocks it too.
        fs::remove_file(out.join("clip.mp4")).unwrap();
        assert!(publish(&mut manifest, &mut |_| false).is_err());
        assert!(!out.join("clip.mp4").exists());
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn sidecars_share_the_media_stem() {
        let out = temp_dir("sidecars");
        let (reference, _) = open_workspace(&out, &id()).unwrap();
        for name in ["clip.mp4", "clip.en.srt", "clip.es.vtt", "clip2.en.srt", "clip.f137.mp4.part", "other.srt"] {
            fs::write(reference.path.join(name), b"x").unwrap();
        }
        assert_eq!(sidecars(&reference.path, Path::new("clip.mp4")), vec![PathBuf::from("clip.en.srt"), PathBuf::from("clip.es.vtt")]);
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn legacy_partials_match_exact_suffixes_only() {
        for good in [".part", ".ytdl", ".part-Frag3", ".part-Frag12.part"] {
            assert!(is_legacy_partial("clip.mp4", &format!("clip.mp4{good}")), "{good}");
        }
        for bad in ["clip.mp4", "clip.mp4.partnership", "clip.mp4.part-Frag", "clip.mp4.part-FragX", "clip.mp4.ytdl.bak", "other.part"] {
            assert!(!is_legacy_partial("clip.mp4", bad), "{bad}");
        }
    }
}

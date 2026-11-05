//! Each job's files live in its own work folder, `<output>/.claws-work/<short id>/`, on the
//! destination volume. yt-dlp writes only there; Claws publishes the finished files to the
//! output folder with a no-replace move, and cancelling deletes the verified work folder,
//! never a file chosen by matching names in the user's folder.
//!
//! Ownership is decided by layout plus a marker: the folder must be named after the job
//! and attempt, sit directly in a `.claws-work` folder, not be a link or junction, and hold
//! a marker naming the same job and attempt. Paths reported by tools are untrusted and are
//! only accepted when they resolve inside the work folder.
//!
//! A cancelled job that is tried again starts a new attempt in a new folder, so a cleanup
//! still finishing (or retried at the next start) for the cancelled attempt can never
//! reach the new attempt's files.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const WORK_DIR: &str = ".claws-work";
pub const MARKER: &str = ".claws-job.json";
/// 2: per-output move journal (`moves`) with file identities.
pub const MANIFEST_SCHEMA: u16 = 2;
/// The saved reference to a work folder; unchanged by the manifest's schema 2 (an absent
/// attempt reads as 0).
pub const WORKSPACE_REF_SCHEMA: u16 = 1;

/// Side files published next to the media when they share its name.
const SIDECAR_EXTENSIONS: &[&str] = &["srt", "vtt", "ass", "ssa", "lrc", "ttml", "sbv", "dfxp", "srv1", "srv2", "srv3", "json3"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRef {
    pub schema_version: u16,
    pub path: PathBuf,
    /// Which attempt of the job this folder belongs to. References saved before attempts
    /// existed read as 0, the attempt whose folder has the plain name.
    #[serde(default)]
    pub attempt: u32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Publication {
    /// yt-dlp may be writing here.
    Active,
    /// Finished and verified; `outputs` are waiting to be published.
    Ready,
    /// Some of `outputs` may already be in the output folder (see `moves`).
    Publishing,
    Published,
}

/// What proves that the file at a destination is the very file a job moved there, so a
/// move interrupted before its record was saved can be recognised, and nothing else can.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum FileIdentity {
    /// NTFS and ReFS keep a file's 128-bit id through a rename on the same volume.
    #[serde(rename_all = "camelCase")]
    FileId { volume: u64, id: String },
    /// Other file systems (FAT, exFAT) can renumber a moved file, so size and content are
    /// compared instead. This reads the whole file once before it is moved.
    #[serde(rename_all = "camelCase")]
    Content { size: u64, sha256: String },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MoveState {
    /// Saved before the move: if the source is gone, the move happened and the
    /// destination must match `identity`.
    Moving,
    Done,
    /// A side file whose name was taken; it stays in the work folder.
    Withheld,
}

/// One output's publication, journalled around its move.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Move {
    pub relative: String,
    pub destination: PathBuf,
    pub identity: FileIdentity,
    pub state: MoveState,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u16,
    pub job_id: String,
    /// See `WorkspaceRef::attempt`; missing in markers written before attempts existed.
    #[serde(default)]
    pub attempt: u32,
    pub workspace: PathBuf,
    /// Where finished files go: the frozen output folder.
    pub destination_root: PathBuf,
    pub state: Publication,
    /// Files to publish, relative to the work folder; the media file first.
    #[serde(default)]
    pub outputs: Vec<String>,
    /// Destinations published, as schema 1 recorded them (after the move). Still read;
    /// schema 2 records `moves` instead.
    #[serde(default)]
    pub published: Vec<PathBuf>,
    #[serde(default)]
    pub moves: Vec<Move>,
}

impl Manifest {
    fn is_done(&self, relative: &str, target: &Path) -> bool {
        self.published.iter().any(|p| p == target) || self.moves.iter().any(|m| m.relative == relative && m.state == MoveState::Done)
    }

    /// The published media file, once its move is recorded as done.
    pub fn media(&self) -> Option<PathBuf> {
        let first = self.outputs.first()?;
        let target = destination(&self.destination_root, Path::new(first)).ok()?;
        self.is_done(first, &target).then_some(target)
    }

    /// Side files kept back because their names were taken.
    pub fn withheld(&self) -> Vec<PathBuf> {
        self.moves.iter().filter(|m| m.state == MoveState::Withheld).map(|m| m.destination.clone()).collect()
    }

    fn record(&mut self, entry: Move) {
        self.moves.retain(|m| m.relative != entry.relative);
        self.moves.push(entry);
    }

    fn set_state(&mut self, relative: &str, state: MoveState) {
        if let Some(entry) = self.moves.iter_mut().find(|m| m.relative == relative) {
            entry.state = state;
        }
    }
}

/// The identity a moved file keeps (see `FileIdentity`).
pub fn identify(path: &Path) -> io::Result<FileIdentity> {
    #[cfg(windows)]
    if let Some(identity) = file_id(path)? {
        return Ok(identity);
    }
    content_identity(path)
}

#[cfg(windows)]
fn file_id(path: &Path) -> io::Result<Option<FileIdentity>> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{FileIdInfo, GetFileInformationByHandleEx, GetVolumeInformationByHandleW, FILE_ID_INFO};
    let file = fs::File::open(path)?;
    let handle = HANDLE(file.as_raw_handle());
    let mut name = [0u16; 32];
    unsafe { GetVolumeInformationByHandleW(handle, None, None, None, None, Some(&mut name)) }.map_err(|e| io::Error::other(e.message()))?;
    let end = name.iter().position(|c| *c == 0).unwrap_or(name.len());
    if !matches!(String::from_utf16_lossy(&name[..end]).as_str(), "NTFS" | "ReFS") {
        return Ok(None);
    }
    let mut info = FILE_ID_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(handle, FileIdInfo, &mut info as *mut FILE_ID_INFO as *mut std::ffi::c_void, std::mem::size_of::<FILE_ID_INFO>() as u32)
    }
    .map_err(|e| io::Error::other(e.message()))?;
    let id = info.FileId.Identifier.iter().map(|b| format!("{b:02x}")).collect();
    Ok(Some(FileIdentity::FileId { volume: info.VolumeSerialNumber, id }))
}

pub fn content_identity(path: &Path) -> io::Result<FileIdentity> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size += read as u64;
        digest.update(&buffer[..read]);
    }
    let sha256 = digest.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Ok(FileIdentity::Content { size, sha256 })
}

/// The work folder's name: the first 12 hex digits of the job id, then `-<attempt>` from
/// the second attempt on. Short, because every path yt-dlp builds inside it counts against
/// Windows' 260-character limit; the marker inside holds the full id and the attempt, and
/// is what proves ownership.
pub fn folder_name(job_id: &str, attempt: u32) -> String {
    let short = job_id.chars().filter(|c| c.is_ascii_hexdigit()).take(12).collect::<String>().to_ascii_lowercase();
    if attempt == 0 {
        short
    } else {
        format!("{short}-{attempt}")
    }
}

pub fn workspace_path(output_dir: &Path, job_id: &str, attempt: u32) -> PathBuf {
    output_dir.join(WORK_DIR).join(folder_name(job_id, attempt))
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
pub fn open_workspace(output_dir: &Path, job_id: &str, attempt: u32) -> io::Result<(WorkspaceRef, Manifest)> {
    uuid::Uuid::parse_str(job_id).map_err(|_| io::Error::other("invalid job id"))?;
    fs::create_dir_all(output_dir)?;
    let root = output_dir.join(WORK_DIR);
    if !root.exists() {
        fs::create_dir(&root).or_else(|e| if root.is_dir() { Ok(()) } else { Err(e) })?;
        hide(&root);
    }
    plain_dir(&root)?;
    let workspace = workspace_path(output_dir, job_id, attempt);
    let reference = WorkspaceRef {
        schema_version: WORKSPACE_REF_SCHEMA,
        path: workspace.clone(),
        attempt,
    };
    if workspace.exists() {
        let manifest = verify(&reference, job_id)?;
        return Ok((reference, manifest));
    }
    fs::create_dir(&workspace)?;
    let manifest = Manifest {
        schema_version: MANIFEST_SCHEMA,
        job_id: job_id.to_string(),
        attempt,
        workspace: workspace.clone(),
        destination_root: output_dir.to_path_buf(),
        state: Publication::Active,
        outputs: Vec::new(),
        published: Vec::new(),
        moves: Vec::new(),
    };
    write_manifest(&manifest)?;
    Ok((reference, manifest))
}

/// Proves `reference` is this job's work folder and returns its manifest.
pub fn verify(reference: &WorkspaceRef, job_id: &str) -> io::Result<Manifest> {
    let path = &reference.path;
    let named_for_job = path.file_name().and_then(|n| n.to_str()) == Some(folder_name(job_id, reference.attempt).as_str());
    let parent = path.parent().ok_or_else(|| io::Error::other("work folder has no parent"))?;
    let in_work_dir = parent.file_name().and_then(|n| n.to_str()) == Some(WORK_DIR);
    if !named_for_job || !in_work_dir {
        return Err(io::Error::other(format!("{} is not a Claws work folder", path.display())));
    }
    plain_dir(parent)?;
    plain_dir(path)?;
    let manifest = read_manifest(path)?;
    if manifest.job_id != job_id || manifest.attempt != reference.attempt || manifest.schema_version > MANIFEST_SCHEMA {
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

/// Publishes the manifest's outputs. Each move is journalled: its intent and the file's
/// identity are saved before the move and its result after, so a publication interrupted
/// at any point continues where it stopped, and a move whose record was lost is recognised
/// by identity rather than by name. The media file must publish; a side file whose name
/// is taken is kept back (`Withheld`) and reported. `reserve` claims a destination against
/// other jobs for the duration of the move and returns false when another job holds it.
pub fn publish(manifest: &mut Manifest, reserve: &mut dyn FnMut(&Path) -> bool) -> io::Result<Published> {
    if manifest.state == Publication::Ready {
        manifest.state = Publication::Publishing;
        write_manifest(manifest)?;
    }
    let mut skipped = Vec::new();
    for (index, relative) in manifest.outputs.clone().iter().enumerate() {
        let media = index == 0;
        let target = destination(&manifest.destination_root, Path::new(relative))?;
        let source = manifest.workspace.join(relative);
        if manifest.is_done(relative, &target) {
            continue;
        }
        let interrupted = manifest.moves.iter().find(|m| m.relative == *relative && m.state == MoveState::Moving).cloned();
        if let Some(entry) = interrupted.filter(|_| !source.exists()) {
            // The move happened but its result was not saved: the destination must be
            // the very file that was moved, never just a file with the same name.
            let ours = target.is_file() && identify(&target).ok().as_ref() == Some(&entry.identity);
            if ours {
                manifest.set_state(relative, MoveState::Done);
                write_manifest(manifest)?;
                continue;
            }
            let problem = if target.exists() {
                io::Error::new(io::ErrorKind::AlreadyExists, format!("{} is not the file this download saved there", target.display()))
            } else {
                io::Error::new(io::ErrorKind::NotFound, format!("{} was saved but is no longer there", target.display()))
            };
            if media {
                return Err(problem);
            }
            skipped.push(target);
            continue;
        }
        if !source.is_file() {
            let missing = io::Error::new(io::ErrorKind::NotFound, format!("the finished file {} is missing from the download's work folder", source.display()));
            if media {
                return Err(missing);
            }
            skipped.push(target);
            continue;
        }
        let result = if reserve(&target) {
            let identity = identify(&source)?;
            manifest.record(Move { relative: relative.clone(), destination: target.clone(), identity, state: MoveState::Moving });
            write_manifest(manifest)?;
            move_no_replace(&source, &target)
        } else {
            Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("another download is saving {}", target.display())))
        };
        match result {
            Ok(()) => {
                manifest.set_state(relative, MoveState::Done);
                write_manifest(manifest)?;
            }
            // The source is still in the work folder; the journal entry is retried.
            Err(e) if media => return Err(e),
            Err(_) => {
                manifest.set_state(relative, MoveState::Withheld);
                write_manifest(manifest)?;
                skipped.push(target);
            }
        }
    }
    let media = manifest.media().ok_or_else(|| io::Error::other("nothing to publish"))?;
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
        match remove_owned_folder(&reference.path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("could not remove the work folder")))
}

/// Deletes everything but the marker first, then the marker and the folder. A file that is
/// still in use stops the deletion with the marker in place, so the folder can still be
/// proven to be this job's when the cleanup is retried.
fn remove_owned_folder(path: &Path) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_name() == MARKER {
            continue;
        }
        let meta = fs::symlink_metadata(entry.path())?;
        if is_link(&meta) {
            // A link or junction: only the link goes, never what it points to. Links to
            // folders are removed as folders.
            if fs::remove_file(entry.path()).is_err() {
                fs::remove_dir(entry.path())?;
            }
        } else if meta.is_dir() {
            // remove_dir_all does not follow links or junctions inside the folder.
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
    }
    match fs::remove_file(marker_path(path)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    fs::remove_dir(path)
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
        let (reference, manifest) = open_workspace(&out, &job, 0).unwrap();
        assert_eq!(reference.path, out.join(WORK_DIR).join(folder_name(&job, 0)));
        assert_eq!(folder_name(&job, 0).len(), 12);
        assert_eq!(manifest.state, Publication::Active);
        fs::write(reference.path.join("clip.f137.mp4.part"), b"12345").unwrap();
        let (again, _) = open_workspace(&out, &job, 0).unwrap();
        assert_eq!(again, reference);
        assert_eq!(fs::read(again.path.join("clip.f137.mp4.part")).unwrap(), b"12345");
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn a_folder_of_another_job_is_refused() {
        let out = temp_dir("foreign");
        let (a, _) = open_workspace(&out, &id(), 0).unwrap();
        let other = id();
        let forged = WorkspaceRef { schema_version: 1, path: a.path.clone(), attempt: 0 };
        assert!(verify(&forged, &other).is_err());
        assert!(remove_workspace(&forged, &other).is_err());
        assert!(a.path.exists());
        // A folder named for the job but without its marker is not adopted either.
        let plain = workspace_path(&out, &other, 0);
        fs::create_dir_all(&plain).unwrap();
        assert!(open_workspace(&out, &other, 0).is_err());
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn cancel_removes_only_the_work_folder() {
        let out = temp_dir("cancel");
        let job = id();
        let (reference, _) = open_workspace(&out, &job, 0).unwrap();
        for name in ["clip.f137.mp4", "clip.f140.m4a.part", "clip.temp.mp4", "clip.en.vtt"] {
            fs::write(reference.path.join(name), b"x").unwrap();
        }
        // Unrelated files in the user's folder, including look-alikes.
        for name in ["clip.mp4", "clip.mp4.partnership", "clip.temp.mp4", "clip.f137.mp4"] {
            fs::write(out.join(name), b"keep").unwrap();
        }
        let (neighbour, _) = open_workspace(&out, &id(), 0).unwrap();
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
        let (reference, _) = open_workspace(&out, &job, 0).unwrap();
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
        let (reference, _) = open_workspace(&out, &id(), 0).unwrap();
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
        let (reference, mut manifest) = open_workspace(&out, &job, 0).unwrap();
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
        let (reference, mut manifest) = open_workspace(&out, &id(), 0).unwrap();
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
    fn attempts_have_their_own_folders_and_never_adopt_each_other() {
        let out = temp_dir("attempts");
        let job = id();
        let (first, _) = open_workspace(&out, &job, 0).unwrap();
        let (second, manifest) = open_workspace(&out, &job, 1).unwrap();
        assert_ne!(first.path, second.path);
        assert!(second.path.ends_with(format!("{}-1", folder_name(&job, 0))));
        assert_eq!((second.attempt, manifest.attempt), (1, 1));
        fs::write(second.path.join("clip.f18.mp4.part"), b"retry").unwrap();
        // A cleanup of the first attempt that points at the second folder is refused.
        let stale = WorkspaceRef { attempt: 0, ..second.clone() };
        assert!(remove_workspace(&stale, &job).is_err());
        remove_workspace(&first, &job).unwrap();
        assert_eq!(fs::read(second.path.join("clip.f18.mp4.part")).unwrap(), b"retry");
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn references_and_markers_from_before_attempts_still_open() {
        let out = temp_dir("pre-attempts");
        let job = id();
        let folder = workspace_path(&out, &job, 0);
        fs::create_dir_all(&folder).unwrap();
        // Exactly what the previous build wrote.
        let marker = format!(
            r#"{{"schemaVersion":1,"jobId":"{job}","workspace":{},"destinationRoot":{},"state":"active","outputs":[],"published":[]}}"#,
            serde_json::to_string(&folder).unwrap(),
            serde_json::to_string(&out).unwrap()
        );
        fs::write(folder.join(MARKER), marker).unwrap();
        let reference: WorkspaceRef = serde_json::from_str(&format!(r#"{{"schemaVersion":1,"path":{}}}"#, serde_json::to_string(&folder).unwrap())).unwrap();
        assert_eq!(reference.attempt, 0);
        assert_eq!(verify(&reference, &job).unwrap().attempt, 0);
        assert_eq!(open_workspace(&out, &job, 0).unwrap().0, reference);
        let _ = fs::remove_dir_all(out);
    }

    /// A workspace whose media (and optionally a side file) is ready to publish.
    fn ready(tag: &str, sidecar: bool) -> (PathBuf, Manifest) {
        let out = temp_dir(tag);
        let (reference, mut manifest) = open_workspace(&out, &id(), 0).unwrap();
        fs::write(reference.path.join("clip.mp4"), b"media").unwrap();
        manifest.outputs = vec!["clip.mp4".into()];
        if sidecar {
            fs::write(reference.path.join("clip.en.srt"), b"subs").unwrap();
            manifest.outputs.push("clip.en.srt".into());
        }
        manifest.state = Publication::Ready;
        write_manifest(&manifest).unwrap();
        (out, manifest)
    }

    /// The state on disk after a crash between moving `relative` and saving that.
    fn crash_after_move(manifest: &mut Manifest, relative: &str) {
        let source = manifest.workspace.join(relative);
        let target = destination(&manifest.destination_root, Path::new(relative)).unwrap();
        manifest.state = Publication::Publishing;
        manifest.record(Move { relative: relative.into(), destination: target.clone(), identity: identify(&source).unwrap(), state: MoveState::Moving });
        write_manifest(manifest).unwrap();
        move_no_replace(&source, &target).unwrap();
    }

    fn reopen(manifest: &Manifest) -> Manifest {
        read_manifest(&manifest.workspace).unwrap()
    }

    #[test]
    fn a_move_whose_record_was_lost_is_recognised_by_identity() {
        let (out, mut manifest) = ready("journal-moved", false);
        crash_after_move(&mut manifest, "clip.mp4");
        let mut recovered = reopen(&manifest);
        let done = publish(&mut recovered, &mut |_| true).unwrap();
        assert_eq!(done.media, out.join("clip.mp4"));
        assert_eq!(fs::read(out.join("clip.mp4")).unwrap(), b"media");
        assert_eq!(reopen(&manifest).state, Publication::Published);
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn a_crash_before_the_move_simply_moves() {
        let (out, mut manifest) = ready("journal-before", false);
        let source = manifest.workspace.join("clip.mp4");
        manifest.state = Publication::Publishing;
        manifest.record(Move { relative: "clip.mp4".into(), destination: out.join("clip.mp4"), identity: identify(&source).unwrap(), state: MoveState::Moving });
        write_manifest(&manifest).unwrap();
        let mut recovered = reopen(&manifest);
        assert_eq!(publish(&mut recovered, &mut |_| true).unwrap().media, out.join("clip.mp4"));
        assert!(!source.exists());
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn a_replaced_or_missing_destination_is_never_taken_as_published() {
        // Replaced by another file with the same name (and even the same size).
        let (out, mut manifest) = ready("journal-replaced", false);
        crash_after_move(&mut manifest, "clip.mp4");
        fs::remove_file(out.join("clip.mp4")).unwrap();
        fs::write(out.join("clip.mp4"), b"other").unwrap();
        let err = publish(&mut reopen(&manifest), &mut |_| true).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "{err}");
        assert_eq!(fs::read(out.join("clip.mp4")).unwrap(), b"other");
        assert_ne!(reopen(&manifest).state, Publication::Published);
        let _ = fs::remove_dir_all(out);

        // Gone altogether.
        let (out, mut manifest) = ready("journal-missing", false);
        crash_after_move(&mut manifest, "clip.mp4");
        fs::remove_file(out.join("clip.mp4")).unwrap();
        let err = publish(&mut reopen(&manifest), &mut |_| true).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound, "{err}");
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn a_partly_published_set_finishes_without_moving_twice() {
        let (out, mut manifest) = ready("journal-partial", true);
        // The media went through completely; the side file was moved but not recorded.
        let mut first = manifest.clone();
        first.outputs.truncate(1);
        publish(&mut first, &mut |_| true).unwrap();
        manifest.moves = first.moves.clone();
        crash_after_move(&mut manifest, "clip.en.srt");
        let mut recovered = reopen(&manifest);
        let mut claimed = Vec::new();
        let done = publish(&mut recovered, &mut |t| {
            claimed.push(t.to_path_buf());
            true
        })
        .unwrap();
        assert!(claimed.is_empty(), "nothing needed moving again: {claimed:?}");
        assert!(done.skipped.is_empty());
        assert_eq!(fs::read(out.join("clip.en.srt")).unwrap(), b"subs");
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn a_withheld_side_file_is_kept_and_published_once_its_name_is_free() {
        let (out, mut manifest) = ready("journal-withheld", true);
        fs::write(out.join("clip.en.srt"), b"user's own").unwrap();
        let done = publish(&mut manifest, &mut |_| true).unwrap();
        assert_eq!(done.skipped, vec![out.join("clip.en.srt")]);
        let saved = reopen(&manifest);
        assert_eq!(saved.withheld(), vec![out.join("clip.en.srt")]);
        assert_eq!(saved.media(), Some(out.join("clip.mp4")));
        assert_eq!(fs::read(saved.workspace.join("clip.en.srt")).unwrap(), b"subs");
        // Moved away by the user: publishing again delivers it.
        fs::rename(out.join("clip.en.srt"), out.join("mine.srt")).unwrap();
        let again = publish(&mut reopen(&manifest), &mut |_| true).unwrap();
        assert!(again.skipped.is_empty());
        assert_eq!(fs::read(out.join("clip.en.srt")).unwrap(), b"subs");
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn schema_1_records_still_count_as_published() {
        let (out, mut manifest) = ready("journal-v1", false);
        move_no_replace(&manifest.workspace.join("clip.mp4"), &out.join("clip.mp4")).unwrap();
        manifest.state = Publication::Publishing;
        manifest.published = vec![out.join("clip.mp4")];
        write_manifest(&manifest).unwrap();
        let mut recovered = reopen(&manifest);
        assert_eq!(publish(&mut recovered, &mut |_| true).unwrap().media, out.join("clip.mp4"));
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn identities_tell_moved_files_from_lookalikes() {
        let dir = temp_dir("identity");
        fs::write(dir.join("a"), b"same").unwrap();
        fs::write(dir.join("b"), b"same").unwrap();
        let a = identify(&dir.join("a")).unwrap();
        assert_ne!(a, identify(&dir.join("b")).unwrap(), "an equal copy is a different file");
        move_no_replace(&dir.join("a"), &dir.join("c")).unwrap();
        assert_eq!(identify(&dir.join("c")).unwrap(), a, "a moved file keeps its identity");
        // The content fallback: equal for equal bytes, different otherwise.
        assert_eq!(content_identity(&dir.join("b")).unwrap(), content_identity(&dir.join("c")).unwrap());
        fs::write(dir.join("b"), b"diff").unwrap();
        assert_ne!(content_identity(&dir.join("b")).unwrap(), content_identity(&dir.join("c")).unwrap());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sidecars_share_the_media_stem() {
        let out = temp_dir("sidecars");
        let (reference, _) = open_workspace(&out, &id(), 0).unwrap();
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

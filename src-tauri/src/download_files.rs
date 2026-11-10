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
//! The marker is data, not authority: `verify` binds it to the folders it was read from (its
//! `workspace` and `destination_root` must be those real folders, and every output and move
//! must stay below them), and only a manifest bound that way can be written or published.
//! A file is deleted for a Library entry only through `remove_verified`, which checks the
//! identity of the object it has open and deletes that same object.
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
/// The saved reference to a work folder; an absent `attempt` reads as 0.
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
    /// Set only by `verify`/`open_workspace`: this manifest's workspace and destination
    /// root were proven to be the folders it was read from. Never read from disk.
    #[serde(skip)]
    bound: bool,
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

    /// A recorded move is not enough: the published file must still be present and ours.
    fn validate_done(&self, relative: &str, target: &Path) -> io::Result<()> {
        let metadata = fs::symlink_metadata(target)?;
        if !metadata.is_file() || is_link(&metadata) {
            return Err(io::Error::other(format!("{} is not a regular published file", target.display())));
        }
        if let Some(entry) = self.moves.iter().find(|m| m.relative == relative && m.state == MoveState::Done) {
            if identify(target)? != entry.identity {
                return Err(io::Error::other(format!("{} is no longer the file this download published", target.display())));
            }
        }
        // Schema 1 has no identity evidence; require existence, without inventing an identity.
        Ok(())
    }

    pub fn validated_media(&self) -> io::Result<PathBuf> {
        let relative = self.outputs.first().ok_or_else(|| io::Error::other("no published media recorded"))?;
        let target = self.media().ok_or_else(|| io::Error::other("media publication is incomplete"))?;
        self.validate_done(relative, &target)?;
        Ok(target)
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

pub fn identify(path: &Path) -> io::Result<FileIdentity> {
    identify_open(&mut fs::File::open(path)?)
}

/// The identity of an already opened file: its file id where the volume keeps one, else its
/// size and content hash.
fn identify_open(file: &mut fs::File) -> io::Result<FileIdentity> {
    #[cfg(windows)]
    if let Some(identity) = file_id_of(file)? {
        return Ok(identity);
    }
    content_of(file)
}

/// Whether the opened file is the object `expected` describes, compared the way `expected`
/// was taken (a file id never matches a content hash, nor the reverse).
#[cfg(windows)]
fn matches_identity(file: &mut fs::File, expected: &FileIdentity) -> io::Result<bool> {
    Ok(match expected {
        FileIdentity::FileId { .. } => file_id_of(file)?.as_ref() == Some(expected),
        FileIdentity::Content { .. } => content_of(file)? == *expected,
    })
}

#[cfg(windows)]
fn file_id_of(file: &fs::File) -> io::Result<Option<FileIdentity>> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{FileIdInfo, GetFileInformationByHandleEx, GetVolumeInformationByHandleW, FILE_ID_INFO};
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

#[cfg(test)]
pub fn content_identity(path: &Path) -> io::Result<FileIdentity> {
    content_of(&mut fs::File::open(path)?)
}

fn content_of(file: &mut fs::File) -> io::Result<FileIdentity> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
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

/// The outcome of deleting a file proven to be the one a download saved.
#[derive(Debug, PartialEq, Eq)]
pub enum Removal {
    Deleted,
    /// Nothing is at the path any more.
    Missing,
}

#[derive(Debug)]
pub enum RemoveError {
    /// Something else is at the path now (replaced, a folder, a link): left alone.
    Replaced,
    /// This platform cannot delete the verified object itself, so nothing is deleted.
    #[cfg_attr(windows, allow(dead_code))]
    Unsupported,
    /// The file could not be opened or deleted (in use, no permission).
    Io(io::Error),
    /// The file was opened but could not be identified (its file id or content could not
    /// be read), so nothing was deleted. Not the same as a file in use.
    Unverifiable(io::Error),
}

impl std::fmt::Display for RemoveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Replaced => f.write_str("it is no longer the file Claws saved"),
            Self::Unsupported => f.write_str("this system cannot confirm which file would be deleted"),
            Self::Io(e) => write!(f, "{e}"),
            Self::Unverifiable(e) => write!(f, "its identity could not be read: {e}"),
        }
    }
}

/// Deletes the file at `path` only if it is the object `expected` describes. The file is
/// opened once, identified through that open handle (a file id, or a content hash read from
/// the handle on volumes without ids) and then deleted through the same handle, so a file
/// swapped into the path after the check can never be the one removed. A comparison
/// followed by `remove_file(path)` would not give that guarantee. Blocking: it may hash a
/// large file.
pub fn remove_verified(path: &Path, expected: &FileIdentity) -> Result<Removal, RemoveError> {
    remove_verified_with(path, expected, &mut || {})
}

/// `between` runs after the check and before the deletion; tests swap the path there.
#[cfg(windows)]
fn remove_verified_with(path: &Path, expected: &FileIdentity, between: &mut dyn FnMut()) -> Result<Removal, RemoveError> {
    use std::os::windows::fs::OpenOptionsExt;
    const DELETE: u32 = 0x0001_0000;
    const GENERIC_READ: u32 = 0x8000_0000;
    const SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let mut file = match fs::OpenOptions::new()
        .access_mode(GENERIC_READ | DELETE)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Removal::Missing),
        Err(e) => return Err(RemoveError::Io(e)),
    };
    let meta = file.metadata().map_err(RemoveError::Unverifiable)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(RemoveError::Replaced);
    }
    identity_verdict(matches_identity(&mut file, expected))?;
    between();
    delete_on_handle(&file).map_err(RemoveError::Io)?;
    Ok(Removal::Deleted)
}

/// What the comparison of an opened file with its recorded identity means. A comparison that
/// could not be made (the file id or the content could not be read) is neither a match nor a
/// replacement and not a file in use: it is reported as unverifiable.
#[cfg(windows)]
fn identity_verdict(compared: io::Result<bool>) -> Result<(), RemoveError> {
    match compared {
        Ok(true) => Ok(()),
        Ok(false) => Err(RemoveError::Replaced),
        Err(e) => Err(RemoveError::Unverifiable(e)),
    }
}

/// Marks the open file for deletion; it goes when the handle closes (at once, by name,
/// where the file system supports POSIX semantics). A read-only attribute does not keep a
/// download Claws saved from being deleted, as it did not for `remove_file`.
#[cfg(windows)]
fn delete_on_handle(file: &fs::File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FileDispositionInfo, FileDispositionInfoEx, SetFileInformationByHandle, FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
        FILE_DISPOSITION_FLAG_POSIX_SEMANTICS, FILE_DISPOSITION_INFO, FILE_DISPOSITION_INFO_EX, FILE_DISPOSITION_INFO_EX_FLAGS,
    };
    let handle = HANDLE(file.as_raw_handle());
    let ex = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_INFO_EX_FLAGS(FILE_DISPOSITION_FLAG_DELETE.0 | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS.0 | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE.0),
    };
    let tried = unsafe {
        SetFileInformationByHandle(handle, FileDispositionInfoEx, &ex as *const FILE_DISPOSITION_INFO_EX as *const std::ffi::c_void, std::mem::size_of::<FILE_DISPOSITION_INFO_EX>() as u32)
    };
    if tried.is_ok() {
        return Ok(());
    }
    // Older file systems: plain delete-on-close, still on this very handle.
    let info = FILE_DISPOSITION_INFO { DeleteFile: true };
    unsafe {
        SetFileInformationByHandle(handle, FileDispositionInfo, &info as *const FILE_DISPOSITION_INFO as *const std::ffi::c_void, std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32)
    }
    .map_err(|e| io::Error::other(e.message()))
}

#[cfg(not(windows))]
fn remove_verified_with(_path: &Path, _expected: &FileIdentity, _between: &mut dyn FnMut()) -> Result<Removal, RemoveError> {
    Err(RemoveError::Unsupported)
}

/// What proves a published download is Claws' to delete: the identity of the media file
/// and the side files that were really published next to it.
pub struct Evidence {
    pub identity: FileIdentity,
    /// Only moves recorded as done in a verified marker; a side file whose name was taken
    /// stays the user's.
    pub sidecars: Vec<Move>,
}

/// The ownership evidence of a finished download, from its verified work-folder marker.
/// Nothing is derived from a path that merely exists: a marker without identities (schema
/// 1), another media path, a missing or refused marker, or a download that did not publish
/// gives `Ok(None)`: no evidence, and the entry is then never assumed to own the file.
///
/// `Err` is only a marker that could not be read *right now* (held by a scanner or indexer),
/// which says nothing about ownership. It is tried again a few times first; if it still
/// cannot be read the caller must not save the entry without evidence, or the download
/// would lose its proof for good. Blocking.
pub fn evidence(reference: &WorkspaceRef, job_id: &str, media: &Path) -> io::Result<Option<Evidence>> {
    evidence_pausing(reference, job_id, media, EVIDENCE_ATTEMPTS, || std::thread::sleep(std::time::Duration::from_millis(100)))
}

const EVIDENCE_ATTEMPTS: u32 = 5;

/// `evidence` with the number of attempts and the wait between them supplied, so a test can
/// end a lock exactly there instead of racing the clock.
fn evidence_pausing(reference: &WorkspaceRef, job_id: &str, media: &Path, attempts: u32, mut pause: impl FnMut()) -> io::Result<Option<Evidence>> {
    let mut attempt = 1;
    loop {
        match read_evidence(reference, job_id, media) {
            Err(_) if attempt < attempts => {
                pause();
                attempt += 1;
            }
            other => return other,
        }
    }
}

/// Whether a failure to read a file is the kind that passes (another program holds it, an
/// unreachable or dropped network volume, a refused access, a retryable system condition) as
/// opposed to a verdict about the file itself (missing, damaged, not this download's).
///
/// The verdicts are the narrow set: Claws' own refusals and parse errors (no OS error at
/// all) and the system saying the file or folder is not there. Any other OS error is
/// transient, whatever `ErrorKind` it maps to: a share that blips reports NotFound or
/// Uncategorized, and taking that for "no evidence" would lose the proof for good.
fn is_transient(error: &io::Error) -> bool {
    // 2 and 3: file / path not found; 267 (Windows): the directory name is invalid, i.e. not a folder.
    matches!(error.kind(), io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) || error.raw_os_error().is_some_and(|code| !matches!(code, 2 | 3 | 267))
}

fn read_evidence(reference: &WorkspaceRef, job_id: &str, media: &Path) -> io::Result<Option<Evidence>> {
    let manifest = match verify(reference, job_id) {
        Ok(manifest) => manifest,
        Err(e) if is_transient(&e) => return Err(e),
        Err(_) => return Ok(None),
    };
    if manifest.state != Publication::Published || manifest.media().as_deref() != Some(media) {
        return Ok(None);
    }
    let Some(first) = manifest.outputs.first() else { return Ok(None) };
    let Some(done) = manifest.moves.iter().find(|m| m.relative == *first && m.state == MoveState::Done) else { return Ok(None) };
    let sidecars = manifest
        .moves
        .iter()
        .filter(|m| m.relative != *first && m.state == MoveState::Done && m.destination != media)
        .cloned()
        .collect();
    Ok(Some(Evidence { identity: done.identity.clone(), sidecars }))
}

/// Side files a finished download left in its work folder because their names were taken in
/// the output folder. The only durable pointer to them once the job leaves the Queue: saved
/// with the Library entry. `sources` are plain paths relative to the work folder (never the
/// occupied destinations), and they are only ever resolved through a verified work folder.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KeptFiles {
    pub workspace: WorkspaceRef,
    pub sources: Vec<String>,
}

/// The kept side files of a finished download, from its verified work-folder marker: the
/// moves recorded as withheld whose files are still there. `media`, when given, must be the
/// media the marker published. None when nothing is kept or the marker proves nothing.
pub fn kept_files(reference: &WorkspaceRef, job_id: &str, media: Option<&Path>) -> Option<KeptFiles> {
    let manifest = verify(reference, job_id).ok()?;
    if manifest.state != Publication::Published || media.is_some_and(|media| !manifest.media().is_some_and(|published| same_path(&published, media))) {
        return None;
    }
    let sources: Vec<String> = withheld_sources(&manifest).into_iter().map(|(relative, _)| relative).collect();
    (!sources.is_empty()).then(|| KeptFiles { workspace: reference.clone(), sources })
}

/// Whether the work folder is a finished download's and still holds a side file it kept back. Such a
/// folder is the only copy of that file: no cleanup may delete it, whatever became of the job.
pub fn holds_kept_files(reference: &WorkspaceRef, job_id: &str) -> bool {
    verify(reference, job_id).is_ok_and(|m| m.state == Publication::Published && !withheld_sources(&m).is_empty())
}

/// The same path as Windows sees it: letter case and the direction of separators do not matter.
fn same_path(a: &Path, b: &Path) -> bool {
    let spelled = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    spelled(a) == spelled(b)
}

/// The side files a verified marker records as withheld that are still there as plain files
/// in the work folder: each relative path with its full path. Never a path the marker does
/// not record, and never one reached through a link.
fn withheld_sources(manifest: &Manifest) -> Vec<(String, PathBuf)> {
    manifest
        .moves
        .iter()
        .filter(|m| m.state == MoveState::Withheld)
        .filter(|m| plain_below(&manifest.workspace, Path::new(&m.relative)).is_ok())
        .map(|m| (m.relative.clone(), manifest.workspace.join(&m.relative)))
        .filter(|(_, path)| fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file() && !is_link(&meta)))
        .collect()
}

/// Where kept files can be found now.
#[derive(Debug, PartialEq, Eq)]
pub enum KeptState {
    /// The kept files that exist, as paths inside the verified work folder.
    Available(Vec<PathBuf>),
    /// The output folder (or its drive) cannot be reached.
    Offline,
    /// The output folder is there but the work folder or its files are not.
    Missing,
    /// A folder is there but its marker does not prove it is this download's.
    Unverified,
}

/// A work folder found in a known output folder whose marker names a job.
#[derive(Debug)]
pub struct Found {
    pub job_id: String,
    pub reference: WorkspaceRef,
}

/// The most work folders looked at in one output folder.
pub const MAX_WORK_FOLDERS: usize = 512;

/// Lists the work folders directly inside `<output_root>/.claws-work`: no other folder of
/// the output root is entered. `Ok(vec![])` when there is none. Blocking.
pub fn work_folders(output_root: &Path, limit: usize) -> io::Result<Vec<Found>> {
    let work = output_root.join(WORK_DIR);
    match fs::symlink_metadata(&work) {
        Ok(meta) if meta.is_dir() && !is_link(&meta) => {}
        Ok(_) => return Ok(Vec::new()), // a link or a file under that name is nothing of ours
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    }
    let mut found = Vec::new();
    // Bounded in the entries looked at, not only in those that match.
    for entry in fs::read_dir(&work)?.flatten().take(limit.saturating_mul(2)) {
        if found.len() >= limit {
            break;
        }
        let path = entry.path();
        if plain_dir(&path).is_err() {
            continue;
        }
        let Ok(manifest) = read_manifest(&path) else { continue };
        found.push(Found { job_id: manifest.job_id, reference: WorkspaceRef { schema_version: WORKSPACE_REF_SCHEMA, path, attempt: manifest.attempt } });
    }
    Ok(found)
}

/// Resolves `kept` through a verified work folder. Blocking.
pub fn locate_kept(kept: &KeptFiles, job_id: &str) -> KeptState {
    let path = &kept.workspace.path;
    let output_root = path.parent().and_then(Path::parent);
    if output_root.is_none_or(|root| !root.is_dir()) {
        return KeptState::Offline;
    }
    if !path.exists() {
        return KeptState::Missing;
    }
    let Ok(manifest) = verify(&kept.workspace, job_id) else {
        return KeptState::Unverified;
    };
    // Only what the marker itself records as withheld, whatever the saved list says.
    let found: Vec<PathBuf> = withheld_sources(&manifest)
        .into_iter()
        .filter(|(relative, _)| kept.sources.contains(relative))
        .map(|(_, path)| path)
        .collect();
    if found.is_empty() {
        KeptState::Missing
    } else {
        KeptState::Available(found)
    }
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
    require_bound(manifest)?;
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
        bound: true,
    };
    write_manifest(&manifest)?;
    Ok((reference, manifest))
}

/// Proves `reference` is this job's work folder and returns its manifest, bound to it: the
/// marker's `workspace` and `destination_root` must be the very folders the reference
/// implies (the work folder, and the output folder two levels above it), compared as real
/// paths, and every output and recorded move must stay a plain relative path below them.
/// A copied or edited marker is refused, never followed. Blocking (it canonicalizes).
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
    let mut manifest = read_manifest(path)?;
    let supported = (1..=MANIFEST_SCHEMA).contains(&manifest.schema_version);
    if manifest.job_id != job_id || manifest.attempt != reference.attempt || !supported {
        return Err(io::Error::other(format!("{} belongs to another download", path.display())));
    }
    let output_root = parent.parent().ok_or_else(|| io::Error::other("work folder has no output folder"))?;
    let same_folder = |claimed: &Path, expected: &Path| -> io::Result<bool> { Ok(fs::canonicalize(claimed)? == fs::canonicalize(expected)?) };
    if !same_folder(&manifest.workspace, path)? || !same_folder(&manifest.destination_root, output_root)? {
        return Err(io::Error::other(format!("{} names folders that are not this download's", marker_path(path).display())));
    }
    for relative in &manifest.outputs {
        destination(&manifest.destination_root, Path::new(relative))?;
    }
    for entry in &manifest.moves {
        if entry.destination != destination(&manifest.destination_root, Path::new(&entry.relative))? {
            return Err(io::Error::other(format!("{} records a file outside the output folder", marker_path(path).display())));
        }
    }
    // Proven equal as real paths; from here on use the folders the reference implies, not
    // the marker's spelling of them (a link could be retargeted after this check).
    manifest.workspace = path.clone();
    manifest.destination_root = output_root.to_path_buf();
    for entry in &mut manifest.moves {
        entry.destination = destination(&manifest.destination_root, Path::new(&entry.relative))?;
    }
    manifest.bound = true;
    Ok(manifest)
}

/// Fails if an existing folder on the way from `root` to the parent of `relative` is a
/// link or junction, which could send a move somewhere else.
fn plain_below(root: &Path, relative: &Path) -> io::Result<()> {
    let mut current = root.to_path_buf();
    for part in relative.parent().unwrap_or(Path::new("")).components() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.is_dir() && !is_link(&meta) => {}
            Ok(_) => return Err(io::Error::other(format!("{} is not a plain folder", current.display()))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => break,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn require_bound(manifest: &Manifest) -> io::Result<()> {
    if manifest.bound {
        Ok(())
    } else {
        Err(io::Error::other("the download's marker was not verified against its folders"))
    }
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
    /// The part of `skipped` that is recorded in the marker as withheld, so a Library record
    /// can point at it. The rest has no way back to it.
    pub kept: Vec<PathBuf>,
}

/// Publishes the manifest's outputs. Each move's intent and file identity are saved before
/// the move and its result after, so an interrupted publication resumes where it stopped
/// and a move whose result was lost is recognised by identity, not by name. The media file
/// must publish; a side file whose name is taken is `Withheld`. `reserve` claims a
/// destination against other jobs and returns false when another job holds it.
pub fn publish(manifest: &mut Manifest, reserve: &mut dyn FnMut(&Path) -> bool) -> io::Result<Published> {
    require_bound(manifest)?;
    if manifest.state == Publication::Ready {
        manifest.state = Publication::Publishing;
        write_manifest(manifest)?;
    }
    let mut skipped = Vec::new();
    let mut kept = Vec::new();
    for (index, relative) in manifest.outputs.clone().iter().enumerate() {
        let media = index == 0;
        let target = destination(&manifest.destination_root, Path::new(relative))?;
        let source = manifest.workspace.join(relative);
        if manifest.is_done(relative, &target) {
            // Published sidecars now belong to the user: moving or editing one must
            // not block recording an otherwise valid video in the Library.
            if media {
                manifest.validate_done(relative, &target)?;
            }
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
        // A link or junction between the roots and the file would send the move elsewhere.
        if let Err(e) = plain_below(&manifest.destination_root, Path::new(relative)).and_then(|()| plain_below(&manifest.workspace, Path::new(relative))) {
            if media {
                return Err(e);
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
                kept.push(target.clone());
                skipped.push(target);
            }
        }
    }
    // Every move was validated or performed above; no re-check here, which would hash a
    // freshly moved file again on FAT/exFAT.
    let media = manifest.media().ok_or_else(|| io::Error::other("nothing to publish"))?;
    manifest.state = Publication::Published;
    write_manifest(manifest)?;
    Ok(Published { media, skipped, kept })
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
        // A schema-1 marker, written before attempts existed.
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

    /// The marker as the next start finds it: read from disk and verified against its folders.
    fn reopen(manifest: &Manifest) -> Manifest {
        verify(&reference_of(manifest), &manifest.job_id).unwrap()
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
    #[test]
    fn a_done_move_with_a_missing_destination_is_not_success() {
        let (out, mut manifest) = ready("done-missing", false);
        publish(&mut manifest, &mut |_| true).unwrap();
        fs::remove_file(out.join("clip.mp4")).unwrap();
        manifest.state = Publication::Publishing;
        assert!(publish(&mut manifest, &mut |_| true).is_err());
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn a_done_move_with_a_replaced_destination_is_not_success() {
        let (out, mut manifest) = ready("done-replaced", false);
        publish(&mut manifest, &mut |_| true).unwrap();
        fs::rename(out.join("clip.mp4"), out.join("original.mp4")).unwrap();
        fs::write(out.join("clip.mp4"), b"someone else's file").unwrap();
        manifest.state = Publication::Publishing;
        assert!(publish(&mut manifest, &mut |_| true).is_err());
        assert_eq!(fs::read(out.join("clip.mp4")).unwrap(), b"someone else's file");
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn missing_published_subtitle_does_not_block_media_retry() {
        let (out, mut manifest) = ready("missing-sidecar", true);
        publish(&mut manifest, &mut |_| true).unwrap();
        fs::rename(out.join("clip.en.srt"), out.join("renamed.srt")).unwrap();
        let result = publish(&mut manifest, &mut |_| true).unwrap();
        assert!(result.media.is_file());
        assert_eq!(fs::read(out.join("renamed.srt")).unwrap(), b"subs");
        let _ = fs::remove_dir_all(out);
    }


    /// Rewrites fields of a workspace's marker the way a copied or edited marker would.
    fn alter_marker(workspace: &Path, change: impl FnOnce(&mut serde_json::Value)) {
        let mut marker: serde_json::Value = serde_json::from_slice(&fs::read(marker_path(workspace)).unwrap()).unwrap();
        change(&mut marker);
        fs::write(marker_path(workspace), serde_json::to_vec(&marker).unwrap()).unwrap();
    }

    fn reference_of(manifest: &Manifest) -> WorkspaceRef {
        WorkspaceRef { schema_version: WORKSPACE_REF_SCHEMA, path: manifest.workspace.clone(), attempt: manifest.attempt }
    }

    /// Verifies the workspace and tries to publish with whatever the marker now says.
    fn verify_then_publish(manifest: &Manifest) -> io::Result<()> {
        verify(&reference_of(manifest), &manifest.job_id).and_then(|mut m| publish(&mut m, &mut |_| true).map(|_| ()))
    }

    #[test]
    fn a_marker_naming_another_destination_root_publishes_nothing_outside() {
        let (out, manifest) = ready("bind-destination", false);
        let outside = temp_dir("bind-outside");
        let sentinel = outside.join("sentinel.txt");
        fs::write(&sentinel, b"keep").unwrap();
        alter_marker(&manifest.workspace, |m| m["destinationRoot"] = serde_json::to_value(&outside).unwrap());
        assert!(verify_then_publish(&manifest).is_err());
        assert_eq!(names(&outside), vec!["sentinel.txt"], "something was published outside the owned roots");
        assert_eq!(fs::read(&sentinel).unwrap(), b"keep");
        assert!(manifest.workspace.join("clip.mp4").is_file(), "the finished file must stay in the work folder");
        let _ = fs::remove_dir_all(out);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn a_marker_naming_another_workspace_is_refused_and_nothing_moves() {
        let (out, manifest) = ready("bind-workspace", false);
        // Another job's folder, holding a file the altered marker would publish.
        let (other, _) = open_workspace(&out, &id(), 0).unwrap();
        fs::write(other.path.join("clip.mp4"), b"theirs").unwrap();
        alter_marker(&manifest.workspace, |m| m["workspace"] = serde_json::to_value(&other.path).unwrap());
        assert!(verify_then_publish(&manifest).is_err());
        assert_eq!(fs::read(other.path.join("clip.mp4")).unwrap(), b"theirs", "another job's file was moved");
        assert!(!out.join("clip.mp4").exists());
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn altered_output_paths_are_refused_before_anything_moves() {
        for bad in ["../../victim.mp4", "..\\..\\victim.mp4", "sub/../../victim.mp4", "C:\\Windows\\victim.mp4", "/victim.mp4", ""] {
            let (out, manifest) = ready("bind-output", false);
            fs::write(out.join("victim.mp4"), b"keep").unwrap();
            alter_marker(&manifest.workspace, |m| m["outputs"] = serde_json::json!(["clip.mp4", bad]));
            assert!(verify_then_publish(&manifest).is_err(), "{bad:?}");
            assert_eq!(fs::read(out.join("victim.mp4")).unwrap(), b"keep", "{bad:?}");
            assert!(manifest.workspace.join("clip.mp4").is_file(), "{bad:?} moved the media anyway");
            let _ = fs::remove_dir_all(out);
        }
    }

    #[test]
    fn a_recorded_move_cannot_name_a_destination_outside_the_owned_roots() {
        let (out, mut manifest) = ready("bind-move", false);
        publish(&mut manifest, &mut |_| true).unwrap();
        let outside = temp_dir("bind-move-outside");
        let victim = outside.join("clip.mp4");
        fs::write(&victim, b"unrelated").unwrap();
        alter_marker(&manifest.workspace, |m| m["moves"][0]["destination"] = serde_json::to_value(&victim).unwrap());
        assert!(verify(&reference_of(&manifest), &manifest.job_id).is_err(), "a move journal pointing outside was trusted");
        assert_eq!(fs::read(&victim).unwrap(), b"unrelated");
        let _ = fs::remove_dir_all(out);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn unsupported_marker_schemas_are_refused() {
        for schema in [0u64, 3, 99] {
            let (out, manifest) = ready("bind-schema", false);
            alter_marker(&manifest.workspace, |m| m["schemaVersion"] = schema.into());
            assert!(verify(&reference_of(&manifest), &manifest.job_id).is_err(), "schema {schema}");
            let _ = fs::remove_dir_all(out);
        }
    }

    #[test]
    fn a_manifest_that_was_not_verified_cannot_be_written_or_published() {
        let (out, manifest) = ready("bind-unverified", false);
        // Read straight from disk: nothing has proven its roots.
        let mut raw = read_manifest(&manifest.workspace).unwrap();
        assert!(write_manifest(&raw).is_err());
        assert!(publish(&mut raw, &mut |_| true).is_err());
        assert!(manifest.workspace.join("clip.mp4").is_file());
        let _ = fs::remove_dir_all(out);
    }

    #[cfg(windows)]
    fn junction(link: &Path, target: &Path) -> bool {
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J", &link.to_string_lossy(), &target.to_string_lossy()])
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_inside_the_destination_cannot_redirect_publication() {
        let (out, mut manifest) = ready("bind-junction", false);
        let outside = temp_dir("bind-junction-outside");
        if junction(&out.join("Channel"), &outside) {
            fs::create_dir_all(manifest.workspace.join("Channel")).unwrap();
            fs::write(manifest.workspace.join("Channel").join("clip.mp4"), b"media").unwrap();
            manifest.outputs = vec!["Channel/clip.mp4".into()];
            assert!(publish(&mut manifest, &mut |_| true).is_err());
            assert!(names(&outside).is_empty(), "published through a junction: {:?}", names(&outside));
        }
        let _ = fs::remove_dir_all(out);
        let _ = fs::remove_dir_all(outside);
    }

    #[cfg(windows)]
    #[test]
    fn a_work_folder_reached_through_a_junction_is_not_adopted() {
        let out = temp_dir("bind-link-ws");
        let real = temp_dir("bind-link-real");
        let job = id();
        let (reference, _) = open_workspace(&real, &job, 0).unwrap();
        fs::create_dir_all(out.join(WORK_DIR)).unwrap();
        // `<out>/.claws-work/<name>` is a junction to the real work folder.
        let link = workspace_path(&out, &job, 0);
        if junction(&link, &reference.path) {
            let forged = WorkspaceRef { schema_version: 1, path: link, attempt: 0 };
            assert!(verify(&forged, &job).is_err());
        }
        let _ = fs::remove_dir_all(out);
        let _ = fs::remove_dir_all(real);
    }

    #[test]
    fn an_output_folder_given_in_another_spelling_still_verifies() {
        let out = temp_dir("bind-spelling");
        let job = id();
        let (reference, manifest) = open_workspace(&out, &job, 0).unwrap();
        // The same folder spelled with a detour and mixed separators.
        let detour = out.join("sub").join("..");
        fs::create_dir_all(out.join("sub")).unwrap();
        alter_marker(&reference.path, |m| m["destinationRoot"] = serde_json::to_value(&detour).unwrap());
        let verified = verify(&reference, &job).expect("the same folder is still the same folder");
        assert_eq!(verified.workspace, manifest.workspace);
        assert_eq!(verified.destination_root, out, "publication must use the folder derived from the reference, not the marker's spelling");
        let _ = fs::remove_dir_all(out);
    }


    #[test]
    fn publication_evidence_holds_the_media_identity_and_only_done_sidecars() {
        let (out, mut manifest) = ready("evidence", true);
        fs::write(out.join("clip.en.srt"), b"user's own").unwrap(); // conflicts: withheld
        manifest.outputs.push("clip.es.srt".into());
        fs::write(manifest.workspace.join("clip.es.srt"), b"hola").unwrap();
        let media_identity = identify(&manifest.workspace.join("clip.mp4")).unwrap();
        publish(&mut manifest, &mut |_| true).unwrap();
        let found = evidence(&reference_of(&manifest), &manifest.job_id, &out.join("clip.mp4")).unwrap().expect("evidence for a published download");
        assert_eq!(found.identity, media_identity);
        let sidecars: Vec<_> = found.sidecars.iter().map(|m| m.destination.clone()).collect();
        assert_eq!(sidecars, vec![out.join("clip.es.srt")], "a withheld or taken side file is not ours");
        assert!(found.sidecars.iter().all(|m| m.state == MoveState::Done));
        // Another file than the published media gets no evidence.
        assert!(evidence(&reference_of(&manifest), &manifest.job_id, &out.join("other.mp4")).unwrap().is_none());
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn publication_evidence_is_refused_for_old_markers_and_unfinished_downloads() {
        // Schema 1: published paths only, no identity to prove ownership with.
        let (out, mut manifest) = ready("evidence-v1", false);
        move_no_replace(&manifest.workspace.join("clip.mp4"), &out.join("clip.mp4")).unwrap();
        manifest.state = Publication::Published;
        manifest.published = vec![out.join("clip.mp4")];
        write_manifest(&manifest).unwrap();
        assert!(evidence(&reference_of(&manifest), &manifest.job_id, &out.join("clip.mp4")).unwrap().is_none());
        let _ = fs::remove_dir_all(out);

        let (out, manifest) = ready("evidence-unfinished", false);
        assert!(evidence(&reference_of(&manifest), &manifest.job_id, &out.join("clip.mp4")).unwrap().is_none());
        let _ = fs::remove_dir_all(out);
    }

    #[cfg(windows)]
    mod evidence_reads {
        use super::*;
        use std::cell::{Cell, RefCell};
        use std::os::windows::fs::OpenOptionsExt;

        /// A published download, and its marker held by another program (nothing may share it).
        fn held_marker(tag: &str) -> (PathBuf, Manifest, RefCell<Option<fs::File>>) {
            let (out, mut manifest) = ready(tag, false);
            publish(&mut manifest, &mut |_| true).unwrap();
            let held = fs::OpenOptions::new().read(true).share_mode(0).open(marker_path(&manifest.workspace)).unwrap();
            (out, manifest, RefCell::new(Some(held)))
        }

        #[test]
        fn a_marker_held_for_a_moment_still_gives_its_evidence() {
            let (out, manifest, held) = held_marker("evidence-brief-lock");
            let identity = identify(&out.join("clip.mp4")).unwrap();
            let pauses = Cell::new(0);
            let found = evidence_pausing(&reference_of(&manifest), &manifest.job_id, &out.join("clip.mp4"), 5, || {
                pauses.set(pauses.get() + 1);
                held.borrow_mut().take(); // the other program lets go during the first wait
            });
            // Not a download without evidence: the proof was there, only unreadable for a moment.
            let found = found.expect("a briefly held marker is retried").expect("the marker proves the download");
            assert_eq!(found.identity, identity);
            assert_eq!(pauses.get(), 1);
            let _ = fs::remove_dir_all(out);
        }

        #[test]
        fn a_marker_that_stays_unreadable_is_an_error_not_a_download_without_evidence() {
            let (out, manifest, held) = held_marker("evidence-stuck-lock");
            let pauses = Cell::new(0);
            let outcome = evidence_pausing(&reference_of(&manifest), &manifest.job_id, &out.join("clip.mp4"), 4, || pauses.set(pauses.get() + 1));
            assert!(outcome.is_err(), "an unreadable marker became 'no evidence'");
            assert_eq!(pauses.get(), 3, "bounded: three waits between four attempts");
            held.borrow_mut().take();
            // Readable again, the same call proves the download.
            assert!(evidence(&reference_of(&manifest), &manifest.job_id, &out.join("clip.mp4")).unwrap().is_some());
            let _ = fs::remove_dir_all(out);
        }

        #[test]
        fn a_network_volume_that_blips_is_a_marker_that_cannot_be_read_not_one_that_is_missing() {
            // Windows reports these for a share that is unreachable or dropped mid-read; some
            // of them read as NotFound or Uncategorized, none says anything about ownership.
            for code in [21, 53, 59, 64, 67, 1167, 1224, 32, 33, 5] {
                assert!(is_transient(&io::Error::from_raw_os_error(code)), "os error {code} was taken for a verdict about the marker");
            }
            // A marker or folder that is really missing, and Claws' own verdicts, are not retried.
            for code in [2, 3, 267] {
                assert!(!is_transient(&io::Error::from_raw_os_error(code)), "os error {code} is a missing file or folder");
            }
            assert!(!is_transient(&io::Error::other("belongs to another download")));
            assert!(!is_transient(&io::Error::new(io::ErrorKind::InvalidData, "not a marker")));
        }

        #[test]
        fn a_missing_or_refused_marker_is_no_evidence_and_is_not_retried() {
            let (out, manifest) = ready("evidence-no-retry", false);
            let reference = reference_of(&manifest);
            let pauses = Cell::new(0);
            let mut count = || pauses.set(pauses.get() + 1);
            // No marker at all (a download of an old version).
            fs::remove_file(marker_path(&manifest.workspace)).unwrap();
            assert!(evidence_pausing(&reference, &manifest.job_id, &out.join("clip.mp4"), 5, &mut count).unwrap().is_none());
            // A marker that is not this download's.
            fs::write(marker_path(&manifest.workspace), b"{ not a marker").unwrap();
            assert!(evidence_pausing(&reference, &manifest.job_id, &out.join("clip.mp4"), 5, &mut count).unwrap().is_none());
            assert!(evidence_pausing(&reference, "another-job", &out.join("clip.mp4"), 5, &mut count).unwrap().is_none());
            assert_eq!(pauses.get(), 0, "a verdict about the marker was retried as if it were a lock");
            let _ = fs::remove_dir_all(out);
        }
    }


    /// A published download whose `clip.en.srt` was taken by the user's own file.
    fn withheld_set(tag: &str) -> (PathBuf, Manifest, WorkspaceRef) {
        let (out, mut manifest) = ready(tag, true);
        fs::write(out.join("clip.en.srt"), b"user's own").unwrap();
        publish(&mut manifest, &mut |_| true).unwrap();
        let reference = reference_of(&manifest);
        (out, manifest, reference)
    }

    #[test]
    fn kept_files_name_the_withheld_sources_in_the_work_folder_not_the_taken_destinations() {
        let (out, manifest, reference) = withheld_set("kept-sources");
        let kept = kept_files(&reference, &manifest.job_id, Some(&out.join("clip.mp4"))).expect("a withheld side file is kept");
        assert_eq!(kept.sources, ["clip.en.srt"]);
        assert_eq!(kept.workspace, reference);
        assert_eq!(fs::read(reference.path.join("clip.en.srt")).unwrap(), b"subs", "the retained copy");
        assert_eq!(fs::read(out.join("clip.en.srt")).unwrap(), b"user's own", "the user's file");

        // A download whose side files were all published keeps nothing.
        let (out2, manifest2) = ready("kept-none", true);
        let mut manifest2 = manifest2;
        publish(&mut manifest2, &mut |_| true).unwrap();
        assert!(kept_files(&reference_of(&manifest2), &manifest2.job_id, Some(&out2.join("clip.mp4"))).is_none());
        // Not for another media file, not for an unfinished download, not without the file.
        assert!(kept_files(&reference, &manifest.job_id, Some(&out.join("other.mp4"))).is_none());
        fs::remove_file(reference.path.join("clip.en.srt")).unwrap();
        assert!(kept_files(&reference, &manifest.job_id, Some(&out.join("clip.mp4"))).is_none(), "a file that is gone is not kept");
        let _ = fs::remove_dir_all(out);
        let _ = fs::remove_dir_all(out2);
    }

    #[test]
    fn kept_files_are_only_those_of_a_download_that_finished_publishing() {
        let (out, mut manifest, reference) = withheld_set("kept-unfinished");
        let media = out.join("clip.mp4");
        assert!(kept_files(&reference, &manifest.job_id, Some(&media)).is_some());
        for state in [Publication::Publishing, Publication::Ready] {
            manifest.state = state;
            write_manifest(&manifest).unwrap();
            assert!(kept_files(&reference, &manifest.job_id, Some(&media)).is_none(), "{state:?} is not finished");
        }
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn the_media_is_matched_by_path_the_way_windows_does() {
        let (out, manifest, reference) = withheld_set("kept-case");
        let spelled_differently = PathBuf::from(out.join("clip.mp4").to_string_lossy().to_uppercase().replace('\\', "/"));
        assert_ne!(spelled_differently, out.join("clip.mp4"));
        let kept = kept_files(&reference, &manifest.job_id, Some(&spelled_differently));
        assert_eq!(kept.map(|k| k.sources), Some(vec!["clip.en.srt".to_string()]), "the same file in another spelling");
        assert!(kept_files(&reference, &manifest.job_id, Some(&out.join("clip2.mp4"))).is_none(), "another file is still another file");
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn publishing_says_which_skipped_side_files_have_a_way_back() {
        let (out, mut manifest) = ready("kept-subset", true);
        // Taken by the user's own file: recorded as withheld, so it can be found again.
        fs::write(out.join("clip.en.srt"), b"user's own").unwrap();
        // Its source is gone: skipped, and nothing is left to keep.
        manifest.outputs.push("clip.es.srt".into());
        let done = publish(&mut manifest, &mut |_| true).unwrap();
        assert_eq!(done.skipped, vec![out.join("clip.en.srt"), out.join("clip.es.srt")]);
        assert_eq!(done.kept, vec![out.join("clip.en.srt")]);
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn kept_files_are_not_taken_from_a_marker_that_is_not_this_downloads() {
        let (out, manifest, reference) = withheld_set("kept-forged");
        alter_marker(&reference.path, |m| m["destinationRoot"] = serde_json::json!(std::env::temp_dir()));
        assert!(kept_files(&reference, &manifest.job_id, None).is_none());
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn kept_files_are_found_through_the_verified_folder_and_say_why_when_they_are_not() {
        let (out, manifest, reference) = withheld_set("kept-locate");
        let job = manifest.job_id.clone();
        let kept = kept_files(&reference, &job, None).unwrap();
        assert_eq!(locate_kept(&kept, &job), KeptState::Available(vec![reference.path.join("clip.en.srt")]));

        // A source the marker does not record as withheld is not offered, whatever the Library says.
        let wandering = KeptFiles { sources: vec!["clip.en.srt".into(), MARKER.into(), "../../clip.mp4".into(), "clip.mp4".into()], ..kept.clone() };
        assert_eq!(locate_kept(&wandering, &job), KeptState::Available(vec![reference.path.join("clip.en.srt")]));
        // Another job's id never opens this folder.
        assert_eq!(locate_kept(&kept, &id()), KeptState::Unverified);
        let mut elsewhere = kept.clone();
        elsewhere.workspace.path = out.join(WORK_DIR).join("not-a-job-folder");
        assert_eq!(locate_kept(&elsewhere, &job), KeptState::Missing, "the output folder is there; that folder is not");

        fs::remove_file(reference.path.join("clip.en.srt")).unwrap();
        assert_eq!(locate_kept(&kept, &job), KeptState::Missing, "the work folder is there, the file is not");
        remove_workspace(&reference, &job).unwrap();
        assert_eq!(locate_kept(&kept, &job), KeptState::Missing, "the work folder is gone");
        let offline = out.join("drive-that-went-away");
        let mut gone = kept.clone();
        gone.workspace.path = offline.join(WORK_DIR).join(folder_name(&job, 0));
        assert_eq!(locate_kept(&gone, &job), KeptState::Offline, "the output folder cannot be reached");
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn a_marker_naming_another_job_is_unverified_not_missing() {
        let (out, manifest, reference) = withheld_set("kept-unverified");
        let kept = kept_files(&reference, &manifest.job_id, None).unwrap();
        alter_marker(&reference.path, |m| m["jobId"] = serde_json::json!(id()));
        assert_eq!(locate_kept(&kept, &manifest.job_id), KeptState::Unverified);
        assert_eq!(fs::read(reference.path.join("clip.en.srt")).unwrap(), b"subs", "nothing was touched");
        let _ = fs::remove_dir_all(out);
    }

    #[test]
    fn the_work_folder_listing_enters_nothing_but_the_work_dir_and_is_bounded() {
        let out = temp_dir("listing");
        assert!(work_folders(&out, 10).unwrap().is_empty(), "no work dir is not an error");
        let jobs: Vec<String> = (0..4).map(|_| id()).collect();
        for job in &jobs {
            open_workspace(&out, job, 0).unwrap();
        }
        // Other folders of the output folder, and stray files in the work dir, are not visited.
        fs::create_dir_all(out.join("Music").join(WORK_DIR)).unwrap();
        fs::write(out.join(WORK_DIR).join("notes.txt"), b"x").unwrap();
        let found = work_folders(&out, 10).unwrap();
        let mut ids: Vec<&str> = found.iter().map(|f| f.job_id.as_str()).collect();
        ids.sort_unstable();
        let mut expected: Vec<&str> = jobs.iter().map(String::as_str).collect();
        expected.sort_unstable();
        assert_eq!(ids, expected);
        assert_eq!(work_folders(&out, 2).unwrap().len(), 2, "bounded");
        // A folder whose marker names another job than its own name does is still listed
        // (the caller verifies); one without a marker is not.
        fs::create_dir_all(out.join(WORK_DIR).join("deadbeef")).unwrap();
        assert_eq!(work_folders(&out, 10).unwrap().len(), 4);
        let _ = fs::remove_dir_all(out);
    }

    #[cfg(windows)]
    mod verified_deletion {
        use super::*;
        use std::os::windows::fs::OpenOptionsExt;

        fn file(tag: &str, bytes: &[u8]) -> (PathBuf, PathBuf, FileIdentity) {
            let dir = temp_dir(tag);
            let path = dir.join("clip.mp4");
            fs::write(&path, bytes).unwrap();
            let identity = identify(&path).unwrap();
            (dir, path, identity)
        }

        #[test]
        fn the_verified_file_is_deleted_and_a_missing_one_is_not_an_error() {
            let (dir, path, identity) = file("rm-ok", b"media");
            assert_eq!(remove_verified(&path, &identity).unwrap(), Removal::Deleted);
            assert!(!path.exists());
            assert_eq!(remove_verified(&path, &identity).unwrap(), Removal::Missing);
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn a_replacement_with_the_same_name_and_size_is_never_deleted() {
            let (dir, path, identity) = file("rm-replaced", b"media");
            fs::rename(&path, dir.join("original.mp4")).unwrap();
            fs::write(&path, b"MEDIA").unwrap(); // same name, same size, other file
            assert!(matches!(remove_verified(&path, &identity), Err(RemoveError::Replaced)));
            assert_eq!(fs::read(&path).unwrap(), b"MEDIA");
            // Even byte-identical content is another file when the identity is a file id.
            fs::write(dir.join("copy.mp4"), b"media").unwrap();
            fs::remove_file(&path).unwrap();
            fs::rename(dir.join("copy.mp4"), &path).unwrap();
            assert!(matches!(remove_verified(&path, &identity), Err(RemoveError::Replaced)));
            assert_eq!(fs::read(&path).unwrap(), b"media");
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn a_swap_between_verification_and_deletion_deletes_only_the_verified_object() {
            let (dir, path, identity) = file("rm-race", b"media");
            let moved = dir.join("moved.mp4");
            let (path_for_swap, moved_for_swap) = (path.clone(), moved.clone());
            let mut swap = move || {
                // The user moves the original away and a new file takes its place.
                fs::rename(&path_for_swap, &moved_for_swap).unwrap();
                fs::write(&path_for_swap, b"replacement").unwrap();
            };
            assert_eq!(remove_verified_with(&path, &identity, &mut swap).unwrap(), Removal::Deleted);
            assert_eq!(fs::read(&path).unwrap(), b"replacement", "the replacement was deleted as if it were the download");
            assert!(!moved.exists(), "the verified object was not the one deleted");
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn a_read_only_download_is_still_deleted() {
            let (dir, path, identity) = file("rm-readonly", b"media");
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&path, permissions).unwrap();
            let outcome = remove_verified(&path, &identity);
            if path.exists() {
                let mut permissions = fs::metadata(&path).unwrap().permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                permissions.set_readonly(false);
                fs::set_permissions(&path, permissions).unwrap();
            }
            assert_eq!(outcome.unwrap(), Removal::Deleted);
            assert!(!path.exists(), "a read-only download could not be deleted");
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn a_locked_file_is_reported_and_kept() {
            let (dir, path, identity) = file("rm-locked", b"media");
            let held = fs::OpenOptions::new().read(true).share_mode(0).open(&path).unwrap();
            assert!(matches!(remove_verified(&path, &identity), Err(RemoveError::Io(_))));
            drop(held);
            assert_eq!(fs::read(&path).unwrap(), b"media");
            assert_eq!(remove_verified(&path, &identity).unwrap(), Removal::Deleted);
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn an_identity_that_could_not_be_read_is_not_reported_as_a_file_in_use() {
            assert!(identity_verdict(Ok(true)).is_ok());
            assert!(matches!(identity_verdict(Ok(false)), Err(RemoveError::Replaced)));
            // The file opened (so it is not in use), but its file id or content could not be read.
            let failed = identity_verdict(Err(io::Error::other("the file id could not be read")));
            assert!(matches!(failed, Err(RemoveError::Unverifiable(_))), "{failed:?}");
            assert!(failed.unwrap_err().to_string().contains("identity could not be read"));
        }

        #[test]
        fn a_content_identity_is_hashed_from_the_open_file() {
            let (dir, path, _) = file("rm-content", b"media");
            let content = content_identity(&path).unwrap();
            fs::write(&path, b"other").unwrap();
            assert!(matches!(remove_verified(&path, &content), Err(RemoveError::Replaced)));
            fs::write(&path, b"media").unwrap();
            assert_eq!(remove_verified(&path, &content).unwrap(), Removal::Deleted);
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn a_folder_or_a_link_at_the_path_is_not_a_deletable_download() {
            let (dir, path, identity) = file("rm-folder", b"media");
            fs::remove_file(&path).unwrap();
            fs::create_dir(&path).unwrap();
            assert!(remove_verified(&path, &identity).is_err());
            assert!(path.is_dir());
            let _ = fs::remove_dir_all(dir);
        }
    }
}

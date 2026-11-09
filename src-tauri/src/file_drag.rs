//! Copy-only Windows file drag on a dedicated OLE apartment, never the window thread.
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{AppError, Result};
use crate::history::HistoryItem;
use crate::{AppHandle, WebviewWindow};

const MOVED: &str = "The file has been moved or deleted.";
const UNREADABLE: &str =
    "Windows cannot find the file to drag. It may have been moved or deleted, or its folder may be offline.";


#[derive(serde::Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DragOutcome {
    /// Released over another window. Whether that window copied it is its own business.
    Dropped,
    /// Escape, or the drag ended without a drop.
    Cancelled,
    /// Released over Claws itself: nothing was copied anywhere.
    DroppedOnSelf,
    /// The button was already up when the drag would have begun.
    NotStarted,
}

/// What a finished drag means, from the OLE result and whether Claws' own window
/// received the drop.
pub fn outcome(dropped: bool, on_claws: bool) -> DragOutcome {
    match (dropped, on_claws) {
        (true, false) => DragOutcome::Dropped,
        (true, true) => DragOutcome::DroppedOnSelf,
        (false, _) => DragOutcome::Cancelled,
    }
}

/// Set when Claws' own window receives dropped files.
static DROPPED_ON_CLAWS: AtomicBool = AtomicBool::new(false);

/// Claws' webview received dropped files (`WindowEvent::DragDrop(Drop)`, lib.rs). Claws
/// never opens or enqueues dropped files; this only tells a drag that began here that it
/// ended on Claws. wry posts the event from inside the drop, so the event loop delivers it
/// before any task posted after the drag returned.
pub fn note_window_drop() {
    DROPPED_ON_CLAWS.store(true, Ordering::SeqCst);
}

/// Forgets earlier drops just before a drag begins.
#[cfg_attr(not(windows), allow(dead_code))]
fn forget_window_drops() {
    DROPPED_ON_CLAWS.store(false, Ordering::SeqCst);
}

/// Whether Claws' window received a drop since `forget_window_drops`.
#[cfg_attr(not(windows), allow(dead_code))]
fn window_received_drop() -> bool {
    DROPPED_ON_CLAWS.load(Ordering::SeqCst)
}

/// The location the Library recorded for `id`. Reads the entry only (no file-system
/// access), so it is quick under the history lock.
pub fn recorded_location(items: &[HistoryItem], id: &str) -> Result<PathBuf> {
    let item = items
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| AppError::msg("That download is no longer in the Library."))?;
    item.file_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| AppError::msg("The file location was not recorded."))
}

#[cfg(test)]
pub fn recorded_file(items: &[HistoryItem], id: &str) -> Result<PathBuf> {
    draggable_file(&recorded_location(items, id)?)
}

/// A regular file (not a folder or a link) in the form Windows Shell accepts.
/// Blocking: a network location can take a while to answer.
pub fn draggable_file(path: &Path) -> Result<PathBuf> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| unreachable_file(&e))?;
    // A symbolic link or junction. Other reparse points, such as OneDrive's cloud files,
    // are ordinary files to the Shell and stay draggable.
    if metadata.file_type().is_symlink() {
        return Err(AppError::msg(
            "This Library entry points to a link, not to the downloaded file, so it cannot be dragged. Use Show in folder instead.",
        ));
    }
    if !metadata.is_file() {
        return Err(AppError::msg("Only downloaded files can be dragged, and this location is a folder."));
    }
    // The file was just found, so a failure here is the drive's, never a missing file.
    let shell_path = dunce::canonicalize(path).map_err(|e| {
        AppError::msg(format!(
            "Windows cannot give a draggable location for files on this drive ({e}). Use Show in folder and drag it from there."
        ))
    })?;
    let shell_path = shell_location(&shell_path)?;
    #[cfg(windows)]
    if !shell_can_parse(&shell_path) {
        return Err(AppError::msg(UNREADABLE));
    }
    Ok(shell_path)
}

/// Remove the filesystem-only prefix for Shell APIs, retaining UNC paths and long names.
fn shell_location(path: &Path) -> Result<PathBuf> {
    // Reserved/trailing-dot names must not resolve to a different ordinary file.
    for part in path.components() {
        if let Component::Normal(name) = part {
            let name = name.to_string_lossy();
            // Win32 strips spaces left before the extension, so "NUL .mp4" still names the device.
            let stem = name.split('.').next().unwrap_or("").trim_end_matches(' ').to_ascii_uppercase();
            // COM/LPT take one ASCII digit 1-9 or a superscript 1, 2 or 3 (U+00B9, U+00B2, U+00B3).
            let mut suffix = stem.strip_prefix("COM").or_else(|| stem.strip_prefix("LPT")).unwrap_or("").chars();
            let numbered = matches!((suffix.next(), suffix.next()), (Some('1'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'), None));
            let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered;
            if reserved || name.ends_with(['.', ' ']) {
                return Err(AppError::msg("This file uses a name Windows reserves. Use Show in folder instead."));
            }
        }
    }
    let text = path.as_os_str().to_string_lossy();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") { return Ok(PathBuf::from(format!(r"\\{unc}"))); }
    if let Some(local) = text.strip_prefix(r"\\?\") {
        if matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::VerbatimDisk(_))) {
            return Ok(PathBuf::from(local));
        }
        return Err(AppError::msg("This device path cannot be dragged. Use Show in folder instead."));
    }
    Ok(path.to_path_buf())
}

/// Only "not found" means moved or deleted. Anything else (an offline network drive, no
/// permission, a name the drive rejects) says what Windows reported.
fn unreachable_file(error: &std::io::Error) -> AppError {
    if error.kind() == std::io::ErrorKind::NotFound {
        AppError::msg(MOVED)
    } else {
        AppError::msg(format!(
            "Claws cannot reach the file to drag ({error}). Its drive or folder may be offline or not accessible."
        ))
    }
}

/// Fallibly check that Windows Shell can resolve a parsing name.
#[cfg(windows)]
pub fn shell_can_parse(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{SHParseDisplayName, ILFree};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        let mut pidl = std::ptr::null_mut();
        let result = SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut pidl, 0, None);
        if !pidl.is_null() { ILFree(Some(pidl)); }
        result.is_ok()
    }
}

/// Drags `path` (from `draggable_file`) out of `window` and reports how it ended.
#[cfg(all(windows, not(test)))]
pub async fn start(app: &AppHandle, _window: WebviewWindow, path: PathBuf) -> Result<DragOutcome> {
    // At most one OLE worker may exist, including a receiver call stuck past timeout.
    static DRAGGING: AtomicBool = AtomicBool::new(false);
    struct Busy;
    impl Drop for Busy {
        fn drop(&mut self) {
            DRAGGING.store(false, Ordering::SeqCst);
        }
    }
    if DRAGGING.swap(true, Ordering::SeqCst) {
        return Err(AppError::msg("A file drag is already in progress."));
    }
    let busy = Busy;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    // All OLE objects are created and destroyed on the dedicated STA thread.
    std::thread::Builder::new().name("claws-file-drag".into()).spawn(move || {
        // Ownership stays here even if the caller times out: never spawn an unbounded
        // number of stuck receiver calls. Do not terminate a thread owning COM objects.
        let _busy = busy;
        let _ = sender.send(native::drag(path));
    }).map_err(|e| AppError::msg(e.to_string()))?;
    let ended = tokio::time::timeout(std::time::Duration::from_secs(65), receiver).await
        .map_err(|_| AppError::msg("The receiving app has not finished the drag. Claws remains usable; check the receiver before trying again."))?
        .map_err(|_| AppError::msg("The drag was interrupted."))??;
    let Some(dropped) = ended else { return Ok(DragOutcome::NotStarted) };
    if !dropped {
        return Ok(DragOutcome::Cancelled);
    }
    // A drop on Claws reaches the event loop as a message wry posted during the drop. This
    // task is posted after the drag returned, so it runs after that message was handled.
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(window_received_drop());
    })?;
    let on_claws = receiver.await.map_err(|_| AppError::msg("The drag was interrupted."))?;
    Ok(outcome(dropped, on_claws))
}

#[cfg(any(not(windows), test))]
pub async fn start(_app: &AppHandle, _window: WebviewWindow, _path: PathBuf) -> Result<DragOutcome> {
    Err(AppError::msg("Native file dragging requires the Windows app."))
}

#[cfg(windows)]
mod native {
    use std::{path::PathBuf, time::{Duration, Instant}};
    use windows::core::{implement, BOOL, HRESULT, PCWSTR};
    use windows::Win32::{Foundation::{S_OK, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS}, System::{Com::IDataObject,
        Ole::{OleInitialize, OleUninitialize, DoDragDrop, IDropSource, IDropSource_Impl,
            DROPEFFECT, DROPEFFECT_COPY},
        SystemServices::{MODIFIERKEYS_FLAGS, MK_LBUTTON, MK_RBUTTON}},
        UI::{Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON},
            WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON},
            Shell::{SHParseDisplayName, SHCreateShellItemArrayFromIDLists, ILFree, BHID_DataObject}}};
    use crate::error::{AppError, Result};

    #[implement(IDropSource)]
    struct Source { started: Instant, primary: MODIFIERKEYS_FLAGS }
    impl IDropSource_Impl for Source_Impl {
        fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
            if escape.as_bool() || self.started.elapsed() > Duration::from_secs(60) { DRAGDROP_S_CANCEL }
            else if (keys & self.primary).0 == 0 { DRAGDROP_S_DROP } else { S_OK }
        }
        fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT { DRAGDROP_S_USEDEFAULTCURSORS }
    }

    /// Fallible Shell parsing: disappearing files never reach an unwrap.
    pub(super) unsafe fn data_object(path: &std::path::Path) -> windows::core::Result<IDataObject> {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut pidl = std::ptr::null_mut();
        SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut pidl, 0, None)?;
        let items = SHCreateShellItemArrayFromIDLists(&[pidl]);
        ILFree(Some(pidl));
        items?.BindToHandler(None, &BHID_DataObject)
    }

    pub fn drag(path: PathBuf) -> Result<Option<bool>> {
        unsafe {
            OleInitialize(None).map_err(|e| AppError::msg(e.to_string()))?;
            struct Apartment;
            impl Drop for Apartment { fn drop(&mut self) { unsafe { OleUninitialize(); } } }
            let _apartment = Apartment;
            let swapped = GetSystemMetrics(SM_SWAPBUTTON) != 0;
            let primary = if swapped { VK_RBUTTON } else { VK_LBUTTON };
            if GetAsyncKeyState(i32::from(primary.0)) >= 0 { return Ok(None); }
            let data = data_object(&path).map_err(|e| AppError::msg(format!("Windows cannot prepare this file for dragging ({e}). Use Show in folder instead.")))?;
            // Recheck after Shell preparation, which may hydrate a cloud/network file.
            if GetAsyncKeyState(i32::from(primary.0)) >= 0 { return Ok(None); }
            super::forget_window_drops();
            let source: IDropSource = Source { started: Instant::now(), primary: if swapped { MK_RBUTTON } else { MK_LBUTTON } }.into();
            let mut effect = DROPEFFECT::default();
            let status = DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect);
            if status == DRAGDROP_S_CANCEL { return Ok(Some(false)); }
            status.ok().map_err(|e| AppError::msg(format!("Windows reported a drag problem ({e}); check the receiver before retrying.")))?;
            Ok(Some(status == DRAGDROP_S_DROP && (effect & DROPEFFECT_COPY).0 != 0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, path: Option<&Path>) -> HistoryItem {
        serde_json::from_value(serde_json::json!({
            "id": id, "title": "Test", "url": "https://fixture.invalid/test",
            "uploader": null, "duration": null, "thumbnail": null, "thumbnailLocal": null,
            "filePath": path.map(|p| p.to_string_lossy().into_owned()), "fileSize": 5, "mode": "video",
            "quality": "Original", "finishedAt": 1
        }))
        .unwrap()
    }

    fn message(result: Result<PathBuf>) -> String {
        result.expect_err("refused").to_string()
    }

    fn is_verbatim(path: &Path) -> bool {
        path.as_os_str().to_string_lossy().starts_with(r"\\?\")
    }

    #[test]
    fn recorded_files_with_unicode_and_spaces_resolve_to_a_plain_shell_path() {
        let dir = crate::download_tests::temp_dir("drag-unicode");
        for name in ["日本語 video with spaces.mp4", "Ünïcödé — ✓ 🎬 clip.mkv", "  leading spaces.m4a"] {
            let file = dir.join(name);
            std::fs::write(&file, b"media").unwrap();
            let resolved = recorded_file(&[entry("saved", Some(&file))], "saved").unwrap();
            // Ordinary paths retain their canonical spelling.
            assert_eq!(resolved, dunce::canonicalize(&file).unwrap(), "{name}");
            assert!(!is_verbatim(&resolved), "{name}: {resolved:?}");
            #[cfg(windows)]
            assert!(shell_can_parse(&resolved), "{name}");
            assert_eq!(std::fs::read(&file).unwrap(), b"media", "resolving never touches the file");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unknown_ids_unrecorded_locations_folders_and_deleted_files_are_refused() {
        let dir = crate::download_tests::temp_dir("drag-refused");
        assert!(message(recorded_file(&[], "unrecorded")).contains("no longer in the Library"));
        let file = dir.join("kept.mp4");
        std::fs::write(&file, b"media").unwrap();
        let items = [entry("saved", Some(&file)), entry("unrecorded", None), entry("folder", Some(&dir))];
        assert!(message(recorded_file(&items, "other-id")).contains("no longer in the Library"));
        assert!(message(recorded_file(&items, "unrecorded")).contains("not recorded"));
        assert!(message(recorded_file(&items, "folder")).contains("Only downloaded files"));
        assert!(recorded_file(&items, "saved").is_ok());
        // Deleted (or moved) after it reached the Library.
        std::fs::remove_file(&file).unwrap();
        assert!(message(recorded_file(&items, "saved")).contains("moved or deleted"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Long paths need Shell parsing names rather than filesystem-only prefixes.
    #[test]
    fn long_locations_prepare_without_crashing() {
        let dir = crate::download_tests::temp_dir("drag-long");
        let deep = dir.join("a".repeat(60)).join("b".repeat(60)).join("c".repeat(60)).join("d".repeat(60));
        std::fs::create_dir_all(&deep).unwrap();
        let file = deep.join("日本語 long path.mp4");
        std::fs::write(&file, b"media").unwrap();
        assert!(file.as_os_str().len() > 260);
        let canonical = dunce::canonicalize(&file).unwrap();
        assert!(is_verbatim(&canonical), "dunce keeps the verbatim form: {canonical:?}");
        #[cfg(windows)]
        assert!(shell_can_parse(&shell_location(&canonical).unwrap()), "the Shell accepts the normal long path");
        let resolved = recorded_file(&[entry("long", Some(&file))], "long").unwrap();
        assert!(!is_verbatim(&resolved));
        assert_eq!(std::fs::read(&file).unwrap(), b"media");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Network sources retain UNC semantics after removing the filesystem prefix.
    /// Uses the loopback administrative share only when it is reachable.
    #[cfg(windows)]
    #[test]
    fn files_on_a_reachable_network_share_prepare_without_crashing() {
        let dir = crate::download_tests::temp_dir("drag-network");
        let file = dir.join("shared clip.mp4");
        std::fs::write(&file, b"media").unwrap();
        let local = file.to_string_lossy().into_owned();
        let Some(rest) = local.strip_prefix(r"C:\") else {
            eprintln!("skipped: the temporary folder is not on C:");
            return;
        };
        let unc = PathBuf::from(format!(r"\\localhost\C$\{rest}"));
        if std::fs::metadata(&unc).is_err() {
            eprintln!("skipped: \\\\localhost\\C$ is not reachable here");
            let _ = std::fs::remove_dir_all(dir);
            return;
        }
        let canonical = dunce::canonicalize(&unc).unwrap();
        assert!(canonical.as_os_str().to_string_lossy().starts_with(r"\\?\UNC\"), "{canonical:?}");
        assert!(shell_can_parse(&shell_location(&canonical).unwrap()));
        assert!(recorded_file(&[entry("shared", Some(&unc))], "shared").is_ok());
        assert_eq!(std::fs::read(&file).unwrap(), b"media");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn file_links_are_refused() {
        let dir = crate::download_tests::temp_dir("drag-link");
        let target = dir.join("target.mp4");
        std::fs::write(&target, b"media").unwrap();
        let link = dir.join("link.mp4");
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&target, &link);
        #[cfg(not(windows))]
        let made = std::os::unix::fs::symlink(&target, &link);
        match made {
            Ok(()) => assert!(message(recorded_file(&[entry("link", Some(&link))], "link")).contains("link")),
            // Creating symbolic links needs Developer Mode or an elevated token on Windows.
            Err(e) => eprintln!("skipped the symbolic-link case: {e}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn the_shell_parse_check_refuses_a_file_that_vanished() {
        let dir = crate::download_tests::temp_dir("drag-shell");
        let file = dir.join("gone.mp4");
        std::fs::write(&file, b"media").unwrap();
        let path = dunce::canonicalize(&file).unwrap();
        assert!(shell_can_parse(&path));
        std::fs::remove_file(&file).unwrap();
        assert!(!shell_can_parse(&path), "a vanished file must fail parsing");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_a_release_over_another_window_counts_as_dropped() {
        assert_eq!(outcome(true, false), DragOutcome::Dropped);
        assert_eq!(outcome(true, true), DragOutcome::DroppedOnSelf);
        assert_eq!(outcome(false, false), DragOutcome::Cancelled);
        assert_eq!(outcome(false, true), DragOutcome::Cancelled);
        let names: Vec<String> = [DragOutcome::Dropped, DragOutcome::Cancelled, DragOutcome::DroppedOnSelf, DragOutcome::NotStarted]
            .iter()
            .map(|o| serde_json::to_value(o).unwrap().as_str().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["dropped", "cancelled", "droppedOnSelf", "notStarted"]);
    }

    /// Self-drops come from Claws' own drop target, not from where the cursor ends up.
    #[test]
    fn a_drop_reported_by_claws_window_marks_the_drag_as_dropped_on_itself() {
        forget_window_drops();
        assert!(!window_received_drop());
        assert_eq!(outcome(true, window_received_drop()), DragOutcome::Dropped);
        note_window_drop();
        assert_eq!(outcome(true, window_received_drop()), DragOutcome::DroppedOnSelf);
        // The next drag starts clean.
        forget_window_drops();
        assert_eq!(outcome(true, window_received_drop()), DragOutcome::Dropped);
    }

    /// Only "not found" means moved or deleted; an offline drive or a refused name says so.
    #[test]
    fn only_a_missing_file_is_reported_as_moved_or_deleted() {
        use std::io::{Error, ErrorKind};
        assert_eq!(unreachable_file(&Error::from(ErrorKind::NotFound)).to_string(), MOVED);
        for kind in [ErrorKind::PermissionDenied, ErrorKind::TimedOut, ErrorKind::Other] {
            let text = unreachable_file(&Error::from(kind)).to_string();
            assert!(!text.contains("moved or deleted"), "{kind:?}: {text}");
            assert!(text.contains("offline or not accessible"), "{kind:?}: {text}");
        }
        // A name the file system rejects outright is not a missing file either.
        #[cfg(windows)]
        {
            let dir = crate::download_tests::temp_dir("drag-bad-name");
            let error = message(draggable_file(&dir.join("bad<name>.mp4")));
            assert!(!error.contains("moved or deleted"), "{error}");
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn shell_paths_keep_long_and_network_locations_without_filesystem_prefixes() {
        assert_eq!(shell_location(Path::new(r"\\?\UNC\server\share\file.mp4")).unwrap(), PathBuf::from(r"\\server\share\file.mp4"));
        assert_eq!(shell_location(Path::new(r"\\?\C:\media\file.mp4")).unwrap(), PathBuf::from(r"C:\media\file.mp4"));
        for path in [r"C:\media\NUL.mp4", r"C:\media\file.", r"C:\media\file "] {
            assert!(shell_location(Path::new(path)).is_err());
        }
    }

    #[test]
    fn reserved_com_and_lpt_names_include_superscript_digits_but_not_neighbors() {
        for name in ["COM1", "lpt9", "COM1.txt", "com9.MP4", "COM\u{b9}.txt", "com\u{b2}", "LPT\u{b3}", "lpt\u{b9}.mp4", "COM1 .txt", "NUL  .mp4", "com\u{b9} .mp4"] {
            let path = Path::new(r"C:\media").join(name);
            assert!(shell_location(&path).is_err(), "{name} is a reserved device name");
        }
        for name in ["COM0", "COM10", "LPT0.txt", "COM\u{b9}-title.mp4", "COMx", "COM", "LPT\u{b9}0", "COM\u{2074}", "xCOM1.mp4", "COM1 x.mp4", "COM 1.mp4"] {
            let path = Path::new(r"C:\media").join(name);
            assert_eq!(shell_location(&path).unwrap(), path, "{name} is an ordinary file name");
        }
        // A reserved component anywhere in the path is refused, and long/UNC forms still normalize.
        assert!(shell_location(&Path::new(r"C:\").join("COM\u{b2}").join("file.mp4")).is_err());
        assert!(shell_location(&Path::new(r"\\?\C:\media").join("LPT\u{b3}.txt")).is_err());
        assert_eq!(shell_location(Path::new(r"\\?\UNC\server\share\COM10.mp4")).unwrap(), PathBuf::from(r"\\server\share\COM10.mp4"));
    }

    #[cfg(windows)]
    #[test]
    fn native_data_object_is_fallible_for_a_disappearing_file() {
        let dir = crate::download_tests::temp_dir("drag-com");
        let deep = dir.join("a".repeat(60)).join("b".repeat(60)).join("c".repeat(60)).join("d".repeat(60));
        std::fs::create_dir_all(&deep).unwrap();
        unsafe {
            windows::Win32::System::Ole::OleInitialize(None).unwrap();
            for file in [dir.join("日本語 saved.mp4"), deep.join("日本語 long.mp4")] {
                std::fs::write(&file, b"media").unwrap();
                let path = shell_location(&dunce::canonicalize(&file).unwrap()).unwrap();
                let data = super::native::data_object(&path).unwrap();
                drop(data);
                std::fs::remove_file(&file).unwrap();
                assert!(super::native::data_object(&path).is_err());
            }
            windows::Win32::System::Ole::OleUninitialize();
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}

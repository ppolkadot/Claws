//! Native outgoing file drag (Windows), through the `drag` crate 2.1.1 (CrabNebula,
//! Apache-2.0 OR MIT, https://github.com/crabnebula-dev/drag-rs).
//!
//! The page sends a Library id. The path comes only from the saved Library entry and is
//! checked here; the drag offers a copy only, so no receiver can move or delete the file.
//!
//! What drag 2.1.1 does on Windows (read from its source, `platform_impl/windows/mod.rs`):
//! - `start_drag` runs OLE `DoDragDrop` synchronously on the calling thread, which must be
//!   the window's OLE apartment (the main thread). Its own `IDropSource` cancels on Escape
//!   and drops as soon as the left button is up, so a drag that begins after the button was
//!   released drops at once, wherever the cursor is.
//! - The data object is the Shell's own (`IShellItemArray` bound to `BHID_DataObject`),
//!   which carries CF_HDROP and the Shell ID list. `DragMode::Copy` offers DROPEFFECT_COPY
//!   only.
//! - Its callback runs before `start_drag` returns. It reports `Dropped` for any release
//!   (DRAGDROP_S_DROP) and discards the receiver's effect, so "dropped" means released over
//!   a window, never that a copy or an import happened. After `DoDragDrop` returns it still
//!   calls `GetCursorPos(..)?`, so an error can come back after a drop that happened.
//! - It canonicalizes again with dunce and `unwrap`s the Shell's parse of the result. A path
//!   dunce leaves verbatim (`\\?\`: over 260 characters, a reserved name, or any network
//!   share, `\\?\UNC\`) or a file that vanished parses to NULL and would panic, which aborts
//!   a release build. All of these are refused before the crate sees them.
//!
//! A drop on Claws itself is recognised from Claws' own drop target (wry reports it as
//! `WindowEvent::DragDrop(Drop)`, see `note_window_drop`), not from where the cursor is after
//! the drag: a receiver may show a dialog before `DoDragDrop` returns, and the cursor can be
//! anywhere by then.
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{AppError, Result};
use crate::history::HistoryItem;
use crate::{AppHandle, WebviewWindow};

const MOVED: &str = "The file has been moved or deleted.";
const UNREADABLE: &str =
    "Windows cannot find the file to drag. It may have been moved or deleted, or its folder may be offline.";
const NETWORK: &str =
    "Files on a network location cannot be dragged from Claws yet. Use Show in folder and drag it from there.";

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

/// What a finished drag means, from the crate's result and whether Claws' own window
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

/// Forgets earlier drops. Main thread, just before a drag begins.
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

/// A regular file (not a folder or a link), in exactly the form drag 2.1.1 will give the
/// Shell. Blocking: a network location can take a while to answer.
pub fn draggable_file(path: &Path) -> Result<PathBuf> {
    // Refused below in any case (the crate cannot drag them); refusing first avoids waiting
    // for an offline share to time out.
    if is_network_path(path) {
        return Err(AppError::msg(NETWORK));
    }
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
    // dunce strips `\\?\` only from short, ordinary drive paths, never from network ones.
    match shell_path.components().next() {
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::VerbatimUNC(..)) => {
            return Err(AppError::msg(NETWORK))
        }
        Some(Component::Prefix(prefix)) if prefix.kind().is_verbatim() => {
            return Err(AppError::msg(
                "The file's location is too long for Windows to drag (over 260 characters) or uses a name Windows reserves. Use Show in folder instead.",
            ))
        }
        _ => {}
    }
    #[cfg(windows)]
    if !shell_can_parse(&shell_path) {
        return Err(AppError::msg(UNREADABLE));
    }
    Ok(shell_path)
}

/// `\\server\share\…`, plain or verbatim. A mapped drive letter is caught after
/// canonicalization, which resolves it to `\\?\UNC\…`.
fn is_network_path(path: &Path) -> bool {
    matches!(
        path.components().next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..))
    )
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

/// Whether the Shell can turn `path` into an item ID list, the call drag 2.1.1 unwraps.
#[cfg(windows)]
pub fn shell_can_parse(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    // SAFETY: `wide` is NUL-terminated and outlives the call; the list is freed once.
    unsafe {
        let pidl = ILCreateFromPathW(PCWSTR(wide.as_ptr()));
        if pidl.is_null() {
            return false;
        }
        ILFree(Some(pidl));
    }
    true
}

/// Drags `path` (from `draggable_file`) out of `window` and reports how it ended.
#[cfg(all(windows, not(test)))]
pub async fn start(app: &AppHandle, window: WebviewWindow, path: PathBuf) -> Result<DragOutcome> {
    // The first drag holds the main thread until it ends; a second request is refused.
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
    let _busy = Busy;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    // Called from an async command (a tokio thread), this posts the task to the event loop,
    // so the drag's modal loop starts in a fresh turn rather than inside a WebView2 callback.
    // If the task never runs, the closure (and `sender`) is dropped and the receiver says so.
    app.run_on_main_thread(move || {
        let _ = sender.send(native::drag(&window, path));
    })?;
    let ended = receiver.await.map_err(|_| AppError::msg("The drag was interrupted."))??;
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

#[cfg(all(windows, not(test)))]
mod native {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};

    use super::{forget_window_drops, shell_can_parse, UNREADABLE};
    use crate::error::{AppError, Result};
    use crate::WebviewWindow;

    /// Main thread only. Blocks it (and so Claws' window) until the drag ends. `None`: the
    /// drag never began; otherwise whether it ended in a drop (anywhere, Claws included).
    pub fn drag(window: &WebviewWindow, path: PathBuf) -> Result<Option<bool>> {
        // SAFETY: plain Win32 queries without pointers into Rust memory.
        let primary = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 { VK_RBUTTON } else { VK_LBUTTON };
        // Released while the request travelled here: the crate would drop at once, wherever
        // the cursor is, so nothing starts. (GetAsyncKeyState reads the physical button.)
        if unsafe { GetAsyncKeyState(i32::from(primary.0)) } >= 0 {
            return Ok(None);
        }
        // Again here, on this thread, the moment before the crate unwraps the same parse.
        if !shell_can_parse(&path) {
            return Err(AppError::msg(UNREADABLE));
        }
        // Any drop Claws' window reports from now on belongs to this drag.
        forget_window_drops();
        let dropped = Arc::new(AtomicBool::new(false));
        let seen = dropped.clone();
        drag::start_drag(
            window,
            drag::DragItem::Files(vec![path]),
            drag::Image::Raw(include_bytes!("../icons/32x32.png").to_vec()),
            // Runs before start_drag returns.
            move |result, _| seen.store(matches!(result, drag::DragResult::Dropped), Ordering::SeqCst),
            drag::Options { mode: drag::DragMode::Copy, ..Default::default() },
        )
        // The crate can fail before the drag begins or after the drop (it reads the cursor
        // position last), so this must not claim that nothing happened.
        .map_err(|e| {
            AppError::msg(format!(
                "Windows reported a problem with the drag ({e}). If you released it over a folder or an app, check there before trying again."
            ))
        })?;
        Ok(Some(dropped.load(Ordering::SeqCst)))
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
            // Exactly what drag 2.1.1 hands the Shell after its own dunce canonicalization.
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

    /// drag 2.1.1 re-canonicalizes with dunce, which keeps `\\?\` past 260 characters; the
    /// Shell cannot parse that form and the crate `unwrap`s the result, aborting a release
    /// build. Such a location must be refused with a message instead.
    #[test]
    fn locations_too_long_for_the_shell_are_refused_instead_of_crashing() {
        let dir = crate::download_tests::temp_dir("drag-long");
        let deep = dir.join("a".repeat(60)).join("b".repeat(60)).join("c".repeat(60)).join("d".repeat(60));
        std::fs::create_dir_all(&deep).unwrap();
        let file = deep.join("日本語 long path.mp4");
        std::fs::write(&file, b"media").unwrap();
        assert!(file.as_os_str().len() > 260);
        let canonical = dunce::canonicalize(&file).unwrap();
        assert!(is_verbatim(&canonical), "dunce keeps the verbatim form: {canonical:?}");
        #[cfg(windows)]
        assert!(!shell_can_parse(&canonical), "the Shell cannot parse it, so the crate would panic");
        let error = message(recorded_file(&[entry("long", Some(&file))], "long"));
        assert!(error.contains("too long"), "{error}");
        assert_eq!(std::fs::read(&file).unwrap(), b"media");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// dunce never strips `\\?\UNC\`, and the Shell cannot parse that form, so drag 2.1.1
    /// would abort on any file stored on a network share. Uses this machine's loopback
    /// administrative share when it is reachable.
    #[cfg(windows)]
    #[test]
    fn files_on_a_network_share_are_refused_with_a_network_message() {
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
        assert!(!shell_can_parse(&canonical), "the crate would unwrap a NULL parse");
        let error = message(recorded_file(&[entry("shared", Some(&unc))], "shared"));
        assert!(error.contains("network"), "{error}");
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
        assert!(!shell_can_parse(&path), "a NULL parse is what the crate would unwrap");
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

    /// A share path is refused before any file-system access, so an offline server cannot
    /// hold the request for the network timeout.
    #[cfg(windows)]
    #[test]
    fn network_share_paths_are_refused_before_touching_the_network() {
        for path in [r"\\claws-offline.invalid\share\clip.mp4", r"\\?\UNC\claws-offline.invalid\share\clip.mp4"] {
            let started = std::time::Instant::now();
            let error = message(draggable_file(Path::new(path)));
            assert!(error.contains("network"), "{path}: {error}");
            assert!(started.elapsed() < std::time::Duration::from_millis(500), "{path} waited for the network");
        }
        assert!(!is_network_path(Path::new(r"C:\Users\clip.mp4")));
        assert!(!is_network_path(Path::new(r"\\?\C:\Users\clip.mp4")));
    }
}

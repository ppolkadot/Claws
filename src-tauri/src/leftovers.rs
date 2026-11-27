//! Files an earlier Claws left behind. Version 1.0.9e could install Valve's SteamCMD into its
//! tools folder, and SteamCMD kept its own copies of downloaded wallpapers there, which can be
//! gigabytes. Claws no longer uses it. Nothing is deleted on its own: Settings shows the folder
//! and its size, and only a confirmed press of **Remove…** deletes it.
//!
//! Only the folder Claws itself created (`<tools folder>\steamcmd`) is ever touched. A path the
//! user chose is never read here. The folder must be a plain directory, and one that holds any
//! link, junction or other special entry is left alone, so a removal can never reach outside it.
use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;
use tauri::State;

use crate::{
    download_files::is_link,
    error::{AppError, Result},
    state::AppState,
};

/// The folder an earlier Claws installed it into, inside the tools folder.
const FOLDER: &str = "steamcmd";
/// Safety bounds for the scan, far above a real SteamCMD folder.
const MAX_ENTRIES: usize = 500_000;
const MAX_DEPTH: usize = 64;

#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LegacyFiles {
    pub bytes: u64,
}

/// What a scan of the folder found.
#[derive(Debug, PartialEq, Eq)]
enum Scan {
    Plain { bytes: u64 },
    /// A link, junction or special file is inside: the folder is not removed.
    Linked,
    /// More entries than any real install has: the folder is not removed.
    TooLarge,
}

fn folder_in(bin: &Path) -> PathBuf {
    bin.join(FOLDER)
}

/// The size of everything under `folder`, or why it is not safe to remove.
fn scan(folder: &Path, depth: usize, seen: &mut usize) -> std::io::Result<Scan> {
    if depth > MAX_DEPTH {
        return Ok(Scan::TooLarge);
    }
    let mut bytes = 0u64;
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        *seen += 1;
        if *seen > MAX_ENTRIES {
            return Ok(Scan::TooLarge);
        }
        let meta = fs::symlink_metadata(entry.path())?;
        if is_link(&meta) {
            return Ok(Scan::Linked);
        }
        if meta.is_dir() {
            match scan(&entry.path(), depth + 1, seen)? {
                Scan::Plain { bytes: inner } => bytes = bytes.saturating_add(inner),
                other => return Ok(other),
            }
        } else if meta.is_file() {
            bytes = bytes.saturating_add(meta.len());
        } else {
            return Ok(Scan::Linked);
        }
    }
    Ok(Scan::Plain { bytes })
}

/// Hold the managed tools path in place and reject links in every component.
#[cfg(windows)]
fn hold_plain_directory(path: &Path) -> std::io::Result<Vec<fs::File>> {
    use std::os::windows::fs::OpenOptionsExt;
    let mut current = PathBuf::new();
    let mut held = Vec::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(component, std::path::Component::Prefix(_)) { continue; }
        if matches!(component, std::path::Component::ParentDir | std::path::Component::CurDir) {
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Unsafe tools path"));
        }
        let file = fs::OpenOptions::new().read(true).share_mode(3).custom_flags(0x0220_0000).open(&current)?;
        let meta = file.metadata()?;
        if !meta.is_dir() || is_link(&meta) { return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Linked tools path")); }
        held.push(file);
    }
    Ok(held)
}

#[cfg(not(windows))]
fn hold_plain_directory(path: &Path) -> std::io::Result<Vec<fs::File>> {
    let mut current = PathBuf::new();
    let mut held = Vec::new();
    for component in path.components() {
        current.push(component.as_os_str());
        let meta = fs::symlink_metadata(&current)?;
        if !meta.is_dir() || is_link(&meta) { return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Linked tools path")); }
        held.push(fs::File::open(&current)?);
    }
    Ok(held)
}

/// The leftover folder's size, or `None` when there is nothing (or nothing safe) to offer.
fn find(bin: &Path) -> Option<LegacyFiles> {
    let _held = hold_plain_directory(bin).ok()?;
    let folder = folder_in(bin);
    let meta = fs::symlink_metadata(&folder).ok()?;
    if !meta.is_dir() || is_link(&meta) {
        return None;
    }
    match scan(&folder, 0, &mut 0) {
        Ok(Scan::Plain { bytes }) => Some(LegacyFiles { bytes }),
        // Too odd to delete, but still there: the row appears so the user can see it.
        Ok(_) | Err(_) => Some(LegacyFiles { bytes: 0 }),
    }
}

/// Removes the folder after the same checks. Refuses a folder that holds a link.
fn remove(bin: &Path) -> Result<()> {
    let folder = folder_in(bin);
    let meta = match fs::symlink_metadata(&folder) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    if !meta.is_dir() || is_link(&meta) {
        return Err(AppError::msg("That folder is not a plain folder, so Claws left it alone."));
    }
    let _held = hold_plain_directory(bin).map_err(|_| AppError::msg("Claws' tools path contains a link or cannot be checked, so it was left alone."))?;
    // The tools folder must itself be where it should be, so a removal can only reach below it.
    let parent = fs::canonicalize(bin)?;
    let target = fs::canonicalize(&folder)?;
    if target.parent() != Some(parent.as_path()) {
        return Err(AppError::msg("That folder is not inside Claws' tools folder, so Claws left it alone."));
    }
    match scan(&folder, 0, &mut 0)? {
        Scan::Plain { .. } => {}
        Scan::Linked => {
            return Err(AppError::msg(
                "The folder holds a link to somewhere else, so Claws left it alone. Delete it yourself in File Explorer if you want it gone.",
            ))
        }
        Scan::TooLarge => return Err(AppError::msg("The folder is unusually large, so Claws left it alone.")),
    }
    fs::remove_dir_all(&folder)?;
    Ok(())
}

#[tauri::command]
pub async fn legacy_files(state: State<'_, AppState>) -> Result<Option<LegacyFiles>> {
    let bin = state.data_dir.join("bin");
    tauri::async_runtime::spawn_blocking(move || find(&bin))
        .await
        .map_err(|_| AppError::msg("Couldn't look at the tools folder."))
}

/// Deletes the leftover folder. The interface asks for confirmation first.
#[tauri::command]
pub async fn remove_legacy_files(state: State<'_, AppState>) -> Result<()> {
    let bin = state.data_dir.join("bin");
    tauri::async_runtime::spawn_blocking(move || remove(&bin))
        .await
        .map_err(|_| AppError::msg("Couldn't remove the files."))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bin(tag: &str) -> PathBuf {
        let dir = crate::download_tests::temp_dir(tag);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fixture(bin: &Path) {
        let folder = bin.join("steamcmd");
        fs::create_dir_all(folder.join("steamapps/workshop/content/431960/1")).unwrap();
        fs::write(folder.join("steamcmd.exe"), vec![1u8; 1000]).unwrap();
        fs::write(folder.join("steamapps/workshop/content/431960/1/video.mp4"), vec![2u8; 5000]).unwrap();
    }

    #[test]
    fn nothing_is_offered_when_the_folder_is_absent() {
        let dir = bin("leftover-none");
        assert_eq!(find(&dir), None);
        assert!(remove(&dir).is_ok(), "removing nothing is fine");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn measures_the_folder_and_removes_only_it() {
        let dir = bin("leftover-remove");
        fixture(&dir);
        fs::write(dir.join("ffmpeg.exe"), b"keep me").unwrap();
        assert_eq!(find(&dir), Some(LegacyFiles { bytes: 6000 }));
        remove(&dir).unwrap();
        assert!(!dir.join("steamcmd").exists());
        assert_eq!(fs::read(dir.join("ffmpeg.exe")).unwrap(), b"keep me");
        assert_eq!(find(&dir), None, "the row disappears once the folder is gone");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_named_like_the_folder_is_not_removed() {
        let dir = bin("leftover-file");
        fs::write(dir.join("steamcmd"), b"not a folder").unwrap();
        assert_eq!(find(&dir), None);
        assert!(remove(&dir).is_err());
        assert!(dir.join("steamcmd").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    fn make_junction(link: &Path, target: &Path) {
        use std::process::{Command, Stdio};
        let made = Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert!(made.status.success(), "creating a junction must succeed on a standard account: {}", String::from_utf8_lossy(&made.stderr));
    }

    #[cfg(windows)]
    #[test]
    fn a_link_at_the_tools_directory_is_refused() {
        let parent = bin("linked-tools-parent");
        let target = bin("linked-tools-target");
        fixture(&target);
        let link = parent.join("bin");
        make_junction(&link, &target);
        assert!(find(&link).is_none());
        assert!(remove(&link).is_err());
        assert!(target.join("steamcmd/steamcmd.exe").exists());
        fs::remove_dir(&link).unwrap();
        fs::remove_dir_all(parent).unwrap();
        fs::remove_dir_all(target).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_inside_the_folder_stops_the_removal_and_its_target_survives() {
        let dir = bin("leftover-junction");
        let target = bin("leftover-junction-target");
        fs::write(target.join("precious.txt"), b"mine").unwrap();
        fixture(&dir);
        let link = dir.join("steamcmd").join("linked");
        make_junction(&link, &target);
        assert!(remove(&dir).is_err(), "a folder holding a link is never removed");
        assert!(dir.join("steamcmd").join("steamcmd.exe").exists(), "nothing in it was deleted either");
        assert_eq!(fs::read(target.join("precious.txt")).unwrap(), b"mine");
        // Take the junction out ourselves; the target must still be there.
        fs::remove_dir(&link).unwrap();
        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(&target).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn a_linked_folder_itself_is_not_followed() {
        let dir = bin("leftover-linked-root");
        let target = bin("leftover-linked-root-target");
        fs::write(target.join("precious.txt"), b"mine").unwrap();
        let link = dir.join("steamcmd");
        make_junction(&link, &target);
        assert_eq!(find(&dir), None);
        assert!(remove(&dir).is_err());
        assert_eq!(fs::read(target.join("precious.txt")).unwrap(), b"mine");
        fs::remove_dir(&link).unwrap();
        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(&target).unwrap();
    }
}

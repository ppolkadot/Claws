//! Resolve a recorded, completed download; the frontend never supplies a path.
use std::path::{Path, PathBuf};
use crate::error::{AppError, Result};
use crate::history::HistoryItem;

pub fn recorded_file(items: &[HistoryItem], id: &str) -> Result<PathBuf> {
    let item = items.iter().find(|item| item.id == id)
        .ok_or_else(|| AppError::msg("That download is no longer in the Library."))?;
    regular_file(Path::new(item.file_path.as_deref().ok_or_else(|| AppError::msg("The file location was not recorded."))?))
}

fn regular_file(path: &Path) -> Result<PathBuf> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| AppError::msg("The file has been moved or deleted."))?;
    if !metadata.is_file() || is_link(&metadata) {
        return Err(AppError::msg("Only regular downloaded files can be dragged."));
    }
    std::fs::canonicalize(path).map_err(|e| AppError::msg(e.to_string()))
}

fn is_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)] { use std::os::windows::fs::MetadataExt; metadata.file_attributes() & 0x400 != 0 }
    #[cfg(not(windows))] { metadata.file_type().is_symlink() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_files_and_directories_cannot_be_dragged() {
        let dir = crate::download_tests::temp_dir("drag-validation");
        assert!(regular_file(&dir).is_err());
        assert!(regular_file(&dir.join("missing")).is_err());
        let file = dir.join("日本語 video with spaces.mp4");
        std::fs::write(&file, b"media").unwrap();
        assert_eq!(regular_file(&file).unwrap(), std::fs::canonicalize(&file).unwrap());
        assert_eq!(std::fs::read(&file).unwrap(), b"media");
        assert!(recorded_file(&[], "unrecorded").is_err());
        let item: HistoryItem = serde_json::from_value(serde_json::json!({
            "id": "saved", "title": "Test", "url": "https://fixture.invalid/test",
            "uploader": null, "duration": null, "thumbnail": null, "thumbnailLocal": null,
            "filePath": file.to_string_lossy(), "fileSize": 5, "mode": "video",
            "quality": "Original", "finishedAt": 1
        })).unwrap();
        assert!(recorded_file(std::slice::from_ref(&item), "other-id").is_err());
        assert_eq!(recorded_file(&[item], "saved").unwrap(), std::fs::canonicalize(&file).unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }
}

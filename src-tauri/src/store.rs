use std::fs;
use std::path::Path;

use serde::{de::DeserializeOwned, Serialize};

use crate::error::Result;

/// Reads a JSON file, falling back to the default when it is missing or corrupt.
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> T {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Writes JSON through a temp file so a crash never leaves a half-written file.
pub fn save<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

//! Free-space checks for large downloads.
//!
//! Before a transfer starts, the peak extra space the download will need is estimated from
//! its plan (selected stream sizes and the processing steps yt-dlp will run) and checked
//! against the free space on the volume holding the job's work folder, minus what other
//! running jobs have reserved there, plus a safety margin. When the shortfall is certain,
//! the job stops before the transfer and keeps what it already has. Unknown sizes are
//! treated as unknown, never as zero: those jobs are watched instead (every few seconds at
//! most), and stopped if the volume is about to run out.
//!
//! These are estimates. A merge or conversion can come out larger or smaller than the
//! inputs; the margin is there for that.

use std::io;
use std::path::Path;

const MIB: u64 = 1 << 20;
/// Smallest safety margin; it grows to 10% of large requirements.
const MIN_MARGIN: u64 = 512 * MIB;
/// While watching a job of unknown size, stop below this much free space.
pub const WATCH_FLOOR: u64 = 256 * MIB;

/// What the download will do with its streams after transferring them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Steps {
    /// Video and audio are merged into a new file (both inputs exist meanwhile).
    pub merge: bool,
    /// Metadata or subtitles are embedded: one more whole-file copy.
    pub rewrite: bool,
    /// Audio-only: the converted output, in bytes, when it can be estimated.
    pub audio_output: Option<u64>,
}

/// Peak bytes the job's files reach in its work folder, or `None` when a size is unknown.
pub fn peak_bytes(streams: &[Option<u64>], steps: Steps) -> Option<u64> {
    let total: u64 = streams.iter().copied().sum::<Option<u64>>()?;
    if let Some(output) = steps.audio_output {
        // The source stays until the converted file is complete.
        return Some(total + output);
    }
    // A merge writes the joined file while both streams still exist; later rewrites copy
    // the joined file once more. Either way the peak is about one extra copy.
    Some(if steps.merge || steps.rewrite { total * 2 } else { total })
}

/// Estimated output size of an audio conversion. Uncompressed and lossless formats are
/// sized from the duration; lossy targets are taken to be about the source's size.
pub fn audio_output(format: &str, source: Option<u64>, duration: Option<f64>) -> Option<u64> {
    let per_second = match format {
        "wav" => Some(48_000.0 * 2.0 * 2.0), // 48 kHz, 16-bit, stereo
        "flac" => Some(48_000.0 * 2.0 * 2.0 * 0.65),
        _ => None,
    };
    match per_second {
        Some(rate) => duration.filter(|d| *d > 0.0).map(|d| (d * rate) as u64),
        None => source,
    }
}

pub fn margin(additional: u64) -> u64 {
    MIN_MARGIN.max(additional / 10)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shortfall {
    /// Extra bytes needed, including the margin.
    pub required: u64,
    /// Free bytes minus other jobs' reservations.
    pub available: u64,
}

/// Whether `additional` bytes (plus margin) fit in `free` after `reserved_by_others`.
pub fn check(additional: u64, free: u64, reserved_by_others: u64) -> Result<(), Shortfall> {
    let required = additional + margin(additional);
    let available = free.saturating_sub(reserved_by_others);
    if required <= available {
        Ok(())
    } else {
        Err(Shortfall { required, available })
    }
}

/// Bytes already on disk in a folder (the job's partial and finished streams).
pub fn bytes_in(folder: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(folder) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => bytes_in(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

/// Free bytes available to this user on the volume holding `path`.
#[cfg(windows)]
pub fn free_bytes(path: &Path) -> io::Result<u64> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(&HSTRING::from(path.as_os_str()), Some(&mut free), None, None) }.map_err(|e| io::Error::other(e.message()))?;
    Ok(free)
}

#[cfg(not(windows))]
pub fn free_bytes(_path: &Path) -> io::Result<u64> {
    Err(io::Error::other("free space is only checked on Windows"))
}

/// A key naming the volume that holds `path` ("c:\", a mount point, or a share), so
/// jobs on the same volume share one budget.
#[cfg(windows)]
pub fn volume_key(path: &Path) -> String {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetVolumePathNameW;
    let mut buffer = [0u16; 1024];
    match unsafe { GetVolumePathNameW(&HSTRING::from(path.as_os_str()), &mut buffer) } {
        Ok(()) => {
            let len = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
            String::from_utf16_lossy(&buffer[..len]).to_lowercase()
        }
        Err(_) => path.components().next().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).unwrap_or_default(),
    }
}

#[cfg(not(windows))]
pub fn volume_key(path: &Path) -> String {
    path.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    #[test]
    fn video_needs_a_second_copy_only_when_something_rewrites_it() {
        let streams = [Some(4 * GIB), Some(GIB / 4)];
        assert_eq!(peak_bytes(&streams, Steps { merge: true, ..Steps::default() }), Some(2 * (4 * GIB + GIB / 4)));
        let single = [Some(4 * GIB)];
        assert_eq!(peak_bytes(&single, Steps::default()), Some(4 * GIB));
        assert_eq!(peak_bytes(&single, Steps { rewrite: true, ..Steps::default() }), Some(8 * GIB));
    }

    #[test]
    fn unknown_sizes_are_unknown_not_zero() {
        assert_eq!(peak_bytes(&[Some(GIB), None], Steps { merge: true, ..Steps::default() }), None);
        assert_eq!(audio_output("wav", Some(GIB), None), None);
    }

    #[test]
    fn audio_conversion_is_sized_by_format() {
        assert_eq!(audio_output("mp3", Some(100), Some(60.0)), Some(100));
        assert_eq!(audio_output("wav", Some(100), Some(60.0)), Some(60 * 192_000));
        let steps = Steps { audio_output: audio_output("wav", Some(50 * MIB), Some(3600.0)), ..Steps::default() };
        assert_eq!(peak_bytes(&[Some(50 * MIB)], steps), Some(50 * MIB + 3600 * 192_000));
    }

    #[test]
    fn the_margin_is_at_least_512_mib_and_grows_with_size() {
        assert_eq!(margin(0), 512 * MIB);
        assert_eq!(margin(100 * GIB), 10 * GIB);
    }

    #[test]
    fn zero_free_space_fails_with_the_numbers() {
        assert_eq!(check(GIB, 0, 0), Err(Shortfall { required: GIB + 512 * MIB, available: 0 }));
        assert!(check(GIB, 2 * GIB, 0).is_ok());
    }

    #[test]
    fn two_jobs_cannot_both_count_the_same_free_space() {
        let free = 10 * GIB;
        // The first job reserves 6 GiB (plus margin) of the 10 GiB.
        assert!(check(6 * GIB, free, 0).is_ok());
        let first = 6 * GIB + margin(6 * GIB);
        // On its own the second would fit; with the first's reservation it does not.
        assert!(check(6 * GIB, free, 0).is_ok());
        assert!(check(6 * GIB, free, first).is_err());
    }

    #[test]
    fn work_folder_bytes_include_subfolders() {
        let dir = crate::download_tests::temp_dir("budget");
        std::fs::create_dir_all(dir.join("Channel")).unwrap();
        std::fs::write(dir.join("a.part"), vec![0u8; 1000]).unwrap();
        std::fs::write(dir.join("Channel").join("b.part"), vec![0u8; 24]).unwrap();
        assert_eq!(bytes_in(&dir), 1024);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn the_real_volume_is_queried() {
        let dir = std::env::temp_dir();
        assert!(free_bytes(&dir).unwrap() > 0);
        let key = volume_key(&dir);
        assert!(key.ends_with('\\'), "{key}");
        // A deep path that does not exist yet on the same drive shares the volume.
        // A long path that does not exist yet still resolves to its volume.
        let deep = dir.join("claws-budget-not-created").join("a".repeat(200)).join("b");
        assert_eq!(volume_key(&deep), key);
        // An unavailable drive is an error, not zero.
        assert!(free_bytes(Path::new("Q:\\definitely\\missing")).is_err() || Path::new("Q:\\").exists());
    }
}

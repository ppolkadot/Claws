//! Free-space checks for large downloads.
//!
//! Before a transfer, the download's peak extra space is estimated from its plan (stream
//! sizes and processing steps) and checked against the free space on the work folder's
//! volume, minus other running jobs' reservations, plus a safety margin for estimate
//! error. Unknown sizes are never treated as zero: those jobs are watched instead and
//! stopped if the volume is about to run out.
//!
//! Reservations are of *remaining* demand (estimated peak minus bytes already on disk),
//! because free space already reflects written bytes. Space for a pending merge or rewrite
//! stays reserved until the job ends.

use std::collections::HashMap;
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

/// What one download does with the streams it selects, as far as disk space goes. Both the
/// admission check (downloader.rs `Budget`) and the size preview (estimate.rs) size a run
/// through this, so the preview shows exactly the peak admission would reserve.
///
/// Extension point: a later processing stage (a clipped range, an editing transcode) adds
/// its own term here, so admission and the preview keep agreeing.
#[derive(Clone, Copy, Debug)]
pub struct RunPlan<'a> {
    /// "video" or "audio".
    pub mode: &'a str,
    pub audio_format: &'a str,
    /// Metadata or subtitles are written into the finished file (one more whole copy).
    pub rewrite: bool,
    pub duration: Option<f64>,
}

impl RunPlan<'_> {
    fn converts_audio(&self) -> bool {
        self.mode == "audio" && self.audio_format != "best"
    }

    /// Size of the file that is kept, or `None` when any input is unknown.
    pub fn output(&self, streams: &[Option<u64>]) -> Option<u64> {
        let total: u64 = streams.iter().copied().sum::<Option<u64>>()?;
        if self.converts_audio() {
            audio_output(self.audio_format, Some(total), self.duration)
        } else {
            Some(total)
        }
    }

    /// Peak bytes in the work folder, or `None` when it cannot be sized. A conversion whose
    /// output cannot be sized is unknown, even when the source size is known.
    pub fn peak(&self, streams: &[Option<u64>]) -> Option<u64> {
        let audio_output = if self.converts_audio() { Some(self.output(streams)?) } else { None };
        peak_bytes(streams, Steps { merge: streams.len() > 1, rewrite: self.rewrite, audio_output })
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

/// Free bytes a download needs for `additional` bytes of demand: the demand and its safety
/// margin. Admission (`check`) and the size preview (estimate.rs) both use this, so the
/// free space the preview names is the free space admission asks for.
pub fn required(additional: u64) -> u64 {
    additional.saturating_add(margin(additional))
}

/// Whether `additional` bytes (plus margin) fit in `free` after `reserved_by_others`.
pub fn check(additional: u64, free: u64, reserved_by_others: u64) -> Result<(), Shortfall> {
    let required = required(additional);
    let available = free.saturating_sub(reserved_by_others);
    if required <= available {
        Ok(())
    } else {
        Err(Shortfall { required, available })
    }
}

/// One running job's claim on a volume.
#[derive(Clone, Debug, PartialEq)]
struct Reservation {
    volume: String,
    /// Estimated peak of the job's work folder.
    peak: u64,
    /// What the work folder held when last sampled.
    on_disk: u64,
}

impl Reservation {
    fn remaining(&self) -> u64 {
        self.peak.saturating_sub(self.on_disk)
    }

    /// What it keeps from other jobs: its remaining demand and that demand's margin.
    fn held(&self) -> u64 {
        let remaining = self.remaining();
        remaining + margin(remaining)
    }
}

/// The reservations of running jobs. Free space must be read while the ledger's lock is
/// held, so no admission counts space another job has claimed or subtracts written bytes
/// twice.
#[derive(Debug, Default)]
pub struct Ledger {
    jobs: HashMap<String, Reservation>,
}

impl Ledger {
    pub fn reserved_by_others(&self, job: &str, volume: &str) -> u64 {
        self.jobs.iter().filter(|(id, r)| id.as_str() != job && r.volume == volume).map(|(_, r)| r.held()).sum()
    }

    /// Admits a job that will grow its work folder to `peak` and already has `on_disk`
    /// there, if `free` (read under this lock) holds the rest after everyone else's.
    pub fn admit(&mut self, job: &str, volume: &str, peak: u64, on_disk: u64, free: u64) -> Result<(), Shortfall> {
        check(peak.saturating_sub(on_disk), free, self.reserved_by_others(job, volume))?;
        self.jobs.insert(job.to_string(), Reservation { volume: volume.to_string(), peak, on_disk });
        Ok(())
    }

    /// Bytes that reached the disk stop being reserved: free space now shows them.
    pub fn update(&mut self, job: &str, on_disk: u64) {
        if let Some(r) = self.jobs.get_mut(job) {
            r.on_disk = on_disk;
        }
    }

    /// Before a whole-file processing step: whether what the job still needs fits. A job
    /// without a reservation (unknown sizes) is watched instead and passes.
    pub fn recheck(&self, job: &str, free: u64) -> Result<(), Shortfall> {
        match self.jobs.get(job) {
            Some(r) => check(r.remaining(), free, self.reserved_by_others(job, &r.volume)),
            None => Ok(()),
        }
    }

    pub fn release(&mut self, job: &str) {
        self.jobs.remove(job);
    }

    #[cfg(test)]
    pub fn held(&self, job: &str) -> Option<u64> {
        self.jobs.get(job).map(Reservation::held)
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

    fn plan<'a>(mode: &'a str, audio_format: &'a str, rewrite: bool, duration: Option<f64>) -> RunPlan<'a> {
        RunPlan { mode, audio_format, rewrite, duration }
    }

    #[test]
    fn a_run_plan_gives_the_peak_admission_reserves() {
        // Merged video: one extra copy, whether or not something rewrites it afterwards.
        let streams = [Some(4 * GIB), Some(GIB / 4)];
        assert_eq!(plan("video", "mp3", false, None).peak(&streams), Some(2 * (4 * GIB + GIB / 4)));
        assert_eq!(plan("video", "mp3", true, None).peak(&streams), Some(2 * (4 * GIB + GIB / 4)));
        // A single combined format is copied only when metadata or subtitles go into it.
        assert_eq!(plan("video", "mp3", false, None).peak(&[Some(GIB)]), Some(GIB));
        assert_eq!(plan("video", "mp3", true, None).peak(&[Some(GIB)]), Some(2 * GIB));
        // Audio conversion keeps the source until the output is complete.
        assert_eq!(plan("audio", "mp3", true, Some(60.0)).peak(&[Some(100)]), Some(200));
        assert_eq!(plan("audio", "wav", false, Some(60.0)).peak(&[Some(100)]), Some(100 + 60 * 192_000));
        // "Original" is not converted.
        assert_eq!(plan("audio", "best", false, Some(60.0)).peak(&[Some(100)]), Some(100));
    }

    #[test]
    fn a_run_plan_never_turns_an_unknown_into_zero() {
        assert_eq!(plan("video", "mp3", false, None).peak(&[Some(GIB), None]), None);
        assert_eq!(plan("video", "mp3", false, None).output(&[Some(GIB), None]), None);
        // WAV and FLAC are sized from the duration; without one the peak is unknown, even
        // though the source size is known.
        assert_eq!(plan("audio", "wav", false, None).peak(&[Some(GIB)]), None);
        assert_eq!(plan("audio", "flac", false, None).output(&[Some(GIB)]), None);
        assert_eq!(plan("audio", "mp3", false, None).peak(&[]), Some(0), "no streams is a plan of nothing, not an unknown");
    }

    #[test]
    fn a_run_plan_names_the_size_of_what_is_kept() {
        assert_eq!(plan("video", "mp3", true, None).output(&[Some(300), Some(10)]), Some(310));
        assert_eq!(plan("audio", "mp3", false, Some(60.0)).output(&[Some(100)]), Some(100));
        assert_eq!(plan("audio", "wav", false, Some(60.0)).output(&[Some(100)]), Some(60 * 192_000));
        assert_eq!(plan("audio", "best", false, Some(60.0)).output(&[Some(100)]), Some(100));
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
        assert!(check(6 * GIB, free, 0).is_ok());
        let first = 6 * GIB + margin(6 * GIB);
        // On its own the second would fit; with the first's reservation it does not.
        assert!(check(6 * GIB, free, 0).is_ok());
        assert!(check(6 * GIB, free, first).is_err());
    }

    #[test]
    fn written_bytes_are_not_reserved_twice() {
        // A job reserving about 22 GiB (a 20 GiB peak) has written 10 GiB, and the volume
        // now shows 30 GiB free.
        let mut ledger = Ledger::default();
        assert!(ledger.admit("a", "d:\\", 20 * GIB, 0, 40 * GIB).is_ok());
        assert_eq!(ledger.held("a"), Some(22 * GIB));
        ledger.update("a", 10 * GIB);
        assert_eq!(ledger.held("a"), Some(10 * GIB + margin(10 * GIB)));
        // A second job needing 15 GiB fits in 30 - 11 GiB. Holding the original 22 GiB
        // would have left it 8 GiB.
        assert!(ledger.admit("b", "d:\\", 15 * GIB, 0, 30 * GIB).is_ok());
    }

    #[test]
    fn simultaneous_jobs_cannot_claim_the_same_space() {
        let mut ledger = Ledger::default();
        assert!(ledger.admit("a", "d:\\", 6 * GIB, 0, 10 * GIB).is_ok());
        // Nothing written yet: free space is unchanged and the first claim stands.
        let refused = ledger.admit("b", "d:\\", 6 * GIB, 0, 10 * GIB).unwrap_err();
        assert_eq!(refused.available, 10 * GIB - (6 * GIB + margin(6 * GIB)));
        // Another volume is a separate budget.
        assert!(ledger.admit("c", "e:\\", 6 * GIB, 0, 10 * GIB).is_ok());
    }

    #[test]
    fn resumed_bytes_only_need_the_rest() {
        let mut ledger = Ledger::default();
        // 6 GiB of a 10 GiB download is already on disk from before a pause.
        assert!(ledger.admit("a", "d:\\", 10 * GIB, 6 * GIB, 5 * GIB).is_ok());
        assert_eq!(ledger.held("a"), Some(4 * GIB + margin(4 * GIB)));
    }

    #[test]
    fn a_pending_merge_stays_reserved_until_the_job_ends() {
        let mut ledger = Ledger::default();
        let streams = 8 * GIB;
        let peak = peak_bytes(&[Some(streams)], Steps { merge: true, ..Steps::default() }).unwrap();
        assert!(ledger.admit("a", "d:\\", peak, 0, 30 * GIB).is_ok());
        // Transfer done: the merged copy is still to come.
        ledger.update("a", streams);
        assert_eq!(ledger.held("a"), Some(streams + margin(streams)));
        // Before merging, the merge must still fit; it does with 10 GiB free, not with 8.
        assert!(ledger.recheck("a", 10 * GIB).is_ok());
        assert!(ledger.recheck("a", 8 * GIB).is_err());
        // Ending (finished, paused, cancelled or failed) releases everything.
        ledger.release("a");
        assert_eq!(ledger.reserved_by_others("b", "d:\\"), 0);
        assert!(ledger.recheck("a", 0).is_ok(), "no reservation, nothing to recheck");
    }

    #[test]
    fn a_folder_larger_than_its_estimate_reserves_nothing_more() {
        let mut ledger = Ledger::default();
        assert!(ledger.admit("a", "d:\\", GIB, 0, 10 * GIB).is_ok());
        ledger.update("a", 2 * GIB);
        assert_eq!(ledger.held("a"), Some(margin(0)));
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
        // A long path that does not exist yet still resolves to its volume.
        let deep = dir.join("claws-budget-not-created").join("a".repeat(200)).join("b");
        assert_eq!(volume_key(&deep), key);
        // An unavailable drive is an error, not zero.
        assert!(free_bytes(Path::new("Q:\\definitely\\missing")).is_err() || Path::new("Q:\\").exists());
    }
}

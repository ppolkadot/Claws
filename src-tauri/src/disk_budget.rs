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
use std::sync::{Mutex, MutexGuard, PoisonError};

const MIB: u64 = 1 << 20;
/// Smallest safety margin; it grows to 10% of large requirements.
const MIN_MARGIN: u64 = 512 * MIB;
/// While watching a job of unknown size, stop below this much free space.
pub const WATCH_FLOOR: u64 = 256 * MIB;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Steps {
    /// Video and audio are merged into a new file (both inputs exist meanwhile).
    pub merge: bool,
    /// Metadata, subtitles or cover art are embedded: one more whole-file copy.
    pub rewrite: bool,
    /// Audio-only: the extracted or converted output, in bytes, when it can be estimated.
    pub audio_output: Option<u64>,
}

/// The sum of the stream sizes, or `None` when one is unknown (or the sum overflows).
pub fn known_total(streams: &[Option<u64>]) -> Option<u64> {
    streams.iter().try_fold(0u64, |sum, size| sum.checked_add((*size)?))
}

/// Peak bytes the job's files reach in its work folder, or `None` when a size is unknown,
/// the arithmetic overflows, or the result would be zero (an empty or zero-sized plan is
/// not a measurement, so it never becomes a zero-byte reservation).
///
/// Conservatively keeps source `S` alongside output `O`: `S + O`, or `S + 2 * O`
/// when a later metadata or cover-art pass rewrites the output. A failed source
/// deletion can leave both files on disk, and deletion order is not checked at
/// runtime, so the lower `max(S + O, 2 * O)` estimate is unsafe. Lossy output size
/// `O` is itself an assumption (see `audio_output`).
pub fn peak_bytes(streams: &[Option<u64>], steps: Steps) -> Option<u64> {
    let total = known_total(streams)?;
    let peak = match steps.audio_output {
        Some(0) => return None,
        Some(output) => total.checked_add(output.checked_mul(if steps.rewrite { 2 } else { 1 })?)?,
        // A merge writes the joined file while both streams still exist; later rewrites copy
        // the joined file once more. Either way the peak is about one extra copy.
        None if steps.merge || steps.rewrite => total.checked_mul(2)?,
        None => total,
    };
    (peak > 0).then_some(peak)
}

/// Estimated output size of an audio conversion. Uncompressed and lossless formats are
/// sized from the duration; lossy targets are taken to be about the source's size (an
/// assumption: a high-quality target made from a low-bitrate source can be larger, which
/// the safety margin only partly covers). `None` for an unusable duration or a zero size.
pub fn audio_output(format: &str, source: Option<u64>, duration: Option<f64>) -> Option<u64> {
    let per_second = match format {
        "wav" => Some(48_000.0 * 2.0 * 2.0), // 48 kHz, 16-bit, stereo
        "flac" => Some(48_000.0 * 2.0 * 2.0 * 0.65),
        _ => None,
    };
    match per_second {
        Some(rate) => {
            let bytes = duration.filter(|d| d.is_finite() && *d > 0.0)? * rate;
            (bytes >= 1.0 && bytes < u64::MAX as f64).then_some(bytes as u64)
        }
        None => source.filter(|bytes| *bytes > 0),
    }
}

/// What one download does with the streams it selects, as far as disk space goes. Both the
/// admission check (downloader.rs `Budget`) and the size preview (estimate.rs) size a run
/// through this, so the preview shows exactly the peak admission would reserve.
#[derive(Clone, Copy, Debug)]
pub struct RunPlan<'a> {
    /// "video" or "audio".
    pub mode: &'a str,
    pub audio_format: &'a str,
    /// A pass may write the finished file again (`ProcessingPolicy::rewrites_output`):
    /// metadata, embedded subtitles or cover art, each a temporary whole copy.
    pub rewrite: bool,
    pub duration: Option<f64>,
}

impl RunPlan<'_> {
    fn converts_audio(&self) -> bool {
        crate::download_policy::converts_audio(self.mode, self.audio_format)
    }

    /// Audio goes through yt-dlp's extraction (`-x`) for every target, "Original" included.
    fn extracts_audio(&self) -> bool {
        self.mode == "audio"
    }

    /// Size of the file that is kept, or `None` when any input is unknown, the sum
    /// overflows or the size would be zero. "Original" keeps the downloaded size: it is not
    /// converted, but extraction may still write it again (see `peak`).
    pub fn output(&self, streams: &[Option<u64>]) -> Option<u64> {
        let total = known_total(streams)?;
        if self.converts_audio() {
            audio_output(self.audio_format, Some(total), self.duration)
        } else {
            (total > 0).then_some(total)
        }
    }

    /// Peak bytes in the work folder, or `None` when it cannot be sized. A conversion whose
    /// output cannot be sized is unknown, even when the source size is known.
    ///
    /// Extraction is sized as a copy even for "Original": when the source container differs
    /// from the extracted one (webm Opus becomes .opus) yt-dlp writes a whole new file before
    /// it deletes the source, and Claws cannot tell in advance which case it is. This is an
    /// upper bound, as for the conversions.
    pub fn peak(&self, streams: &[Option<u64>]) -> Option<u64> {
        let audio_output = if self.extracts_audio() { Some(self.output(streams)?) } else { None };
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
        // Saturating: an overflowing claim keeps every other job out, it never wraps small.
        let remaining = self.remaining();
        remaining.saturating_add(margin(remaining))
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
        self.jobs.iter().filter(|(id, r)| id.as_str() != job && r.volume == volume).fold(0u64, |sum, (_, r)| sum.saturating_add(r.held()))
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

/// Takes the ledger's lock, also when a panic poisoned it. A sample that panics (a failing
/// free-space provider) does so before it changes anything, and every change the ledger
/// makes is one whole step, so a poisoned ledger is still sound; treating it as unusable
/// would make every later admission, refresh and release panic too. Blocks like any lock:
/// call it on a blocking thread, never an async worker.
pub fn lock(ledger: &Mutex<Ledger>) -> MutexGuard<'_, Ledger> {
    ledger.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The decision of an admission whose free space was read under the ledger's lock.
#[derive(Debug, PartialEq)]
pub enum Admission {
    Admitted,
    /// The volume could not be read, which is not evidence of a shortfall: watch instead.
    Unreadable,
    Refused(Shortfall),
}

/// Reads free space and decides admission as one step under the ledger's lock, so no
/// other admission or update lands between the reading and the decision.
///
/// This blocks for as long as the free-space provider does (a slow disk or network
/// volume), so callers on async workers run it in `spawn_blocking`, with owned arguments.
/// The remaining bound: the ledger is global, so while one sample is out every other
/// ledger access (another volume's admission included) waits for it. Reservations
/// stay correct, but they are not isolated per volume.
pub fn admit_sampled(
    ledger: &Mutex<Ledger>,
    free_space: impl Fn(&Path) -> io::Result<u64>,
    job: &str,
    volume: &str,
    workspace: &Path,
    peak: u64,
    on_disk: u64,
) -> Admission {
    let mut ledger = lock(ledger);
    let Ok(free) = free_space(workspace) else { return Admission::Unreadable };
    match ledger.admit(job, volume, peak, on_disk, free) {
        Ok(()) => Admission::Admitted,
        Err(short) => Admission::Refused(short),
    }
}

/// Refreshes the job's reservation from `on_disk`, then reads free space and rechecks
/// what it still needs, all under the ledger's lock (see `admit_sampled`, which has the
/// same blocking behavior and bound). `None` when it fits, when the job has no
/// reservation, or when the volume cannot be read.
pub fn recheck_sampled(
    ledger: &Mutex<Ledger>,
    free_space: impl Fn(&Path) -> io::Result<u64>,
    job: &str,
    workspace: &Path,
    on_disk: u64,
) -> Option<Shortfall> {
    let mut ledger = lock(ledger);
    ledger.update(job, on_disk);
    let free = free_space(workspace).ok()?;
    ledger.recheck(job, free).err()
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
        assert_eq!(plan("audio", "mp3", false, Some(60.0)).peak(&[Some(100)]), Some(200));
        // A later rewrite of the output adds a second copy (the documented upper bound).
        assert_eq!(plan("audio", "mp3", true, Some(60.0)).peak(&[Some(100)]), Some(300));
        assert_eq!(plan("audio", "wav", false, Some(60.0)).peak(&[Some(100)]), Some(100 + 60 * 192_000));
        // "Original" is still extracted (`-x`): when the container changes (webm Opus to .opus)
        // yt-dlp writes a whole copy while the source exists, so it is sized like a conversion
        // whose output is the source size.
        assert_eq!(plan("audio", "best", false, Some(60.0)).peak(&[Some(100)]), Some(200));
        assert_eq!(plan("audio", "best", true, Some(60.0)).peak(&[Some(100)]), Some(300));
        assert_eq!(plan("audio", "best", false, None).peak(&[Some(100)]), Some(200), "needs no duration");
    }

    #[test]
    fn a_run_plan_never_turns_an_unknown_into_zero() {
        assert_eq!(plan("video", "mp3", false, None).peak(&[Some(GIB), None]), None);
        assert_eq!(plan("video", "mp3", false, None).output(&[Some(GIB), None]), None);
        // WAV and FLAC are sized from the duration; without one the peak is unknown, even
        // though the source size is known.
        assert_eq!(plan("audio", "wav", false, None).peak(&[Some(GIB)]), None);
        assert_eq!(plan("audio", "flac", false, None).output(&[Some(GIB)]), None);
        assert_eq!(plan("audio", "mp3", false, None).peak(&[]), None, "no streams is no size, not a zero-byte reservation");
    }

    #[test]
    fn a_run_plan_names_the_size_of_what_is_kept() {
        assert_eq!(plan("video", "mp3", true, None).output(&[Some(300), Some(10)]), Some(310));
        assert_eq!(plan("audio", "mp3", false, Some(60.0)).output(&[Some(100)]), Some(100));
        assert_eq!(plan("audio", "wav", false, Some(60.0)).output(&[Some(100)]), Some(60 * 192_000));
        assert_eq!(plan("audio", "best", false, Some(60.0)).output(&[Some(100)]), Some(100));
    }

    #[test]
    fn expanded_audio_requires_space_for_its_rewrite() {
        // The converted output is larger than the source and then rewritten once more
        // (metadata or cover art): the rewrite's copy must fit even when the source is
        // not known to be gone by then.
        let steps = Steps { merge: false, rewrite: true, audio_output: Some(1000) };
        assert!(peak_bytes(&[Some(100)], steps).unwrap() >= 2000);
        // A rewrite always costs more than the same conversion without one.
        let plain = Steps { rewrite: false, ..steps };
        assert!(peak_bytes(&[Some(100)], steps).unwrap() > peak_bytes(&[Some(100)], plain).unwrap());
        // A source larger than the output (lossless to lossy) is covered as well.
        let shrunk = Steps { merge: false, rewrite: true, audio_output: Some(100) };
        assert!(peak_bytes(&[Some(1000)], shrunk).unwrap() >= 1100);
    }

    #[test]
    fn overflowing_sizes_stay_unknown() {
        assert_eq!(peak_bytes(&[Some(u64::MAX), Some(1)], Steps::default()), None);
        assert_eq!(peak_bytes(&[Some(u64::MAX)], Steps { rewrite: true, ..Steps::default() }), None);
        assert_eq!(peak_bytes(&[Some(u64::MAX / 2 + 1)], Steps { merge: true, ..Steps::default() }), None);
        let audio = |output, rewrite| Steps { merge: false, rewrite, audio_output: Some(output) };
        assert_eq!(peak_bytes(&[Some(u64::MAX)], audio(1, false)), None);
        assert_eq!(peak_bytes(&[Some(1)], audio(u64::MAX, false)), None);
        assert_eq!(peak_bytes(&[Some(1)], audio(u64::MAX / 2 + 1, true)), None);
        // The same inputs through the plan the preview and admission share.
        let huge = [Some(u64::MAX), Some(1)];
        assert_eq!(plan("video", "mp3", false, None).peak(&huge), None);
        assert_eq!(plan("video", "mp3", false, None).output(&huge), None);
        assert_eq!(plan("audio", "mp3", true, Some(60.0)).peak(&[Some(u64::MAX)]), None);
        assert_eq!(known_total(&[Some(u64::MAX), Some(1)]), None);
        assert_eq!(known_total(&[Some(2), Some(3)]), Some(5));
        assert_eq!(known_total(&[Some(2), None]), None);
    }

    #[test]
    fn nonfinite_and_unusable_durations_size_nothing() {
        for bad in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN, 0.0, -5.0, f64::MAX] {
            assert_eq!(audio_output("wav", Some(GIB), Some(bad)), None, "{bad}");
            assert_eq!(audio_output("flac", Some(GIB), Some(bad)), None, "{bad}");
            assert_eq!(plan("audio", "wav", true, Some(bad)).peak(&[Some(GIB)]), None, "{bad}");
        }
        // A lossy target is sized from the source, whatever the duration says.
        assert_eq!(audio_output("mp3", Some(100), Some(f64::INFINITY)), Some(100));
    }

    #[test]
    fn a_zero_size_is_never_a_zero_byte_reservation() {
        assert_eq!(peak_bytes(&[Some(0)], Steps::default()), None);
        assert_eq!(peak_bytes(&[Some(0), Some(0)], Steps { merge: true, ..Steps::default() }), None);
        assert_eq!(peak_bytes(&[], Steps::default()), None);
        assert_eq!(plan("video", "mp3", false, None).peak(&[Some(0)]), None);
        assert_eq!(plan("audio", "mp3", false, Some(60.0)).peak(&[Some(0)]), None);
        assert_eq!(plan("audio", "mp3", false, None).peak(&[]), None);
        // A duration too small to hold a sample is no size either.
        assert_eq!(audio_output("wav", Some(100), Some(1e-9)), None);
    }

    #[test]
    fn reservation_totals_saturate_instead_of_wrapping() {
        let mut ledger = Ledger::default();
        // A claim near the top of the range keeps every other job out; it never wraps small.
        ledger.jobs.insert("a".into(), Reservation { volume: "d:\\".into(), peak: u64::MAX, on_disk: 0 });
        assert_eq!(ledger.held("a"), Some(u64::MAX));
        ledger.jobs.insert("b".into(), Reservation { volume: "d:\\".into(), peak: u64::MAX, on_disk: 0 });
        assert_eq!(ledger.reserved_by_others("c", "d:\\"), u64::MAX);
        assert!(ledger.admit("c", "d:\\", GIB, 0, 100 * GIB).is_err(), "no capacity is left");
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

    /// A provider that takes a while and records how many samples overlapped.
    fn slow_provider(free: u64, overlap: &std::sync::atomic::AtomicUsize, peak_overlap: &std::sync::atomic::AtomicUsize) -> io::Result<u64> {
        use std::sync::atomic::Ordering::SeqCst;
        let now = overlap.fetch_add(1, SeqCst) + 1;
        peak_overlap.fetch_max(now, SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(80));
        overlap.fetch_sub(1, SeqCst);
        Ok(free)
    }

    fn admit_concurrently(volumes: [&str; 2], free: u64) -> (Vec<Admission>, usize) {
        use std::sync::atomic::AtomicUsize;
        let ledger = Mutex::new(Ledger::default());
        let (overlap, peak_overlap) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let gate = std::sync::Barrier::new(2);
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = ["a", "b"]
                .into_iter()
                .zip(volumes)
                .map(|(job, volume)| {
                    let (ledger, overlap, peak_overlap, gate) = (&ledger, &overlap, &peak_overlap, &gate);
                    scope.spawn(move || {
                        gate.wait();
                        let provider = |_: &Path| slow_provider(free, overlap, peak_overlap);
                        admit_sampled(ledger, provider, job, volume, Path::new("."), 6 * GIB, 0)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        (results, peak_overlap.load(std::sync::atomic::Ordering::SeqCst))
    }

    #[test]
    fn two_slow_admissions_on_one_volume_cannot_reserve_the_same_bytes() {
        // 10 GiB free holds one 6 GiB job and its margin, not two.
        let (results, overlap) = admit_concurrently(["d:\\", "d:\\"], 10 * GIB);
        let admitted = results.iter().filter(|r| **r == Admission::Admitted).count();
        let refused = results.iter().filter(|r| matches!(r, Admission::Refused(_))).count();
        assert_eq!((admitted, refused), (1, 1), "{results:?}");
        // Each reading and its decision are taken one at a time under the ledger's lock.
        assert_eq!(overlap, 1, "samples overlapped, so a decision could use a stale reading");
    }

    #[test]
    fn slow_admissions_on_different_volumes_each_get_their_own_budget() {
        let (results, overlap) = admit_concurrently(["d:\\", "e:\\"], 10 * GIB);
        assert_eq!(results, [Admission::Admitted, Admission::Admitted]);
        // The ledger is global: the second volume's sample still waited for the first's.
        assert_eq!(overlap, 1);
    }

    #[test]
    fn an_unreadable_volume_is_neither_admitted_nor_refused() {
        let ledger = Mutex::new(Ledger::default());
        let unreadable = |_: &Path| -> io::Result<u64> { Err(io::Error::other("offline")) };
        let result = admit_sampled(&ledger, unreadable, "a", "d:\\", Path::new("."), 6 * GIB, 0);
        assert_eq!(result, Admission::Unreadable);
        assert_eq!(ledger.lock().unwrap().held("a"), None, "an unreadable volume reserves nothing");
    }

    #[test]
    fn a_refused_admission_reserves_nothing() {
        let ledger = Mutex::new(Ledger::default());
        let result = admit_sampled(&ledger, |_: &Path| Ok(GIB), "a", "d:\\", Path::new("."), 6 * GIB, 0);
        assert!(matches!(result, Admission::Refused(_)));
        assert_eq!(ledger.lock().unwrap().held("a"), None);
    }

    #[test]
    fn a_sampled_recheck_refreshes_the_claim_then_judges_what_is_left() {
        let ledger = Mutex::new(Ledger::default());
        let free = |_: &Path| Ok(10 * GIB);
        assert_eq!(admit_sampled(&ledger, free, "a", "d:\\", Path::new("."), 6 * GIB, 0), Admission::Admitted);
        // 5 GiB landed, so 1 GiB (and its margin) is still needed and fits in 2 GiB.
        assert_eq!(recheck_sampled(&ledger, |_: &Path| Ok(2 * GIB), "a", Path::new("."), 5 * GIB), None);
        assert_eq!(ledger.lock().unwrap().held("a"), Some(GIB + margin(GIB)));
        // With nothing written, 6 GiB is still needed and 2 GiB cannot hold it.
        let short = recheck_sampled(&ledger, |_: &Path| Ok(2 * GIB), "a", Path::new("."), 0).expect("a shortfall");
        assert_eq!(short.available, 2 * GIB);
        // An unreadable volume still records the bytes, and is not a shortfall.
        let offline = |_: &Path| -> io::Result<u64> { Err(io::Error::other("offline")) };
        assert_eq!(recheck_sampled(&ledger, offline, "a", Path::new("."), 6 * GIB), None);
        assert_eq!(ledger.lock().unwrap().held("a"), Some(margin(0)));
        // A job without a reservation (unknown sizes) passes.
        assert_eq!(recheck_sampled(&ledger, |_: &Path| Ok(0), "unknown", Path::new("."), 0), None);
    }

    #[test]
    fn a_panicking_sample_does_not_make_every_later_admission_panic() {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        let ledger = Mutex::new(Ledger::default());
        let panicking = |_: &Path| -> io::Result<u64> { panic!("the free-space sample panicked") };
        // Admission and the processing recheck each panic under the ledger's lock, poisoning it.
        let first = catch_unwind(AssertUnwindSafe(|| admit_sampled(&ledger, panicking, "a", "d:\\", Path::new("."), GIB, 0)));
        assert!(first.is_err() && ledger.is_poisoned(), "the sample was meant to poison the ledger");
        let claim = |job: &str| lock(&ledger).held(job);
        // The ledger only changed in whole steps, so it is still sound: the next admission,
        // recheck and release work as on a healthy ledger instead of panicking forever.
        let free = |_: &Path| Ok(10 * GIB);
        assert_eq!(admit_sampled(&ledger, free, "b", "d:\\", Path::new("."), 6 * GIB, 0), Admission::Admitted);
        assert_eq!(claim("b"), Some(6 * GIB + margin(6 * GIB)));
        assert_eq!(claim("a"), None, "the panicking admission reserved nothing");
        let second = catch_unwind(AssertUnwindSafe(|| recheck_sampled(&ledger, panicking, "b", Path::new("."), GIB)));
        assert!(second.is_err());
        let short = recheck_sampled(&ledger, |_: &Path| Ok(GIB), "b", Path::new("."), 0).expect("a shortfall is still found");
        assert_eq!(short.available, GIB);
        assert_eq!(recheck_sampled(&ledger, free, "b", Path::new("."), 5 * GIB), None);
        assert_eq!(claim("b"), Some(GIB + margin(GIB)));
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

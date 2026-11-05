use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Mutex;

use crate::downloader::{Job, PendingCleanup, Running};
use crate::history::HistoryItem;
use crate::probe::ProbeResult;
use crate::settings::Settings;

/// Everything the commands share. Locks are only ever held for short, synchronous
/// sections and never across an `.await`.
///
/// Lock order, outermost first: `store`'s write lock, `running`, `jobs`, `live_jobs`.
/// `settings`, `history`, `probes`, `recent_probes`, `pending_cleanup` and `destinations`
/// are leaf locks and are never held
/// while another one is taken.
pub struct AppState {
    pub data_dir: PathBuf,
    pub settings: Mutex<Settings>,
    pub history: Mutex<Vec<HistoryItem>>,
    pub jobs: Mutex<Vec<Job>>,
    pub running: Mutex<HashMap<String, Running>>,
    pub http: reqwest::Client,
    /// Read by every yt-dlp run, written by tool installs, so a tool is never replaced
    /// while in use.
    pub engine: tokio::sync::RwLock<()>,
    /// Problems found at startup (damaged files, no desktop blur) that the UI shows once.
    pub notices: Mutex<Vec<String>>,
    /// Set as the app exits. Checked under the `running` and `probes` locks before any
    /// yt-dlp process starts, so nothing starts after shutdown has collected them.
    pub shutting_down: AtomicBool,
    /// Download workers (`run_job`) still alive; shutdown waits for them to settle.
    pub workers: AtomicUsize,
    /// Jobs with a live worker; `pump` never gives a job a second one (a pause and resume
    /// can requeue it while the first still waits). Leaf lock, taken inside `jobs`.
    pub live_jobs: Mutex<HashSet<String>>,
    /// Running link lookups (probe id → process id), so shutdown can stop their trees.
    pub probes: Mutex<HashMap<u64, u32>>,
    /// The last few lookups, for the link cache.
    pub recent_probes: Mutex<Vec<(String, ProbeResult)>>,
    /// Launched by "Open at login": stay in the notification area until asked.
    pub start_hidden: AtomicBool,
    /// Cancelled jobs' files that could not be deleted yet (still in use). Leaf lock.
    pub pending_cleanup: Mutex<Vec<PendingCleanup>>,
    /// Destinations being published right now (lower-cased), so two jobs never move
    /// files to the same name at once. Leaf lock.
    pub destinations: Mutex<HashSet<String>>,
}

impl AppState {
    pub fn settings_path(&self) -> PathBuf {
        self.data_dir.join("settings.json")
    }
    pub fn history_path(&self) -> PathBuf {
        self.data_dir.join("history.json")
    }
    pub fn queue_path(&self) -> PathBuf {
        self.data_dir.join("queue.json")
    }
    pub fn thumbs_dir(&self) -> PathBuf {
        self.data_dir.join("thumbs")
    }
    pub fn bin_dir(&self) -> PathBuf {
        self.data_dir.join("bin")
    }
    pub fn cleanup_path(&self) -> PathBuf {
        self.data_dir.join("cleanup.json")
    }
    pub fn probe_cache_dir(&self) -> PathBuf {
        self.data_dir.join("cache").join("probe")
    }
}

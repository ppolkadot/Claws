use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::Mutex;

use crate::downloader::{Job, PendingCleanup, Running};
use crate::history::HistoryItem;
use crate::probe::ProbeResult;
use crate::queue_writer::QueueWriter;
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
    /// Bumped after every queue change; the writer saves snapshots in revision order.
    pub queue_revision: AtomicU64,
    /// Disk space reserved by running jobs: job id -> (volume, bytes). Leaf lock.
    pub disk_reservations: Mutex<HashMap<String, (String, u64)>>,
    /// Source of `Job::revision`; only advanced with the jobs lock held.
    pub job_revision: AtomicU64,
    pub queue_writer: QueueWriter,
}

impl AppState {
    pub fn new(
        data_dir: PathBuf,
        settings: Settings,
        history: Vec<HistoryItem>,
        pending_cleanup: Vec<PendingCleanup>,
        queue_writer: QueueWriter,
        start_hidden: bool,
    ) -> Self {
        Self {
            data_dir,
            settings: Mutex::new(settings),
            history: Mutex::new(history),
            jobs: Mutex::new(Vec::new()),
            running: Mutex::new(HashMap::new()),
            http: reqwest::Client::builder()
                .user_agent(concat!("Claws/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
            engine: tokio::sync::RwLock::new(()),
            notices: Mutex::new(Vec::new()),
            shutting_down: AtomicBool::new(false),
            workers: AtomicUsize::new(0),
            live_jobs: Mutex::new(HashSet::new()),
            probes: Mutex::new(HashMap::new()),
            recent_probes: Mutex::new(Vec::new()),
            start_hidden: AtomicBool::new(start_hidden),
            pending_cleanup: Mutex::new(pending_cleanup),
            destinations: Mutex::new(HashSet::new()),
            queue_revision: AtomicU64::new(0),
            disk_reservations: Mutex::new(HashMap::new()),
            job_revision: AtomicU64::new(0),
            queue_writer,
        }
    }

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

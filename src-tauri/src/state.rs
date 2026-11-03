use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::downloader::{Job, Running};
use crate::history::HistoryItem;
use crate::settings::Settings;

/// Everything the commands share. Locks are only ever held for short, synchronous
/// sections and never across an `.await`.
pub struct AppState {
    pub data_dir: PathBuf,
    pub settings: Mutex<Settings>,
    pub history: Mutex<Vec<HistoryItem>>,
    pub jobs: Mutex<Vec<Job>>,
    pub running: Mutex<HashMap<String, Running>>,
    pub http: reqwest::Client,
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
    pub fn probe_cache_dir(&self) -> PathBuf {
        self.data_dir.join("cache").join("probe")
    }
}

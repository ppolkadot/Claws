//! What a job was started with, frozen so later settings changes cannot move or reshape a
//! download that already has bytes on disk.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::settings::Settings;

pub const EXECUTION_SCHEMA: u16 = 1;

/// The fragment concurrency used until benchmarks justify another value (plan task 13).
pub const FRAGMENT_CONCURRENCY: u8 = 8;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum AudioSelection {
    /// yt-dlp's own pick of the best audio, converted as needed.
    #[default]
    Best,
    /// Prefer an audio stream already in the requested codec, so it can be copied.
    PreferMatching,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Transport {
    #[default]
    Native,
    Aria2c,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobExecution {
    pub schema_version: u16,
    pub output_dir: PathBuf,
    pub filename_template: String,
    pub embed_metadata: bool,
    pub audio_selection: AudioSelection,
    pub fragment_concurrency: u8,
    pub transport: Transport,
}

impl JobExecution {
    /// Binds the settings in force when the job first starts. `fast_save` turns off the
    /// optional whole-file metadata pass for this job only.
    pub fn freeze(settings: &Settings, fast_save: bool) -> Self {
        Self {
            schema_version: EXECUTION_SCHEMA,
            output_dir: PathBuf::from(&settings.output_dir),
            filename_template: settings.filename_template.clone(),
            embed_metadata: settings.embed_metadata && !fast_save,
            audio_selection: settings.audio_selection,
            fragment_concurrency: FRAGMENT_CONCURRENCY,
            transport: Transport::Native,
        }
    }
}

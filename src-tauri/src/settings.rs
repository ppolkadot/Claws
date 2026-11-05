use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::download_policy::AudioSelection;
use crate::error::{AppError, Result};

/// Settings written by this version. Older files are migrated in `repair`.
pub const SETTINGS_SCHEMA: u16 = 2;

/// The system-wide shortcuts offered for Quick Open, in the global-shortcut syntax.
/// Chosen to avoid combinations Windows, browsers and AltGr layouts already use.
pub const SHORTCUTS: &[&str] = &["ctrl+alt+d", "ctrl+alt+space", "ctrl+shift+space", "off"];

pub const BROWSERS: &[&str] = &[
    "chrome", "edge", "firefox", "brave", "opera", "vivaldi", "chromium", "whale",
];

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct CookieSource {
    /// "none", "browser" or "file".
    pub kind: String,
    /// Browser name for "browser", absolute path for "file".
    pub value: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Missing in files from before 1.0.5, which read as 0 and are migrated.
    #[serde(default)]
    pub schema_version: u16,
    pub output_dir: String,
    pub filename_template: String,
    pub cookie_source: CookieSource,
    /// "system", "light" or "dark".
    pub theme: String,
    /// 0..=100: how much of the desktop shows through the window.
    pub glass_intensity: u8,
    /// "glass", "acrylic", "mica", "tabbed" or "none".
    pub backdrop: String,
    pub max_concurrent: u8,
    /// Costs one extra full rewrite of each file after it is merged.
    pub embed_metadata: bool,
    /// System-wide shortcut that brings Claws forward (one of `SHORTCUTS`).
    pub quick_open_shortcut: String,
    /// Closing the window leaves Claws in the notification area.
    pub keep_running: bool,
    /// Start (hidden, in the notification area) when the user signs in to Windows.
    pub open_at_login: bool,
    /// Which audio stream to fetch for audio-only downloads.
    #[serde(default)]
    pub audio_selection: AudioSelection,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA,
            output_dir: String::new(),
            filename_template: "%(title).150B [%(id)s].%(ext)s".into(),
            cookie_source: CookieSource {
                kind: "none".into(),
                value: String::new(),
            },
            theme: "system".into(),
            glass_intensity: 70,
            backdrop: "glass".into(),
            max_concurrent: 3,
            embed_metadata: true,
            quick_open_shortcut: "ctrl+alt+d".into(),
            keep_running: true,
            open_at_login: false,
            // New installations prefer audio that can be copied rather than converted.
            audio_selection: AudioSelection::PreferMatching,
        }
    }
}

impl Settings {
    /// Clamps and checks user input. Anything that ends up on the yt-dlp command
    /// line has to pass here first.
    pub fn validate(&mut self) -> Result<()> {
        self.glass_intensity = self.glass_intensity.min(100);
        self.max_concurrent = self.max_concurrent.clamp(1, 8);

        if !["system", "light", "dark"].contains(&self.theme.as_str()) {
            self.theme = "system".into();
        }
        if !["glass", "acrylic", "mica", "tabbed", "none"].contains(&self.backdrop.as_str()) {
            self.backdrop = "glass".into();
        }
        if !SHORTCUTS.contains(&self.quick_open_shortcut.as_str()) {
            self.quick_open_shortcut = Settings::default().quick_open_shortcut;
        }

        let template = self.filename_template.trim();
        if template.is_empty() {
            return Err(AppError::msg("The filename template cannot be empty."));
        }
        // The template is joined onto the output folder, so it must stay relative
        // and inside it.
        let escapes = template.contains("..")
            || template.starts_with(['/', '\\'])
            || template.chars().nth(1) == Some(':');
        if escapes {
            return Err(AppError::msg(
                "The filename template must be a relative path without \"..\".",
            ));
        }
        self.filename_template = template.to_string();

        match self.cookie_source.kind.as_str() {
            "none" => self.cookie_source.value.clear(),
            "browser" => {
                if !BROWSERS.contains(&self.cookie_source.value.as_str()) {
                    return Err(AppError::msg("Unsupported browser for cookie import."));
                }
            }
            "file" => {
                let path = PathBuf::from(&self.cookie_source.value);
                if !path.is_file() {
                    return Err(AppError::msg("The cookies file does not exist."));
                }
            }
            _ => {
                self.cookie_source = CookieSource {
                    kind: "none".into(),
                    value: String::new(),
                }
            }
        }

        if self.output_dir.trim().is_empty() {
            return Err(AppError::msg("Choose an output folder."));
        }
        Ok(())
    }

    /// For settings loaded from disk: resets what `validate` would reject (a hand-edited
    /// file, a deleted cookies file). Returns a message when something was reset.
    pub fn repair(&mut self) -> Option<String> {
        self.migrate();
        let defaults = Settings::default();
        let mut reset = Vec::new();
        // The output folder is filled in by the caller when empty.
        let placeholder = self.output_dir.trim().is_empty();
        if placeholder {
            self.output_dir = ".".into();
        }
        for _ in 0..3 {
            let Err(e) = self.validate() else { break };
            let message = e.to_string();
            if message.contains("template") {
                self.filename_template = defaults.filename_template.clone();
                reset.push("the filename template");
            } else if message.contains("cookie") || message.contains("browser") {
                self.cookie_source = defaults.cookie_source.clone();
                reset.push("the cookie source");
            } else {
                break;
            }
        }
        if placeholder {
            self.output_dir.clear();
        }
        (!reset.is_empty()).then(|| format!("Claws reset {} because the saved value could not be used.", reset.join(" and ")))
    }
}

impl Settings {
    /// Brings settings from an older version up to `SETTINGS_SCHEMA` without changing
    /// behaviour the user already had: fields added since then keep their old meaning.
    pub fn migrate(&mut self) {
        if self.schema_version < 2 {
            // 1.0.4 and earlier always fetched yt-dlp's best audio.
            self.audio_selection = AudioSelection::Best;
        }
        self.schema_version = self.schema_version.max(SETTINGS_SCHEMA);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> Settings {
        Settings {
            output_dir: "C:\\Downloads".into(),
            ..Settings::default()
        }
    }

    #[test]
    fn templates_must_stay_inside_the_folder() {
        for bad in ["../x.%(ext)s", "C:\\x.%(ext)s", "\\\\server\\x", "/x", "   "] {
            let mut s = valid();
            s.filename_template = bad.into();
            assert!(s.validate().is_err(), "{bad} should be rejected");
        }
        let mut s = valid();
        s.filename_template = "%(uploader)s/%(title)s.%(ext)s".into();
        assert!(s.validate().is_ok());
    }

    #[test]
    fn every_offered_shortcut_parses() {
        use tauri_plugin_global_shortcut::Shortcut;
        for choice in SHORTCUTS.iter().filter(|c| **c != "off") {
            assert!(choice.parse::<Shortcut>().is_ok(), "{choice} should parse");
        }
        let mut s = valid();
        s.quick_open_shortcut = "win+l".into();
        s.validate().unwrap();
        assert_eq!(s.quick_open_shortcut, "ctrl+alt+d");
    }

    #[test]
    fn browser_must_be_known() {
        let mut s = valid();
        s.cookie_source = CookieSource { kind: "browser".into(), value: "C:\\cookies.txt".into() };
        assert!(s.validate().is_err());
        s.cookie_source.value = "firefox".into();
        assert!(s.validate().is_ok());
    }

    #[test]
    fn settings_from_1_0_4_keep_best_audio() {
        let old = r#"{"outputDir":"D:/Downloads","filenameTemplate":"%(title)s.%(ext)s","embedMetadata":true}"#;
        let mut s: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(s.schema_version, 0);
        assert!(s.repair().is_none());
        assert_eq!(s.audio_selection, AudioSelection::Best);
        assert_eq!(s.schema_version, SETTINGS_SCHEMA);
        assert_eq!(Settings::default().audio_selection, AudioSelection::PreferMatching);
        let round: Settings = serde_json::from_str(&serde_json::to_string(&Settings { audio_selection: AudioSelection::PreferMatching, ..s }).unwrap()).unwrap();
        assert_eq!(round.audio_selection, AudioSelection::PreferMatching);
    }

    #[test]
    fn repair_resets_only_bad_values() {
        let mut s = valid();
        s.filename_template = "../escape".into();
        s.cookie_source = CookieSource { kind: "file".into(), value: "Z:\\definitely\\missing.txt".into() };
        s.max_concurrent = 8;
        assert!(s.repair().is_some());
        assert_eq!(s.filename_template, Settings::default().filename_template);
        assert_eq!(s.cookie_source.kind, "none");
        assert_eq!(s.max_concurrent, 8);
        assert!(s.validate().is_ok());
    }
}

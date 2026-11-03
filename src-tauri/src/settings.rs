use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
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
}

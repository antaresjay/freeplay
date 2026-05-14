//! What the interface remembers between launches.
//!
//! One small json file. Nothing here is worth a database, and a file you can
//! open in notepad fits an app whose whole argument is that you can read what
//! it does.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// "system", "dark" or "light".
    pub theme: String,
    pub accent: String,
    /// Game keys, in the order they were pinned.
    pub pinned: Vec<String>,
    pub favourites: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            accent: "amber".into(),
            pinned: Vec::new(),
            favourites: Vec::new(),
        }
    }
}

impl Settings {
    /// Keeps unknown values out of the file, so a hand edit cannot leave the
    /// interface pointing at a theme that does not exist.
    pub fn tidy(&mut self) {
        if !["system", "dark", "light"].contains(&self.theme.as_str()) {
            self.theme = "system".into();
        }
        if !["amber", "violet", "cyan", "rose", "lime"].contains(&self.accent.as_str()) {
            self.accent = "amber".into();
        }
        self.pinned.dedup();
        self.favourites.dedup();
    }
}

pub fn path() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".config"))
                .unwrap_or_else(|_| PathBuf::from("."))
        });
    base.join("freeplay").join("settings.json")
}

pub fn load() -> Settings {
    let mut settings: Settings = std::fs::read_to_string(path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    settings.tidy();
    settings
}

pub fn save(settings: &Settings) -> Result<(), String> {
    let file = path();
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&file, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let settings = Settings::default();
        assert_eq!(settings.theme, "system");
        assert_eq!(settings.accent, "amber");
        assert!(settings.pinned.is_empty());
    }

    #[test]
    fn a_missing_field_falls_back_rather_than_failing() {
        let settings: Settings = serde_json::from_str(r#"{"accent":"cyan"}"#).unwrap();
        assert_eq!(settings.accent, "cyan");
        assert_eq!(settings.theme, "system");
    }

    #[test]
    fn nonsense_written_by_hand_is_corrected() {
        let mut settings: Settings =
            serde_json::from_str(r#"{"theme":"neon","accent":"puce"}"#).unwrap();
        settings.tidy();
        assert_eq!(settings.theme, "system");
        assert_eq!(settings.accent, "amber");
    }

    #[test]
    fn survives_a_round_trip() {
        let before = Settings {
            theme: "light".into(),
            pinned: vec!["steam:20920".into()],
            ..Default::default()
        };

        let text = serde_json::to_string(&before).unwrap();
        let after: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(after.theme, "light");
        assert_eq!(after.pinned, vec!["steam:20920".to_string()]);
    }
}

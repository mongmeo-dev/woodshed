//! App settings, stored as JSON in the app data directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

pub const SETTINGS_FILE: &str = "settings.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Ko,
    En,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// Everything the app persists. Unknown or missing fields fall back to
/// defaults, so older and newer files both load.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// `None` follows the system language.
    pub language: Option<Language>,
    /// `None` follows the system default output.
    pub output_device_id: Option<String>,
    pub theme: Theme,
}

pub struct SettingsStore {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl SettingsStore {
    /// Loads `dir/settings.json`. A missing or unreadable file yields defaults.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(SETTINGS_FILE);
        let current = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
                log::warn!("settings file is invalid, using defaults: {error}");
                Settings::default()
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Settings::default(),
            Err(error) => {
                log::warn!("cannot read settings, using defaults: {error}");
                Settings::default()
            }
        };
        Self {
            path,
            current: Mutex::new(current),
        }
    }

    pub fn get(&self) -> Settings {
        self.current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Writes atomically (temp file + rename) and updates the cached copy.
    pub fn save(&self, settings: Settings) -> io::Result<Settings> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(&settings).map_err(io::Error::other)?;
        let temp = self.path.with_extension("json.tmp");
        fs::write(&temp, text)?;
        fs::rename(&temp, &self.path)?;
        *self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = settings.clone();
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("woodshed-settings-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn missing_file_loads_defaults() {
        let dir = temp_dir("missing");
        assert_eq!(SettingsStore::load(&dir).get(), Settings::default());
    }

    #[test]
    fn saved_settings_load_back() {
        let dir = temp_dir("roundtrip");
        let settings = Settings {
            language: Some(Language::En),
            output_device_id: Some("coreaudio:BuiltInSpeakerDevice".into()),
            theme: Theme::Dark,
        };
        SettingsStore::load(&dir).save(settings.clone()).unwrap();
        assert_eq!(SettingsStore::load(&dir).get(), settings);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_and_unknown_fields_fall_back_to_defaults() {
        let dir = temp_dir("partial");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(SETTINGS_FILE), r#"{"language":"ko","future":1}"#).unwrap();
        let settings = SettingsStore::load(&dir).get();
        assert_eq!(settings.language, Some(Language::Ko));
        assert_eq!(settings.theme, Theme::System);
        assert_eq!(settings.output_device_id, None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_file_loads_defaults() {
        let dir = temp_dir("invalid");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(SETTINGS_FILE), "not json").unwrap();
        assert_eq!(SettingsStore::load(&dir).get(), Settings::default());
        let _ = fs::remove_dir_all(&dir);
    }
}

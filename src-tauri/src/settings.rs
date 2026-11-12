//! App settings, stored as JSON in the app data directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::tuner::TunerSettings;

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
    pub tuner: TunerSettings,
}

pub struct SettingsStore {
    path: PathBuf,
    /// Cached copy. Held only to read or swap it, never during file I/O.
    current: Mutex<Settings>,
    /// Serializes writers (read, change, write the file, swap the cache).
    writer: Mutex<()>,
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
            writer: Mutex::new(()),
        }
    }

    pub fn get(&self) -> Settings {
        self.current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Changes the settings in one step: writers take turns, and each takes
    /// the current value, applies `change`, writes the file (temp file +
    /// rename) and then swaps the cached copy. Every writer changes only its
    /// own fields this way, so concurrent owners (the screen, the output
    /// manager, the tuner) never undo each other. Readers ([`Self::get`])
    /// wait for no file I/O: they see the old value until the swap. If
    /// writing fails, nothing changes.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) -> io::Result<Settings> {
        let _turn = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut next = self.get();
        change(&mut next);
        self.write(&next)?;
        *self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = next.clone();
        Ok(next)
    }

    /// Replaces every field. Prefer [`SettingsStore::update`].
    pub fn save(&self, settings: Settings) -> io::Result<Settings> {
        self.update(|current| *current = settings)
    }

    fn write(&self, settings: &Settings) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(settings).map_err(io::Error::other)?;
        // Per process: several app builds may share the data directory.
        let temp = self
            .path
            .with_extension(format!("json.{}.tmp", std::process::id()));
        fs::write(&temp, text)?;
        fs::rename(&temp, &self.path)
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
            ..Settings::default()
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
        assert_eq!(settings.tuner, TunerSettings::default());
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

    #[test]
    fn concurrent_owners_keep_each_others_changes() {
        use std::sync::{Arc, Barrier};

        let dir = temp_dir("concurrent");
        let store = Arc::new(SettingsStore::load(&dir));
        store
            .update(|s| {
                s.language = Some(Language::En);
                s.theme = Theme::Dark;
            })
            .unwrap();
        const ROUNDS: u16 = 200;
        let barrier = Arc::new(Barrier::new(2));
        let tuner = {
            let (store, barrier) = (Arc::clone(&store), Arc::clone(&barrier));
            std::thread::spawn(move || {
                for round in 0..ROUNDS {
                    barrier.wait();
                    store
                        .update(|s| s.tuner.reference_hz = 430 + round % 21)
                        .unwrap();
                }
            })
        };
        let output = {
            let (store, barrier) = (Arc::clone(&store), Arc::clone(&barrier));
            std::thread::spawn(move || {
                for round in 0..ROUNDS {
                    barrier.wait();
                    store
                        .update(|s| s.output_device_id = Some(format!("coreaudio:dev{round}")))
                        .unwrap();
                }
            })
        };
        tuner.join().unwrap();
        output.join().unwrap();

        let last = ROUNDS - 1;
        for settings in [store.get(), SettingsStore::load(&dir).get()] {
            assert_eq!(settings.tuner.reference_hz, 430 + last % 21);
            assert_eq!(
                settings.output_device_id.as_deref(),
                Some(format!("coreaudio:dev{last}").as_str())
            );
            assert_eq!(settings.language, Some(Language::En));
            assert_eq!(settings.theme, Theme::Dark);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reading_does_not_wait_for_a_save_in_progress() {
        use std::sync::{Arc, mpsc};
        use std::time::{Duration, Instant};

        let dir = temp_dir("read-during-save");
        let store = Arc::new(SettingsStore::load(&dir));
        let (started, wait_started) = mpsc::channel();
        let (release, wait_release) = mpsc::channel::<()>();
        let writer = {
            let store = Arc::clone(&store);
            std::thread::spawn(move || {
                store
                    .update(|s| {
                        // Stands in for a slow write: the writer is mid-save.
                        started.send(()).unwrap();
                        wait_release.recv().unwrap();
                        s.theme = Theme::Dark;
                    })
                    .unwrap();
            })
        };
        wait_started.recv().unwrap();
        let begun = Instant::now();
        assert_eq!(store.get().theme, Theme::System, "old value until saved");
        assert!(begun.elapsed() < Duration::from_millis(50), "read waited");
        release.send(()).unwrap();
        writer.join().unwrap();
        assert_eq!(store.get().theme, Theme::Dark);
        let _ = fs::remove_dir_all(&dir);
    }
}

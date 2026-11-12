//! The metronome's own file in the app data directory: the last settings and
//! the named presets. Kept apart from `settings.json` so it never races the
//! app settings.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use super::pattern::MetronomeSettings;

pub const METRONOME_FILE: &str = "metronome.json";
/// Longest preset name, in characters.
pub const MAX_PRESET_NAME: usize = 40;
/// Most presets kept.
pub const MAX_PRESETS: usize = 100;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub name: String,
    pub settings: MetronomeSettings,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MetronomeData {
    pub settings: MetronomeSettings,
    pub presets: Vec<Preset>,
}

impl MetronomeData {
    /// Normalizes every setting and drops unusable or duplicate presets.
    fn normalized(self) -> Self {
        let mut presets: Vec<Preset> = Vec::new();
        for preset in self.presets {
            let Some(name) = clean_name(&preset.name) else {
                continue;
            };
            if presets.iter().any(|kept| kept.name == name) || presets.len() >= MAX_PRESETS {
                continue;
            }
            presets.push(Preset {
                name,
                settings: preset.settings.normalized(),
            });
        }
        Self {
            settings: self.settings.normalized(),
            presets,
        }
    }
}

/// Why a preset could not be saved or deleted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum PresetError {
    #[error("the preset name is empty")]
    EmptyName,
    #[error("the preset name is longer than {MAX_PRESET_NAME} characters")]
    NameTooLong,
    #[error("there are already {MAX_PRESETS} presets")]
    TooMany,
    #[error("cannot write the presets: {message}")]
    SaveFailed { message: String },
}

/// Trims the name; `None` when nothing is left.
fn clean_name(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

pub struct MetronomeStore {
    path: PathBuf,
    data: Mutex<MetronomeData>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl MetronomeStore {
    /// Loads `dir/metronome.json`. A missing or unreadable file yields defaults.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(METRONOME_FILE);
        let data = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<MetronomeData>(&text).unwrap_or_else(|error| {
                log::warn!("metronome file is invalid, using defaults: {error}");
                MetronomeData::default()
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => MetronomeData::default(),
            Err(error) => {
                log::warn!("cannot read the metronome file, using defaults: {error}");
                MetronomeData::default()
            }
        };
        Self {
            path,
            data: Mutex::new(data.normalized()),
        }
    }

    pub fn get(&self) -> MetronomeData {
        lock(&self.data).clone()
    }

    /// Replaces the last-used settings in memory. [`Self::save`] writes them.
    pub fn set_settings(&self, settings: MetronomeSettings) {
        lock(&self.data).settings = settings.normalized();
    }

    /// Writes what is in memory atomically (temp file + rename).
    pub fn save(&self) -> io::Result<()> {
        self.save_with(|| {})
    }

    /// [`Self::save`], calling `between` after the snapshot and before the
    /// write (tests use it to interleave other saves).
    fn save_with(&self, between: impl FnOnce()) -> io::Result<()> {
        self.update_with(|_| Ok::<(), io::Error>(()), between)
            .map(|_| ())
    }

    /// Changes the data with `change` and writes the result. Every write goes
    /// through here: the lock is held from the snapshot to the rename and
    /// the memory commit, so writers never overwrite each other with older
    /// snapshots. Memory changes only if the write succeeds.
    pub fn update<E: From<io::Error>>(
        &self,
        change: impl FnOnce(&mut MetronomeData) -> Result<(), E>,
    ) -> Result<MetronomeData, E> {
        self.update_with(change, || {})
    }

    fn update_with<E: From<io::Error>>(
        &self,
        change: impl FnOnce(&mut MetronomeData) -> Result<(), E>,
        between: impl FnOnce(),
    ) -> Result<MetronomeData, E> {
        let mut data = lock(&self.data);
        let mut next = data.clone();
        change(&mut next)?;
        between();
        write_atomically(&self.path, &next)?;
        *data = next;
        Ok(data.clone())
    }

    /// Saves `settings` under `name`, replacing a preset of the same name.
    pub fn save_preset(
        &self,
        name: &str,
        settings: MetronomeSettings,
    ) -> Result<Vec<Preset>, PresetError> {
        let name = clean_name(name).ok_or(PresetError::EmptyName)?;
        if name.chars().count() > MAX_PRESET_NAME {
            return Err(PresetError::NameTooLong);
        }
        let preset = Preset {
            name,
            settings: settings.normalized(),
        };
        self.update(|data| {
            match data
                .presets
                .iter()
                .position(|kept| kept.name == preset.name)
            {
                Some(index) => data.presets[index] = preset,
                None if data.presets.len() >= MAX_PRESETS => return Err(PresetError::TooMany),
                None => data.presets.push(preset),
            }
            Ok(())
        })
        .map(|data| data.presets)
    }

    /// Deletes the preset called `name` (nothing happens if there is none).
    pub fn delete_preset(&self, name: &str) -> Result<Vec<Preset>, PresetError> {
        self.update(|data| {
            data.presets.retain(|preset| preset.name != name);
            Ok::<(), PresetError>(())
        })
        .map(|data| data.presets)
    }
}

impl From<io::Error> for PresetError {
    fn from(error: io::Error) -> Self {
        PresetError::SaveFailed {
            message: error.to_string(),
        }
    }
}

/// Temp file + rename. The temp name is per process, so another running
/// copy of the app (sharing the app data directory) never truncates it.
fn write_atomically(path: &Path, data: &MetronomeData) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(data).map_err(io::Error::other)?;
    let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&temp, text)?;
    fs::rename(&temp, path).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("woodshed-metronome-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn at(bpm: f64) -> MetronomeSettings {
        MetronomeSettings {
            bpm,
            ..MetronomeSettings::default()
        }
    }

    #[test]
    fn missing_file_loads_defaults() {
        let dir = temp_dir("missing");
        assert_eq!(MetronomeStore::load(&dir).get(), MetronomeData::default());
    }

    #[test]
    fn presets_save_load_replace_and_delete() {
        let dir = temp_dir("presets");
        let store = MetronomeStore::load(&dir);
        store.save_preset("  Warm up ", at(80.0)).unwrap();
        store.save_preset("Shuffle", at(96.0)).unwrap();
        let presets = store.save_preset("Warm up", at(84.5)).unwrap();
        assert_eq!(
            presets
                .iter()
                .map(|p| (p.name.as_str(), p.settings.bpm))
                .collect::<Vec<_>>(),
            vec![("Warm up", 84.5), ("Shuffle", 96.0)]
        );

        let reloaded = MetronomeStore::load(&dir);
        assert_eq!(reloaded.get().presets, presets);

        let presets = reloaded.delete_preset("Warm up").unwrap();
        assert_eq!(presets.len(), 1);
        assert_eq!(MetronomeStore::load(&dir).get().presets, presets);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_names_are_rejected() {
        let dir = temp_dir("names");
        let store = MetronomeStore::load(&dir);
        assert_eq!(
            store.save_preset("   ", at(90.0)),
            Err(PresetError::EmptyName)
        );
        let long = "가".repeat(MAX_PRESET_NAME + 1);
        assert_eq!(
            store.save_preset(&long, at(90.0)),
            Err(PresetError::NameTooLong)
        );
        assert!(
            store
                .save_preset(&"가".repeat(MAX_PRESET_NAME), at(90.0))
                .is_ok()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_settings_are_saved_and_normalized_on_load() {
        let dir = temp_dir("settings");
        let store = MetronomeStore::load(&dir);
        store.set_settings(at(133.3));
        store.save().unwrap();
        assert_eq!(MetronomeStore::load(&dir).get().settings.bpm, 133.3);

        fs::write(
            dir.join(METRONOME_FILE),
            r#"{"settings":{"bpm":9000,"beats":3},"presets":[{"name":"","settings":{}},{"name":"a","settings":{"subdivision":99}}]}"#,
        )
        .unwrap();
        let data = MetronomeStore::load(&dir).get();
        assert_eq!(data.settings.bpm, 400.0);
        assert_eq!(data.settings.accents.len(), 3);
        assert_eq!(data.presets.len(), 1);
        assert_eq!(data.presets[0].settings.subdivision, 8);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_write_keeps_the_previous_presets() {
        let dir = temp_dir("readonly");
        fs::create_dir_all(&dir).unwrap();
        // A directory where the file should be makes the rename fail.
        fs::create_dir_all(dir.join(METRONOME_FILE)).unwrap();
        let store = MetronomeStore::load(&dir);
        let result = store.save_preset("x", at(90.0));
        assert!(matches!(result, Err(PresetError::SaveFailed { .. })));
        assert!(store.get().presets.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_save_never_overwrites_a_preset_saved_or_deleted_meanwhile() {
        use std::sync::{Arc, Barrier};
        let dir = temp_dir("interleave");
        let store = Arc::new(MetronomeStore::load(&dir));
        store.save_preset("old", at(70.0)).unwrap();

        for round in 0..2 {
            // A takes its snapshot, then lets B start its own save while A
            // has not written yet.
            let barrier = Arc::new(Barrier::new(2));
            let a = {
                let (store, barrier) = (Arc::clone(&store), Arc::clone(&barrier));
                std::thread::spawn(move || {
                    store.set_settings(at(111.0));
                    store.save_with(|| {
                        barrier.wait();
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    })
                })
            };
            barrier.wait();
            let b = if round == 0 {
                store.save_preset("new", at(90.0))
            } else {
                store.delete_preset("old")
            };
            assert!(b.is_ok());
            a.join().unwrap().unwrap();
        }

        let memory = store.get();
        let names: Vec<_> = memory.presets.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["new"],
            "added preset kept, deleted one not revived"
        );
        let text = fs::read_to_string(dir.join(METRONOME_FILE)).unwrap();
        let on_disk: MetronomeData = serde_json::from_str(&text).unwrap();
        assert_eq!(on_disk, memory);
        assert_eq!(MetronomeStore::load(&dir).get(), memory);
        assert_eq!(memory.settings.bpm, 111.0);
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}

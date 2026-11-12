//! The drum machine's own file in the app data directory: the pattern being
//! edited and the patterns the user saved. Kept apart from `settings.json`,
//! which the settings screen rewrites as a whole.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use super::pattern::Pattern;

pub const DRUMS_FILE: &str = "drum-machine.json";
/// Longest pattern name kept, in characters.
pub const MAX_NAME_CHARS: usize = 40;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedPattern {
    pub name: String,
    pub pattern: Pattern,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct DrumsFile {
    current: Option<Pattern>,
    saved: Vec<SavedPattern>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("the pattern name is empty")]
    EmptyName,
    #[error("cannot write the drum machine file: {0}")]
    Io(#[from] io::Error),
}

pub struct PatternStore {
    path: PathBuf,
    file: Mutex<DrumsFile>,
}

impl PatternStore {
    /// Loads `dir/drum-machine.json`. A missing or unreadable file starts empty.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(DRUMS_FILE);
        let file = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
                log::warn!("drum machine file is invalid, starting empty: {error}");
                DrumsFile::default()
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => DrumsFile::default(),
            Err(error) => {
                log::warn!("cannot read the drum machine file, starting empty: {error}");
                DrumsFile::default()
            }
        };
        let file = DrumsFile {
            current: file.current.map(Pattern::normalized),
            saved: file
                .saved
                .into_iter()
                .map(|saved| SavedPattern {
                    name: saved.name,
                    pattern: saved.pattern.normalized(),
                })
                .collect(),
        };
        Self {
            path,
            file: Mutex::new(file),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, DrumsFile> {
        self.file.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The pattern last edited, if one was stored.
    pub fn current(&self) -> Option<Pattern> {
        self.lock().current.clone()
    }

    pub fn saved(&self) -> Vec<SavedPattern> {
        self.lock().saved.clone()
    }

    pub fn save_current(&self, pattern: &Pattern) -> Result<(), StoreError> {
        let mut file = self.lock();
        file.current = Some(pattern.clone());
        self.write(&file)
    }

    /// Saves `pattern` under `name` (trimmed, at most [`MAX_NAME_CHARS`]),
    /// replacing a pattern of the same name in place.
    pub fn save_named(
        &self,
        name: &str,
        pattern: &Pattern,
    ) -> Result<Vec<SavedPattern>, StoreError> {
        let name: String = name.trim().chars().take(MAX_NAME_CHARS).collect();
        if name.is_empty() {
            return Err(StoreError::EmptyName);
        }
        let mut file = self.lock();
        let saved = SavedPattern {
            name,
            pattern: pattern.clone(),
        };
        match file
            .saved
            .iter_mut()
            .find(|existing| existing.name == saved.name)
        {
            Some(existing) => *existing = saved,
            None => file.saved.push(saved),
        }
        self.write(&file)?;
        Ok(file.saved.clone())
    }

    pub fn delete(&self, name: &str) -> Result<Vec<SavedPattern>, StoreError> {
        let mut file = self.lock();
        file.saved.retain(|saved| saved.name != name);
        self.write(&file)?;
        Ok(file.saved.clone())
    }

    /// Writes atomically (temp file + rename).
    fn write(&self, file: &DrumsFile) -> Result<(), StoreError> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(file).map_err(io::Error::other)?;
        let temp = self.path.with_extension("json.tmp");
        fs::write(&temp, text)?;
        fs::rename(&temp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("woodshed-drums-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn pattern(bpm: u16) -> Pattern {
        Pattern {
            bpm,
            ..Pattern::default()
        }
        .normalized()
    }

    #[test]
    fn missing_file_starts_empty() {
        let store = PatternStore::load(&temp_dir("missing"));
        assert_eq!(store.current(), None);
        assert!(store.saved().is_empty());
    }

    #[test]
    fn current_and_saved_patterns_load_back() {
        let dir = temp_dir("roundtrip");
        let store = PatternStore::load(&dir);
        store.save_current(&pattern(91)).unwrap();
        store.save_named("  Groove A  ", &pattern(120)).unwrap();
        store.save_named("Groove B", &pattern(80)).unwrap();
        let saved = store.save_named("Groove A", &pattern(130)).unwrap();
        assert_eq!(
            saved
                .iter()
                .map(|s| (s.name.as_str(), s.pattern.bpm))
                .collect::<Vec<_>>(),
            vec![("Groove A", 130), ("Groove B", 80)],
            "same name replaces in place"
        );

        let reloaded = PatternStore::load(&dir);
        assert_eq!(reloaded.current(), Some(pattern(91)));
        assert_eq!(reloaded.saved(), saved);

        let left = reloaded.delete("Groove A").unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(PatternStore::load(&dir).saved()[0].name, "Groove B");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_are_trimmed_and_must_not_be_empty() {
        let dir = temp_dir("names");
        let store = PatternStore::load(&dir);
        assert!(matches!(
            store.save_named("   ", &pattern(100)),
            Err(StoreError::EmptyName)
        ));
        let long = "가".repeat(MAX_NAME_CHARS + 5);
        let saved = store.save_named(&long, &pattern(100)).unwrap();
        assert_eq!(saved[0].name.chars().count(), MAX_NAME_CHARS);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_file_starts_empty() {
        let dir = temp_dir("invalid");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(DRUMS_FILE), "not json").unwrap();
        assert!(PatternStore::load(&dir).saved().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}

//! The song library in the app data directory (see AGENTS.md
//! "스템 라이브러리 계약"):
//!
//! ```text
//! songs/<id>/manifest.json   Manifest (camelCase JSON)
//! songs/<id>/original.<ext>  the imported file, untouched
//! songs/<id>/audio.wav       playback copy: 16-bit PCM WAV, song rate, 1–2 channels
//! songs/<id>/waveform.json   overview peaks, 0–255 per bin
//! songs/<id>/stems/<name>.wav  separated stems, same format and rate as audio.wav
//! ```
//!
//! Imports are assembled in `songs/.incoming/<id>/` and renamed into place
//! when complete, so the library never lists a half-written song.

use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::decode::DecodedAudio;

pub const SONGS_DIR: &str = "songs";
pub const INCOMING_DIR: &str = ".incoming";
pub const MANIFEST_FILE: &str = "manifest.json";
pub const AUDIO_FILE: &str = "audio.wav";
pub const WAVEFORM_FILE: &str = "waveform.json";
pub const STEMS_DIR: &str = "stems";
pub const MANIFEST_VERSION: u32 = 1;

/// The six stems a separated song has, in mixer order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StemName {
    Vocals,
    Drums,
    Bass,
    Guitar,
    Piano,
    Other,
}

impl StemName {
    pub const ALL: [StemName; 6] = [
        StemName::Vocals,
        StemName::Drums,
        StemName::Bass,
        StemName::Guitar,
        StemName::Piano,
        StemName::Other,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            StemName::Vocals => "vocals",
            StemName::Drums => "drums",
            StemName::Bass => "bass",
            StemName::Guitar => "guitar",
            StemName::Piano => "piano",
            StemName::Other => "other",
        }
    }

    /// Path of the stem inside its song directory: `stems/<name>.wav`.
    pub fn relative_path(self) -> PathBuf {
        Path::new(STEMS_DIR).join(format!("{}.wav", self.as_str()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StemStatus {
    /// Requested, not started.
    Pending,
    /// Being separated.
    Running,
    /// `stems/<name>.wav` is complete.
    Ready,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StemEntry {
    pub name: StemName,
    pub status: StemStatus,
}

/// Where a song came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SongSource {
    #[serde(rename_all = "camelCase")]
    File { file_name: String },
    #[serde(rename_all = "camelCase")]
    Youtube { url: String, video_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub version: u32,
    pub id: String,
    pub title: String,
    pub source: SongSource,
    /// File name of the untouched import inside the song directory.
    pub original: String,
    /// Sample rate of `audio.wav` (and every stem).
    pub sample_rate: u32,
    pub channels: u16,
    /// Length of `audio.wav` in frames.
    pub frames: u64,
    pub added_at_ms: u64,
    /// Separated stems. Empty until separation is requested.
    #[serde(default)]
    pub stems: Vec<StemEntry>,
}

/// One track the player mixes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackName {
    Original,
    Vocals,
    Drums,
    Bass,
    Guitar,
    Piano,
    Other,
}

impl From<StemName> for TrackName {
    fn from(name: StemName) -> Self {
        match name {
            StemName::Vocals => TrackName::Vocals,
            StemName::Drums => TrackName::Drums,
            StemName::Bass => TrackName::Bass,
            StemName::Guitar => TrackName::Guitar,
            StemName::Piano => TrackName::Piano,
            StemName::Other => TrackName::Other,
        }
    }
}

impl Manifest {
    /// True once every listed stem is ready.
    pub fn stems_ready(&self) -> bool {
        !self.stems.is_empty() && self.stems.iter().all(|s| s.status == StemStatus::Ready)
    }

    /// Tracks to play and their files (relative to the song directory): the
    /// stems in mixer order once all are ready, otherwise the original.
    pub fn tracks(&self) -> Vec<(TrackName, PathBuf)> {
        if self.stems_ready() {
            StemName::ALL
                .iter()
                .filter(|name| self.stems.iter().any(|s| s.name == **name))
                .map(|&name| (name.into(), name.relative_path()))
                .collect()
        } else {
            vec![(TrackName::Original, PathBuf::from(AUDIO_FILE))]
        }
    }

    pub fn duration_secs(&self) -> f64 {
        self.frames as f64 / f64::from(self.sample_rate.max(1))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("song not found")]
    NotFound,
    #[error("library I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("manifest is invalid: {0}")]
    Invalid(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct Library {
    root: PathBuf,
}

/// Writes `bytes` to `path` through a temp file and a rename.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = path.with_extension("tmp");
    fs::write(&temp, bytes)?;
    fs::rename(&temp, path)
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// A new song id: time-ordered hex with a random tail.
pub fn new_song_id() -> String {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(now_ms());
    format!("{:011x}{:05x}", now_ms(), hasher.finish() & 0xF_FFFF)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

impl Library {
    /// The library under `app_data/songs`.
    pub fn new(app_data: &Path) -> Self {
        Self {
            root: app_data.join(SONGS_DIR),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn song_dir(&self, id: &str) -> Result<PathBuf, LibraryError> {
        if !valid_id(id) {
            return Err(LibraryError::NotFound);
        }
        Ok(self.root.join(id))
    }

    /// Removes imports left half-done by a previous run.
    pub fn clean_incoming(&self) {
        let _ = fs::remove_dir_all(self.root.join(INCOMING_DIR));
    }

    /// A fresh directory to assemble an import in, with its id.
    pub fn begin_import(&self) -> io::Result<(String, PathBuf)> {
        let id = new_song_id();
        let dir = self.root.join(INCOMING_DIR).join(&id);
        fs::create_dir_all(&dir)?;
        Ok((id, dir))
    }

    /// Writes the manifest and waveform into `staging` and moves the song
    /// into the library.
    pub fn finish_import(
        &self,
        staging: &Path,
        manifest: &Manifest,
        decoded: &DecodedAudio,
    ) -> Result<(), LibraryError> {
        write_atomic(
            &staging.join(WAVEFORM_FILE),
            &serde_json::to_vec(&decoded.waveform)?,
        )?;
        write_atomic(
            &staging.join(MANIFEST_FILE),
            &serde_json::to_vec_pretty(manifest)?,
        )?;
        fs::rename(staging, self.song_dir(&manifest.id)?)?;
        Ok(())
    }

    /// Every complete song, newest first. Unreadable entries are skipped.
    pub fn list(&self) -> Vec<Manifest> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut songs: Vec<Manifest> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_str().is_some_and(valid_id))
            .filter_map(|entry| match self.read_manifest(&entry.path()) {
                Ok(manifest) => Some(manifest),
                Err(error) => {
                    log::warn!("skipping song {:?}: {error}", entry.file_name());
                    None
                }
            })
            .collect();
        songs.sort_by(|a, b| b.added_at_ms.cmp(&a.added_at_ms).then(a.id.cmp(&b.id)));
        songs
    }

    fn read_manifest(&self, dir: &Path) -> Result<Manifest, LibraryError> {
        let text = fs::read_to_string(dir.join(MANIFEST_FILE)).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                LibraryError::NotFound
            } else {
                error.into()
            }
        })?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn get(&self, id: &str) -> Result<Manifest, LibraryError> {
        self.read_manifest(&self.song_dir(id)?)
    }

    /// Saves a changed manifest (stem-engine updates `stems` this way).
    pub fn save(&self, manifest: &Manifest) -> Result<(), LibraryError> {
        let dir = self.song_dir(&manifest.id)?;
        if !dir.is_dir() {
            return Err(LibraryError::NotFound);
        }
        write_atomic(
            &dir.join(MANIFEST_FILE),
            &serde_json::to_vec_pretty(manifest)?,
        )?;
        Ok(())
    }

    pub fn waveform(&self, id: &str) -> Result<Vec<u8>, LibraryError> {
        let text = fs::read_to_string(self.song_dir(id)?.join(WAVEFORM_FILE))?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn delete(&self, id: &str) -> Result<(), LibraryError> {
        let dir = self.song_dir(id)?;
        match fs::remove_dir_all(dir) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(LibraryError::NotFound),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_library(name: &str) -> (PathBuf, Library) {
        let dir =
            std::env::temp_dir().join(format!("woodshed-library-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        (dir.clone(), Library::new(&dir))
    }

    fn manifest(id: &str, added_at_ms: u64) -> Manifest {
        Manifest {
            version: MANIFEST_VERSION,
            id: id.into(),
            title: format!("Song {id}"),
            source: SongSource::File {
                file_name: "song.mp3".into(),
            },
            original: "original.mp3".into(),
            sample_rate: 44_100,
            channels: 2,
            frames: 441_000,
            added_at_ms,
            stems: Vec::new(),
        }
    }

    fn decoded() -> DecodedAudio {
        DecodedAudio {
            sample_rate: 44_100,
            channels: 2,
            frames: 441_000,
            waveform: vec![0, 128, 255],
        }
    }

    fn import(library: &Library, added_at_ms: u64) -> Manifest {
        let (id, staging) = library.begin_import().unwrap();
        fs::write(staging.join(AUDIO_FILE), b"wav").unwrap();
        let manifest = manifest(&id, added_at_ms);
        library
            .finish_import(&staging, &manifest, &decoded())
            .unwrap();
        manifest
    }

    #[test]
    fn imported_songs_are_listed_after_reopening() {
        let (dir, library) = temp_library("reopen");
        let older = import(&library, 1_000);
        let newer = import(&library, 2_000);
        drop(library);

        let reopened = Library::new(&dir);
        assert_eq!(reopened.list(), vec![newer.clone(), older.clone()]);
        assert_eq!(reopened.get(&older.id).unwrap(), older);
        assert_eq!(reopened.waveform(&newer.id).unwrap(), vec![0, 128, 255]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn half_done_imports_are_not_listed_and_get_cleaned() {
        let (dir, library) = temp_library("incoming");
        let (_, staging) = library.begin_import().unwrap();
        fs::write(staging.join(AUDIO_FILE), b"partial").unwrap();
        assert!(library.list().is_empty());
        library.clean_incoming();
        assert!(!staging.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_removes_the_song_and_its_files() {
        let (dir, library) = temp_library("delete");
        let song = import(&library, 1);
        library.delete(&song.id).unwrap();
        assert!(library.list().is_empty());
        assert!(!dir.join(SONGS_DIR).join(&song.id).exists());
        assert!(matches!(
            library.delete(&song.id),
            Err(LibraryError::NotFound)
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_round_trips_stems_and_sources() {
        let (dir, library) = temp_library("stems");
        let mut song = import(&library, 5);
        song.source = SongSource::Youtube {
            url: "https://youtu.be/abc".into(),
            video_id: "abc".into(),
        };
        song.stems = StemName::ALL
            .iter()
            .map(|&name| StemEntry {
                name,
                status: StemStatus::Running,
            })
            .collect();
        library.save(&song).unwrap();
        assert_eq!(library.get(&song.id).unwrap(), song);
        let text =
            fs::read_to_string(dir.join(SONGS_DIR).join(&song.id).join(MANIFEST_FILE)).unwrap();
        assert!(text.contains(r#""kind": "youtube""#), "{text}");
        assert!(text.contains(r#""status": "running""#), "{text}");
        assert!(text.contains(r#""sampleRate": 44100"#), "{text}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tracks_switch_to_six_stems_when_all_are_ready() {
        let mut song = manifest("a1", 0);
        assert_eq!(
            song.tracks(),
            vec![(TrackName::Original, PathBuf::from(AUDIO_FILE))]
        );
        song.stems = StemName::ALL
            .iter()
            .rev()
            .map(|&name| StemEntry {
                name,
                status: StemStatus::Ready,
            })
            .collect();
        song.stems[0].status = StemStatus::Running;
        assert_eq!(song.tracks().len(), 1, "not until every stem is ready");
        song.stems[0].status = StemStatus::Ready;
        let tracks = song.tracks();
        assert_eq!(tracks.len(), 6);
        assert_eq!(
            tracks[0],
            (TrackName::Vocals, PathBuf::from("stems/vocals.wav"))
        );
        assert_eq!(
            tracks[5],
            (TrackName::Other, PathBuf::from("stems/other.wav"))
        );
    }

    #[test]
    fn manifest_without_stems_field_loads() {
        let json = r#"{"version":1,"id":"x1","title":"t","source":{"kind":"file","fileName":"a.wav"},
            "original":"original.wav","sampleRate":48000,"channels":1,"frames":10,"addedAtMs":1}"#;
        let manifest: Manifest = serde_json::from_str(json).unwrap();
        assert!(manifest.stems.is_empty());
    }

    #[test]
    fn ids_outside_the_library_are_rejected() {
        let (_, library) = temp_library("ids");
        for id in ["", "../x", "a/b", ".incoming"] {
            assert!(
                matches!(library.get(id), Err(LibraryError::NotFound)),
                "{id}"
            );
        }
    }
}

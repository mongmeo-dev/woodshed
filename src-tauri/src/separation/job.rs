//! One separation of one library song, and the recovery that runs when the
//! app starts. Inside `songs/<id>/`:
//!
//! ```text
//! .separating/<name>.wav  stems being written (deleted on cancel, failure, restart)
//! stems/<name>.wav        the stems the manifest lists
//! .stems-old/             the previous stems, only while the new ones move in
//! ```
//!
//! New stems replace `stems/` only once all six are complete, by renaming
//! the whole directory. Until then `stems/` is untouched, and so is the
//! manifest of a song that was already separated: cancelling, failing or
//! quitting while re-separating leaves its ready stems as they were. A song
//! not separated yet shows `pending`/`running` entries while it waits and
//! runs; they return to an empty list on cancel or restart and become
//! `failed` on failure. Manifests change only through `Library::update`.

use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::stems::{
    AUDIO_FILE, Library, LibraryError, Manifest, STEMS_DIR, StemEntry, StemName, StemStatus,
};

use super::audio_io::{StemWriters, read_song};
use super::dsp::Stft;
use super::engine::{SeparationOutput, Workspace};
use super::model::{SeparationError, StemModel};

/// Stems being written.
pub const WORK_DIR: &str = ".separating";
/// The previous stems while the new ones move in.
pub const OLD_STEMS_DIR: &str = ".stems-old";

/// How a job that did not finish ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Abandon {
    Cancelled,
    Failed,
}

fn in_progress(manifest: &Manifest) -> bool {
    manifest
        .stems
        .iter()
        .any(|stem| matches!(stem.status, StemStatus::Pending | StemStatus::Running))
}

fn all_stems(status: StemStatus) -> Vec<StemEntry> {
    StemName::ALL
        .iter()
        .map(|&name| StemEntry { name, status })
        .collect()
}

fn remove_dir_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Shows a song that is not separated yet as `status` (`pending` while it
/// waits, `running` once it starts). A song whose stems are all ready keeps
/// its manifest as it is.
pub fn mark(library: &Library, id: &str, status: StemStatus) -> Result<(), LibraryError> {
    if library.get(id)?.stems_ready() {
        return Ok(());
    }
    library.update(id, |manifest| {
        if !manifest.stems_ready() {
            manifest.stems = all_stems(status);
        }
    })?;
    Ok(())
}

/// Cleans up after a job that will not finish: removes its partial stems
/// and, for a song not separated before, clears (`Cancelled`) or fails
/// (`Failed`) its entries. Ready stems and their manifest stay as they were.
pub fn abandon(library: &Library, id: &str, how: Abandon) -> Result<(), SeparationError> {
    let dir = library.song_dir(id)?;
    remove_dir_if_present(&dir.join(WORK_DIR))?;
    if !in_progress(&library.get(id)?) {
        return Ok(());
    }
    library.update(id, |manifest| {
        if in_progress(manifest) {
            manifest.stems = match how {
                Abandon::Cancelled => Vec::new(),
                Abandon::Failed => all_stems(StemStatus::Failed),
            };
        }
    })?;
    Ok(())
}

/// Feeds separated planes to the stem files and reports chunk progress.
struct Output<'a> {
    writers: StemWriters,
    progress: &'a mut dyn FnMut(f32),
}

impl SeparationOutput for Output<'_> {
    fn write(&mut self, planes: &[f32], len: usize) -> Result<(), SeparationError> {
        self.writers.push(planes, len)
    }

    fn chunk_done(&mut self, done: usize, total: usize) {
        (self.progress)(done as f32 / total.max(1) as f32);
    }
}

/// Separates song `id` into `.separating/`, then moves the six stems into
/// `stems/` and marks them ready. `progress` gets 0.0 when the chunks start
/// and then the share of chunks done. On an error the caller runs
/// [`abandon`].
pub fn run(
    library: &Library,
    id: &str,
    stft: &Stft,
    model: &mut dyn StemModel,
    work: &mut Workspace,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<(), SeparationError> {
    let manifest = library.get(id)?;
    let dir = library.song_dir(id)?;
    let audio = read_song(&dir.join(AUDIO_FILE), cancel)?;
    if audio.rate != manifest.sample_rate || audio.frames != manifest.frames {
        return Err(SeparationError::Input(format!(
            "{AUDIO_FILE} is {} frames at {} Hz, the manifest says {} at {}",
            audio.frames, audio.rate, manifest.frames, manifest.sample_rate
        )));
    }
    let scratch = dir.join(WORK_DIR);
    remove_dir_if_present(&scratch)?;
    // Not create_dir_all: a song deleted meanwhile must not come back.
    fs::create_dir(&scratch)?;
    let writers = StemWriters::create(&scratch, audio.rate, audio.channels, audio.frames)?;
    progress(0.0);
    let mut output = Output { writers, progress };
    work.separate(&audio.left, &audio.right, stft, model, cancel, &mut output)?;
    drop(audio);
    output.writers.finish()?;
    if cancel.load(Ordering::Relaxed) {
        return Err(SeparationError::Cancelled);
    }
    install(library, id, &dir)
}

/// Swaps the finished `.separating/` in as `stems/` and marks all six ready.
fn install(library: &Library, id: &str, dir: &Path) -> Result<(), SeparationError> {
    let stems = dir.join(STEMS_DIR);
    let old = dir.join(OLD_STEMS_DIR);
    remove_dir_if_present(&old)?;
    let had_stems = stems.exists();
    if had_stems {
        fs::rename(&stems, &old)?;
    }
    if let Err(error) = fs::rename(dir.join(WORK_DIR), &stems) {
        if had_stems {
            let _ = fs::rename(&old, &stems);
        }
        return Err(error.into());
    }
    library.update(id, |manifest| manifest.stems = all_stems(StemStatus::Ready))?;
    if let Err(error) = remove_dir_if_present(&old) {
        log::warn!("cannot remove the previous stems of {id}: {error}");
    }
    Ok(())
}

/// True when `stems/` holds all six files in the song's format and length.
fn stems_complete(dir: &Path, manifest: &Manifest) -> bool {
    StemName::ALL.iter().all(|name| {
        hound::WavReader::open(dir.join(name.relative_path())).is_ok_and(|reader| {
            let spec = reader.spec();
            spec.sample_rate == manifest.sample_rate
                && spec.bits_per_sample == 16
                && spec.sample_format == hound::SampleFormat::Int
                && u64::from(reader.duration()) == manifest.frames
        })
    })
}

/// Puts every song back in a consistent state after the app stopped in the
/// middle of a separation: brings back previous stems a swap left aside,
/// deletes partial stems, and resolves `pending`/`running` entries no job
/// owns any more: `ready` if a finished swap's stems are all in place,
/// otherwise an empty list. Run once at start, before any job.
pub fn recover(library: &Library) {
    for manifest in library.list() {
        if let Err(error) = recover_song(library, &manifest) {
            log::warn!("cannot recover the stems of {}: {error}", manifest.id);
        }
    }
}

fn recover_song(library: &Library, manifest: &Manifest) -> Result<(), SeparationError> {
    let dir = library.song_dir(&manifest.id)?;
    let stems = dir.join(STEMS_DIR);
    let old = dir.join(OLD_STEMS_DIR);
    if old.exists() {
        if stems.exists() {
            remove_dir_if_present(&old)?;
        } else {
            fs::rename(&old, &stems)?;
        }
    }
    remove_dir_if_present(&dir.join(WORK_DIR))?;
    if in_progress(manifest) {
        let complete = stems_complete(&dir, manifest);
        library.update(&manifest.id, |manifest| {
            if in_progress(manifest) {
                manifest.stems = if complete {
                    all_stems(StemStatus::Ready)
                } else {
                    Vec::new()
                };
            }
        })?;
    }
    Ok(())
}

//! Stem engine: separates a library song into six stems with Demucs v4
//! `htdemucs_6s` running as a Core ML model inside the app, and writes them
//! where the stem player reads them (see AGENTS.md "스템 라이브러리 계약").
//!
//! - [`dsp`]: STFT/iSTFT, normalisation and chunking around the model (the
//!   numpy reference is `tools/stem-model/dsp.py`).
//! - [`StemModel`]: the model behind a trait; `CoreMlModel` on macOS,
//!   guarded by [`VerifiedModel`] against wrong GPU results.
//! - [`Workspace::separate`]: streaming chunked separation at 44.1 kHz.

mod audio_io;
#[cfg(target_os = "macos")]
mod coreml;
pub mod dsp;
mod engine;
mod model;
mod verify;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use crate::stems::StemName;

pub use audio_io::{SongAudio, StemWriters, read_song};
#[cfg(target_os = "macos")]
pub use coreml::{ComputeUnits, CoreMlModel};
pub use engine::{PLANES, SeparationOutput, Workspace};
pub use model::{
    MODEL_ENV, MODEL_NAME, SeparationError, StemModel, default_cache_dir, is_model, locate_model,
    model_candidates,
};
pub use verify::{FallbackLoader, MAX_ATTEMPTS, VerifiedModel, VerifyStats};

/// The model's output order (`htdemucs_6s` sources).
pub const MODEL_STEMS: [StemName; 6] = [
    StemName::Drums,
    StemName::Bass,
    StemName::Other,
    StemName::Vocals,
    StemName::Guitar,
    StemName::Piano,
];

/// Loads the first complete model among `candidates` (on macOS; elsewhere
/// the model is always missing): the GPU model, checked by
/// [`VerifiedModel`] with a CPU instance as its fallback.
pub fn load_model(candidates: &[PathBuf]) -> Result<Box<dyn StemModel>, SeparationError> {
    let path = locate_model(candidates).ok_or(SeparationError::ModelMissing)?;
    Ok(Box::new(load_verified(&path)?))
}

/// The GPU model at `path` behind [`VerifiedModel`], with a CPU fallback
/// loaded when first needed.
#[cfg(target_os = "macos")]
pub fn load_verified(path: &Path) -> Result<VerifiedModel, SeparationError> {
    log::info!("loading the stem model from {}", path.display());
    let gpu = CoreMlModel::load(path, ComputeUnits::Gpu)?;
    let cpu_path = path.to_path_buf();
    Ok(VerifiedModel::new(
        Box::new(gpu),
        Box::new(move || {
            log::info!("loading the CPU stem model");
            Ok(Box::new(CoreMlModel::load(&cpu_path, ComputeUnits::Cpu)?) as Box<dyn StemModel>)
        }),
    ))
}

#[cfg(not(target_os = "macos"))]
pub fn load_verified(_path: &Path) -> Result<VerifiedModel, SeparationError> {
    Err(SeparationError::ModelMissing)
}

//! The separation model behind a small trait, so the pipeline runs on the
//! Core ML model in the app and on fakes in tests, and where the compiled
//! model is found.

use std::path::{Path, PathBuf};

use crate::stems::LibraryError;

/// Directory name of the compiled model (`tools/stem-model/build.sh`).
pub const MODEL_NAME: &str = "htdemucs_6s_fp32.mlmodelc";
/// Overrides where the model is looked for first (tests, examples).
pub const MODEL_ENV: &str = "WOODSHED_STEM_MODEL";

#[derive(Debug, thiserror::Error)]
pub enum SeparationError {
    #[error("the stem model is not installed")]
    ModelMissing,
    #[error("the stem model failed: {0}")]
    Model(String),
    #[error("cannot read the song: {0}")]
    Input(String),
    #[error("cannot write the stems: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot write the stems: {0}")]
    Wav(#[from] hound::Error),
    #[error("{0}")]
    Library(#[from] LibraryError),
    #[error("cancelled")]
    Cancelled,
}

/// The htdemucs_6s core: one 7.8 s chunk in, six sources out.
///
/// Inputs: `mix` `[2][SEGMENT]` (the normalised chunk) and `spec`
/// `[4][FREQS][FRAMES]` (its STFT). Outputs: `spec_out`
/// `[24][FREQS][FRAMES]` and `wave_out` `[12][SEGMENT]` (see `dsp`), sources
/// in model order ([`super::MODEL_STEMS`]).
pub trait StemModel {
    fn predict(
        &mut self,
        mix: &[f32],
        spec: &[f32],
        spec_out: &mut [f32],
        wave_out: &mut [f32],
    ) -> Result<(), SeparationError>;
}

/// True for a complete compiled model (the build placeholder is an empty
/// directory).
pub fn is_model(path: &Path) -> bool {
    path.join("coremldata.bin").is_file() && path.join("model.mil").is_file()
}

/// Places to look for the model, best first: [`MODEL_ENV`], the bundled
/// resource, then the build cache (`~/Library/Caches/dev.mongmeo.woodshed/models`),
/// which dev builds use.
pub fn model_candidates(resource_dir: Option<&Path>, cache_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os(MODEL_ENV) {
        candidates.push(PathBuf::from(path));
    }
    if let Some(dir) = resource_dir {
        candidates.push(dir.join("models").join(MODEL_NAME));
    }
    if let Some(dir) = cache_dir {
        candidates.push(dir.join("models").join(MODEL_NAME));
    }
    candidates
}

/// The first complete model among `candidates`.
pub fn locate_model(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|path| is_model(path)).cloned()
}

/// The build cache directory of the app (`app_cache_dir` in Tauri), for code
/// that runs outside the app (tests, examples).
pub fn default_cache_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library/Caches")
            .join("dev.mongmeo.woodshed")
    })
}

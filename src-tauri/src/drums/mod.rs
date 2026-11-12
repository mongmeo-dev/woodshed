//! Drum machine: a sample kit, a step sequencer with swing, and the Tauri
//! commands of its screen.
//!
//! - [`Kit`]: the bundled CC0 samples, decoded at startup and resampled per
//!   output rate off the audio thread.
//! - [`DrumSource`]: the engine [`crate::audio::Source`] that plays them.
//! - [`Sequencer`]: schedules a [`Pattern`]'s hits sample-accurately and
//!   reschedules on edits and output changes without stopping.
//! - [`DrumControl`]: sequencer plus step announcements; follows output
//!   changes on every entry point.
//! - [`DrumService`]: Tauri state, scheduler thread, step channel, storage.

mod announce;
mod control;
mod kit;
mod pattern;
mod resample;
mod sequencer;
pub mod service;
mod source;
mod store;
mod wav;

#[cfg(test)]
mod tests;

pub use announce::{Announcer, HeardClock, HeardStep, StepEvent};
pub use control::{DrumControl, DrumEngine};
pub use kit::{Kit, KitSamples, TRACK_COUNT, Track};
pub use pattern::{
    LENGTHS, MAX_BPM, MAX_LEVEL, MAX_STEPS, MAX_SWING, MIN_BPM, MIN_SWING, Pattern, SwingUnit,
    TrackPattern, level_velocity, swung_position,
};
pub use resample::resample;
pub use sequencer::{ScheduledStep, Sequencer};
pub use service::{DrumService, DrumState, PatternStoreError};
pub use source::DrumSource;
pub use store::{PatternStore, SavedPattern};
pub use wav::{Decoded, WavError, decode_wav};

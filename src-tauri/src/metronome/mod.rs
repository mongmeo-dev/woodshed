//! Sample-accurate metronome.
//!
//! - [`pattern`]: what the user edits (tempo, meter, accents, subdivision,
//!   sounds, polyrhythm, tempo trainer).
//! - [`source`]: the click source; timbres are rendered in `prepare`, the
//!   audio thread only plays them back.
//! - [`timeline`]: pure beat generator with exact integer frame positions.
//! - [`scheduler`]: feeds the timeline into the engine ahead of time and
//!   handles changes and output switches.

pub mod pattern;
pub mod scheduler;
pub mod source;
pub mod timeline;

pub use pattern::{Accent, MetronomeSettings, Polyrhythm, Sound, Trainer, TrainerEnd};
pub use scheduler::{Announcement, Scheduler};
pub use source::MetronomeSource;
pub use timeline::{BeatMark, PulseMark, Timeline};

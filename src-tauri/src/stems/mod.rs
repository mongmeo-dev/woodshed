//! Stem splitter screen backend: the song library, file and YouTube import,
//! and the multi-track player (see AGENTS.md "스템 라이브러리 계약").
//!
//! - [`StemPlayer`]: an engine [`crate::audio::Source`] that plays every track
//!   of a song in sample lockstep, plus the feeder that streams them from disk.

mod decode;
mod feeder;
mod import;
mod library;
mod player;
mod reader;
mod source;
mod track;
mod youtube;

#[cfg(test)]
mod tests;

pub use decode::{DecodeError, DecodedAudio, WAVEFORM_BINS, decode_to_wav, wav_waveform};
pub use feeder::{BLOCK_FRAMES, BYPASS_SPEED, Feeder, MAX_TRACKS, MIN_SPEED, POOL_BLOCKS};
pub use import::{
    AUDIO_EXTENSIONS, ImportError, ImportProgress, ImportStage, import_file, import_link,
};
pub use library::{
    Library, LibraryError, Manifest, SongSource, StemEntry, StemName, StemStatus, TrackName,
};
pub use player::{FeederThread, PlayerError, StemPlayer};
pub use reader::LoopRange;
pub use source::{PlayerShared, StemSource, loop_events, param};
pub use track::{MemoryTrack, TrackError, TrackReader, WavTrack};
pub use youtube::{ProcessControl, Tools, YoutubeLink, parse_youtube_link};

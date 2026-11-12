//! Stem splitter screen backend: the song library, file and YouTube import,
//! and the multi-track player (see AGENTS.md "스템 라이브러리 계약").

mod decode;
mod track;

pub use decode::{DecodeError, DecodedAudio, WAVEFORM_BINS, decode_to_wav, wav_waveform};
pub use track::{MemoryTrack, TrackError, TrackReader, WavTrack};

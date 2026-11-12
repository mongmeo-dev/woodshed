//! Plain data types shared by the engine, its handle and the app layer.

use serde::{Deserialize, Serialize};

/// Absolute position on the engine's sample clock, counted in frames since the
/// engine was created. It never resets, not even when the output device changes.
pub type FramePos = u64;

/// Largest number of frames the engine renders in one internal block. Device
/// callbacks larger than this are split, so sources never see more frames.
pub const MAX_BLOCK_FRAMES: usize = 1024;

/// Largest interleaved channel count the engine renders.
pub const MAX_CHANNELS: usize = 8;

/// Identifies a source attached to the engine's mixer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceId(pub u32);

/// Format the engine renders in. Sources receive it in [`crate::audio::Source::prepare`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamFormat {
    pub sample_rate: u32,
    pub channels: u16,
    /// Upper bound of `frames` in a single `Source::render` call.
    pub max_block_frames: usize,
}

impl StreamFormat {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            sample_rate,
            channels,
            max_block_frames: MAX_BLOCK_FRAMES,
        }
    }

    /// Converts a duration in seconds to a whole number of frames (rounded).
    pub fn frames_for_secs(&self, secs: f64) -> u64 {
        (secs * f64::from(self.sample_rate)).round().max(0.0) as u64
    }
}

/// What a scheduled event asks its target source to do. `Copy` and small so it
/// can travel through the lock-free queue without allocating.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EventKind {
    /// Start a one-shot sound (metronome click, drum hit). `note` picks the
    /// voice or pitch, `velocity` is 0.0..=1.0.
    Trigger { note: u8, velocity: f32 },
    /// Start continuous playback (stem player).
    Start,
    /// Stop continuous playback.
    Stop,
    /// Jump to a position inside the source's own material, in frames.
    Seek { frame: u64 },
    /// Set a source-defined parameter.
    Param { id: u16, value: f32 },
}

/// An event placed on the sample clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScheduledEvent {
    /// Absolute frame at which the event takes effect.
    pub frame: FramePos,
    pub target: SourceId,
    pub kind: EventKind,
    /// When true the engine reports the event after rendering it, so the UI can
    /// learn when it becomes audible (see [`RenderedEvent`]).
    pub notify: bool,
}

/// Report of an event the audio thread has rendered (only for `notify` events).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderedEvent {
    /// Frame the event was scheduled for.
    pub scheduled_frame: FramePos,
    /// Frame it was actually rendered at. Equal to `scheduled_frame` unless the
    /// event arrived after its frame had already been rendered.
    pub rendered_frame: FramePos,
    pub target: SourceId,
    pub kind: EventKind,
    /// Output generation the event was rendered in.
    pub generation: u64,
    /// Stream-clock time (ns) at which `rendered_frame` reaches the DAC,
    /// captured when it was rendered. Only meaningful for `generation`.
    pub playback_ns: u64,
}

/// Timing of the most recent device callback, on the stream's own clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallbackTiming {
    /// When the callback was invoked, in nanoseconds on the stream clock.
    pub callback_ns: u64,
    /// When the first frame of this callback reaches the DAC, same clock.
    pub playback_ns: u64,
}

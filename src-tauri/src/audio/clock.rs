//! Maps engine frames to wall-clock time, so the UI can draw something at the
//! moment a sound is actually heard.
//!
//! The audio thread publishes, per callback, which frame it started at and
//! when that frame reaches the DAC on the stream clock ([`TimingSnapshot`]).
//! A normal thread reads the stream clock and the system clock back to back
//! ([`ClockPair`]) and combines both into a Unix-epoch time in milliseconds.
//! The webview turns that into its own timeline with
//! `epochMs - performance.timeOrigin`.

use std::time::{SystemTime, UNIX_EPOCH};

use super::shared::TimingSnapshot;
use super::types::FramePos;

/// The stream clock and the system clock sampled at (nearly) the same moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClockPair {
    pub stream_now_ns: u64,
    pub epoch_now_ms: f64,
}

impl ClockPair {
    /// Pairs `stream_now_ns` with the current system time.
    pub fn now(stream_now_ns: u64) -> Self {
        let epoch_now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64() * 1_000.0)
            .unwrap_or(0.0);
        Self {
            stream_now_ns,
            epoch_now_ms,
        }
    }
}

/// Unix-epoch milliseconds at which `frame` is heard.
pub fn heard_at_epoch_ms(snapshot: &TimingSnapshot, clock: ClockPair, frame: FramePos) -> f64 {
    let offset_ns = snapshot.playback_ns_of(frame) - i128::from(clock.stream_now_ns);
    clock.epoch_now_ms + offset_ns as f64 / 1_000_000.0
}

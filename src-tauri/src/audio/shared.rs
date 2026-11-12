//! State the audio thread publishes for other threads, using atomics only.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};

use super::types::{CallbackTiming, FramePos};

/// Snapshot of the clock at the start of the latest rendered callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimingSnapshot {
    /// Output generation the callback belongs to (see [`SharedState::generation`]).
    pub generation: u64,
    /// Engine frame at the start of the callback.
    pub block_frame: FramePos,
    pub callback_ns: u64,
    /// Stream-clock time at which `block_frame` reaches the DAC.
    pub playback_ns: u64,
    /// Sample rate the callback rendered at.
    pub sample_rate: u32,
}

impl TimingSnapshot {
    /// Output latency of the stream: callback to DAC.
    pub fn output_latency_ns(&self) -> u64 {
        self.playback_ns.saturating_sub(self.callback_ns)
    }

    /// Stream-clock time (ns) at which `frame` reaches the DAC. Valid for
    /// frames before and after `block_frame` within the same generation.
    pub fn playback_ns_of(&self, frame: FramePos) -> i128 {
        let delta_frames = i128::from(frame) - i128::from(self.block_frame);
        i128::from(self.playback_ns)
            + delta_frames * 1_000_000_000 / i128::from(self.sample_rate.max(1))
    }
}

/// Counters and clock values written by the audio thread.
#[derive(Debug, Default)]
pub struct SharedState {
    sample_rate: AtomicU32,
    /// Bumped every time the render format / output stream changes.
    generation: AtomicU64,
    frames_rendered: AtomicU64,
    callbacks: AtomicU64,
    max_callback_frames: AtomicU32,
    late_events: AtomicU64,
    dropped_events: AtomicU64,
    // Seqlock-protected timing snapshot.
    seq: AtomicU64,
    timing_generation: AtomicU64,
    timing_rate: AtomicU32,
    block_frame: AtomicU64,
    callback_ns: AtomicU64,
    playback_ns: AtomicU64,
}

/// Engine counters, for status display and diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStats {
    pub generation: u64,
    pub frames_rendered: FramePos,
    pub callbacks: u64,
    pub max_callback_frames: u32,
    /// Events that reached the engine after their frame had been rendered.
    pub late_events: u64,
    /// Events dropped because the schedule was full.
    pub dropped_events: u64,
}

impl SharedState {
    pub fn new(sample_rate: u32) -> Self {
        let state = Self::default();
        state.sample_rate.store(sample_rate, Ordering::Relaxed);
        state.generation.store(1, Ordering::Relaxed);
        state
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate.load(Ordering::Relaxed)
    }

    /// Current output generation. Starts at 1 and increases each time the
    /// engine is reformatted for a (re)opened output stream. Timing snapshots
    /// and rendered-event reports from older generations are stale.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Starts a new generation with `sample_rate`. Timing reads return `None`
    /// until the first callback of the new generation.
    pub(crate) fn begin_generation(&self, sample_rate: u32) -> u64 {
        self.sample_rate.store(sample_rate, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Frames rendered so far. Events scheduled at or after this frame (plus
    /// one callback of margin) will be rendered exactly on time.
    pub fn frames_rendered(&self) -> FramePos {
        self.frames_rendered.load(Ordering::Acquire)
    }

    pub(crate) fn publish_rendered(&self, frame: FramePos, callback_frames: u32) {
        self.frames_rendered.store(frame, Ordering::Release);
        self.callbacks.fetch_add(1, Ordering::Relaxed);
        self.max_callback_frames
            .fetch_max(callback_frames, Ordering::Relaxed);
    }

    pub(crate) fn count_late(&self) {
        self.late_events.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn count_dropped_event(&self) {
        self.dropped_events.fetch_add(1, Ordering::Relaxed);
    }

    pub fn stats(&self) -> EngineStats {
        EngineStats {
            generation: self.generation(),
            frames_rendered: self.frames_rendered(),
            callbacks: self.callbacks.load(Ordering::Relaxed),
            max_callback_frames: self.max_callback_frames.load(Ordering::Relaxed),
            late_events: self.late_events.load(Ordering::Relaxed),
            dropped_events: self.dropped_events.load(Ordering::Relaxed),
        }
    }

    /// Writer side of the seqlock. Only the audio thread calls this.
    pub(crate) fn publish_timing(
        &self,
        generation: u64,
        sample_rate: u32,
        block_frame: FramePos,
        timing: CallbackTiming,
    ) {
        let seq = self.seq.load(Ordering::Relaxed);
        self.seq.store(seq.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        self.timing_generation.store(generation, Ordering::Relaxed);
        self.timing_rate.store(sample_rate, Ordering::Relaxed);
        self.block_frame.store(block_frame, Ordering::Relaxed);
        self.callback_ns
            .store(timing.callback_ns, Ordering::Relaxed);
        self.playback_ns
            .store(timing.playback_ns, Ordering::Relaxed);
        self.seq.store(seq.wrapping_add(2), Ordering::Release);
    }

    /// Reader side of the seqlock. `None` until the first callback of the
    /// current generation has run.
    pub fn timing(&self) -> Option<TimingSnapshot> {
        let generation = self.generation();
        loop {
            let before = self.seq.load(Ordering::Acquire);
            if before == 0 {
                return None;
            }
            if before % 2 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let snapshot = TimingSnapshot {
                generation: self.timing_generation.load(Ordering::Relaxed),
                sample_rate: self.timing_rate.load(Ordering::Relaxed),
                block_frame: self.block_frame.load(Ordering::Relaxed),
                callback_ns: self.callback_ns.load(Ordering::Relaxed),
                playback_ns: self.playback_ns.load(Ordering::Relaxed),
            };
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == before {
                return (snapshot.generation == generation).then_some(snapshot);
            }
        }
    }
}

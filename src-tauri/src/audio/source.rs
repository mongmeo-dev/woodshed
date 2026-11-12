//! The [`Source`] trait every sound generator implements, plus small built-in
//! sources used by the shell and by tests.

use super::types::{EventKind, StreamFormat};

/// A sound generator attached to the engine's mixer (metronome, drum machine,
/// stem player, ...).
///
/// Threading contract:
/// - [`Source::prepare`] runs on a normal thread before the source is attached
///   and again when the output format changes. Allocate buffers here.
/// - [`Source::on_event`] and [`Source::render`] run on the audio thread. They
///   must not allocate, free, lock, block, log or make system calls.
pub trait Source: Send + 'static {
    /// Called off the audio thread with the format the engine renders in.
    fn prepare(&mut self, format: StreamFormat) {
        let _ = format;
    }

    /// Applies a scheduled event. The engine calls this exactly at the event's
    /// frame: frames rendered before this call precede the event, frames
    /// rendered after it follow it.
    fn on_event(&mut self, kind: &EventKind);

    /// Renders `out.len() / channels` interleaved frames. `out` arrives zeroed;
    /// the engine applies the source gain and mixes the result.
    fn render(&mut self, out: &mut [f32], channels: usize);
}

/// Short sine blip with an exponential decay. The settings screen uses it as
/// the output test sound.
pub struct ClickSource {
    frequency: f32,
    decay_secs: f32,
    sample_rate: f32,
    phase: f32,
    phase_step: f32,
    envelope: f32,
    decay_per_frame: f32,
}

impl ClickSource {
    pub fn new(frequency: f32, decay_secs: f32) -> Self {
        Self {
            frequency,
            decay_secs,
            sample_rate: 48_000.0,
            phase: 0.0,
            phase_step: 0.0,
            envelope: 0.0,
            decay_per_frame: 0.0,
        }
    }
}

impl Source for ClickSource {
    fn prepare(&mut self, format: StreamFormat) {
        self.sample_rate = format.sample_rate as f32;
        self.phase_step = std::f32::consts::TAU * self.frequency / self.sample_rate;
        // Envelope reaches -60 dB after `decay_secs`.
        self.decay_per_frame = (0.001_f32.ln() / (self.decay_secs * self.sample_rate)).exp();
    }

    fn on_event(&mut self, kind: &EventKind) {
        match *kind {
            EventKind::Trigger { velocity, .. } => {
                self.phase = 0.0;
                self.envelope = velocity.clamp(0.0, 1.0);
            }
            EventKind::Stop => self.envelope = 0.0,
            _ => {}
        }
    }

    fn render(&mut self, out: &mut [f32], channels: usize) {
        if self.envelope < 1.0e-5 {
            return;
        }
        for frame in out.chunks_exact_mut(channels) {
            let sample = self.phase.sin() * self.envelope;
            frame.fill(sample);
            self.phase += self.phase_step;
            if self.phase > std::f32::consts::TAU {
                self.phase -= std::f32::consts::TAU;
            }
            self.envelope *= self.decay_per_frame;
        }
    }
}

/// Sources meant for render tests (this crate's and later tool nodes').
pub mod testing {
    use super::{EventKind, Source};

    /// Writes `velocity` to every channel of exactly one frame: the frame at
    /// which a `Trigger` event lands. Everything else stays silent, so the
    /// output shows precisely where the engine rendered the event.
    #[derive(Default)]
    pub struct ImpulseSource {
        pending: Option<f32>,
    }

    impl Source for ImpulseSource {
        fn on_event(&mut self, kind: &EventKind) {
            if let EventKind::Trigger { velocity, .. } = *kind {
                self.pending = Some(velocity);
            }
        }

        fn render(&mut self, out: &mut [f32], channels: usize) {
            if let Some(value) = self.pending.take()
                && let Some(frame) = out.chunks_exact_mut(channels).next()
            {
                frame.fill(value);
            }
        }
    }

    /// Outputs a constant value on every channel while started.
    pub struct ConstantSource {
        value: f32,
        playing: bool,
    }

    impl ConstantSource {
        pub fn new(value: f32) -> Self {
            Self {
                value,
                playing: false,
            }
        }
    }

    impl Source for ConstantSource {
        fn on_event(&mut self, kind: &EventKind) {
            match kind {
                EventKind::Start => self.playing = true,
                EventKind::Stop => self.playing = false,
                _ => {}
            }
        }

        fn render(&mut self, out: &mut [f32], _channels: usize) {
            if self.playing {
                out.fill(self.value);
            }
        }
    }
}

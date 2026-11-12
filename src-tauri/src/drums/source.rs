//! The drum machine's sound source: plays kit samples on `Trigger` events.
//!
//! - `Trigger { note: track index, velocity }` starts the track's sample.
//!   Retriggering a track fades the sound still ringing on it over a few
//!   milliseconds instead of cutting it (no click), and a closed hi-hat
//!   chokes a ringing open hi-hat.
//! - `Param { id: track index, value }` sets the track's gain (volume or 0
//!   when muted), gliding over a few milliseconds.

use std::sync::Arc;

use crate::audio::{EventKind, Source, StreamFormat};

use super::kit::{Kit, KitSamples, TRACK_COUNT, Track};

/// Fade applied to a track's previous sound when it is hit again.
const RETRIGGER_FADE_SECS: f32 = 0.003;
/// Fade applied to a ringing open hi-hat when the closed hi-hat is hit.
const CHOKE_FADE_SECS: f32 = 0.012;
/// Track gain changes glide over roughly this long.
const GAIN_GLIDE_SECS: f32 = 0.005;
/// Sounds that can overlap on one track (the new hit and a fading one).
const VOICES_PER_TRACK: usize = 2;

#[derive(Clone, Copy, Debug, Default)]
struct Voice {
    active: bool,
    position: usize,
    velocity: f32,
    /// 1.0 while playing normally; ramps down by `fade_step` per frame.
    fade: f32,
    fade_step: f32,
}

impl Voice {
    fn fade_out(&mut self, step: f32) {
        if self.active {
            self.fade_step = self.fade_step.max(step);
        }
    }
}

pub struct DrumSource {
    kit: Arc<Kit>,
    samples: Arc<KitSamples>,
    voices: [[Voice; VOICES_PER_TRACK]; TRACK_COUNT],
    gains: [f32; TRACK_COUNT],
    target_gains: [f32; TRACK_COUNT],
    glide: f32,
    retrigger_step: f32,
    choke_step: f32,
}

impl DrumSource {
    /// A source playing `kit`, silent until [`Source::prepare`] picks the
    /// samples for the output rate. Track gains start at 1.0 until the
    /// sequencer sets them.
    pub fn new(kit: Arc<Kit>) -> Self {
        Self {
            kit,
            samples: Arc::new(std::array::from_fn(|_| Vec::new())),
            voices: [[Voice::default(); VOICES_PER_TRACK]; TRACK_COUNT],
            gains: [1.0; TRACK_COUNT],
            target_gains: [1.0; TRACK_COUNT],
            glide: 1.0,
            retrigger_step: 1.0,
            choke_step: 1.0,
        }
    }

    fn trigger(&mut self, track: Track, velocity: f32) {
        let retrigger_step = self.retrigger_step;
        let voices = &mut self.voices[track.index()];
        for voice in voices.iter_mut() {
            voice.fade_out(retrigger_step);
        }
        // Reuse a free voice, else the quieter of the two fading ones.
        let slot = voices
            .iter()
            .position(|voice| !voice.active)
            .unwrap_or_else(|| usize::from(voices[1].fade < voices[0].fade));
        voices[slot] = Voice {
            active: true,
            position: 0,
            velocity: velocity.clamp(0.0, 1.0),
            fade: 1.0,
            fade_step: 0.0,
        };
        if track == Track::ClosedHat {
            let choke_step = self.choke_step;
            for voice in &mut self.voices[Track::OpenHat.index()] {
                voice.fade_out(choke_step);
            }
        }
    }

    fn set_gain(&mut self, index: usize, gain: f32) {
        let gain = if gain.is_finite() {
            gain.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.target_gains[index] = gain;
        if self.voices[index].iter().all(|voice| !voice.active) {
            // Nothing is sounding: no need to glide.
            self.gains[index] = gain;
        }
    }
}

fn frames_for(secs: f32, sample_rate: f32) -> f32 {
    (secs * sample_rate).max(1.0)
}

impl Source for DrumSource {
    fn prepare(&mut self, format: StreamFormat) {
        let rate = format.sample_rate as f32;
        self.samples = self.kit.at_rate(format.sample_rate);
        self.glide = 1.0 - (-1.0 / frames_for(GAIN_GLIDE_SECS, rate)).exp();
        self.retrigger_step = 1.0 / frames_for(RETRIGGER_FADE_SECS, rate);
        self.choke_step = 1.0 / frames_for(CHOKE_FADE_SECS, rate);
        // Positions were counted at the old rate; drop what was ringing.
        self.voices = [[Voice::default(); VOICES_PER_TRACK]; TRACK_COUNT];
        self.gains = self.target_gains;
    }

    fn on_event(&mut self, kind: &EventKind) {
        match *kind {
            EventKind::Trigger { note, velocity } => {
                if let Some(track) = Track::from_index(usize::from(note)) {
                    self.trigger(track, velocity);
                }
            }
            EventKind::Param { id, value } if usize::from(id) < TRACK_COUNT => {
                self.set_gain(usize::from(id), value);
            }
            _ => {}
        }
    }

    fn render(&mut self, out: &mut [f32], channels: usize) {
        let frames = out.len() / channels;
        let samples = &*self.samples;
        for (index, voices) in self.voices.iter_mut().enumerate() {
            if voices.iter().all(|voice| !voice.active) {
                self.gains[index] = self.target_gains[index];
                continue;
            }
            let sample = &samples[index];
            let mix = Track::ALL[index].mix_gain();
            let target = self.target_gains[index];
            let mut gain = self.gains[index];
            for frame in out.chunks_exact_mut(channels).take(frames) {
                gain += (target - gain) * self.glide;
                let mut value = 0.0;
                for voice in voices.iter_mut().filter(|voice| voice.active) {
                    let Some(&amplitude) = sample.get(voice.position) else {
                        voice.active = false;
                        continue;
                    };
                    value += amplitude * voice.velocity * voice.fade;
                    voice.position += 1;
                    if voice.fade_step > 0.0 {
                        voice.fade -= voice.fade_step;
                        if voice.fade <= 0.0 {
                            voice.active = false;
                        }
                    }
                }
                let value = value * gain * mix;
                for out in frame {
                    *out += value;
                }
            }
            self.gains[index] = if (target - gain).abs() < 1.0e-4 {
                target
            } else {
                gain
            };
        }
    }
}

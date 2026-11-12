//! The metronome's sound source: every click timbre is synthesized once in
//! [`Source::prepare`] (a normal thread), and the audio thread only plays
//! those tables back through a fixed voice pool, so it never allocates.

use std::f32::consts::TAU;

use crate::audio::{EventKind, Source, StreamFormat};

use super::pattern::Sound;

/// Pitch variant of a sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// Accents and the polyrhythm layer.
    High,
    /// Regular beats and subdivisions.
    Low,
}

/// Number of distinct `Trigger` notes: every sound in both variants.
pub const NOTE_COUNT: usize = Sound::ALL.len() * 2;

/// The `Trigger` note that plays `sound` in `variant`.
pub fn note(sound: Sound, variant: Variant) -> u8 {
    sound.index() * 2
        + match variant {
            Variant::High => 0,
            Variant::Low => 1,
        }
}

/// Voices that can ring at once. The oldest is reused when all are busy
/// (only reachable with long tails at very fast tempos).
pub const MAX_VOICES: usize = 16;

#[derive(Clone, Copy, Debug, Default)]
struct Voice {
    table: usize,
    position: usize,
    gain: f32,
    active: bool,
}

/// Plays pre-rendered click tables on `Trigger { note, velocity }`.
pub struct MetronomeSource {
    sample_rate: u32,
    tables: Vec<Vec<f32>>,
    voices: [Voice; MAX_VOICES],
}

impl Default for MetronomeSource {
    fn default() -> Self {
        Self::new()
    }
}

impl MetronomeSource {
    pub fn new() -> Self {
        Self {
            sample_rate: 0,
            tables: Vec::new(),
            voices: [Voice::default(); MAX_VOICES],
        }
    }

    fn start_voice(&mut self, table: usize, gain: f32) {
        let slot = match self.voices.iter().position(|voice| !voice.active) {
            Some(free) => free,
            None => {
                // All busy: steal the one that has played longest.
                let mut oldest = 0;
                for (index, voice) in self.voices.iter().enumerate() {
                    if voice.position > self.voices[oldest].position {
                        oldest = index;
                    }
                }
                oldest
            }
        };
        self.voices[slot] = Voice {
            table,
            position: 0,
            gain,
            active: true,
        };
    }
}

impl Source for MetronomeSource {
    fn prepare(&mut self, format: StreamFormat) {
        if self.sample_rate != format.sample_rate || self.tables.is_empty() {
            self.sample_rate = format.sample_rate;
            self.tables = render_tables(format.sample_rate);
        }
        // Positions refer to the old tables; nothing is mid-sound across a
        // format change anyway (the stream is stopped while preparing).
        self.voices = [Voice::default(); MAX_VOICES];
    }

    fn on_event(&mut self, kind: &EventKind) {
        match *kind {
            EventKind::Trigger { note, velocity } => {
                let table = usize::from(note);
                let gain = velocity.clamp(0.0, 1.0);
                if table < self.tables.len() && gain > 0.0 {
                    self.start_voice(table, gain);
                }
            }
            EventKind::Stop => {
                for voice in &mut self.voices {
                    voice.active = false;
                }
            }
            _ => {}
        }
    }

    fn render(&mut self, out: &mut [f32], channels: usize) {
        let frames = out.len() / channels;
        for voice in &mut self.voices {
            if !voice.active {
                continue;
            }
            let table = &self.tables[voice.table];
            let available = table.len() - voice.position;
            let count = available.min(frames);
            let samples = &table[voice.position..voice.position + count];
            for (frame, &sample) in out.chunks_exact_mut(channels).zip(samples) {
                let value = sample * voice.gain;
                for channel in frame {
                    *channel += value;
                }
            }
            voice.position += count;
            if voice.position >= table.len() {
                voice.active = false;
            }
        }
    }
}

/// Renders every note's table at `sample_rate`, indexed by [`note`].
pub fn render_tables(sample_rate: u32) -> Vec<Vec<f32>> {
    let mut tables = vec![Vec::new(); NOTE_COUNT];
    for sound in Sound::ALL {
        for variant in [Variant::High, Variant::Low] {
            tables[usize::from(note(sound, variant))] = synthesize(sound, variant, sample_rate);
        }
    }
    tables
}

/// One damped partial: `amplitude * cos(2π f t) * e^(-t/decay)`.
struct Partial {
    frequency: f32,
    amplitude: f32,
    decay_secs: f32,
}

/// Synthesizes one click. Every table starts with a non-zero sample (cosine
/// phase and an attack ramp that begins above zero), so a click's first
/// frame is exactly its scheduled frame, and ends at a quiet tail.
pub fn synthesize(sound: Sound, variant: Variant, sample_rate: u32) -> Vec<f32> {
    let high = variant == Variant::High;
    let (partials, attack_secs, length_secs, hold_secs): (Vec<Partial>, f32, f32, f32) = match sound
    {
        Sound::Click => {
            let f = if high { 2_600.0 } else { 1_900.0 };
            (
                vec![
                    Partial {
                        frequency: f,
                        amplitude: 1.0,
                        decay_secs: 0.004,
                    },
                    Partial {
                        frequency: f * 2.3,
                        amplitude: 0.35,
                        decay_secs: 0.002,
                    },
                ],
                0.000_2,
                0.03,
                0.0,
            )
        }
        Sound::Woodblock => {
            let f = if high { 1_250.0 } else { 940.0 };
            (
                vec![
                    Partial {
                        frequency: f,
                        amplitude: 1.0,
                        decay_secs: 0.018,
                    },
                    Partial {
                        frequency: f * 2.71,
                        amplitude: 0.3,
                        decay_secs: 0.008,
                    },
                ],
                0.000_5,
                0.12,
                0.0,
            )
        }
        Sound::Beep => {
            let f = if high { 1_320.0 } else { 880.0 };
            (
                vec![Partial {
                    frequency: f,
                    amplitude: 1.0,
                    decay_secs: 1.0,
                }],
                0.002,
                0.075,
                0.055,
            )
        }
        Sound::Cowbell => {
            // Two inharmonic square-ish partials, in the classic 587:845 ratio.
            let scale = if high { 1.25 } else { 1.0 };
            let (a, b) = (587.0 * scale, 845.0 * scale);
            (
                vec![
                    Partial {
                        frequency: a,
                        amplitude: 1.0,
                        decay_secs: 0.09,
                    },
                    Partial {
                        frequency: b,
                        amplitude: 0.8,
                        decay_secs: 0.07,
                    },
                    Partial {
                        frequency: a * 3.0,
                        amplitude: 0.25,
                        decay_secs: 0.03,
                    },
                    Partial {
                        frequency: b * 3.0,
                        amplitude: 0.2,
                        decay_secs: 0.03,
                    },
                ],
                0.000_5,
                0.3,
                0.0,
            )
        }
    };

    let rate = sample_rate.max(1) as f32;
    let frames = ((length_secs * rate) as usize).max(2);
    let attack = ((attack_secs * rate) as usize).max(1);
    let hold = (hold_secs * rate) as usize;
    let release = frames.saturating_sub(hold).max(1);
    let mut table = Vec::with_capacity(frames);
    for n in 0..frames {
        let t = n as f32 / rate;
        let mut value = 0.0;
        for partial in &partials {
            value += partial.amplitude
                * (TAU * partial.frequency * t).cos()
                * (-t / partial.decay_secs).exp();
        }
        let attack_gain = ((n + 1) as f32 / attack as f32).min(1.0);
        // Beep: flat hold, then a linear release; others: fade the last 20%
        // so every table ends at exactly zero without a step.
        let tail_gain = if hold > 0 {
            if n < hold {
                1.0
            } else {
                1.0 - (n - hold) as f32 / release as f32
            }
        } else {
            let fade_start = frames * 4 / 5;
            if n < fade_start {
                1.0
            } else {
                1.0 - (n - fade_start) as f32 / (frames - fade_start) as f32
            }
        };
        table.push(value * attack_gain * tail_gain);
    }
    normalize(&mut table, 0.8);
    table
}

fn normalize(table: &mut [f32], peak: f32) {
    let max = table.iter().fold(0.0_f32, |max, &s| max.max(s.abs()));
    if max > 0.0 {
        let scale = peak / max;
        for sample in table {
            *sample *= scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trigger(note: u8, velocity: f32) -> EventKind {
        EventKind::Trigger { note, velocity }
    }

    #[test]
    fn every_table_starts_non_zero_and_stays_in_range() {
        for rate in [44_100, 48_000, 96_000] {
            let tables = render_tables(rate);
            assert_eq!(tables.len(), NOTE_COUNT);
            for (index, table) in tables.iter().enumerate() {
                assert!(table[0] != 0.0, "note {index} at {rate} Hz starts at zero");
                assert!(table.iter().all(|s| s.abs() <= 0.8 + 1e-6));
                assert!(
                    table.last().unwrap().abs() < 1e-3,
                    "note {index} ends quietly"
                );
                // The longest tail (cowbell) is 300 ms.
                assert!(table.len() <= (0.3 * rate as f32) as usize + 1);
            }
        }
    }

    #[test]
    fn variants_and_sounds_are_distinct() {
        let tables = render_tables(48_000);
        for a in 0..NOTE_COUNT {
            for b in a + 1..NOTE_COUNT {
                assert_ne!(tables[a], tables[b], "notes {a} and {b}");
            }
        }
    }

    #[test]
    fn a_trigger_plays_its_table_scaled_by_velocity() {
        let mut source = MetronomeSource::new();
        source.prepare(StreamFormat::new(48_000, 2));
        let note = note(Sound::Woodblock, Variant::Low);
        source.on_event(&trigger(note, 0.5));
        let mut out = vec![0.0; 64 * 2];
        source.render(&mut out, 2);
        let table = &render_tables(48_000)[usize::from(note)];
        for (frame, samples) in out.as_chunks::<2>().0.iter().enumerate() {
            assert_eq!(samples[0], table[frame] * 0.5);
            assert_eq!(samples[1], samples[0]);
        }
    }

    #[test]
    fn overlapping_triggers_mix_and_unknown_notes_are_ignored() {
        let mut source = MetronomeSource::new();
        source.prepare(StreamFormat::new(48_000, 1));
        source.on_event(&trigger(99, 1.0));
        source.on_event(&trigger(0, 0.0));
        let mut silent = vec![0.0; 32];
        source.render(&mut silent, 1);
        assert!(silent.iter().all(|&s| s == 0.0));

        source.on_event(&trigger(0, 1.0));
        source.on_event(&trigger(1, 1.0));
        let mut out = vec![0.0; 1];
        source.render(&mut out, 1);
        let tables = render_tables(48_000);
        assert_eq!(out[0], tables[0][0] + tables[1][0]);
    }

    #[test]
    fn a_full_pool_steals_the_oldest_voice() {
        let mut source = MetronomeSource::new();
        source.prepare(StreamFormat::new(48_000, 1));
        let cowbell = note(Sound::Cowbell, Variant::Low);
        for _ in 0..MAX_VOICES + 4 {
            source.on_event(&trigger(cowbell, 1.0));
            let mut out = vec![0.0; 16];
            source.render(&mut out, 1);
        }
        assert!(source.voices.iter().all(|voice| voice.active));
        assert!(source.voices.iter().any(|voice| voice.position == 16));
    }

    #[test]
    fn voices_end_after_their_table() {
        let mut source = MetronomeSource::new();
        source.prepare(StreamFormat::new(48_000, 1));
        source.on_event(&trigger(note(Sound::Click, Variant::High), 1.0));
        let mut out = vec![0.0; 48_000 / 10];
        source.render(&mut out, 1);
        assert!(source.voices.iter().all(|voice| !voice.active));
        let len = render_tables(48_000)[0].len();
        assert!(out[len..].iter().all(|&s| s == 0.0));
    }
}

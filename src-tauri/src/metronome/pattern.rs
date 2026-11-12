//! What the user edits and presets store: tempo, meter, accents, sounds,
//! the polyrhythm layer and the tempo trainer.
//!
//! Tempo is kept in tenths of a BPM internally so every computation is exact
//! integer arithmetic. One beat is one note of the meter's denominator (6/8
//! clicks six eighth notes per bar; the BPM counts those eighths).

use serde::{Deserialize, Serialize};

/// 20.0 BPM.
pub const MIN_BPM_TENTHS: u32 = 200;
/// 400.0 BPM.
pub const MAX_BPM_TENTHS: u32 = 4_000;
pub const DEFAULT_BPM_TENTHS: u32 = 1_200;
/// Largest meter numerator.
pub const MAX_BEATS: u8 = 16;
/// Allowed meter denominators.
pub const NOTE_VALUES: [u8; 6] = [1, 2, 4, 8, 16, 32];
/// Largest subdivision (clicks per beat, 1 = none).
pub const MAX_SUBDIVISION: u8 = 8;
/// Largest number on either side of a polyrhythm ratio.
pub const MAX_POLY: u8 = 16;
/// Largest trainer interval in bars.
pub const MAX_TRAINER_BARS: u16 = 64;
/// Trainer step range in tenths of a BPM (0.1 ..= 50.0).
pub const MAX_TRAINER_STEP_TENTHS: u32 = 500;

/// How strongly a beat is played.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Accent {
    Strong,
    #[default]
    Normal,
    Weak,
    /// Silent, but still counted and shown.
    Mute,
}

/// Click timbre. Each one has a high variant (accents, polyrhythm) and a low
/// variant (regular beats, subdivisions).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Sound {
    #[default]
    Click,
    Woodblock,
    Beep,
    Cowbell,
}

impl Sound {
    pub const ALL: [Sound; 4] = [Sound::Click, Sound::Woodblock, Sound::Beep, Sound::Cowbell];

    pub fn index(self) -> u8 {
        match self {
            Sound::Click => 0,
            Sound::Woodblock => 1,
            Sound::Beep => 2,
            Sound::Cowbell => 3,
        }
    }
}

/// Second layer: `pulses` evenly spaced clicks over every `beats` main beats
/// (3:2 = three clicks in the time of two beats).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Polyrhythm {
    pub enabled: bool,
    pub pulses: u8,
    pub beats: u8,
    pub sound: Sound,
}

impl Default for Polyrhythm {
    fn default() -> Self {
        Self {
            enabled: false,
            pulses: 3,
            beats: 2,
            sound: Sound::Woodblock,
        }
    }
}

/// What the trainer does once the tempo has reached `max_bpm` and the bars of
/// that step are played.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrainerEnd {
    /// Keep playing at the maximum tempo.
    #[default]
    Hold,
    /// Stop playback.
    Stop,
}

/// Raises the tempo by `step_bpm` every `every_bars` bars, up to `max_bpm`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Trainer {
    pub enabled: bool,
    pub every_bars: u16,
    pub step_bpm: f64,
    pub max_bpm: f64,
    pub at_max: TrainerEnd,
}

impl Default for Trainer {
    fn default() -> Self {
        Self {
            enabled: false,
            every_bars: 4,
            step_bpm: 5.0,
            max_bpm: 160.0,
            at_max: TrainerEnd::Hold,
        }
    }
}

/// Everything that shapes what the metronome plays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MetronomeSettings {
    /// Beats per minute, 20.0 ..= 400.0 in steps of 0.1.
    pub bpm: f64,
    /// Meter numerator: beats per bar.
    pub beats: u8,
    /// Meter denominator: the note value of one beat.
    pub note_value: u8,
    /// One accent per beat of the bar.
    pub accents: Vec<Accent>,
    /// Clicks per beat (1 = no subdivision, 2 = eighths in 4/4, 3 = triplets...).
    pub subdivision: u8,
    pub sound: Sound,
    pub polyrhythm: Polyrhythm,
    pub trainer: Trainer,
}

impl Default for MetronomeSettings {
    fn default() -> Self {
        Self {
            bpm: tenths_to_bpm(DEFAULT_BPM_TENTHS),
            beats: 4,
            note_value: 4,
            accents: default_accents(4, 4),
            subdivision: 1,
            sound: Sound::Click,
            polyrhythm: Polyrhythm::default(),
            trainer: Trainer::default(),
        }
    }
}

impl MetronomeSettings {
    /// Clamps every value into range and repairs the accent list, so any
    /// input (an old file, a webview message) is playable.
    pub fn normalized(mut self) -> Self {
        self.bpm = tenths_to_bpm(bpm_to_tenths(self.bpm));
        self.beats = self.beats.clamp(1, MAX_BEATS);
        if !NOTE_VALUES.contains(&self.note_value) {
            self.note_value = 4;
        }
        if self.accents.len() != usize::from(self.beats) {
            self.accents = default_accents(self.beats, self.note_value);
        }
        self.subdivision = self.subdivision.clamp(1, MAX_SUBDIVISION);
        self.polyrhythm.pulses = self.polyrhythm.pulses.clamp(1, MAX_POLY);
        self.polyrhythm.beats = self.polyrhythm.beats.clamp(1, MAX_POLY);
        self.trainer.every_bars = self.trainer.every_bars.clamp(1, MAX_TRAINER_BARS);
        self.trainer.step_bpm = tenths_to_bpm(trainer_step_tenths(self.trainer.step_bpm));
        self.trainer.max_bpm = tenths_to_bpm(bpm_to_tenths(self.trainer.max_bpm));
        self
    }

    /// Tempo in tenths of a BPM, clamped to 20.0 ..= 400.0.
    pub fn bpm_tenths(&self) -> u32 {
        bpm_to_tenths(self.bpm)
    }
}

/// Rounds a BPM to tenths and clamps it to 20.0 ..= 400.0.
pub fn bpm_to_tenths(bpm: f64) -> u32 {
    if !bpm.is_finite() {
        return DEFAULT_BPM_TENTHS;
    }
    let tenths = (bpm * 10.0).round().clamp(0.0, f64::from(u32::MAX));
    (tenths as u32).clamp(MIN_BPM_TENTHS, MAX_BPM_TENTHS)
}

pub fn tenths_to_bpm(tenths: u32) -> f64 {
    f64::from(tenths) / 10.0
}

pub(crate) fn trainer_step_tenths(step: f64) -> u32 {
    if !step.is_finite() {
        return 50;
    }
    let tenths = (step * 10.0).round().clamp(0.0, f64::from(u32::MAX));
    (tenths as u32).clamp(1, MAX_TRAINER_STEP_TENTHS)
}

/// Accents a meter starts with: a strong downbeat; in eighth-note (and
/// shorter) meters the other group starts are normal and the rest weak
/// (6/8 → 3+3, 7/8 → 2+2+3); otherwise every other beat is normal.
pub fn default_accents(beats: u8, note_value: u8) -> Vec<Accent> {
    let beats = beats.clamp(1, MAX_BEATS);
    let grouped = note_value >= 8 && beats > 3;
    let group_starts = if grouped {
        group_starts(beats)
    } else {
        Vec::new()
    };
    (0..beats)
        .map(|beat| {
            if beat == 0 {
                Accent::Strong
            } else if !grouped || group_starts.contains(&beat) {
                Accent::Normal
            } else {
                Accent::Weak
            }
        })
        .collect()
}

/// Group starts for compound and odd meters: groups of three when the
/// numerator divides by three, otherwise twos with a final three.
fn group_starts(beats: u8) -> Vec<u8> {
    if beats.is_multiple_of(3) {
        return (0..beats).step_by(3).collect();
    }
    let mut starts = Vec::new();
    let mut beat = 0;
    while beat < beats {
        starts.push(beat);
        beat += if beats - beat == 3 { 3 } else { 2 };
    }
    starts
}

#[cfg(test)]
mod tests {
    use super::*;
    use Accent::{Normal as N, Strong as S, Weak as W};

    #[test]
    fn bpm_rounds_to_tenths_and_clamps() {
        assert_eq!(bpm_to_tenths(120.0), 1_200);
        assert_eq!(bpm_to_tenths(120.04), 1_200);
        assert_eq!(bpm_to_tenths(120.06), 1_201);
        assert_eq!(bpm_to_tenths(5.0), MIN_BPM_TENTHS);
        assert_eq!(bpm_to_tenths(1_000.0), MAX_BPM_TENTHS);
        assert_eq!(bpm_to_tenths(f64::NAN), DEFAULT_BPM_TENTHS);
    }

    #[test]
    fn default_accents_follow_the_meter() {
        assert_eq!(default_accents(4, 4), vec![S, N, N, N]);
        assert_eq!(default_accents(3, 4), vec![S, N, N]);
        assert_eq!(default_accents(6, 8), vec![S, W, W, N, W, W]);
        assert_eq!(default_accents(7, 8), vec![S, W, N, W, N, W, W]);
        assert_eq!(
            default_accents(12, 8),
            vec![S, W, W, N, W, W, N, W, W, N, W, W]
        );
        assert_eq!(default_accents(3, 8), vec![S, N, N]);
    }

    #[test]
    fn normalizing_repairs_out_of_range_values() {
        let settings = MetronomeSettings {
            bpm: 999.0,
            beats: 0,
            note_value: 3,
            accents: vec![],
            subdivision: 20,
            polyrhythm: Polyrhythm {
                pulses: 0,
                beats: 99,
                ..Polyrhythm::default()
            },
            trainer: Trainer {
                every_bars: 0,
                step_bpm: 0.0,
                max_bpm: 1.0,
                ..Trainer::default()
            },
            ..MetronomeSettings::default()
        }
        .normalized();
        assert_eq!(settings.bpm, 400.0);
        assert_eq!(settings.beats, 1);
        assert_eq!(settings.note_value, 4);
        assert_eq!(settings.accents, vec![S]);
        assert_eq!(settings.subdivision, MAX_SUBDIVISION);
        assert_eq!(settings.polyrhythm.pulses, 1);
        assert_eq!(settings.polyrhythm.beats, MAX_POLY);
        assert_eq!(settings.trainer.every_bars, 1);
        assert_eq!(settings.trainer.step_bpm, 0.1);
        assert_eq!(settings.trainer.max_bpm, 20.0);
    }

    #[test]
    fn missing_fields_load_as_defaults() {
        let settings: MetronomeSettings = serde_json::from_str(r#"{"bpm":96.5}"#).unwrap();
        assert_eq!(settings.bpm, 96.5);
        assert_eq!(settings.beats, 4);
        assert_eq!(settings.trainer, Trainer::default());
    }
}

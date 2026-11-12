//! Drum patterns: per-track step levels, tempo, swing and track mix, plus the
//! timing math that turns a step number into a position on the beat.
//!
//! A step is a sixteenth note. A 16-step pattern is one 4/4 bar, a 32-step
//! pattern two bars.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::kit::Track;

/// Steps stored per track; a 16-step pattern ignores the second half.
pub const MAX_STEPS: usize = 32;
pub const LENGTHS: [u8; 2] = [16, 32];
pub const MIN_BPM: u16 = 40;
pub const MAX_BPM: u16 = 300;
/// Swing in percent: 50 is straight, 66–67 a triplet shuffle.
pub const MIN_SWING: u8 = 50;
pub const MAX_SWING: u8 = 75;
/// Step levels: 0 off, 1 soft (ghost), 2 normal, 3 accent.
pub const MAX_LEVEL: u8 = 3;
pub const DEFAULT_VOLUME: f32 = 0.8;

/// Hit velocity for each step level.
pub fn level_velocity(level: u8) -> f32 {
    match level {
        0 => 0.0,
        1 => 0.35,
        2 => 0.7,
        _ => 1.0,
    }
}

/// Which notes swing: offbeat eighths (shuffle, jazz) or offbeat sixteenths.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SwingUnit {
    #[default]
    Eighth,
    Sixteenth,
}

impl SwingUnit {
    /// Steps in one swung pair (a quarter note for eighths, an eighth for sixteenths).
    fn pair_steps(self) -> u64 {
        match self {
            SwingUnit::Eighth => 4,
            SwingUnit::Sixteenth => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TrackPattern {
    /// Level per step (`0..=MAX_LEVEL`), `MAX_STEPS` long once normalized.
    pub steps: Vec<u8>,
    /// Linear, `0.0..=1.0`.
    pub volume: f32,
    pub muted: bool,
}

impl Default for TrackPattern {
    fn default() -> Self {
        Self {
            steps: vec![0; MAX_STEPS],
            volume: DEFAULT_VOLUME,
            muted: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Pattern {
    /// 16 or 32 steps.
    pub length: u8,
    pub bpm: u16,
    /// Percent, `MIN_SWING..=MAX_SWING`.
    pub swing: u8,
    pub swing_unit: SwingUnit,
    pub tracks: BTreeMap<Track, TrackPattern>,
}

impl Default for Pattern {
    fn default() -> Self {
        Self {
            length: 16,
            bpm: 100,
            swing: MIN_SWING,
            swing_unit: SwingUnit::Eighth,
            tracks: Track::ALL
                .iter()
                .map(|&track| (track, TrackPattern::default()))
                .collect(),
        }
    }
}

impl Pattern {
    /// Clamps every value into range and fills in missing tracks and steps,
    /// so a pattern from the webview or an old file is always playable.
    pub fn normalized(mut self) -> Self {
        if !LENGTHS.contains(&self.length) {
            self.length = LENGTHS[0];
        }
        self.bpm = self.bpm.clamp(MIN_BPM, MAX_BPM);
        self.swing = self.swing.clamp(MIN_SWING, MAX_SWING);
        for track in Track::ALL {
            let entry = self.tracks.entry(track).or_default();
            entry.steps.resize(MAX_STEPS, 0);
            for level in &mut entry.steps {
                *level = (*level).min(MAX_LEVEL);
            }
            entry.volume = if entry.volume.is_finite() {
                entry.volume.clamp(0.0, 1.0)
            } else {
                DEFAULT_VOLUME
            };
        }
        self
    }

    /// Level of `track` at pattern step `step` (0 if out of range).
    pub fn level(&self, track: Track, step: usize) -> u8 {
        self.tracks
            .get(&track)
            .and_then(|pattern| pattern.steps.get(step))
            .copied()
            .unwrap_or(0)
    }

    /// Gain the track plays at (0 when muted).
    pub fn track_gain(&self, track: Track) -> f32 {
        self.tracks.get(&track).map_or(DEFAULT_VOLUME, |pattern| {
            if pattern.muted { 0.0 } else { pattern.volume }
        })
    }

    pub fn is_audible(&self, track: Track) -> bool {
        self.track_gain(track) > 0.0
    }

    /// Length of one (straight) sixteenth note in frames.
    pub fn frames_per_step(&self, sample_rate: u32) -> f64 {
        f64::from(sample_rate) * 60.0 / f64::from(self.bpm.max(1)) / 4.0
    }

    /// Where absolute step `step` falls, in straight-sixteenth units, once swing
    /// is applied (see [`swung_position`]).
    pub fn swung_position(&self, step: u64) -> f64 {
        swung_position(step, self.swing, self.swing_unit)
    }
}

/// Position of absolute step `step` in straight-sixteenth units after swing.
///
/// Swing stretches the first half of each pair (two eighths, or two
/// sixteenths) to `swing`% of the pair and squeezes the second half into the
/// rest; steps inside each half keep even spacing. At 50% nothing moves; at
/// 66.7% offbeat eighths land on the last triplet.
pub fn swung_position(step: u64, swing: u8, unit: SwingUnit) -> f64 {
    let pair = unit.pair_steps();
    let ratio = f64::from(swing.clamp(MIN_SWING, MAX_SWING)) / 100.0;
    let start = (step / pair * pair) as f64;
    let local = (step % pair) as f64;
    let half = pair as f64 / 2.0;
    let pair_len = pair as f64;
    let offset = if local < half {
        local * ratio * 2.0
    } else {
        ratio * pair_len + (local - half) * (1.0 - ratio) * 2.0
    };
    start + offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_time_keeps_every_step_in_place() {
        for step in 0..64 {
            assert_eq!(swung_position(step, 50, SwingUnit::Eighth), step as f64);
            assert_eq!(swung_position(step, 50, SwingUnit::Sixteenth), step as f64);
        }
    }

    #[test]
    fn eighth_swing_moves_offbeat_eighths_and_the_sixteenths_between() {
        // 66.67% ≈ triplets: the "and" lands two thirds into the beat.
        let triplet = |step| swung_position(step, 67, SwingUnit::Eighth);
        assert_eq!(triplet(0), 0.0);
        assert!((triplet(2) - 2.68).abs() < 1e-9);
        assert!((triplet(1) - 1.34).abs() < 1e-9);
        assert!((triplet(3) - 3.34).abs() < 1e-9);
        assert_eq!(triplet(4), 4.0);
        assert!((swung_position(6, 75, SwingUnit::Eighth) - 7.0).abs() < 1e-9);
    }

    #[test]
    fn sixteenth_swing_moves_only_odd_steps() {
        for step in (0..32).step_by(2) {
            assert_eq!(swung_position(step, 60, SwingUnit::Sixteenth), step as f64);
            assert!(
                (swung_position(step + 1, 60, SwingUnit::Sixteenth) - (step as f64 + 1.2)).abs()
                    < 1e-9
            );
        }
    }

    #[test]
    fn normalizing_fills_and_clamps() {
        let mut raw = Pattern {
            length: 20,
            bpm: 999,
            swing: 10,
            ..Pattern::default()
        };
        raw.tracks.clear();
        raw.tracks.insert(
            Track::Snare,
            TrackPattern {
                steps: vec![9, 2],
                volume: f32::NAN,
                muted: true,
            },
        );
        let pattern = raw.normalized();
        assert_eq!(
            (pattern.length, pattern.bpm, pattern.swing),
            (16, MAX_BPM, MIN_SWING)
        );
        assert_eq!(pattern.tracks.len(), Track::ALL.len());
        let snare = &pattern.tracks[&Track::Snare];
        assert_eq!(snare.steps.len(), MAX_STEPS);
        assert_eq!(&snare.steps[..3], &[MAX_LEVEL, 2, 0]);
        assert_eq!(snare.volume, DEFAULT_VOLUME);
        assert_eq!(pattern.track_gain(Track::Snare), 0.0);
        assert_eq!(pattern.track_gain(Track::Kick), DEFAULT_VOLUME);
    }

    #[test]
    fn patterns_round_trip_through_json_and_accept_partial_objects() {
        let pattern = Pattern::default().normalized();
        let json = serde_json::to_string(&pattern).unwrap();
        assert!(json.contains("\"swingUnit\":\"eighth\""));
        assert!(json.contains("\"closedHat\""));
        let back: Pattern = serde_json::from_str(&json).unwrap();
        assert_eq!(back, pattern);

        let partial: Pattern =
            serde_json::from_str(r#"{"bpm":90,"tracks":{"kick":{"steps":[2]}}}"#).unwrap();
        let partial = partial.normalized();
        assert_eq!(partial.bpm, 90);
        assert_eq!(partial.level(Track::Kick, 0), 2);
        assert_eq!(partial.tracks[&Track::Kick].volume, DEFAULT_VOLUME);
    }
}

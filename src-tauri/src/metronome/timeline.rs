//! Pure beat generator: turns settings into clicks on the engine's frame
//! clock, one beat at a time.
//!
//! Drift-free by construction: within a tempo segment every position is
//! `segment_frame + round(beats_since_segment × 60 × sample_rate / bpm)`,
//! computed exactly in integers from the segment start, so rounding never
//! accumulates. The tempo only changes on a beat, and the new segment starts
//! at that beat's frame as computed under the old tempo.

use crate::audio::FramePos;

use super::pattern::{
    Accent, MetronomeSettings, Sound, TrainerEnd, bpm_to_tenths, trainer_step_tenths,
};
use super::source::{Variant, note};

pub const STRONG_VELOCITY: f32 = 1.0;
pub const NORMAL_VELOCITY: f32 = 0.8;
pub const WEAK_VELOCITY: f32 = 0.5;
pub const SUBDIVISION_VELOCITY: f32 = 0.35;
pub const POLY_VELOCITY: f32 = 0.75;

/// One sound to schedule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Click {
    pub frame: FramePos,
    pub note: u8,
    pub velocity: f32,
}

/// A main beat, for the screen and for tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BeatMark {
    pub frame: FramePos,
    /// Beats since playback started.
    pub index: u64,
    /// Bars since playback started.
    pub bar: u64,
    /// Position in the bar, from 0.
    pub beat: u8,
    /// Beats in this bar.
    pub beats: u8,
    pub accent: Accent,
    /// Tempo of this beat in tenths of a BPM (moves with the trainer).
    pub bpm_tenths: u32,
}

/// A click of the polyrhythm layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PulseMark {
    pub frame: FramePos,
    /// Position in the cycle, from 0 (the cycle start falls on a main beat).
    pub pulse: u8,
    pub pulses: u8,
}

/// Everything one beat produces: its click, subdivisions and polyrhythm
/// pulses up to (not including) the next beat.
#[derive(Clone, Debug, Default)]
pub struct BeatOutput {
    pub clicks: Vec<Click>,
    pub pulses: Vec<PulseMark>,
}

/// Where the generator stands. Small and `Copy`, so the scheduler can keep
/// one per planned beat and rewind to it.
///
/// The trainer state (`bpm_tenths`, `bars_at_tempo`) only changes on the
/// first beat of a bar, where the bar-end decision is taken lazily
/// (`decision_pending`). A cursor taken before a bar's first beat is
/// therefore a snapshot of that bar boundary: restoring it and changing the
/// rules makes the boundary decide again under the new rules. Stopping is
/// never stored; it is the outcome of the pending decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    /// Frame and beat index where the current tempo began.
    pub segment_frame: FramePos,
    pub segment_beat: u64,
    pub bpm_tenths: u32,
    /// Next beat to generate.
    pub beat: u64,
    pub bar: u64,
    pub beat_in_bar: u8,
    /// Bars completed at the current tempo, not counting a bar whose end
    /// is still pending.
    pub bars_at_tempo: u32,
    /// Beat at which the polyrhythm cycle count starts.
    pub poly_origin: u64,
    /// The last bar ended and the trainer has not decided on it yet; it
    /// does so when the next beat is generated.
    pub decision_pending: bool,
}

/// What the trainer does at a bar boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Boundary {
    bars_at_tempo: u32,
    bpm_tenths: u32,
    stop: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PolyRule {
    pulses: u8,
    beats: u8,
    sound: Sound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TrainerRule {
    every_bars: u32,
    step_tenths: u32,
    max_tenths: u32,
    end: TrainerEnd,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Rules {
    base_tenths: u32,
    beats: u8,
    accents: Vec<Accent>,
    subdivision: u8,
    sound: Sound,
    poly: Option<PolyRule>,
    trainer: Option<TrainerRule>,
}

impl Boundary {
    fn stopping(self) -> Self {
        Self { stop: true, ..self }
    }
}

impl Rules {
    fn from_settings(settings: &MetronomeSettings) -> Self {
        let s = settings.clone().normalized();
        Self {
            base_tenths: s.bpm_tenths(),
            beats: s.beats,
            accents: s.accents,
            subdivision: s.subdivision,
            sound: s.sound,
            poly: s.polyrhythm.enabled.then_some(PolyRule {
                pulses: s.polyrhythm.pulses,
                beats: s.polyrhythm.beats,
                sound: s.polyrhythm.sound,
            }),
            trainer: s.trainer.enabled.then_some(TrainerRule {
                every_bars: u32::from(s.trainer.every_bars),
                step_tenths: trainer_step_tenths(s.trainer.step_bpm),
                max_tenths: bpm_to_tenths(s.trainer.max_bpm),
                end: s.trainer.at_max,
            }),
        }
    }
}

/// Frames for `numerator / denominator` beats at `bpm_tenths`, rounded to the
/// nearest frame (exact integer arithmetic).
pub fn beats_to_frames(
    numerator: u128,
    denominator: u128,
    sample_rate: u32,
    bpm_tenths: u32,
) -> u64 {
    // beats × 60 s × rate / (tenths / 10) = beats × 600 × rate / tenths
    let a = numerator * 600 * u128::from(sample_rate);
    let b = denominator * u128::from(bpm_tenths.max(1));
    u64::try_from((2 * a + b) / (2 * b)).unwrap_or(u64::MAX)
}

pub struct Timeline {
    sample_rate: u32,
    rules: Rules,
    cursor: Cursor,
}

impl Timeline {
    /// Starts at bar 0, beat 0 on `start_frame`.
    pub fn new(settings: &MetronomeSettings, sample_rate: u32, start_frame: FramePos) -> Self {
        let rules = Rules::from_settings(settings);
        let cursor = Cursor {
            segment_frame: start_frame,
            segment_beat: 0,
            bpm_tenths: rules.base_tenths,
            beat: 0,
            bar: 0,
            beat_in_bar: 0,
            bars_at_tempo: 0,
            poly_origin: 0,
            decision_pending: false,
        };
        Self {
            sample_rate,
            rules,
            cursor,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    /// Rewinds (or forwards) to a cursor taken earlier from this timeline.
    pub fn restore(&mut self, cursor: Cursor) {
        self.cursor = cursor;
    }

    /// True when the trainer, under the current rules, stops at the next
    /// bar boundary: nothing follows [`Self::next_frame`]. Not final until
    /// that frame has played; changing the rules decides again.
    pub fn is_finished(&self) -> bool {
        self.boundary().stop
    }

    /// Frame of the next beat (where playback ends when finished).
    pub fn next_frame(&self) -> FramePos {
        self.frame_at(self.cursor.beat, 0, 1)
    }

    /// The pending bar-end decision under the current rules.
    fn boundary(&self) -> Boundary {
        let c = &self.cursor;
        let keep = Boundary {
            bars_at_tempo: c.bars_at_tempo,
            bpm_tenths: c.bpm_tenths,
            stop: false,
        };
        let Some(trainer) = self.rules.trainer.filter(|_| c.decision_pending) else {
            return keep;
        };
        let bars = c.bars_at_tempo + 1;
        if bars < trainer.every_bars {
            return Boundary {
                bars_at_tempo: bars,
                ..keep
            };
        }
        if c.bpm_tenths < trainer.max_tenths {
            return Boundary {
                bars_at_tempo: 0,
                bpm_tenths: (c.bpm_tenths + trainer.step_tenths).min(trainer.max_tenths),
                stop: false,
            };
        }
        match trainer.end {
            TrainerEnd::Stop => Boundary {
                bars_at_tempo: bars,
                ..keep
            }
            .stopping(),
            TrainerEnd::Hold => Boundary {
                bars_at_tempo: 0,
                ..keep
            },
        }
    }

    /// Frame of `beat + numerator / denominator` under the current tempo
    /// segment (`beat` at or after the segment start).
    fn frame_at(&self, beat: u64, numerator: u64, denominator: u64) -> FramePos {
        let c = &self.cursor;
        let whole = u128::from(beat.saturating_sub(c.segment_beat));
        let denominator = u128::from(denominator.max(1));
        c.segment_frame
            + beats_to_frames(
                whole * denominator + u128::from(numerator),
                denominator,
                self.sample_rate,
                c.bpm_tenths,
            )
    }

    /// Produces the next beat into `out` (cleared first) and moves past it.
    /// `None` once the trainer has stopped playback.
    pub fn generate(&mut self, out: &mut BeatOutput) -> Option<BeatMark> {
        out.clicks.clear();
        out.pulses.clear();
        let boundary = self.boundary();
        if boundary.stop {
            return None;
        }
        let frame = self.frame_at(self.cursor.beat, 0, 1);
        {
            let c = &mut self.cursor;
            if boundary.bpm_tenths != c.bpm_tenths {
                // The new tempo starts on this beat, placed by the old one.
                c.segment_frame = frame;
                c.segment_beat = c.beat;
                c.bpm_tenths = boundary.bpm_tenths;
            }
            c.bars_at_tempo = boundary.bars_at_tempo;
            c.decision_pending = false;
        }
        let c = self.cursor;
        let rules = &self.rules;
        let accent = rules.accents[usize::from(c.beat_in_bar)];
        let main = match accent {
            Accent::Strong => Some((Variant::High, STRONG_VELOCITY)),
            Accent::Normal => Some((Variant::Low, NORMAL_VELOCITY)),
            Accent::Weak => Some((Variant::Low, WEAK_VELOCITY)),
            Accent::Mute => None,
        };
        if let Some((variant, velocity)) = main {
            out.clicks.push(Click {
                frame,
                note: note(rules.sound, variant),
                velocity,
            });
        }
        for step in 1..rules.subdivision {
            out.clicks.push(Click {
                frame: self.frame_at(c.beat, u64::from(step), u64::from(rules.subdivision)),
                note: note(rules.sound, Variant::Low),
                velocity: SUBDIVISION_VELOCITY,
            });
        }
        if let Some(poly) = rules.poly {
            // Pulse p sits at p × beats / pulses main beats after the origin;
            // take the ones inside [beat, beat + 1).
            let x = u128::from(poly.pulses);
            let y = u128::from(poly.beats);
            let rel = u128::from(c.beat - c.poly_origin);
            for p in (rel * x).div_ceil(y)..((rel + 1) * x).div_ceil(y) {
                let offset = u64::try_from(p * y - rel * x).unwrap_or(0);
                let frame = self.frame_at(c.beat, offset, u64::from(poly.pulses));
                out.clicks.push(Click {
                    frame,
                    note: note(poly.sound, Variant::High),
                    velocity: POLY_VELOCITY,
                });
                out.pulses.push(PulseMark {
                    frame,
                    pulse: u8::try_from(p % x).unwrap_or(0),
                    pulses: poly.pulses,
                });
            }
        }
        let mark = BeatMark {
            frame,
            index: c.beat,
            bar: c.bar,
            beat: c.beat_in_bar,
            beats: rules.beats,
            accent,
            bpm_tenths: c.bpm_tenths,
        };
        self.advance();
        Some(mark)
    }

    fn advance(&mut self) {
        let rules = &self.rules;
        let c = &mut self.cursor;
        c.beat += 1;
        c.beat_in_bar += 1;
        if c.beat_in_bar >= rules.beats {
            c.beat_in_bar = 0;
            c.bar += 1;
            c.decision_pending = true;
        }
    }

    /// Switches to new settings from the next beat to generate.
    ///
    /// - A new tempo starts on that beat. The trainer's current step is kept
    ///   when only other things change, and restarts from the new tempo when
    ///   the tempo itself changes or the trainer is switched on.
    /// - If the bar has no room for that beat in the new meter, a new bar
    ///   starts there.
    /// - Changing the polyrhythm or the meter restarts the polyrhythm cycle
    ///   at the start of the current bar.
    /// - A bar-end decision still pending on that beat (including a planned
    ///   stop) is taken under the new rules.
    pub fn apply(&mut self, settings: &MetronomeSettings) {
        let new = Rules::from_settings(settings);
        let frame = self.frame_at(self.cursor.beat, 0, 1);
        let old = std::mem::replace(&mut self.rules, new);
        let new = &self.rules;
        let c = &mut self.cursor;

        if c.beat_in_bar >= new.beats {
            c.beat_in_bar = 0;
            c.bar += 1;
        }

        // A fresh start (tempo changed, trainer switched) drops the pending
        // decision: nothing has been played at the new tempo yet.
        let tempo = match (old.trainer, new.trainer) {
            (_, None) => {
                c.decision_pending = false;
                new.base_tenths
            }
            (None, Some(_)) => {
                c.bars_at_tempo = 0;
                c.decision_pending = false;
                new.base_tenths
            }
            (Some(_), Some(_)) if old.base_tenths != new.base_tenths => {
                c.bars_at_tempo = 0;
                c.decision_pending = false;
                new.base_tenths
            }
            (Some(_), Some(trainer)) => c
                .bpm_tenths
                .clamp(new.base_tenths, trainer.max_tenths.max(new.base_tenths)),
        };
        if tempo != c.bpm_tenths {
            c.segment_frame = frame;
            c.segment_beat = c.beat;
            c.bpm_tenths = tempo;
        }

        if old.poly != new.poly || old.beats != new.beats {
            c.poly_origin = c.beat - u64::from(c.beat_in_bar);
        }
    }

    /// Moves the next beat to `frame` at a new sample rate (output device
    /// changed), keeping tempo and position in the bar.
    pub fn rebase(&mut self, sample_rate: u32, frame: FramePos) {
        self.sample_rate = sample_rate;
        let c = &mut self.cursor;
        c.segment_frame = frame;
        c.segment_beat = c.beat;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metronome::pattern::{Polyrhythm, Trainer};

    const RATE: u32 = 48_000;

    fn beats(timeline: &mut Timeline, count: usize) -> Vec<(BeatMark, BeatOutput)> {
        (0..count)
            .map(|_| {
                let mut out = BeatOutput::default();
                let mark = timeline.generate(&mut out).expect("beat");
                (mark, out)
            })
            .collect()
    }

    #[test]
    fn beat_frames_are_exact_and_do_not_accumulate_rounding() {
        // 44.1 kHz at 130.7 BPM: 20 244.45... frames per beat.
        let settings = MetronomeSettings {
            bpm: 130.7,
            ..MetronomeSettings::default()
        };
        let mut timeline = Timeline::new(&settings, 44_100, 1_000);
        for (n, (mark, _)) in beats(&mut timeline, 5_000).into_iter().enumerate() {
            let exact = n as f64 * 60.0 * 44_100.0 / 130.7;
            assert_eq!(mark.frame, 1_000 + exact.round() as u64, "beat {n}");
        }
    }

    #[test]
    fn subdivisions_split_the_beat_evenly() {
        let settings = MetronomeSettings {
            subdivision: 3,
            ..MetronomeSettings::default()
        };
        let mut timeline = Timeline::new(&settings, RATE, 0);
        let (_, out) = beats(&mut timeline, 1).remove(0);
        let frames: Vec<_> = out.clicks.iter().map(|c| c.frame).collect();
        assert_eq!(frames, vec![0, 8_000, 16_000]);
        assert_eq!(out.clicks[1].velocity, SUBDIVISION_VELOCITY);
    }

    #[test]
    fn polyrhythm_places_pulses_between_beats() {
        let settings = MetronomeSettings {
            polyrhythm: Polyrhythm {
                enabled: true,
                pulses: 3,
                beats: 2,
                ..Polyrhythm::default()
            },
            ..MetronomeSettings::default()
        };
        let mut timeline = Timeline::new(&settings, RATE, 0);
        let pulses: Vec<_> = beats(&mut timeline, 4)
            .into_iter()
            .flat_map(|(_, out)| out.pulses)
            .map(|p| (p.frame, p.pulse))
            .collect();
        assert_eq!(
            pulses,
            vec![
                (0, 0),
                (16_000, 1),
                (32_000, 2),
                (48_000, 0),
                (64_000, 1),
                (80_000, 2)
            ]
        );
    }

    #[test]
    fn trainer_raises_the_tempo_every_n_bars_and_holds_at_max() {
        let settings = MetronomeSettings {
            trainer: Trainer {
                enabled: true,
                every_bars: 2,
                step_bpm: 10.0,
                max_bpm: 135.0,
                ..Trainer::default()
            },
            ..MetronomeSettings::default()
        };
        let mut timeline = Timeline::new(&settings, RATE, 0);
        let tempos: Vec<_> = beats(&mut timeline, 32)
            .chunks(8)
            .map(|two_bars| two_bars[0].0.bpm_tenths)
            .collect();
        assert_eq!(tempos, vec![1_200, 1_300, 1_350, 1_350]);
    }

    #[test]
    fn trainer_stops_after_the_bars_at_max() {
        let settings = MetronomeSettings {
            trainer: Trainer {
                enabled: true,
                every_bars: 1,
                step_bpm: 30.0,
                max_bpm: 150.0,
                at_max: TrainerEnd::Stop,
            },
            ..MetronomeSettings::default()
        };
        let mut timeline = Timeline::new(&settings, RATE, 0);
        let mut out = BeatOutput::default();
        let mut count = 0;
        while timeline.generate(&mut out).is_some() {
            count += 1;
        }
        // 120 (1 bar), 150 (1 bar), then stop.
        assert_eq!(count, 8);
        assert!(timeline.is_finished());
        assert_eq!(timeline.next_frame(), 4 * 24_000 + 4 * 19_200);
    }

    #[test]
    fn a_tempo_change_starts_on_the_next_beat() {
        let mut timeline = Timeline::new(&MetronomeSettings::default(), RATE, 0);
        beats(&mut timeline, 2);
        timeline.apply(&MetronomeSettings {
            bpm: 60.0,
            ..MetronomeSettings::default()
        });
        let frames: Vec<_> = beats(&mut timeline, 3)
            .into_iter()
            .map(|(mark, _)| mark.frame)
            .collect();
        assert_eq!(frames, vec![48_000, 96_000, 144_000]);
    }

    #[test]
    fn a_shorter_meter_starts_a_new_bar_when_the_beat_no_longer_fits() {
        let mut timeline = Timeline::new(&MetronomeSettings::default(), RATE, 0);
        beats(&mut timeline, 3);
        timeline.apply(&MetronomeSettings {
            beats: 3,
            accents: vec![Accent::Strong, Accent::Weak, Accent::Weak],
            ..MetronomeSettings::default()
        });
        let marks: Vec<_> = beats(&mut timeline, 4)
            .into_iter()
            .map(|(mark, _)| (mark.bar, mark.beat, mark.beats))
            .collect();
        assert_eq!(marks, vec![(1, 0, 3), (1, 1, 3), (1, 2, 3), (2, 0, 3)]);
    }
}

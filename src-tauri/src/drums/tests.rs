//! Drum machine render tests on the offline engine: every hit must land on
//! the frame of its swung step, across lengths, swing settings, live edits
//! and output changes.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::audio::{FramePos, OfflineEngine, StreamFormat};

use super::*;

const CALLBACK: usize = 128;
/// Source gain in tests, low enough that summed hits never reach the
/// engine's output clamp.
const TEST_GAIN: f32 = 0.25;
/// How far ahead the test driver keeps the schedule (like the live thread).
const LOOKAHEAD: FramePos = 4_800;

/// A kit whose every sample is a single full-scale impulse, so each hit
/// shows up as exactly one non-zero frame.
fn impulse_kit(rate: u32) -> Arc<Kit> {
    Arc::new(Kit::from_decoded(
        (0..TRACK_COUNT)
            .map(|_| Decoded {
                sample_rate: rate,
                samples: vec![1.0],
            })
            .collect(),
    ))
}

fn kit_with(rate: u32, samples: impl Fn(Track) -> Vec<f32>) -> Arc<Kit> {
    Arc::new(Kit::from_decoded(
        Track::ALL
            .iter()
            .map(|&track| Decoded {
                sample_rate: rate,
                samples: samples(track),
            })
            .collect(),
    ))
}

/// A pattern at full track volume with accents (velocity 1.0) on `hits`.
fn pattern(
    length: u8,
    bpm: u16,
    swing: u8,
    unit: SwingUnit,
    hits: &[(Track, &[usize])],
) -> Pattern {
    let mut tracks = BTreeMap::new();
    for track in Track::ALL {
        tracks.insert(
            track,
            TrackPattern {
                steps: vec![0; MAX_STEPS],
                volume: 1.0,
                muted: false,
            },
        );
    }
    for &(track, steps) in hits {
        for &step in steps {
            tracks.get_mut(&track).unwrap().steps[step] = MAX_LEVEL;
        }
    }
    Pattern {
        length,
        bpm,
        swing,
        swing_unit: unit,
        tracks,
    }
    .normalized()
}

struct Rig {
    engine: OfflineEngine,
    sequencer: Sequencer,
    out: Vec<f32>,
}

impl Rig {
    fn new(rate: u32, kit: Arc<Kit>, pattern: Pattern) -> Self {
        let mut engine = OfflineEngine::new(StreamFormat::new(rate, 2));
        let source = engine
            .handle
            .add_source(Box::new(DrumSource::new(kit)), TEST_GAIN)
            .unwrap();
        Self {
            engine,
            sequencer: Sequencer::new(source, pattern),
            out: Vec::new(),
        }
    }

    fn start(&mut self, at: FramePos) {
        self.sequencer.start(&mut self.engine.handle, at).unwrap();
    }

    /// Renders `frames` frames in device-sized callbacks, topping up the
    /// schedule before each one as the scheduler thread does.
    fn run(&mut self, frames: usize) {
        let mut buffer = vec![0.0; CALLBACK * 2];
        let mut left = frames;
        while left > 0 {
            let chunk = left.min(CALLBACK);
            let until = self.engine.handle.frames_rendered() + LOOKAHEAD;
            self.sequencer.fill(&mut self.engine.handle, until).unwrap();
            buffer[..chunk * 2].fill(0.0);
            self.engine.render_callback(&mut buffer[..chunk * 2]);
            self.out.extend_from_slice(&buffer[..chunk * 2]);
            left -= chunk;
        }
    }

    fn rendered(&self) -> FramePos {
        self.engine.handle.frames_rendered()
    }

    /// Frames where sound starts after silence (first channel).
    fn onsets(&self) -> Vec<FramePos> {
        let mut previous = 0.0;
        let mut onsets = Vec::new();
        for (index, frame) in self.out.as_chunks::<2>().0.iter().enumerate() {
            if frame[0] != 0.0 && previous == 0.0 {
                onsets.push(index as FramePos);
            }
            previous = frame[0];
        }
        onsets
    }

    /// Non-zero frames of the first channel and their values.
    fn hits(&self) -> BTreeMap<FramePos, f32> {
        self.out
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, frame)| frame[0] != 0.0)
            .map(|(index, frame)| {
                assert_eq!(frame[0], frame[1], "both channels carry the hit");
                (index as FramePos, frame[0])
            })
            .collect()
    }
}

/// The level each impulse kit hit should render at, per pattern step.
fn expected_value(pattern: &Pattern, index: usize) -> f32 {
    Track::ALL
        .iter()
        .filter(|&&track| pattern.level(track, index) > 0)
        .map(|&track| {
            level_velocity(pattern.level(track, index))
                * pattern.track_gain(track)
                * track.mix_gain()
        })
        .sum::<f32>()
        * TEST_GAIN
}

/// Expected hits for `steps` absolute steps from `start` on an unchanged timeline.
fn expected_hits(
    pattern: &Pattern,
    rate: u32,
    start: FramePos,
    steps: u64,
) -> BTreeMap<FramePos, f32> {
    let frames_per_step = pattern.frames_per_step(rate);
    (0..steps)
        .filter_map(|step| {
            let index = (step % u64::from(pattern.length)) as usize;
            let value = expected_value(pattern, index);
            let frame = (start as f64 + pattern.swung_position(step) * frames_per_step).round();
            (value > 0.0).then_some((frame as FramePos, value))
        })
        .collect()
}

fn assert_hits_match(got: &BTreeMap<FramePos, f32>, want: &BTreeMap<FramePos, f32>, label: &str) {
    let got_frames: Vec<_> = got.keys().collect();
    let want_frames: Vec<_> = want.keys().collect();
    assert_eq!(got_frames, want_frames, "{label}: hit frames");
    for (frame, value) in want {
        assert!(
            (got[frame] - value).abs() < 1.0e-5,
            "{label}: frame {frame} is {} not {value}",
            got[frame]
        );
    }
}

/// A groove whose second bar differs from the first, with off-beat and
/// sixteenth hits so swing moves something.
fn groove(length: u8, bpm: u16, swing: u8, unit: SwingUnit) -> Pattern {
    pattern(
        length,
        bpm,
        swing,
        unit,
        &[
            (Track::Kick, &[0, 7, 10, 16, 19, 26]),
            (Track::Snare, &[4, 12, 20, 28, 31]),
            (
                Track::ClosedHat,
                &[0, 2, 4, 6, 8, 10, 12, 14, 17, 18, 21, 22, 25, 26, 29, 30],
            ),
            (Track::Crash, &[16]),
        ],
    )
}

#[test]
fn every_hit_lands_on_its_swung_frame() {
    // 44.1 kHz at 97 BPM gives a step of 6 819.59 frames, so rounding matters.
    for rate in [48_000, 44_100] {
        for length in LENGTHS {
            for (swing, unit) in [
                (50, SwingUnit::Eighth),
                (58, SwingUnit::Eighth),
                (67, SwingUnit::Eighth),
                (75, SwingUnit::Eighth),
                (54, SwingUnit::Sixteenth),
                (62, SwingUnit::Sixteenth),
            ] {
                for bpm in [97, 180] {
                    let pattern = groove(length, bpm, swing, unit);
                    let start = 1_234;
                    let steps = u64::from(length) * 2;
                    let frames = (start as f64
                        + (steps as f64 + 1.0) * pattern.frames_per_step(rate))
                        as usize;
                    let mut rig = Rig::new(rate, impulse_kit(rate), pattern.clone());
                    rig.start(start);
                    rig.run(frames);
                    let want = expected_hits(&pattern, rate, start, steps + 1);
                    let want: BTreeMap<_, _> = want
                        .into_iter()
                        .filter(|&(f, _)| (f as usize) < frames)
                        .collect();
                    assert_hits_match(
                        &rig.hits(),
                        &want,
                        &format!("{rate} Hz, {length} steps, {bpm} BPM, swing {swing} {unit:?}"),
                    );
                }
            }
        }
    }
}

#[test]
fn swing_moves_offbeats_to_the_expected_frames() {
    // 120 BPM at 48 kHz: a sixteenth is 6 000 frames, a beat 24 000.
    let rate = 48_000;
    let hats = pattern(
        16,
        120,
        75,
        SwingUnit::Eighth,
        &[(Track::ClosedHat, &[0, 1, 2, 3, 4])],
    );
    let mut rig = Rig::new(rate, impulse_kit(rate), hats);
    rig.start(0);
    rig.run(30_000);
    // Eighth swing at 75%: the "and" moves from 12 000 to 18 000, the
    // sixteenths around it stay evenly inside each half.
    let frames: Vec<FramePos> = rig.hits().keys().copied().collect();
    assert_eq!(frames, vec![0, 9_000, 18_000, 21_000, 24_000]);

    let sixteenths = pattern(
        16,
        120,
        60,
        SwingUnit::Sixteenth,
        &[(Track::ClosedHat, &[0, 1, 2, 3])],
    );
    let mut rig = Rig::new(rate, impulse_kit(rate), sixteenths);
    rig.start(0);
    rig.run(24_000);
    let frames: Vec<FramePos> = rig.hits().keys().copied().collect();
    assert_eq!(frames, vec![0, 7_200, 12_000, 19_200]);
}

#[test]
fn a_32_step_pattern_plays_both_bars_and_16_steps_repeat_the_first() {
    let rate = 48_000;
    let hits: &[(Track, &[usize])] = &[(Track::Kick, &[0, 20])];
    let step = 6_000; // 120 BPM
    for (length, want) in [
        (32, vec![0, 20 * step, 32 * step, 52 * step]),
        (16, vec![0, 16 * step, 32 * step, 48 * step]),
    ] {
        let mut rig = Rig::new(
            rate,
            impulse_kit(rate),
            pattern(length, 120, 50, SwingUnit::Eighth, hits),
        );
        rig.start(0);
        rig.run(60 * step as usize);
        let frames: Vec<FramePos> = rig.hits().keys().copied().collect();
        assert_eq!(frames, want, "{length} steps");
    }
}

#[test]
fn step_levels_set_the_hit_velocity_and_mix_controls_apply() {
    let rate = 48_000;
    let mut groove = pattern(16, 120, 50, SwingUnit::Eighth, &[]);
    let snare = groove.tracks.get_mut(&Track::Snare).unwrap();
    snare.steps[0] = 1;
    snare.steps[1] = 2;
    snare.steps[2] = 3;
    snare.volume = 0.5;
    let kick = groove.tracks.get_mut(&Track::Kick).unwrap();
    kick.steps[3] = 3;
    kick.muted = true;
    let mut rig = Rig::new(rate, impulse_kit(rate), groove);
    rig.start(0);
    rig.run(30_000);
    let values: Vec<f32> = rig.hits().values().copied().collect();
    let base = 0.5 * Track::Snare.mix_gain() * TEST_GAIN;
    let want = [0.35 * base, 0.7 * base, base];
    assert_eq!(values.len(), 3, "the muted kick does not play: {values:?}");
    for (got, want) in values.iter().zip(want) {
        assert!((got - want).abs() < 1.0e-6, "{got} vs {want}");
    }
}

#[test]
fn edits_while_playing_apply_from_the_reschedule_frame_without_gaps() {
    let rate = 48_000;
    let before = groove(16, 120, 50, SwingUnit::Eighth);
    let after = groove(16, 150, 67, SwingUnit::Eighth);
    let mut rig = Rig::new(rate, impulse_kit(rate), before.clone());
    rig.start(0);
    rig.run(50_000);

    // The live service reschedules a little ahead of the audio thread.
    let from = rig.rendered() + 960;
    rig.sequencer
        .set_pattern(&mut rig.engine.handle, after.clone(), from)
        .unwrap();
    rig.run(150_000);

    // Old timing before `from`, the new tempo and swing after it, with the
    // timeline continuing where it was.
    let old_step = before.frames_per_step(rate);
    let position = from as f64 / old_step;
    let new_step = after.frames_per_step(rate);
    let mut want = BTreeMap::new();
    for step in 0..200_u64 {
        let index = (step % 16) as usize;
        let old_frame = (before.swung_position(step) * old_step).round();
        let (frame, value) = if old_frame < from as f64 {
            (old_frame, expected_value(&before, index))
        } else {
            let frame = (from as f64 + (after.swung_position(step) - position) * new_step).round();
            (frame.max(from as f64), expected_value(&after, index))
        };
        if value > 0.0 && frame < rig.rendered() as f64 {
            want.insert(frame as FramePos, value);
        }
    }
    assert_hits_match(&rig.hits(), &want, "tempo and swing change while playing");
}

#[test]
fn step_edits_are_heard_on_the_next_pass_without_restarting() {
    let rate = 48_000;
    let empty = pattern(16, 120, 50, SwingUnit::Eighth, &[(Track::Kick, &[0])]);
    let mut rig = Rig::new(rate, impulse_kit(rate), empty);
    rig.start(0);
    rig.run(30_000); // a little past step 4
    let with_snare = pattern(
        16,
        120,
        50,
        SwingUnit::Eighth,
        &[(Track::Kick, &[0]), (Track::Snare, &[4, 8])],
    );
    let from = rig.rendered() + 960;
    rig.sequencer
        .set_pattern(&mut rig.engine.handle, with_snare, from)
        .unwrap();
    rig.run(100_000);
    let frames: Vec<FramePos> = rig.hits().keys().copied().collect();
    // Step 4 (24 000) already played without the snare; step 8 has it,
    // and the bar keeps its place (kick at 96 000).
    assert_eq!(frames, vec![0, 48_000, 96_000, 120_000]);
}

#[test]
fn an_output_change_reschedules_at_the_new_sample_rate() {
    let groove = groove(16, 110, 62, SwingUnit::Sixteenth);
    let mut rig = Rig::new(48_000, impulse_kit(48_000), groove.clone());
    rig.start(0);
    rig.run(40_000);
    let generation = rig.engine.handle.generation();

    rig.engine.reformat(StreamFormat::new(44_100, 2));
    assert_ne!(rig.engine.handle.generation(), generation);
    assert_ne!(rig.sequencer.generation(), rig.engine.handle.generation());
    let from = rig.rendered() + 882;
    rig.sequencer.retime(&mut rig.engine.handle, from).unwrap();
    assert_eq!(rig.sequencer.generation(), rig.engine.handle.generation());
    rig.run(120_000);

    let old_step = groove.frames_per_step(48_000);
    let new_step = groove.frames_per_step(44_100);
    let position = from as f64 / old_step;
    // At 44.1 kHz each impulse is the kit resampled from 48 kHz, so compare
    // where each hit starts.
    let want: Vec<FramePos> = (0..100_u64)
        .filter(|step| expected_value(&groove, (step % 16) as usize) > 0.0)
        .map(|step| {
            let old = (groove.swung_position(step) * old_step).round();
            if old < from as f64 {
                old
            } else {
                (from as f64 + (groove.swung_position(step) - position) * new_step).round()
            }
        })
        .filter(|&frame| frame < rig.rendered() as f64)
        .map(|frame| frame as FramePos)
        .collect();
    assert_eq!(rig.onsets(), want, "48 kHz -> 44.1 kHz");
}

#[test]
fn stopping_drops_scheduled_hits_and_restarting_begins_at_step_one() {
    let rate = 48_000;
    let groove = pattern(
        16,
        120,
        50,
        SwingUnit::Eighth,
        &[(Track::Kick, &[0, 4, 8, 12])],
    );
    let mut rig = Rig::new(rate, impulse_kit(rate), groove);
    rig.start(0);
    rig.run(30_000);
    rig.sequencer.stop(&mut rig.engine.handle).unwrap();
    rig.run(30_000);
    let restart = rig.rendered() + 1_200;
    rig.start(restart);
    rig.run(30_000);
    let frames: Vec<FramePos> = rig.hits().keys().copied().collect();
    assert_eq!(frames, vec![0, 24_000, restart, restart + 24_000]);
}

#[test]
fn a_closed_hi_hat_chokes_the_open_one() {
    let rate = 48_000;
    let kit = kit_with(rate, |track| match track {
        Track::OpenHat => vec![0.5; 40_000],
        _ => vec![1.0],
    });
    // 120 BPM: open at 0, closed at step 2 (12 000).
    let choked = pattern(
        16,
        120,
        50,
        SwingUnit::Eighth,
        &[(Track::OpenHat, &[0]), (Track::ClosedHat, &[2])],
    );
    let mut rig = Rig::new(rate, kit.clone(), choked);
    rig.start(0);
    rig.run(20_000);
    let open = 0.5 * Track::OpenHat.mix_gain() * TEST_GAIN;
    let at = |rig: &Rig, frame: usize| rig.out[frame * 2];
    assert!(
        (at(&rig, 11_999) - open).abs() < 1.0e-6,
        "rings until the closed hat"
    );
    assert!(at(&rig, 12_100) > 0.0 && at(&rig, 12_100) < open, "fading");
    // 12 ms choke = 576 frames; nothing after that.
    assert!(
        rig.out[(12_000 + 600) * 2..].iter().all(|&v| v == 0.0),
        "choked"
    );

    let alone = pattern(
        16,
        120,
        50,
        SwingUnit::Eighth,
        &[(Track::OpenHat, &[0]), (Track::Kick, &[2])],
    );
    let mut rig = Rig::new(rate, kit, alone);
    rig.start(0);
    rig.run(20_000);
    assert!(
        (at(&rig, 15_000) - open).abs() < 1.0e-6,
        "other tracks do not choke it"
    );
}

#[test]
fn retriggering_a_track_fades_its_previous_hit_instead_of_cutting_it() {
    let rate = 48_000;
    let kit = kit_with(rate, |_| vec![0.5; 20_000]);
    let twice = pattern(16, 120, 50, SwingUnit::Eighth, &[(Track::Snare, &[0, 1])]);
    let mut rig = Rig::new(rate, kit, twice);
    rig.start(0);
    rig.run(10_000);
    let one = 0.5 * Track::Snare.mix_gain() * TEST_GAIN;
    let at = |frame: usize| rig.out[frame * 2];
    assert!((at(5_999) - one).abs() < 1.0e-6);
    // Both sound while the first fades over 3 ms (144 frames)...
    assert!(at(6_050) > one * 1.2, "overlap at {}", at(6_050));
    // ...then only the new hit remains.
    assert!((at(6_200) - one).abs() < 1.0e-6);
}

#[test]
fn the_bundled_kit_plays_at_any_output_rate() {
    let kit = Arc::new(Kit::bundled().unwrap());
    for rate in [44_100, 48_000] {
        for track in Track::ALL {
            let single = pattern(16, 120, 50, SwingUnit::Eighth, &[(track, &[4])]);
            let mut rig = Rig::new(rate, Arc::clone(&kit), single.clone());
            rig.start(0);
            rig.run(rate as usize / 2 + 30_000);
            let hit = (4.0 * single.frames_per_step(rate)).round() as usize;
            // Silent before the step, sounding within a millisecond of it.
            let onset = rig.onsets()[0] as usize;
            assert!(
                onset >= hit && onset <= hit + rate as usize / 1_000,
                "{track:?} at {rate} Hz starts at {onset}, step at {hit}"
            );
            let peak = rig.out.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
            assert!(
                peak > 0.2 * TEST_GAIN && peak <= TEST_GAIN,
                "{track:?} at {rate} Hz peak {peak}"
            );
        }
    }
}

#[test]
fn less_swing_while_playing_plays_a_step_moved_behind_the_edit_point_once() {
    // 120 BPM: a sixteenth is 6 000 frames. At 75% eighth swing the steps of
    // a beat sit at 0, 1.5, 3.0 and 3.5 sixteenths; straight they are 0–3.
    let rate = 48_000;
    let hats: &[(Track, &[usize])] = &[(Track::ClosedHat, &[0, 1, 2, 3, 4, 5, 6, 7])];
    let mut rig = Rig::new(
        rate,
        impulse_kit(rate),
        pattern(16, 120, 75, SwingUnit::Eighth, hats),
    );
    rig.start(0);
    rig.run(18_000);
    // Rescheduling at 3.2 sixteenths: step 3 (was 3.5) now belongs at 3.0,
    // just behind the point, so it plays at the point instead of vanishing.
    let from = rig.rendered() + 1_200;
    rig.sequencer
        .set_pattern(
            &mut rig.engine.handle,
            pattern(16, 120, 50, SwingUnit::Eighth, hats),
            from,
        )
        .unwrap();
    rig.run(32_000);
    let frames: Vec<FramePos> = rig.hits().keys().copied().collect();
    assert_eq!(
        frames,
        vec![0, 9_000, 18_000, 19_200, 24_000, 30_000, 36_000, 42_000]
    );
}

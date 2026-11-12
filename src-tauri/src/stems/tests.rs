//! Offline render tests of the stem player: what the engine outputs for
//! known track material.

use std::f32::consts::TAU;
use std::sync::Arc;

use crate::audio::{EventKind, OfflineEngine, SourceId, StreamFormat};

use super::*;

const RATE: u32 = 48_000;
const CALLBACK: usize = 128;

struct Rig {
    engine: OfflineEngine,
    id: SourceId,
    feeder: Feeder,
    shared: Arc<PlayerShared>,
}

impl Rig {
    fn new(tracks: Vec<Box<dyn TrackReader>>, out_rate: u32) -> Self {
        let StemPlayer {
            source,
            feeder,
            shared,
            ..
        } = StemPlayer::new(tracks, 0).unwrap();
        let mut engine = OfflineEngine::new(StreamFormat::new(out_rate, 2));
        let id = engine.handle.add_source(Box::new(source), 1.0).unwrap();
        Self {
            engine,
            id,
            feeder,
            shared,
        }
    }

    fn at(&mut self, frame: u64, kind: EventKind) {
        self.engine.handle.schedule(frame, self.id, kind).unwrap();
    }

    fn param(&mut self, frame: u64, id: u16, value: f32) {
        self.at(frame, EventKind::Param { id, value });
    }

    /// Renders `frames` frames in device-sized callbacks, letting the
    /// feeder run between callbacks as its thread would.
    fn render(&mut self, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        for chunk in out.chunks_mut(CALLBACK * 2) {
            self.feeder.fill();
            self.engine.render_callback(chunk);
        }
        out
    }

    /// Renders until playback ends (or `limit` frames); returns the output.
    fn render_to_end(&mut self, limit: usize) -> Vec<f32> {
        let mut out = Vec::new();
        while out.len() < limit * 2 {
            out.extend(self.render(CALLBACK * 8));
            if self.shared.ended() {
                break;
            }
        }
        out
    }
}

fn left(buffer: &[f32]) -> Vec<f32> {
    buffer
        .as_chunks::<2>()
        .0
        .iter()
        .map(|frame| frame[0])
        .collect()
}

fn impulses(frames: usize, at: &[(usize, f32)]) -> Box<dyn TrackReader> {
    let mut samples = vec![0.0; frames];
    for &(frame, value) in at {
        samples[frame] = value;
    }
    Box::new(MemoryTrack::mono(RATE, &samples))
}

fn constant(frames: usize, value: f32) -> Box<dyn TrackReader> {
    Box::new(MemoryTrack::mono(RATE, &vec![value; frames]))
}

fn sine(rate: u32, frames: usize, hz: f32, amplitude: f32) -> Box<dyn TrackReader> {
    let samples: Vec<f32> = (0..frames)
        .map(|i| (TAU * hz * i as f32 / rate as f32).sin() * amplitude)
        .collect();
    Box::new(MemoryTrack::mono(rate, &samples))
}

/// Fundamental frequency from rising zero crossings (linear interpolation).
fn frequency(samples: &[f32], rate: u32) -> f32 {
    let crossings: Vec<f32> = samples
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0)
        .map(|(i, w)| i as f32 + w[0] / (w[0] - w[1]))
        .collect();
    let periods = (crossings.len() - 1) as f32;
    periods * rate as f32 / (crossings[crossings.len() - 1] - crossings[0])
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

const START: u64 = 1_000;

#[test]
fn tracks_play_in_sample_lockstep() {
    // Each track has its own impulse plus one shared impulse at song frame 6 000.
    let mut rig = Rig::new(
        vec![
            impulses(10_000, &[(1_000, 0.25), (6_000, 0.1)]),
            impulses(10_000, &[(2_345, 0.25), (6_000, 0.2)]),
            impulses(10_000, &[(5_001, 0.25), (6_000, 0.3)]),
        ],
        RATE,
    );
    rig.at(START, EventKind::Start);
    let out = left(&rig.render(20_000));
    let start = START as usize;
    let nonzero: Vec<(usize, f32)> = out
        .iter()
        .enumerate()
        .filter(|(_, v)| v.abs() > 1e-6)
        .map(|(i, &v)| (i - start, v))
        .collect();
    assert_eq!(nonzero.len(), 4, "{nonzero:?}");
    assert_eq!(nonzero[0], (1_000, 0.25));
    assert_eq!(nonzero[1], (2_345, 0.25));
    assert_eq!(nonzero[2], (5_001, 0.25));
    assert_eq!(nonzero[3].0, 6_000);
    assert!((nonzero[3].1 - 0.6).abs() < 1e-6, "{}", nonzero[3].1);
}

#[test]
fn volume_mute_and_solo_shape_the_mix() {
    let tracks = || {
        vec![
            constant(96_000, 0.1),
            constant(96_000, 0.2),
            constant(96_000, 0.4),
        ]
    };
    let level = |out: &[f32], at: usize| left(out)[at];
    let settle = 4_800; // 100 ms after a change

    let mut rig = Rig::new(tracks(), RATE);
    rig.param(0, param::track(0, param::VOLUME), 0.5);
    rig.at(0, EventKind::Start);
    // 1: mute track 1. 2: solo track 2. 3: also solo track 0. 4: unmute 1, unsolo all.
    rig.param(10_000, param::track(1, param::MUTE), 1.0);
    rig.param(20_000, param::track(2, param::SOLO), 1.0);
    rig.param(30_000, param::track(0, param::SOLO), 1.0);
    rig.param(40_000, param::track(1, param::MUTE), 0.0);
    rig.param(40_000, param::track(0, param::SOLO), 0.0);
    rig.param(40_000, param::track(2, param::SOLO), 0.0);
    let out = rig.render(50_000);

    let expect = [
        (5_000, 0.05 + 0.2 + 0.4),
        (10_000 + settle, 0.05 + 0.4),
        (20_000 + settle, 0.4),
        (30_000 + settle, 0.05 + 0.4),
        (40_000 + settle, 0.05 + 0.2 + 0.4),
    ];
    for (frame, value) in expect {
        let got = level(&out, frame);
        assert!(
            (got - value).abs() < 1e-4,
            "frame {frame}: {got} != {value}"
        );
    }
    // Changes glide instead of jumping (no click).
    let jumps = left(&out)
        .windows(2)
        .skip(500)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(jumps < 0.01, "largest step {jumps}");
}

#[test]
fn ab_loop_wraps_at_the_exact_frame() {
    // Every song frame carries its own value.
    let ramp: Vec<f32> = (0..100_000).map(|i| i as f32 * 1e-6).collect();
    let mut rig = Rig::new(vec![Box::new(MemoryTrack::mono(RATE, &ramp))], RATE);
    let (a, b) = (20_000_u64, 30_000_u64);
    for event in loop_events(LoopRange::new(a, b)) {
        rig.at(0, event);
    }
    rig.at(START, EventKind::Start);
    let out = left(&rig.render(60_000));
    let start = START as usize;
    let song = |frame: u64| ramp[frame as usize];
    let crossfade = 480; // 10 ms at 48 kHz

    // Untouched up to the crossfade before B.
    for frame in [a, b - crossfade - 1] {
        assert_eq!(out[start + frame as usize], song(frame));
    }
    // Every lap restarts exactly at A and is exactly B - A frames long.
    for lap in 0..3_u64 {
        let lap_start = start + (b + lap * (b - a)) as usize;
        for k in 0..100_u64 {
            assert_eq!(
                out[lap_start + k as usize],
                song(a + k),
                "lap {lap} frame {k}"
            );
        }
    }
    // The crossfade has no jump larger than the material's own slope allows.
    let seam = start + (b - crossfade) as usize;
    let max_step = out[seam - 1..seam + crossfade as usize + 1]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(max_step < 0.01, "seam step {max_step}");
    assert!(!rig.shared.ended());
}

#[test]
fn clearing_the_loop_plays_on_past_b() {
    let ramp: Vec<f32> = (0..50_000).map(|i| i as f32 * 1e-6).collect();
    let mut rig = Rig::new(vec![Box::new(MemoryTrack::mono(RATE, &ramp))], RATE);
    for event in loop_events(LoopRange::new(1_000, 5_000)) {
        rig.at(0, event);
    }
    rig.at(0, EventKind::Start);
    rig.render(8_000);
    for event in loop_events(None) {
        rig.at(8_000, event);
    }
    let out = rig.render_to_end(100_000);
    assert!(rig.shared.ended());
    let tail = left(&out);
    // After the restart the song runs to its end and stops.
    let last = tail.iter().rposition(|v| v.abs() > 0.0).unwrap();
    assert!(
        (tail[last] - 49_999.0 * 1e-6).abs() < 1e-6,
        "{}",
        tail[last]
    );
}

#[test]
fn seek_jumps_to_the_target_frame() {
    let ramp: Vec<f32> = (0..100_000).map(|i| i as f32 * 1e-6).collect();
    let mut rig = Rig::new(vec![Box::new(MemoryTrack::mono(RATE, &ramp))], RATE);
    rig.at(0, EventKind::Start);
    rig.render(4_096);
    rig.at(4_096, EventKind::Seek { frame: 70_000 });
    let out = left(&rig.render(8_000));
    // After the fade-in (3 ms) the output is the material right after 70 000.
    let settled = out
        .iter()
        .position(|&v| v > 70_100.0 * 1e-6)
        .expect("plays from the seek target");
    let value = out[settled + 10];
    assert!((70_100.0e-6..70_500.0e-6).contains(&value), "{value}");
    assert!(out[settled + 11] - value > 0.0);
    assert!(rig.shared.position() > 70_000.0);
}

#[test]
fn slower_speed_stretches_length_and_keeps_pitch() {
    for speed in [0.5_f32, 0.75] {
        let frames = RATE as usize * 2;
        let mut rig = Rig::new(vec![sine(RATE, frames, 220.0, 0.5)], RATE);
        rig.param(0, param::SPEED, speed);
        rig.at(0, EventKind::Start);
        let out = left(&rig.render_to_end(frames * 3));
        assert!(rig.shared.ended(), "speed {speed}: playback ends");

        let expected = frames as f32 / speed;
        let length = out.len() as f32;
        assert!(
            (length - expected).abs() < 0.02 * expected,
            "speed {speed}: {length} frames, expected about {expected}"
        );
        let middle = &out[out.len() / 4..out.len() * 3 / 4];
        let pitch = frequency(middle, RATE);
        assert!(
            (pitch - 220.0).abs() < 2.0,
            "speed {speed}: pitch {pitch} Hz"
        );
        let level = rms(middle);
        assert!(
            (level - 0.5 / 2_f32.sqrt()).abs() < 0.05,
            "speed {speed}: rms {level}"
        );
    }
}

#[test]
fn speed_change_while_playing_keeps_pitch() {
    let frames = RATE as usize * 3;
    let mut rig = Rig::new(vec![sine(RATE, frames, 330.0, 0.5)], RATE);
    rig.at(0, EventKind::Start);
    let normal = left(&rig.render(24_000));
    rig.param(24_000, param::SPEED, 0.8);
    rig.render(12_000);
    let slower = left(&rig.render(24_000));
    rig.param(60_000, param::SPEED, 0.6);
    rig.render(12_000);
    let slowest = left(&rig.render(24_000));
    for (name, part) in [
        ("100%", &normal[2_000..]),
        ("80%", &slower[..]),
        ("60%", &slowest[..]),
    ] {
        let pitch = frequency(part, RATE);
        assert!((pitch - 330.0).abs() < 3.0, "{name}: {pitch} Hz");
    }
    // Position advances at the playback speed.
    let before = rig.shared.position();
    rig.render(48_000);
    let advanced = rig.shared.position() - before;
    assert!((advanced - 0.6 * 48_000.0).abs() < 400.0, "{advanced}");
}

#[test]
fn song_rate_is_converted_to_the_output_rate() {
    let song_rate = 44_100;
    let mut rig = Rig::new(vec![sine(song_rate, song_rate as usize, 441.0, 0.5)], RATE);
    rig.at(0, EventKind::Start);
    let out = left(&rig.render_to_end(RATE as usize * 2));
    assert!(rig.shared.ended());
    assert!(
        (out.len() as i64 - RATE as i64).abs() < 1_200,
        "{} frames",
        out.len()
    );
    let pitch = frequency(&out[4_800..43_200], RATE);
    assert!((pitch - 441.0).abs() < 0.5, "{pitch} Hz");
}

#[test]
fn output_rate_change_restarts_at_the_same_place() {
    let ramp: Vec<f32> = (0..200_000).map(|i| i as f32 * 1e-6).collect();
    let mut rig = Rig::new(vec![Box::new(MemoryTrack::mono(RATE, &ramp))], RATE);
    rig.at(0, EventKind::Start);
    rig.render(48_000);
    let before = rig.shared.position();
    rig.engine.reformat(StreamFormat::new(44_100, 2));
    rig.render(4_410);
    let after = rig.shared.position();
    // Playing on at the new rate: 4 410 output frames ≈ 4 800 song frames.
    assert!(
        after > before + 4_000.0 && after < before + 5_000.0,
        "{before} → {after}"
    );
    assert!(rig.shared.playing());
}

#[test]
fn mismatched_track_rates_are_rejected() {
    let tracks = vec![constant(10, 0.0), sine(44_100, 10, 1.0, 0.0)];
    assert_eq!(
        StemPlayer::new(tracks, 0).err(),
        Some(PlayerError::MixedRates)
    );
}

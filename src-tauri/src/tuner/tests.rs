//! Accuracy and first-reading time of the pitch analyzer on synthetic signals.
//!
//! Each case is silence (or noise), then a tone from `ONSET_SECS`. Times are
//! in samples: from the onset to the first frame that shows a pitch. They
//! measure the analyzer alone (windows, hops, confirmation); the input
//! callback (64 frames ≈ 1.3 ms), the analysis thread's polling (≤ 1 ms),
//! IPC and the screen's next animation frame come on top.

use std::f64::consts::TAU;

use super::analyzer::{Analyzer, Frame};
use super::notes::cents_between;

const ONSET_SECS: f64 = 0.1;
const TONE_SECS: f64 = 0.6;
/// Callback size of the input stream.
const BLOCK: usize = 64;

/// B0 (5-string bass) up to E6.
const NOTES: [(&str, f64); 15] = [
    ("B0", 30.8677),
    ("E1", 41.2034),
    ("A1", 55.0),
    ("D2", 73.4162),
    ("E2", 82.4069),
    ("A2", 110.0),
    ("D3", 146.832),
    ("G3", 195.998),
    ("B3", 246.942),
    ("E4", 329.628),
    ("A4", 440.0),
    ("E5", 659.255),
    ("A5", 880.0),
    ("C6", 1046.5),
    ("E6", 1318.51),
];

#[derive(Clone, Copy, Debug)]
enum Wave {
    Sine,
    /// Band-limited sawtooth (every harmonic below Nyquist, 1/n).
    Saw,
    /// Plucked-string-like: harmonics 1/n with random phases, each decaying
    /// faster than the one below.
    String,
    /// Weak fundamental under a strong second harmonic (amplitudes 0.1, 1,
    /// then 0.3 and 0.15 on harmonics 4 and 6; no other odd harmonic), so
    /// half the period nearly repeats. `phase` shifts the second harmonic
    /// against the fundamental; `fundamental_decay` is per second (the even
    /// harmonics decay at 3/s).
    WeakFundamental {
        phase: f64,
        fundamental_decay: f64,
    },
}

/// Deterministic noise (xorshift64), uniform in −1..1 (mean 0, RMS 1/√3).
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        // 53 random bits → [0, 1) → [−1, 1).
        (self.0 >> 11) as f64 / (1_u64 << 53) as f64 * 2.0 - 1.0
    }
}

#[test]
fn test_noise_is_uniform_with_zero_mean() {
    let mut noise = Noise(0x2545_F491_4F6C_DD1D);
    let values: Vec<f64> = (0..200_000).map(|_| noise.next()).collect();
    let (min, max) = values
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let rms = (values.iter().map(|v| v * v).sum::<f64>() / values.len() as f64).sqrt();
    assert!(min >= -1.0 && max < 1.0, "range {min}..{max}");
    assert!(
        min < -0.999 && max > 0.999,
        "covers the range: {min}..{max}"
    );
    assert!(mean.abs() < 0.01, "mean {mean}");
    assert!((rms - 1.0 / 3_f64.sqrt()).abs() < 0.005, "rms {rms}");
}

fn harmonics(frequency: f64, rate: f64) -> usize {
    ((rate / 2.0 * 0.95) / frequency).floor().max(1.0) as usize
}

/// The tone alone, `TONE_SECS` long, peak around `amplitude`.
fn tone(wave: Wave, frequency: f64, rate: f64, amplitude: f64) -> Vec<f64> {
    let len = (TONE_SECS * rate) as usize;
    let count = harmonics(frequency, rate);
    let mut phases = Noise(0x9E37_79B9_7F4A_7C15 ^ frequency.to_bits());
    let partials: Vec<(f64, f64, f64)> = (1..=count)
        .map(|n| {
            let n_f = n as f64;
            match wave {
                Wave::Sine => (if n == 1 { 1.0 } else { 0.0 }, 0.0, 0.0),
                Wave::Saw => (1.0 / n_f, 0.0, 0.0),
                // (amplitude, phase, decay per second)
                Wave::String => (1.0 / n_f, phases.next() * TAU, 1.5 + 0.8 * n_f),
                Wave::WeakFundamental {
                    phase,
                    fundamental_decay,
                } => match n {
                    1 => (0.1, 0.0, fundamental_decay),
                    2 => (1.0, phase, 3.0),
                    4 => (0.3, phase * 2.0 + 0.5, 3.0),
                    6 => (0.15, phase * 3.0 + 1.0, 3.0),
                    _ => (0.0, 0.0, 0.0),
                },
            }
        })
        .collect();
    let norm = match wave {
        Wave::Sine => 1.0,
        Wave::Saw | Wave::String => 0.6,
        Wave::WeakFundamental { .. } => 0.9,
    };
    (0..len)
        .map(|i| {
            let t = i as f64 / rate;
            partials
                .iter()
                .enumerate()
                .filter(|(_, (a, _, _))| *a > 0.0)
                .map(|(k, &(a, phase, decay))| {
                    let n = (k + 1) as f64;
                    a * (-decay * t).exp() * (TAU * n * frequency * t + phase).sin()
                })
                .sum::<f64>()
                * amplitude
                * norm
        })
        .collect()
}

/// Silence (or noise), then the tone from `ONSET_SECS`.
fn signal(wave: Wave, frequency: f64, rate: f64, noise_db: Option<f64>) -> Vec<f32> {
    let onset = (ONSET_SECS * rate) as usize;
    let tone = tone(wave, frequency, rate, 0.3);
    let mut noise = Noise(0x2545_F491_4F6C_DD1D);
    let noise_amp = noise_db.map_or(0.0, |db| 10_f64.powf(db / 20.0) * 3_f64.sqrt());
    (0..onset + tone.len())
        .map(|i| {
            let sample = if i >= onset { tone[i - onset] } else { 0.0 };
            (sample + noise_amp * noise.next()) as f32
        })
        .collect()
}

struct Outcome {
    /// Seconds from the onset to the first frame with a pitch.
    first_secs: f64,
    /// Cents error of the first reading.
    first_cents: f64,
    /// Largest cents error of every reading after `SETTLE_SECS`.
    settled_cents: f64,
    /// Seconds from the onset to the first reading within ±1 cent.
    within_secs: f64,
    /// Seconds from the onset after which every reading stays within ±1 cent.
    stays_secs: f64,
    /// Frames after the first reading without a pitch or with another note.
    dropouts: usize,
}

/// Readings later than this after the onset must be within ±1 cent.
const SETTLE_SECS: f64 = 0.15;

fn run(samples: &[f32], rate: f64, frequency: f64) -> Outcome {
    let mut analyzer = Analyzer::new(rate as u32, 440.0);
    let mut frames: Vec<Frame> = Vec::new();
    for block in samples.chunks(BLOCK) {
        analyzer.push(block, |frame| frames.push(*frame));
    }
    let onset = (ONSET_SECS * rate) as u64;
    let early: Vec<&Frame> = frames
        .iter()
        .filter(|f| f.sample <= onset && f.pitch.is_some())
        .collect();
    assert!(early.is_empty(), "pitch before the onset: {:?}", early[0]);
    let first = frames
        .iter()
        .find(|f| f.pitch.is_some())
        .unwrap_or_else(|| panic!("no reading for {frequency} Hz"));
    let first_reading = first.pitch.unwrap();
    let target_midi = (69.0 + 12.0 * (frequency / 440.0).log2()).round() as i32;
    let settle = onset + (SETTLE_SECS * rate) as u64;
    let mut settled_cents: f64 = 0.0;
    let mut dropouts = 0;
    let mut within = None;
    let mut stays = first.sample;
    for frame in frames.iter().filter(|f| f.sample >= first.sample) {
        match frame.pitch {
            Some(reading) if reading.midi == target_midi => {
                let error = cents_between(frequency, reading.frequency).abs();
                if error <= 1.0 {
                    within.get_or_insert(frame.sample);
                } else {
                    stays = frame.sample;
                }
                if frame.sample >= settle {
                    settled_cents = settled_cents.max(error);
                }
            }
            _ => dropouts += 1,
        }
    }
    let secs = |sample: u64| (sample - onset) as f64 / rate;
    assert_eq!(
        first_reading.midi, target_midi,
        "{frequency} Hz: first note {first_reading:?}"
    );
    Outcome {
        first_secs: secs(first.sample),
        first_cents: cents_between(frequency, first_reading.frequency),
        settled_cents,
        within_secs: within.map_or(f64::INFINITY, secs),
        stays_secs: secs(stays),
        dropouts,
    }
}

/// c1 latency bound: 50 ms from E2 (82 Hz) up, two periods + 20 ms below.
fn latency_limit(frequency: f64) -> f64 {
    if frequency >= 82.0 {
        0.05
    } else {
        2.0 / frequency + 0.02
    }
}

fn check(wave: Wave, rate: f64, noise_db: Option<f64>) {
    check_notes(wave, rate, noise_db, &NOTES);
}

fn check_notes(wave: Wave, rate: f64, noise_db: Option<f64>, notes: &[(&str, f64)]) {
    let mut report = String::new();
    let mut failures = Vec::new();
    for &(name, frequency) in notes {
        let outcome = run(&signal(wave, frequency, rate, noise_db), rate, frequency);
        let limit = latency_limit(frequency);
        report.push_str(&format!(
            "{wave:?} {rate} Hz noise {noise_db:?} {name:>3} {frequency:>8.3} Hz: first {:>5.1} ms (limit {:>5.1}) first error {:+.2} c, ±1 c from {:>5.1} ms (stays from {:>5.1} ms), settled max {:.3} c, dropouts {}\n",
            outcome.first_secs * 1000.0,
            limit * 1000.0,
            outcome.first_cents,
            outcome.within_secs * 1000.0,
            outcome.stays_secs * 1000.0,
            outcome.settled_cents,
            outcome.dropouts,
        ));
        if outcome.first_secs > limit {
            failures.push(format!(
                "{name}: first reading after {:.1} ms",
                outcome.first_secs * 1000.0
            ));
        }
        if outcome.settled_cents > 1.0 {
            failures.push(format!("{name}: {:.3} cents off", outcome.settled_cents));
        }
        if outcome.first_cents.abs() > 1.0 {
            failures.push(format!(
                "{name}: first reading {:+.2} cents off",
                outcome.first_cents
            ));
        }
        if outcome.dropouts > 0 {
            failures.push(format!("{name}: {} frames lost the note", outcome.dropouts));
        }
    }
    println!("{report}");
    assert!(failures.is_empty(), "{failures:#?}\n{report}");
}

#[test]
fn sine_from_b0_to_e6() {
    check(Wave::Sine, 48_000.0, None);
}

#[test]
fn sawtooth_from_b0_to_e6() {
    check(Wave::Saw, 48_000.0, None);
}

#[test]
fn decaying_string_from_b0_to_e6() {
    check(Wave::String, 48_000.0, None);
}

#[test]
fn decaying_string_with_noise() {
    // Noise 30 dB under the tone's starting level.
    check(Wave::String, 48_000.0, Some(-50.0));
}

#[test]
fn sawtooth_with_noise_at_44_1_khz() {
    check(Wave::Saw, 44_100.0, Some(-45.0));
}

#[test]
fn noise_alone_shows_no_pitch() {
    let rate = 48_000.0;
    let mut noise = Noise(7);
    let samples: Vec<f32> = (0..rate as usize)
        .map(|_| (0.05 * noise.next()) as f32)
        .collect();
    let mut analyzer = Analyzer::new(48_000, 440.0);
    let mut shown = 0;
    for block in samples.chunks(BLOCK) {
        analyzer.push(block, |frame| shown += usize::from(frame.pitch.is_some()));
    }
    assert_eq!(shown, 0);
}

#[test]
fn a_new_note_replaces_the_old_one_quickly() {
    let rate = 48_000.0;
    let mut samples = signal(Wave::String, 110.0, rate, None);
    let change = samples.len();
    let next = tone(Wave::String, 146.832, rate, 0.3);
    samples.extend(next.iter().map(|&s| s as f32));
    let mut analyzer = Analyzer::new(48_000, 440.0);
    let mut frames = Vec::new();
    for block in samples.chunks(BLOCK) {
        analyzer.push(block, |frame| frames.push(*frame));
    }
    let switched = frames
        .iter()
        .find(|f| f.sample > change as u64 && f.pitch.is_some_and(|p| p.midi == 50))
        .expect("D3 shown");
    let secs = (switched.sample - change as u64) as f64 / rate;
    assert!(secs <= 0.05, "D3 after {:.1} ms", secs * 1000.0);
    // Never an unrelated note in between.
    for frame in frames.iter().filter(|f| f.sample > change as u64) {
        if let Some(pitch) = frame.pitch {
            assert!(pitch.midi == 45 || pitch.midi == 50, "{pitch:?}");
        }
    }
}

#[test]
fn reference_pitch_moves_the_cents() {
    let rate = 48_000.0;
    let samples = signal(Wave::Sine, 432.0, rate, None);
    let mut analyzer = Analyzer::new(48_000, 432.0);
    let mut last = None;
    for block in samples.chunks(BLOCK) {
        analyzer.push(block, |frame| last = frame.pitch.or(last));
    }
    let reading = last.expect("reading");
    assert_eq!(reading.midi, 69);
    assert!(reading.cents.abs() < 1.0, "{reading:?}");

    analyzer.set_reference(440.0);
    let mut moved = None;
    analyzer.push(&samples[samples.len() - 4800..], |frame| {
        moved = frame.pitch
    });
    let moved = moved.expect("reading");
    assert_eq!(moved.midi, 69);
    assert!(
        (moved.cents - cents_between(440.0, 432.0)).abs() < 1.0,
        "{moved:?}"
    );
}

// ---- weak fundamental (octave errors) ---------------------------------------

/// Bass and low guitar notes, where a weak fundamental is common.
const LOW_NOTES: [(&str, f64); 7] = [
    ("B0", 30.8677),
    ("E1", 41.2034),
    ("A1", 55.0),
    ("D2", 73.4162),
    ("E2", 82.4069),
    ("A2", 110.0),
    ("E3", 164.814),
];

#[test]
fn weak_fundamental_is_not_read_an_octave_up() {
    for (phase, fundamental_decay) in [(0.0, 3.0), (1.6, 1.0), (3.1, 6.0)] {
        check_notes(
            Wave::WeakFundamental {
                phase,
                fundamental_decay,
            },
            48_000.0,
            None,
            &LOW_NOTES,
        );
    }
}

#[test]
fn weak_fundamental_with_noise() {
    check_notes(
        Wave::WeakFundamental {
            phase: 0.8,
            fundamental_decay: 2.0,
        },
        48_000.0,
        Some(-60.0),
        &LOW_NOTES,
    );
}

// ---- note changes ------------------------------------------------------------

const FROM_SECS: f64 = 0.4;
const TO_SECS: f64 = 0.4;

#[derive(Clone, Copy, Debug)]
enum Change {
    /// Steady sines at the same level, switched without a gap or attack.
    SameLevelSine,
    /// Decaying strings; the new one starts at the level the old one reached
    /// (no louder attack).
    SameLevelString,
    /// Decaying strings with 30 ms of silence between.
    ShortSilence,
    /// A fresh pluck on a decayed note.
    NewPluck,
}

fn rms(samples: &[f64]) -> f64 {
    (samples.iter().map(|s| s * s).sum::<f64>() / samples.len() as f64).sqrt()
}

/// Returns the signal and the sample where the new note starts.
fn note_change(change: Change, from: f64, to: f64, rate: f64) -> (Vec<f32>, usize) {
    let from_len = (FROM_SECS * rate) as usize;
    let to_len = (TO_SECS * rate) as usize;
    let wave = match change {
        Change::SameLevelSine => Wave::Sine,
        _ => Wave::String,
    };
    let old = &tone(wave, from, rate, 0.3)[..from_len];
    let mut new = tone(wave, to, rate, 0.3)[..to_len].to_vec();
    if matches!(change, Change::SameLevelString) {
        let tail = (0.01 * rate) as usize;
        let scale = rms(&old[from_len - tail..]) / rms(&new[..tail]);
        new.iter_mut().for_each(|s| *s *= scale);
    }
    let gap = match change {
        Change::ShortSilence => (0.03 * rate) as usize,
        _ => 0,
    };
    let lead = (ONSET_SECS * rate) as usize;
    let mut samples = vec![0.0_f32; lead];
    samples.extend(old.iter().map(|&s| s as f32));
    samples.extend(std::iter::repeat_n(0.0, gap));
    let start = samples.len();
    samples.extend(new.iter().map(|&s| s as f32));
    (samples, start)
}

#[test]
fn a_low_note_after_a_high_one_is_found_in_time() {
    let rate = 48_000.0;
    let mut report = String::new();
    let mut failures = Vec::new();
    for (from_name, from) in [("A4", 440.0), ("E4", 329.628)] {
        for (to_name, to) in [("B0", 30.8677), ("E1", 41.2034)] {
            for change in [
                Change::SameLevelSine,
                Change::SameLevelString,
                Change::ShortSilence,
                Change::NewPluck,
            ] {
                let (samples, start) = note_change(change, from, to, rate);
                let from_midi = (69.0 + 12.0 * (from / 440.0).log2()).round() as i32;
                let to_midi = (69.0 + 12.0 * (to / 440.0).log2()).round() as i32;
                let mut analyzer = Analyzer::new(48_000, 440.0);
                let mut frames = Vec::new();
                for block in samples.chunks(BLOCK) {
                    analyzer.push(block, |frame| frames.push(*frame));
                }
                let start = start as u64;
                let shown_before = frames
                    .iter()
                    .any(|f| f.sample < start && f.pitch.is_some_and(|p| p.midi == from_midi));
                let first = frames
                    .iter()
                    .find(|f| f.sample >= start && f.pitch.is_some_and(|p| p.midi == to_midi));
                let limit = 2.0 / to + 0.02;
                let label = format!("{from_name}→{to_name} {change:?}");
                let Some(first) = first else {
                    failures.push(format!("{label}: new note never shown"));
                    continue;
                };
                let secs = (first.sample - start) as f64 / rate;
                report.push_str(&format!(
                    "{label}: new note after {:.1} ms (limit {:.1})\n",
                    secs * 1000.0,
                    limit * 1000.0
                ));
                if !shown_before {
                    failures.push(format!("{label}: old note never shown"));
                }
                if secs > limit {
                    failures.push(format!("{label}: new note after {:.1} ms", secs * 1000.0));
                }
                for frame in frames.iter().filter(|f| f.sample >= start) {
                    match frame.pitch {
                        Some(p) if p.midi != from_midi && p.midi != to_midi => {
                            failures.push(format!("{label}: other note {p:?}"));
                            break;
                        }
                        Some(p) if frame.sample > first.sample && p.midi != to_midi => {
                            failures.push(format!("{label}: went back to {p:?}"));
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    println!("{report}");
    assert!(failures.is_empty(), "{failures:#?}\n{report}");
}

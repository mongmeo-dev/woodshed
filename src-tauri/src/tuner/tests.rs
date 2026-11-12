//! Accuracy and first-reading time of the pitch analyzer on synthetic signals.
//!
//! Each case is silence (or noise), then a tone from `ONSET_SECS`. Times are
//! in samples: the time from the onset to the first frame that shows a pitch.

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
}

/// Deterministic noise (xorshift), uniform in −1..1.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1_u64 << 52) as f64 * 2.0 - 1.0
    }
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
            }
        })
        .collect();
    let norm = match wave {
        Wave::Sine => 1.0,
        Wave::Saw | Wave::String => 0.6,
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
    for frame in frames.iter().filter(|f| f.sample > first.sample) {
        match frame.pitch {
            Some(reading) if reading.midi == target_midi => {
                if frame.sample >= settle {
                    let error = cents_between(frequency, reading.frequency);
                    settled_cents = settled_cents.max(error.abs());
                }
            }
            _ => dropouts += 1,
        }
    }
    assert_eq!(
        first_reading.midi, target_midi,
        "{frequency} Hz: first note {first_reading:?}"
    );
    Outcome {
        first_secs: (first.sample - onset) as f64 / rate,
        first_cents: cents_between(frequency, first_reading.frequency),
        settled_cents,
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
    let mut report = String::new();
    let mut failures = Vec::new();
    for (name, frequency) in NOTES {
        let outcome = run(&signal(wave, frequency, rate, noise_db), rate, frequency);
        let limit = latency_limit(frequency);
        report.push_str(&format!(
            "{wave:?} {rate} Hz noise {noise_db:?} {name:>3} {frequency:>8.3} Hz: first {:>5.1} ms (limit {:>5.1}) first error {:+.2} c, settled max {:.3} c, dropouts {}\n",
            outcome.first_secs * 1000.0,
            limit * 1000.0,
            outcome.first_cents,
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
        if outcome.first_cents.abs() > 5.0 {
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

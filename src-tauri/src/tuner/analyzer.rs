//! Turns a stream of mono samples into tuner frames: input level, a noise
//! gate, onset detection, MPM pitch estimates on an adaptive window, and a
//! stabilizer that keeps the reading steady without delaying real changes.
//!
//! Runs on the analysis thread (never the audio callback). All buffers are
//! sized in [`Analyzer::new`]; [`Analyzer::push`] does not allocate.

use serde::Serialize;

use super::mpm::Mpm;
use super::notes::{cents_between, midi_of};

/// Lowest pitch searched (below B0 = 30.87 Hz, so a flat low B still reads).
pub const MIN_FREQUENCY: f64 = 27.0;
/// Highest pitch searched (above E6 = 1318.5 Hz).
pub const MAX_FREQUENCY: f64 = 1600.0;

/// Time between analyses while looking for a new pitch.
const HOP_SECS: f64 = 0.0025;
/// Longest analysis window: four periods of the lowest pitch... capped.
const MAX_WINDOW_SECS: f64 = 0.16;
/// Shortest window once a pitch is tracked (more periods = finer cents).
const MIN_TRACK_WINDOW_SECS: f64 = 0.04;
/// Periods per window while tracking.
const TRACK_PERIODS: f64 = 6.0;
/// Shortest window worth analysing.
const MIN_WINDOW_SECS: f64 = 0.003;

/// Level window for the meter and the gate.
const LEVEL_SECS: f64 = 0.01;
/// Gate opens at this RMS level and closes below the lower one.
const GATE_OPEN_DB: f64 = -60.0;
const GATE_CLOSE_DB: f64 = -66.0;
/// A pluck: the last `ONSET_SHORT_SECS` are this much louder (power ratio)
/// than the `ONSET_LONG_SECS` before them.
const ONSET_SHORT_SECS: f64 = 0.005;
const ONSET_LONG_SECS: f64 = 0.04;
const ONSET_POWER_RATIO: f64 = 4.0;
/// No new onset within this time after the last one.
const ONSET_REFRACTORY_SECS: f64 = 0.03;

/// NSDF clarity needed to start a reading, and to keep following one.
const ACQUIRE_CLARITY: f64 = 0.9;
const TRACK_CLARITY: f64 = 0.8;
/// Two estimates within this distance agree (confirms a new pitch).
const AGREE_CENTS: f64 = 35.0;
/// Smoothing of small changes (share of the new estimate per analysis).
const SMOOTHING: f64 = 0.25;
/// The shown note changes only this far past the half-semitone boundary.
const NOTE_HYSTERESIS: f64 = 0.1;
/// The last reading is kept this long when estimates stop (decay, noise).
const HOLD_SECS: f64 = 0.2;

const SILENCE_DB: f32 = -100.0;

/// Low-pass before analysis: keeps every fundamental up to [`MAX_FREQUENCY`]
/// and removes hiss and high harmonics that blur the NSDF peak.
const LOWPASS_HZ: f64 = 2000.0;
/// The analysed window starts where the signal first reaches this share of
/// the window's peak: silence before a pluck would lower the NSDF peak and
/// delay the first reading.
const TRIM_RATIO: f32 = 0.1;
/// Start-up of the low-pass skipped after a trimmed start.
const SETTLE_SECS: f64 = 0.0015;

/// A pitch as shown on the tuner.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub frequency: f64,
    /// Nearest MIDI note (with hysteresis); 69 = A4.
    pub midi: i32,
    /// Offset from `midi`, about −50..=50 (a little more near the hysteresis edge).
    pub cents: f64,
    pub clarity: f64,
}

/// Result of one analysis step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// Samples pushed so far when this frame was made (its "now").
    pub sample: u64,
    /// RMS level in dBFS over the last 10 ms.
    pub level_db: f32,
    pub pitch: Option<Reading>,
}

#[derive(Clone, Copy, Debug)]
struct Stable {
    frequency: f64,
    midi: i32,
    clarity: f64,
}

/// Biquad low-pass (RBJ cookbook), direct form I.
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Biquad {
    fn lowpass(rate: f64, cutoff: f64, q: f64) -> Self {
        let w = std::f64::consts::TAU * cutoff.min(rate * 0.45) / rate;
        let alpha = w.sin() / (2.0 * q);
        let cos = w.cos();
        let a0 = 1.0 + alpha;
        let b1 = (1.0 - cos) / a0;
        Self {
            b: [b1 / 2.0, b1, b1 / 2.0],
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    fn process(&mut self, input: f64) -> f64 {
        let output = self.b[0] * input + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [input, self.x[0]];
        self.y = [output, self.y[0]];
        output
    }
}

pub struct Analyzer {
    rate: f64,
    /// Fourth-order Butterworth low-pass as two biquads.
    lowpass: [Biquad; 2],
    reference: f64,
    mpm: Mpm,
    history: Vec<f32>,
    window: Vec<f32>,
    total: u64,
    since_analysis: usize,
    hop: usize,
    base_hop: usize,
    min_lag: usize,
    max_lag: usize,
    min_window: usize,
    settle: usize,
    min_track_window: usize,
    level_len: usize,
    onset_short: usize,
    onset_long: usize,
    onset_refractory: u64,
    hold: u64,

    gate_open: bool,
    segment_start: u64,
    last_onset: Option<u64>,
    previous: Option<f64>,
    stable: Option<Stable>,
    last_estimate: u64,
}

fn frames(rate: f64, secs: f64) -> usize {
    (rate * secs).round() as usize
}

fn power_db(power: f64) -> f64 {
    if power <= 1e-12 {
        f64::from(SILENCE_DB)
    } else {
        10.0 * power.log10()
    }
}

impl Analyzer {
    pub fn new(sample_rate: u32, reference_hz: f64) -> Self {
        let rate = f64::from(sample_rate);
        let max_window = frames(rate, MAX_WINDOW_SECS);
        let onset_long = frames(rate, ONSET_LONG_SECS);
        let onset_short = frames(rate, ONSET_SHORT_SECS);
        let base_hop = frames(rate, HOP_SECS).max(1);
        Self {
            rate,
            reference: reference_hz,
            lowpass: [
                Biquad::lowpass(rate, LOWPASS_HZ, 0.541_196_1),
                Biquad::lowpass(rate, LOWPASS_HZ, 1.306_563),
            ],
            mpm: Mpm::new(max_window),
            history: vec![0.0; max_window.max(onset_long + onset_short)],
            window: vec![0.0; max_window],
            total: 0,
            since_analysis: 0,
            hop: base_hop,
            base_hop,
            min_lag: (rate / MAX_FREQUENCY).floor() as usize,
            max_lag: (rate / MIN_FREQUENCY).ceil() as usize,
            min_window: frames(rate, MIN_WINDOW_SECS),
            settle: frames(rate, SETTLE_SECS),
            min_track_window: frames(rate, MIN_TRACK_WINDOW_SECS),
            level_len: frames(rate, LEVEL_SECS),
            onset_short,
            onset_long,
            onset_refractory: frames(rate, ONSET_REFRACTORY_SECS) as u64,
            hold: frames(rate, HOLD_SECS) as u64,
            gate_open: false,
            segment_start: 0,
            last_onset: None,
            previous: None,
            stable: None,
            last_estimate: 0,
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.rate
    }

    /// Changes A4; the next frame uses it.
    pub fn set_reference(&mut self, reference_hz: f64) {
        if (reference_hz - self.reference).abs() > f64::EPSILON {
            self.reference = reference_hz;
            if let Some(stable) = &mut self.stable {
                stable.midi = midi_of(stable.frequency, reference_hz).round() as i32;
            }
        }
    }

    /// Adds samples and calls `on_frame` after every analysis step.
    pub fn push(&mut self, samples: &[f32], mut on_frame: impl FnMut(&Frame)) {
        let capacity = self.history.len();
        for &sample in samples {
            let first = self.lowpass[0].process(f64::from(sample));
            let filtered = self.lowpass[1].process(first);
            self.history[(self.total % capacity as u64) as usize] = filtered as f32;
            self.total += 1;
            self.since_analysis += 1;
            if self.since_analysis >= self.hop {
                self.since_analysis = 0;
                let frame = self.analyze();
                on_frame(&frame);
            }
        }
    }

    /// Mean power of the `len` samples ending `skip` samples before now.
    fn power(&self, len: usize, skip: usize) -> f64 {
        let capacity = self.history.len() as u64;
        let end = self.total.saturating_sub(skip as u64);
        let start = end.saturating_sub(len as u64);
        if end <= start {
            return 0.0;
        }
        let sum: f64 = (start..end)
            .map(|i| f64::from(self.history[(i % capacity) as usize]).powi(2))
            .sum();
        sum / (end - start) as f64
    }

    fn analyze(&mut self) -> Frame {
        let level = power_db(self.power(self.level_len, 0));
        self.update_gate(level);
        let estimate = if self.gate_open {
            self.estimate()
        } else {
            None
        };
        self.stabilize(estimate);
        Frame {
            sample: self.total,
            level_db: level as f32,
            pitch: self.stable.map(|stable| self.reading(stable)),
        }
    }

    fn update_gate(&mut self, level_db: f64) {
        if !self.gate_open {
            if level_db >= GATE_OPEN_DB {
                self.gate_open = true;
                // The sound began somewhere in the level window.
                self.start_segment((self.level_len + self.hop) as u64);
            }
            return;
        }
        if level_db < GATE_CLOSE_DB {
            self.gate_open = false;
            self.previous = None;
            return;
        }
        // Compare only against a stretch inside the current sound: right after
        // an onset the stretch before would still hold the silence.
        let reference_start = self
            .total
            .saturating_sub((self.onset_short + self.onset_long) as u64);
        if reference_start < self.segment_start {
            return;
        }
        let recent = self.power(self.onset_short, 0);
        let before = self.power(self.onset_long, self.onset_short);
        let refractory = self
            .last_onset
            .is_some_and(|onset| self.total - onset < self.onset_refractory);
        if !refractory && recent > ONSET_POWER_RATIO * before.max(1e-12) {
            self.start_segment((self.onset_short + self.hop) as u64);
        }
    }

    /// Starts a new sound `back` samples ago: later windows only cover it, so
    /// the previous note does not blur the new one.
    fn start_segment(&mut self, back: u64) {
        self.segment_start = self.total.saturating_sub(back);
        self.last_onset = Some(self.total);
        self.previous = None;
        self.hop = self.base_hop;
    }

    /// One MPM estimate (frequency, clarity) on the window that fits now.
    fn estimate(&mut self) -> Option<(f64, f64)> {
        let available = (self.total - self.segment_start) as usize;
        let target = match self.stable {
            Some(stable) => ((TRACK_PERIODS * self.rate / stable.frequency).ceil() as usize)
                .max(self.min_track_window),
            None => usize::MAX,
        };
        let len = available.min(target).min(self.window.len());
        if len < self.min_window {
            return None;
        }
        // While tracking, analyse less often for long (low) windows.
        self.hop = if self.stable.is_some() {
            self.base_hop.max(len / 24)
        } else {
            self.base_hop
        };
        let capacity = self.history.len() as u64;
        let start = self.total - len as u64;
        for (offset, slot) in self.window[..len].iter_mut().enumerate() {
            *slot = self.history[((start + offset as u64) % capacity) as usize];
        }
        let window = &self.window[..len];
        let peak = window.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
        let head = match window.iter().position(|s| s.abs() >= TRIM_RATIO * peak) {
            // The sound starts inside the window: also skip the low-pass's
            // start-up, which is not periodic yet.
            Some(start) if start > 0 => start + self.settle,
            _ => 0,
        };
        if len < head + self.min_window {
            return None;
        }
        let estimate = self
            .mpm
            .detect(&self.window[head..len], self.min_lag, self.max_lag)?;
        let needed = if self.stable.is_some() {
            TRACK_CLARITY
        } else {
            ACQUIRE_CLARITY
        };
        (estimate.clarity >= needed).then(|| (self.rate / estimate.period, estimate.clarity))
    }

    fn stabilize(&mut self, estimate: Option<(f64, f64)>) {
        let Some((frequency, clarity)) = estimate else {
            self.previous = None;
            if self.stable.is_some() && self.total - self.last_estimate > self.hold {
                self.stable = None;
            }
            return;
        };
        let previous = self.previous.replace(frequency);
        match &mut self.stable {
            Some(stable) if cents_between(stable.frequency, frequency).abs() <= AGREE_CENTS => {
                // Same pitch: smooth in the log domain.
                let smoothed =
                    (stable.frequency.ln() * (1.0 - SMOOTHING) + frequency.ln() * SMOOTHING).exp();
                stable.frequency = smoothed;
                stable.clarity = clarity;
                let exact = midi_of(smoothed, self.reference);
                if (exact - f64::from(stable.midi)).abs() > 0.5 + NOTE_HYSTERESIS {
                    stable.midi = exact.round() as i32;
                }
                self.last_estimate = self.total;
            }
            _ => {
                // A new pitch is shown once two estimates in a row agree.
                if previous.is_some_and(|p| cents_between(p, frequency).abs() <= AGREE_CENTS) {
                    self.stable = Some(Stable {
                        frequency,
                        midi: midi_of(frequency, self.reference).round() as i32,
                        clarity,
                    });
                    self.last_estimate = self.total;
                } else if self.stable.is_some() && self.total - self.last_estimate > self.hold {
                    self.stable = None;
                }
            }
        }
    }

    fn reading(&self, stable: Stable) -> Reading {
        let exact = midi_of(stable.frequency, self.reference);
        Reading {
            frequency: stable.frequency,
            midi: stable.midi,
            cents: (exact - f64::from(stable.midi)) * 100.0,
            clarity: stable.clarity,
        }
    }
}

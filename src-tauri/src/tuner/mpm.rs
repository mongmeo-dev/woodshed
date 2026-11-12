//! McLeod Pitch Method (MPM): the normalized square difference function
//! (NSDF) of a window, computed through an FFT autocorrelation, then the first
//! "key maximum" close to the highest one, refined by parabolic interpolation.
//!
//! McLeod & Wyvill, "A Smarter Way to Find Pitch" (ICMC 2005).

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

/// A key maximum is taken when it reaches this share of the highest one
/// (the paper's `k`). Lower values prefer shorter periods (octave-up errors),
/// higher values longer ones (octave-down errors).
const KEY_MAX_RATIO: f64 = 0.9;

/// Smallest FFT size kept in the plan cache.
const MIN_FFT: usize = 64;

/// One pitch estimate from a window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    /// Period in samples, with sub-sample precision.
    pub period: f64,
    /// Height of the chosen NSDF peak (0..=1): how periodic the window is.
    pub clarity: f64,
}

struct Plan {
    forward: Arc<dyn RealToComplex<f64>>,
    inverse: Arc<dyn ComplexToReal<f64>>,
}

/// Reusable MPM detector. All buffers and FFT plans are made up front for
/// windows up to `max_window` samples, so [`Mpm::detect`] does not allocate.
pub struct Mpm {
    /// Plans for power-of-two sizes from [`MIN_FFT`], indexed by `log2(n) - log2(MIN_FFT)`.
    plans: Vec<Plan>,
    time: Vec<f64>,
    spectrum: Vec<Complex<f64>>,
    scratch_forward: Vec<Complex<f64>>,
    scratch_inverse: Vec<Complex<f64>>,
    nsdf: Vec<f64>,
    max_window: usize,
}

impl Mpm {
    pub fn new(max_window: usize) -> Self {
        let max_window = max_window.max(MIN_FFT / 2);
        let max_fft = (2 * max_window).next_power_of_two();
        let mut planner = RealFftPlanner::<f64>::new();
        let mut plans = Vec::new();
        let mut scratch = 0;
        let mut size = MIN_FFT;
        while size <= max_fft {
            let forward = planner.plan_fft_forward(size);
            let inverse = planner.plan_fft_inverse(size);
            scratch = scratch
                .max(forward.get_scratch_len())
                .max(inverse.get_scratch_len());
            plans.push(Plan { forward, inverse });
            size *= 2;
        }
        Self {
            plans,
            time: vec![0.0; max_fft],
            spectrum: vec![Complex::default(); max_fft / 2 + 1],
            scratch_forward: vec![Complex::default(); scratch],
            scratch_inverse: vec![Complex::default(); scratch],
            nsdf: vec![0.0; max_window],
            max_window,
        }
    }

    pub fn max_window(&self) -> usize {
        self.max_window
    }

    /// Estimates the period of `window` between `min_lag` and `max_lag`
    /// samples. The search is capped at half the window, so the window holds
    /// at least two periods. `None` when no key maximum is found, or when the
    /// best candidate could still be the octave above a period too long for
    /// this window (a longer window decides).
    pub fn detect(&mut self, window: &[f32], min_lag: usize, max_lag: usize) -> Option<Estimate> {
        let len = window.len().min(self.max_window);
        let window = &window[window.len() - len..];
        let search = max_lag.min(len / 2);
        let min_lag = min_lag.max(2);
        if search <= min_lag + 2 {
            return None;
        }
        self.nsdf_into(window, search + 2);
        let nsdf = &self.nsdf[..search + 2];
        let index = pick_peak(nsdf, min_lag)?;
        let estimate = refine(nsdf, index);
        if 2 * index > max_lag || 1.0 - estimate.clarity <= RESIDUAL_FLOOR {
            return Some(estimate);
        }
        match odd_part_flips(window, index) {
            // Half the true period: report the double, once the window is
            // long enough to measure it (the half-period peak is biased).
            Some(true) => lower_key(nsdf, 2 * index).map(|lag| refine(nsdf, lag)),
            Some(false) => Some(estimate),
            // Too short to tell: only a clean candidate is trusted.
            None => (1.0 - estimate.clarity <= CLEAN_RESIDUAL).then_some(estimate),
        }
    }

    /// Fills `self.nsdf[..lags]` with the NSDF of `window`.
    fn nsdf_into(&mut self, window: &[f32], lags: usize) {
        let len = window.len();
        let fft_size = (2 * len).next_power_of_two().max(MIN_FFT);
        let plan = &self.plans[(fft_size.trailing_zeros() - MIN_FFT.trailing_zeros()) as usize];
        let time = &mut self.time[..fft_size];
        let spectrum = &mut self.spectrum[..fft_size / 2 + 1];

        // Remove the window's mean so a DC offset does not read as periodicity.
        let mean = window.iter().map(|&s| f64::from(s)).sum::<f64>() / len as f64;
        for (slot, &sample) in time.iter_mut().zip(window) {
            *slot = f64::from(sample) - mean;
        }
        time[len..].fill(0.0);

        // Autocorrelation r(τ) = Σ x[j]·x[j+τ] via |FFT|².
        let _ = plan
            .forward
            .process_with_scratch(time, spectrum, &mut self.scratch_forward);
        for bin in spectrum.iter_mut() {
            *bin = Complex::new(bin.norm_sqr(), 0.0);
        }
        // The inverse needs real DC and Nyquist bins, which |X|² already has.
        let _ = plan
            .inverse
            .process_with_scratch(spectrum, time, &mut self.scratch_inverse);
        let scale = 1.0 / fft_size as f64;

        // m(τ) = Σ (x[j]² + x[j+τ]²), updated incrementally.
        let square = |j: usize| {
            let v = f64::from(window[j]) - mean;
            v * v
        };
        let mut m = 2.0 * (0..len).map(square).sum::<f64>();
        let lags = lags.min(len);
        for (tau, (value, &r)) in self.nsdf[..lags].iter_mut().zip(time.iter()).enumerate() {
            if tau > 0 {
                m -= square(tau - 1) + square(len - tau);
            }
            *value = if m > f64::EPSILON {
                2.0 * r * scale / m
            } else {
                0.0
            };
        }
    }
}

/// MPM peak picking on an NSDF: key maxima are the highest points of each
/// positive lobe after the NSDF first goes negative; the first key maximum
/// at or above `KEY_MAX_RATIO` × the highest wins. Returns its lag.
fn pick_peak(nsdf: &[f64], min_lag: usize) -> Option<usize> {
    let last = nsdf.len() - 1;
    // Skip the lobe around τ = 0.
    let mut tau = 1;
    while tau < last && nsdf[tau] > 0.0 {
        tau += 1;
    }
    let mut best = f64::MIN;
    let mut first_key = None;
    // Two passes would need storage; instead keep the keys' running best and
    // re-scan once at the end.
    while tau < last {
        while tau < last && nsdf[tau] <= 0.0 {
            tau += 1;
        }
        let peak = lobe_peak(nsdf, &mut tau);
        if let Some((index, value)) = peak.filter(|&(index, _)| index >= min_lag) {
            best = best.max(value);
            first_key.get_or_insert(index);
        }
    }
    first_key?;
    let threshold = KEY_MAX_RATIO * best;
    let mut tau = 1;
    while tau < last && nsdf[tau] > 0.0 {
        tau += 1;
    }
    while tau < last {
        while tau < last && nsdf[tau] <= 0.0 {
            tau += 1;
        }
        if let Some((index, value)) = lobe_peak(nsdf, &mut tau)
            && index >= min_lag
            && value >= threshold
        {
            return Some(index);
        }
    }
    None
}

/// Highest local maximum of the positive lobe starting at `tau`; advances
/// `tau` past the lobe. A lobe cut off by the end of the range has none
/// unless it already turned down.
fn lobe_peak(nsdf: &[f64], tau: &mut usize) -> Option<(usize, f64)> {
    let last = nsdf.len() - 1;
    let mut peak: Option<(usize, f64)> = None;
    while *tau < last && nsdf[*tau] > 0.0 {
        let t = *tau;
        if nsdf[t] >= nsdf[t - 1]
            && nsdf[t] >= nsdf[t + 1]
            && peak.is_none_or(|(_, value)| nsdf[t] > value)
        {
            peak = Some((t, nsdf[t]));
        }
        *tau += 1;
    }
    peak
}

/// The key maximum within 2 % of `lag`, if the NSDF reaches that far.
fn lower_key(nsdf: &[f64], lag: usize) -> Option<usize> {
    let tolerance = (lag / 50).max(2);
    let from = lag.checked_sub(tolerance)?.max(1);
    let to = (lag + tolerance).min(nsdf.len().checked_sub(2)?);
    if from >= to {
        return None;
    }
    (from..=to)
        .filter(|&t| nsdf[t] > 0.0 && nsdf[t] >= nsdf[t - 1] && nsdf[t] >= nsdf[t + 1])
        .max_by(|&a, &b| nsdf[a].total_cmp(&nsdf[b]))
}

/// The NSDF peak at `index`, refined by parabolic interpolation.
fn refine(nsdf: &[f64], index: usize) -> Estimate {
    let (offset, height) = parabolic(nsdf[index - 1], nsdf[index], nsdf[index + 1]);
    Estimate {
        period: index as f64 + offset,
        clarity: height.min(1.0),
    }
}

/// A candidate this periodic (residual `1 − NSDF` at most this) is taken
/// even when its octave check cannot run yet.
const CLEAN_RESIDUAL: f64 = 0.01;
/// Below this residual the candidate repeats perfectly: no octave check.
const RESIDUAL_FLOOR: f64 = 1e-4;
/// Correlation of `d(t)` and `d(t − lag)` at or below minus this means the
/// residual flips sign every `lag`. Noise alone gives −0.5 (both differences
/// share the sample at `t − lag`), decay and drift give about +1.
const FLIP_CORRELATION: f64 = 0.75;

/// Octave check for a candidate period `lag`. With a weak fundamental (strong
/// second harmonic) half the period repeats almost perfectly and MPM picks
/// it. What does not repeat, `d(t) = x(t) − x(t − lag)`, is then the odd
/// harmonics, which flip sign every `lag`: `d(t − lag) ≈ −d(t)`. When `lag`
/// is the true period, `d` is decay, drift or noise, which does not flip.
/// `Some(true)`: the true period is `2 × lag`. `None`: window too short.
fn odd_part_flips(window: &[f32], lag: usize) -> Option<bool> {
    let span = 2 * lag;
    if window.len() < span + (lag / 4).max(64) {
        return None;
    }
    let x = |t: usize| f64::from(window[t]);
    let (mut cross, mut now, mut before) = (0.0, 0.0, 0.0);
    for t in span..window.len() {
        let d0 = x(t) - x(t - lag);
        let d1 = x(t - lag) - x(t - span);
        cross += d0 * d1;
        now += d0 * d0;
        before += d1 * d1;
    }
    let norm = (now * before).sqrt();
    Some(norm > 0.0 && cross <= -FLIP_CORRELATION * norm)
}

/// Vertex of the parabola through (-1, a), (0, b), (1, c): (offset, height).
fn parabolic(a: f64, b: f64, c: f64) -> (f64, f64) {
    let denominator = a - 2.0 * b + c;
    if denominator.abs() < f64::EPSILON {
        return (0.0, b);
    }
    let offset = (0.5 * (a - c) / denominator).clamp(-0.5, 0.5);
    (offset, b - 0.25 * (a - c) * offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, rate: f64, len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| (0.5 * (std::f64::consts::TAU * freq * i as f64 / rate).sin()) as f32)
            .collect()
    }

    #[test]
    fn nsdf_matches_the_direct_sum() {
        let window = sine(220.0, 48_000.0, 700);
        let mut mpm = Mpm::new(1024);
        mpm.nsdf_into(&window, 350);
        let mean = window.iter().map(|&s| f64::from(s)).sum::<f64>() / window.len() as f64;
        for tau in [0, 1, 50, 218, 300] {
            let (mut r, mut m) = (0.0, 0.0);
            for j in 0..window.len() - tau {
                let x = f64::from(window[j]) - mean;
                let y = f64::from(window[j + tau]) - mean;
                r += x * y;
                m += x * x + y * y;
            }
            assert!((mpm.nsdf[tau] - 2.0 * r / m).abs() < 1e-9, "tau {tau}");
        }
    }

    #[test]
    fn finds_the_period_of_a_sine() {
        let mut mpm = Mpm::new(4096);
        let rate = 48_000.0;
        for freq in [41.2, 110.0, 440.0, 1318.5] {
            let window = sine(freq, rate, 4096);
            let estimate = mpm.detect(&window, 20, 2048).expect("pitch");
            let cents = 1200.0 * (rate / estimate.period / freq).log2();
            assert!(cents.abs() < 0.5, "{freq} Hz off by {cents} cents");
            assert!(estimate.clarity > 0.98);
        }
    }

    /// Fundamental at a tenth of the second harmonic: half the period repeats
    /// with NSDF ≈ 0.98, above the key-maximum threshold.
    #[test]
    fn a_weak_fundamental_is_not_read_an_octave_up() {
        let rate = 48_000.0;
        let mut mpm = Mpm::new(8192);
        for (freq, phase) in [(110.0, 0.0), (55.0, 1.3), (82.41, 2.6), (220.0, 0.7)] {
            let window: Vec<f32> = (0..4096)
                .map(|i| {
                    let t = std::f64::consts::TAU * freq * i as f64 / rate;
                    (0.03 * t.sin() + 0.3 * (2.0 * t + phase).sin()) as f32
                })
                .collect();
            let estimate = mpm.detect(&window, 20, 1778).expect("pitch");
            let cents = 1200.0 * (rate / estimate.period / freq).log2();
            assert!(cents.abs() < 1.0, "{freq} Hz read {cents:+.1} cents off");
        }
    }

    #[test]
    fn a_window_too_short_for_the_fundamental_waits_instead_of_reading_high() {
        let rate = 48_000.0;
        let freq = 110.0;
        let tone = |len: usize| -> Vec<f32> {
            (0..len)
                .map(|i| {
                    let t = std::f64::consts::TAU * freq * i as f64 / rate;
                    (0.03 * t.sin() + 0.3 * (2.0 * t).sin()) as f32
                })
                .collect()
        };
        let mut mpm = Mpm::new(4096);
        // 1.5 periods: only 220 Hz fits the lag range; its odd part flips.
        assert_eq!(mpm.detect(&tone(654), 20, 1778), None);
        // 2.2 periods: the fundamental is measured.
        let estimate = mpm.detect(&tone(960), 20, 1778).expect("pitch");
        let cents = 1200.0 * (rate / estimate.period / freq).log2();
        assert!(cents.abs() < 1.0, "{cents:+.1} cents");
    }

    #[test]
    fn noise_does_not_turn_a_clear_tone_into_its_lower_octave() {
        let rate = 48_000.0;
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        let mut noise = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1_u64 << 53) as f64 * 2.0 - 1.0
        };
        let mut mpm = Mpm::new(8192);
        for freq in [110.0, 220.0, 440.0] {
            let window: Vec<f32> = (0..4096)
                .map(|i| {
                    let t = std::f64::consts::TAU * freq * i as f64 / rate;
                    (0.3 * t.sin() + 0.01 * noise()) as f32
                })
                .collect();
            let estimate = mpm.detect(&window, 20, 1778).expect("pitch");
            let cents = 1200.0 * (rate / estimate.period / freq).log2();
            assert!(cents.abs() < 1.0, "{freq} Hz read {cents:+.1} cents off");
        }
    }

    #[test]
    fn a_decaying_pure_tone_is_not_read_an_octave_down() {
        let rate = 48_000.0;
        let mut mpm = Mpm::new(8192);
        for freq in [61.7, 110.0, 440.0] {
            let window: Vec<f32> = (0..6000)
                .map(|i| {
                    let t = i as f64 / rate;
                    ((-8.0 * t).exp() * (std::f64::consts::TAU * freq * t).sin()) as f32
                })
                .collect();
            let estimate = mpm.detect(&window, 20, 1778).expect("pitch");
            let cents = 1200.0 * (rate / estimate.period / freq).log2();
            assert!(cents.abs() < 1.0, "{freq} Hz read {cents:+.1} cents off");
        }
    }

    #[test]
    fn silence_and_dc_have_no_pitch() {
        let mut mpm = Mpm::new(2048);
        assert_eq!(mpm.detect(&[0.0; 2048], 20, 1024), None);
        assert_eq!(mpm.detect(&[0.3; 2048], 20, 1024), None);
    }
}

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
    /// samples (`max_lag` is capped at half the window, so the window holds at
    /// least two periods). `None` when no key maximum is found.
    pub fn detect(&mut self, window: &[f32], min_lag: usize, max_lag: usize) -> Option<Estimate> {
        let len = window.len().min(self.max_window);
        let window = &window[window.len() - len..];
        let max_lag = max_lag.min(len / 2);
        let min_lag = min_lag.max(2);
        if max_lag <= min_lag + 2 {
            return None;
        }
        self.nsdf_into(window, max_lag + 2);
        pick_peak(&self.nsdf[..max_lag + 2], min_lag)
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
/// at or above `KEY_MAX_RATIO` × the highest wins.
fn pick_peak(nsdf: &[f64], min_lag: usize) -> Option<Estimate> {
    let last = nsdf.len() - 1;
    // Skip the lobe around τ = 0.
    let mut tau = 1;
    while tau < last && nsdf[tau] > 0.0 {
        tau += 1;
    }
    let mut best = f64::MIN;
    // (index, value) of each lobe's maximum, at most a handful.
    let mut keys = [(0_usize, 0.0_f64); 64];
    let mut count = 0;
    while tau < last {
        // Find the next positive-going crossing.
        while tau < last && nsdf[tau] <= 0.0 {
            tau += 1;
        }
        let mut peak = (0, f64::MIN);
        while tau < last && nsdf[tau] > 0.0 {
            if nsdf[tau] > peak.1 && nsdf[tau] >= nsdf[tau - 1] && nsdf[tau] >= nsdf[tau + 1] {
                peak = (tau, nsdf[tau]);
            }
            tau += 1;
        }
        // A lobe cut off by the end of the search range is not a true maximum
        // unless it already turned down.
        if peak.0 >= min_lag && peak.0 > 0 && peak.0 < last && count < keys.len() {
            keys[count] = peak;
            count += 1;
            best = best.max(peak.1);
        }
    }
    let threshold = KEY_MAX_RATIO * best;
    let &(index, _) = keys[..count]
        .iter()
        .find(|(_, value)| *value >= threshold)?;
    let (offset, height) = parabolic(nsdf[index - 1], nsdf[index], nsdf[index + 1]);
    Some(Estimate {
        period: index as f64 + offset,
        clarity: height.min(1.0),
    })
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

    #[test]
    fn silence_and_dc_have_no_pitch() {
        let mut mpm = Mpm::new(2048);
        assert_eq!(mpm.detect(&[0.0; 2048], 20, 1024), None);
        assert_eq!(mpm.detect(&[0.3; 2048], 20, 1024), None);
    }
}

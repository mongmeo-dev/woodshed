//! Stem engine tests: the DSP against the numpy reference
//! (`testdata/dsp_reference.json`, from `tools/stem-model/make_dsp_fixture.py`).

use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::Deserialize;

use super::dsp::{
    FRAMES, FREQS, MIX_LEN, Normalization, SEGMENT, SOURCES, SPEC_LEN, Stft, chunk_count,
};
use super::*;

const PLANE: usize = FREQS * FRAMES;
const SR: f64 = 44_100.0;

/// Separation tests hold ~150 MB of buffers and up to four DSP threads each:
/// one at a time.
static HEAVY: Mutex<()> = Mutex::new(());

fn heavy() -> MutexGuard<'static, ()> {
    HEAVY.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---- inputs shared with make_dsp_fixture.py (keep them identical) ----------

/// Integer-hash noise in [-0.05, 0.05).
fn noise(i: usize, channel: u32) -> f64 {
    let mut u = (i as u32)
        .wrapping_mul(2_654_435_761)
        .wrapping_add(channel.wrapping_mul(40_503));
    u ^= u >> 16;
    u = u.wrapping_mul(0x045D_9F3B);
    u ^= u >> 16;
    (f64::from(u) / 4_294_967_296.0 - 0.5) * 0.1
}

fn test_signal(n: usize) -> (Vec<f32>, Vec<f32>) {
    let tau = 2.0 * std::f64::consts::PI;
    (0..n)
        .map(|i| {
            let x = i as f64;
            let t = x / SR;
            let left = 0.5 * (tau * 220.0 * x / SR).sin()
                + 0.25 * (tau * 3150.7 * x / SR).sin()
                + 0.1 * (tau * 0.37 * x / SR).sin()
                + 0.2 * (tau * (50.0 * t + 750.0 * (t * t))).sin()
                + noise(i, 0);
            let right = 0.4 * (tau * 330.0 * x / SR + 1.0).sin()
                + 0.2 * (tau * 5021.3 * x / SR).sin()
                - 0.05
                + noise(i, 1);
            (left as f32, right as f32)
        })
        .unzip()
}

fn test_spectrum(channels: usize) -> Vec<f32> {
    let mut spec = vec![0.0_f32; channels * PLANE];
    for ch in 0..channels {
        for f in 0..FREQS {
            for k in 0..FRAMES {
                let (chf, ff, kf) = (ch as f64, f as f64, k as f64);
                spec[ch * PLANE + f * FRAMES + k] =
                    ((0.37 * ff + 1.13 * kf + 2.1 * chf).sin() * (-ff / 800.0).exp()) as f32;
            }
        }
    }
    spec
}

const NUMPY_G: [f32; SOURCES] = [0.1, 0.2, 0.3, 0.15, 0.15, 0.1];
const NUMPY_H: [f32; SOURCES] = [0.05, -0.02, 0.01, 0.0, 0.03, -0.07];

/// `fake_core` of make_dsp_fixture.py: a position-dependent stand-in for the
/// model that exercises both branches.
struct NumpyFake;

impl StemModel for NumpyFake {
    fn predict(
        &mut self,
        mix: &[f32],
        spec: &[f32],
        spec_out: &mut [f32],
        wave_out: &mut [f32],
    ) -> Result<(), SeparationError> {
        for (s, &g) in NUMPY_G.iter().enumerate() {
            for j in 0..4 {
                for f in 0..FREQS {
                    let tilt = 1.0_f32 - f as f32 / 4096.0;
                    let from = j * PLANE + f * FRAMES;
                    let to = (s * 4 + j) * PLANE + f * FRAMES;
                    for k in 0..FRAMES {
                        spec_out[to + k] = spec[from + k] * g * tilt;
                    }
                }
            }
        }
        for (s, &h) in NUMPY_H.iter().enumerate() {
            for c in 0..2 {
                for i in 0..SEGMENT {
                    let ramp = i as f32 / SEGMENT as f32;
                    wave_out[(s * 2 + c) * SEGMENT + i] = mix[c * SEGMENT + i] * h * ramp;
                }
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct StftReference {
    points: Vec<(usize, usize, usize, f64)>,
    energy: Vec<f64>,
}

#[derive(Deserialize)]
struct PlanesReference {
    points: Vec<(usize, usize, usize, f64)>,
    energy: Vec<Vec<f64>>,
}

#[derive(Deserialize)]
struct SeparateReference {
    frames: usize,
    mean: f64,
    std: f64,
    points: Vec<(usize, usize, usize, f64)>,
    energy: Vec<Vec<f64>>,
}

#[derive(Deserialize)]
struct Reference {
    stft: StftReference,
    istft: PlanesReference,
    separate: SeparateReference,
}

fn reference() -> Reference {
    serde_json::from_str(include_str!("testdata/dsp_reference.json")).expect("valid fixture")
}

fn energy(samples: &[f32]) -> f64 {
    samples.iter().map(|&s| f64::from(s).powi(2)).sum()
}

fn assert_close(actual: f32, expected: f64, abs: f64, rel: f64, what: &str) {
    let error = (f64::from(actual) - expected).abs();
    assert!(
        error <= abs + rel * expected.abs(),
        "{what}: {actual} vs numpy {expected} (error {error:e})"
    );
}

fn assert_energy(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= 1e-4 * expected.abs(),
        "{what} energy: {actual} vs numpy {expected}"
    );
}

#[test]
fn stft_matches_numpy_reference() {
    let reference = reference().stft;
    let (mut chunk, right) = test_signal(SEGMENT);
    chunk.extend_from_slice(&right);
    let mut spec = vec![0.0; SPEC_LEN];
    Stft::new().forward(&chunk, &mut spec);
    for &(c, f, k, expected) in &reference.points {
        let what = format!("stft[{c}][{f}][{k}]");
        assert_close(
            spec[c * PLANE + f * FRAMES + k],
            expected,
            1e-5,
            1e-4,
            &what,
        );
    }
    for (c, &expected) in reference.energy.iter().enumerate() {
        let plane = &spec[c * PLANE..(c + 1) * PLANE];
        assert_energy(energy(plane), expected, &format!("stft channel {c}"));
    }
}

#[test]
fn istft_matches_numpy_reference() {
    let reference = reference().istft;
    let spec = test_spectrum(8);
    let mut out = vec![0.0; 4 * SEGMENT];
    Stft::new().inverse(&spec, &mut out);
    for &(s, c, i, expected) in &reference.points {
        let what = format!("istft[{s}][{c}][{i}]");
        assert_close(out[(s * 2 + c) * SEGMENT + i], expected, 1e-6, 1e-4, &what);
    }
    for (s, channels) in reference.energy.iter().enumerate() {
        for (c, &expected) in channels.iter().enumerate() {
            let plane = &out[(s * 2 + c) * SEGMENT..(s * 2 + c + 1) * SEGMENT];
            assert_energy(energy(plane), expected, &format!("istft {s}/{c}"));
        }
    }
}

#[test]
fn istft_inverts_stft_away_from_the_chunk_edges() {
    // Band-limited (the Nyquist bin is dropped, as in Demucs).
    let tau = 2.0 * std::f64::consts::PI;
    let chunk: Vec<f32> = (0..MIX_LEN)
        .map(|n| {
            let (c, i) = ((n / SEGMENT) as f64, (n % SEGMENT) as f64);
            let t = i / SR;
            (0.5 * (tau * 220.0 * t + c).sin()
                + 0.25 * (tau * 3150.7 * t).sin()
                + 0.2 * (tau * (50.0 * t + 750.0 * (t * t))).sin()) as f32
        })
        .collect();
    let stft = Stft::new();
    let mut spec = vec![0.0; SPEC_LEN];
    stft.forward(&chunk, &mut spec);
    let mut back = vec![0.0; MIX_LEN];
    stft.inverse(&spec, &mut back);
    // The first and last frames are dropped as in Demucs, so the outer
    // ~1.5k samples of a chunk do not reconstruct; the overlap-add weights
    // make them negligible.
    for c in 0..2 {
        for i in 2048..SEGMENT - 2048 {
            let (x, y) = (chunk[c * SEGMENT + i], back[c * SEGMENT + i]);
            assert!((x - y).abs() < 2e-5, "channel {c} sample {i}: {x} vs {y}");
        }
    }
}

/// Collects every output plane in memory.
struct Collect {
    planes: Vec<Vec<f32>>,
    chunks: Vec<(usize, usize)>,
}

impl Collect {
    fn new() -> Self {
        Self {
            planes: vec![Vec::new(); PLANES],
            chunks: Vec::new(),
        }
    }
}

impl SeparationOutput for Collect {
    fn write(&mut self, planes: &[f32], len: usize) -> Result<(), SeparationError> {
        for (plane, out) in self.planes.iter_mut().enumerate() {
            out.extend_from_slice(&planes[plane * len..(plane + 1) * len]);
        }
        Ok(())
    }

    fn chunk_done(&mut self, done: usize, total: usize) {
        self.chunks.push((done, total));
    }
}

#[test]
fn separation_matches_numpy_reference_across_chunks() {
    let _heavy = heavy();
    let reference = reference().separate;
    let n = reference.frames;
    assert_eq!(chunk_count(n), 3, "two full chunks and a short centred one");
    let (left, right) = test_signal(n);
    let norm = Normalization::of(&left, &right);
    assert!((norm.mean - reference.mean).abs() < 1e-9, "{norm:?}");
    assert!(
        (norm.std - reference.std).abs() < 1e-9 * reference.std,
        "{norm:?}"
    );

    let mut output = Collect::new();
    let mut work = Workspace::new();
    work.separate(
        &left,
        &right,
        &Stft::new(),
        &mut NumpyFake,
        &AtomicBool::new(false),
        &mut output,
    )
    .unwrap();
    assert_eq!(output.chunks, vec![(1, 3), (2, 3), (3, 3)]);
    assert!(output.planes.iter().all(|plane| plane.len() == n));
    for &(s, c, i, expected) in &reference.points {
        let what = format!("stem {s} channel {c} sample {i}");
        assert_close(output.planes[s * 2 + c][i], expected, 2e-5, 1e-4, &what);
    }
    for (s, channels) in reference.energy.iter().enumerate() {
        for (c, &expected) in channels.iter().enumerate() {
            let what = format!("stem {s}/{c}");
            assert_energy(energy(&output.planes[s * 2 + c]), expected, &what);
        }
    }
}

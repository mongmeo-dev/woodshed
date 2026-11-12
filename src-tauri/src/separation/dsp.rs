//! The signal processing around the Core ML model, ported from the numpy
//! reference `tools/stem-model/dsp.py` (Demucs `apply_model` with
//! `shifts=0, split=True, overlap=0.25`): the complex-as-channels STFT the
//! model takes, the iSTFT of its spectral output, the input normalisation and
//! the triangle-weighted overlap-add of 7.8 s chunks.
//!
//! Buffers are planar `f32` in the model's layout: a chunk is `[2][SEGMENT]`,
//! a spectrum `[channels][FREQS][FRAMES]`.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

/// The model's sample rate; songs at other rates are resampled around it.
pub const MODEL_RATE: u32 = 44_100;
/// Chunk length: `int(7.8 * 44100)`.
pub const SEGMENT: usize = 343_980;
pub const NFFT: usize = 4096;
pub const HOP: usize = 1024;
/// Reflect padding before the STFT: `HOP / 2 * 3`.
pub const PAD: usize = HOP / 2 * 3;
/// STFT frames per chunk.
pub const FRAMES: usize = SEGMENT.div_ceil(HOP);
/// STFT bins kept (the Nyquist bin is dropped).
pub const FREQS: usize = NFFT / 2;
/// Chunk hop for overlap 0.25: `int(0.75 * SEGMENT)`.
pub const STRIDE: usize = 257_985;
/// Separated sources the model returns.
pub const SOURCES: usize = 6;
/// Model input `mix`: `[1, 2, SEGMENT]`.
pub const MIX_LEN: usize = 2 * SEGMENT;
/// Model input `spec`: `[1, 4, FREQS, FRAMES]` (L.re, L.im, R.re, R.im).
pub const SPEC_LEN: usize = 4 * FREQS * FRAMES;
/// Model output `spec_out`: `[1, 24, FREQS, FRAMES]`, source s channel c
/// real/imaginary at `s * 4 + c * 2 + {0, 1}`.
pub const SPEC_OUT_LEN: usize = SOURCES * SPEC_LEN;
/// Model output `wave_out`: `[1, 12, SEGMENT]`, source s channel c at `s * 2 + c`.
pub const WAVE_OUT_LEN: usize = SOURCES * 2 * SEGMENT;
/// Upper bound on the DSP worker threads (the Mac is shared).
pub const MAX_THREADS: usize = 4;

/// One spectral plane (real or imaginary part of one channel).
const PLANE: usize = FREQS * FRAMES;
/// iSTFT frames: two silent frames on each side, as `torch.istft` sees them.
const ISTFT_FRAMES: usize = FRAMES + 4;
const ISTFT_LEN: usize = (ISTFT_FRAMES - 1) * HOP + NFFT;
/// First kept iSTFT sample: centre padding (NFFT / 2) plus [`PAD`].
const ISTFT_START: usize = NFFT / 2 + PAD;

/// Index into a chunk for padded-signal position `p` (reflect padding without
/// repeating the edge sample, as numpy `mode="reflect"`).
fn reflect(p: usize) -> usize {
    let j = p as isize - PAD as isize;
    let last = SEGMENT as isize - 1;
    let j = if j < 0 {
        -j
    } else if j > last {
        2 * last - j
    } else {
        j
    };
    j as usize
}

/// STFT and iSTFT in the model's layout. Cheap to share between threads.
pub struct Stft {
    /// Periodic Hann window (`torch.hann_window` default).
    window: Vec<f32>,
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
    /// `1 / Σ window²` over the kept iSTFT samples.
    inv_envelope: Vec<f32>,
    threads: usize,
}

impl Default for Stft {
    fn default() -> Self {
        Self::new()
    }
}

impl Stft {
    pub fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let window64: Vec<f64> = (0..NFFT)
            .map(|n| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / NFFT as f64).cos())
            .collect();
        let mut envelope = vec![0.0_f64; ISTFT_LEN];
        for frame in 0..ISTFT_FRAMES {
            for (n, w) in window64.iter().enumerate() {
                envelope[frame * HOP + n] += w * w;
            }
        }
        let inv_envelope = envelope[ISTFT_START..ISTFT_START + SEGMENT]
            .iter()
            .map(|e| (1.0 / e) as f32)
            .collect();
        let threads = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .clamp(1, MAX_THREADS);
        Self {
            window: window64.iter().map(|&w| w as f32).collect(),
            forward: planner.plan_fft_forward(NFFT),
            inverse: planner.plan_fft_inverse(NFFT),
            inv_envelope,
            threads,
        }
    }

    /// `chunk` `[2][SEGMENT]` → `spec` `[4][FREQS][FRAMES]` (L.re, L.im, R.re, R.im),
    /// normalised by `1 / sqrt(NFFT)`.
    pub fn forward(&self, chunk: &[f32], spec: &mut [f32]) {
        assert_eq!(chunk.len(), MIX_LEN, "chunk length");
        assert_eq!(spec.len(), SPEC_LEN, "spectrum length");
        let (left, right) = spec.split_at_mut(2 * PLANE);
        std::thread::scope(|scope| {
            scope.spawn(|| self.forward_channel(&chunk[..SEGMENT], left));
            self.forward_channel(&chunk[SEGMENT..], right);
        });
    }

    fn forward_channel(&self, x: &[f32], out: &mut [f32]) {
        let norm = 1.0 / (NFFT as f32).sqrt();
        let mut frame = self.forward.make_input_vec();
        let mut bins = self.forward.make_output_vec();
        let mut scratch = self.forward.make_scratch_vec();
        let (re, im) = out.split_at_mut(PLANE);
        for k in 0..FRAMES {
            let start = k * HOP;
            for (n, sample) in frame.iter_mut().enumerate() {
                *sample = x[reflect(start + n)] * self.window[n];
            }
            self.forward
                .process_with_scratch(&mut frame, &mut bins, &mut scratch)
                .expect("buffers come from the plan");
            for (f, bin) in bins.iter().take(FREQS).enumerate() {
                re[f * FRAMES + k] = bin.re * norm;
                im[f * FRAMES + k] = bin.im * norm;
            }
        }
    }

    /// `spec` `[planes * 2][FREQS][FRAMES]` (real, imaginary per plane) →
    /// `out` `[planes][SEGMENT]`. For the model output, plane `s * 2 + c` is
    /// source s channel c. Runs on up to [`MAX_THREADS`] threads.
    pub fn inverse(&self, spec: &[f32], out: &mut [f32]) {
        let planes = out.len() / SEGMENT;
        assert_eq!(out.len(), planes * SEGMENT, "output length");
        assert_eq!(spec.len(), planes * 2 * PLANE, "spectrum length");
        let threads = self.threads.min(planes).max(1);
        let mut groups: Vec<Vec<(usize, &mut [f32])>> = (0..threads).map(|_| Vec::new()).collect();
        for (plane, samples) in out.chunks_mut(SEGMENT).enumerate() {
            groups[plane % threads].push((plane, samples));
        }
        std::thread::scope(|scope| {
            for group in groups {
                scope.spawn(move || {
                    let mut work = InverseWork::new(self.inverse.as_ref());
                    for (plane, samples) in group {
                        let re = &spec[2 * plane * PLANE..(2 * plane + 1) * PLANE];
                        let im = &spec[(2 * plane + 1) * PLANE..(2 * plane + 2) * PLANE];
                        self.inverse_plane(re, im, samples, &mut work);
                    }
                });
            }
        });
    }

    fn inverse_plane(&self, re: &[f32], im: &[f32], out: &mut [f32], work: &mut InverseWork) {
        // realfft's inverse is unnormalised; irfft's 1/N and the 1/NORM undo
        // of the forward scaling leave 1 / sqrt(NFFT).
        let scale = 1.0 / (NFFT as f32).sqrt();
        work.overlap.fill(0.0);
        for k in 0..FRAMES {
            for f in 0..FREQS {
                work.bins[f] = Complex::new(re[f * FRAMES + k], im[f * FRAMES + k]);
            }
            // irfft ignores the imaginary part of DC; the Nyquist bin is zero.
            work.bins[0].im = 0.0;
            work.bins[FREQS] = Complex::new(0.0, 0.0);
            self.inverse
                .process_with_scratch(&mut work.bins, &mut work.frame, &mut work.scratch)
                .expect("buffers come from the plan");
            let at = (k + 2) * HOP;
            let target = &mut work.overlap[at..at + NFFT];
            for ((y, &s), &w) in target.iter_mut().zip(&work.frame).zip(&self.window) {
                *y += s * scale * w;
            }
        }
        let kept = &work.overlap[ISTFT_START..ISTFT_START + SEGMENT];
        for ((o, &y), &e) in out.iter_mut().zip(kept).zip(&self.inv_envelope) {
            *o = y * e;
        }
    }
}

/// Per-thread iSTFT buffers.
struct InverseWork {
    bins: Vec<Complex<f32>>,
    frame: Vec<f32>,
    scratch: Vec<Complex<f32>>,
    overlap: Vec<f32>,
}

impl InverseWork {
    fn new(inverse: &dyn ComplexToReal<f32>) -> Self {
        Self {
            bins: inverse.make_input_vec(),
            frame: inverse.make_output_vec(),
            scratch: inverse.make_scratch_vec(),
            overlap: vec![0.0; ISTFT_LEN],
        }
    }
}

/// Input normalisation of Demucs `api.py`: mean and standard deviation
/// (`ddof = 1`, plus 1e-8) of the mono mix `(L + R) / 2`, in f64.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Normalization {
    pub mean: f64,
    pub std: f64,
}

impl Normalization {
    pub fn of(left: &[f32], right: &[f32]) -> Self {
        let n = left.len().min(right.len());
        let mono = |i: usize| (f64::from(left[i]) + f64::from(right[i])) / 2.0;
        let mean = if n == 0 {
            0.0
        } else {
            (0..n).map(mono).sum::<f64>() / n as f64
        };
        let variance = if n < 2 {
            0.0
        } else {
            (0..n).map(|i| (mono(i) - mean).powi(2)).sum::<f64>() / (n - 1) as f64
        };
        let deviation = if variance.is_finite() {
            variance.sqrt()
        } else {
            0.0
        };
        Self {
            mean,
            std: deviation + 1e-8,
        }
    }
}

/// Overlap-add weights for one chunk: a triangle `1..=SEGMENT/2, SEGMENT/2..=1`
/// over its maximum.
pub fn triangle_weight() -> Vec<f32> {
    let half = SEGMENT / 2;
    (0..SEGMENT)
        .map(|i| {
            let rank = if i < half { i + 1 } else { SEGMENT - i };
            (rank as f64 / half as f64) as f32
        })
        .collect()
}

/// Number of chunks for a song of `frames` model-rate frames.
pub fn chunk_count(frames: usize) -> usize {
    frames.div_ceil(STRIDE)
}

/// Where chunk `index` of a `frames`-long song sits. A short last chunk is
/// centred in its window with zeros around it, as Demucs `TensorChunk.padded`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk {
    /// First song frame the chunk's output covers.
    pub offset: usize,
    /// Song frames it covers.
    pub len: usize,
    /// Song frame at window position 0 (may be negative or past the end).
    pub window_start: isize,
    /// Window position of `offset` (`(SEGMENT - len) / 2`).
    pub trim: usize,
}

impl Chunk {
    pub fn new(index: usize, frames: usize) -> Self {
        let offset = index * STRIDE;
        let len = SEGMENT.min(frames - offset);
        let trim = (SEGMENT - len) / 2;
        Self {
            offset,
            len,
            window_start: offset as isize - trim as isize,
            trim,
        }
    }
}

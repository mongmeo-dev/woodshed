//! Chunked separation of a whole song at the model rate, streaming: every
//! finished stretch of output goes to a sink as soon as no later chunk can
//! change it, so a long song never sits in memory as six stems (see
//! `tools/stem-model/dsp.py` `separate`).

use std::sync::atomic::{AtomicBool, Ordering};

use super::dsp::{
    Chunk, MIX_LEN, Normalization, SEGMENT, SOURCES, SPEC_LEN, SPEC_OUT_LEN, STRIDE, Stft,
    WAVE_OUT_LEN, chunk_count, triangle_weight,
};
use super::model::{SeparationError, StemModel};

/// Planes in the output: source s channel c at `s * 2 + c` (model order).
pub const PLANES: usize = SOURCES * 2;

/// Buffers for one separation run (about 140 MB), reused across chunks.
pub struct Workspace {
    mix: Vec<f32>,
    spec: Vec<f32>,
    spec_out: Vec<f32>,
    wave_out: Vec<f32>,
    stems: Vec<f32>,
    /// Weighted sum for song frames `[base, base + SEGMENT)`.
    acc: Vec<f32>,
    weight_sum: Vec<f32>,
    weight: Vec<f32>,
    block: Vec<f32>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            mix: vec![0.0; MIX_LEN],
            spec: vec![0.0; SPEC_LEN],
            spec_out: vec![0.0; SPEC_OUT_LEN],
            wave_out: vec![0.0; WAVE_OUT_LEN],
            stems: vec![0.0; WAVE_OUT_LEN],
            acc: vec![0.0; PLANES * SEGMENT],
            weight_sum: vec![0.0; SEGMENT],
            weight: triangle_weight(),
            block: vec![0.0; PLANES * SEGMENT],
        }
    }

    /// Runs the model once on a silent chunk (compiles GPU kernels ahead of
    /// the first song).
    pub fn warm_up(
        &mut self,
        stft: &Stft,
        model: &mut dyn StemModel,
    ) -> Result<(), SeparationError> {
        self.mix.fill(0.0);
        stft.forward(&self.mix, &mut self.spec);
        model.predict(
            &self.mix,
            &self.spec,
            &mut self.spec_out,
            &mut self.wave_out,
        )
    }
}

/// Where a separation's results go.
pub trait SeparationOutput {
    /// The next `len` output frames as `[PLANES][len]`, denormalised.
    fn write(&mut self, planes: &[f32], len: usize) -> Result<(), SeparationError>;
    /// Called after every chunk.
    fn chunk_done(&mut self, done: usize, total: usize);
}

impl Workspace {
    /// Separates `left`/`right` (model rate, same length) into `output`.
    /// Stops with [`SeparationError::Cancelled`] at the next chunk once
    /// `cancel` is set.
    pub fn separate(
        &mut self,
        left: &[f32],
        right: &[f32],
        stft: &Stft,
        model: &mut dyn StemModel,
        cancel: &AtomicBool,
        output: &mut dyn SeparationOutput,
    ) -> Result<(), SeparationError> {
        let work = self;
        let frames = left.len();
        if right.len() != frames {
            return Err(SeparationError::Input("channels differ in length".into()));
        }
        let norm = Normalization::of(left, right);
        let inv_std = 1.0 / norm.std;
        let total = chunk_count(frames);
        work.acc.fill(0.0);
        work.weight_sum.fill(0.0);

        for index in 0..total {
            if cancel.load(Ordering::Relaxed) {
                return Err(SeparationError::Cancelled);
            }
            let chunk = Chunk::new(index, frames);
            for (channel, source) in [left, right].into_iter().enumerate() {
                let window = &mut work.mix[channel * SEGMENT..(channel + 1) * SEGMENT];
                for (i, sample) in window.iter_mut().enumerate() {
                    let at = chunk.window_start + i as isize;
                    *sample = if at >= 0 && (at as usize) < frames {
                        ((f64::from(source[at as usize]) - norm.mean) * inv_std) as f32
                    } else {
                        0.0
                    };
                }
            }
            stft.forward(&work.mix, &mut work.spec);
            model.predict(
                &work.mix,
                &work.spec,
                &mut work.spec_out,
                &mut work.wave_out,
            )?;
            stft.inverse(&work.spec_out, &mut work.stems);

            let len = chunk.len;
            for plane in 0..PLANES {
                let from = plane * SEGMENT + chunk.trim;
                let stems = &work.stems[from..from + len];
                let wave = &work.wave_out[from..from + len];
                let acc = &mut work.acc[plane * SEGMENT..plane * SEGMENT + len];
                for (((a, &s), &w), &weight) in
                    acc.iter_mut().zip(stems).zip(wave).zip(&work.weight)
                {
                    *a += weight * (s + w);
                }
            }
            for (sum, &weight) in work.weight_sum[..len].iter_mut().zip(&work.weight) {
                *sum += weight;
            }

            // Frames before the next chunk's offset are final.
            let last = index + 1 == total;
            let done = if last { len } else { STRIDE };
            let (mean, std) = (norm.mean as f32, norm.std as f32);
            for plane in 0..PLANES {
                let acc = &work.acc[plane * SEGMENT..plane * SEGMENT + done];
                let out = &mut work.block[plane * done..(plane + 1) * done];
                for ((o, &a), &sum) in out.iter_mut().zip(acc).zip(&work.weight_sum) {
                    *o = a / sum * std + mean;
                }
            }
            output.write(&work.block[..PLANES * done], done)?;
            if !last {
                for plane in 0..PLANES {
                    let acc = &mut work.acc[plane * SEGMENT..(plane + 1) * SEGMENT];
                    acc.copy_within(done.., 0);
                    acc[SEGMENT - done..].fill(0.0);
                }
                work.weight_sum.copy_within(done.., 0);
                work.weight_sum[SEGMENT - done..].fill(0.0);
            }
            output.chunk_done(index + 1, total);
        }
        Ok(())
    }
}

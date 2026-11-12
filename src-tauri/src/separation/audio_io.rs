//! The song's playback copy in, model-rate stereo out; and the stems back in
//! the library format: 16-bit PCM WAV at the song rate, as many channels as
//! `audio.wav`, exactly `frames` long (see AGENTS.md "스템 라이브러리 계약").

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{Fft, FixedSync, Indexing, Resampler};

use super::MODEL_STEMS;
use super::dsp::MODEL_RATE;
use super::engine::PLANES;
use super::model::SeparationError;

/// Resampler chunk (frames at the input rate).
const RESAMPLE_CHUNK: usize = 1024;
/// Samples read between cancel checks.
const READ_BATCH: usize = 1 << 20;

/// A song's playback copy as two model-rate channels.
pub struct SongAudio {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    /// Rate, channels (1–2) and length of the file, which the stems match.
    pub rate: u32,
    pub channels: u16,
    pub frames: u64,
}

fn input_error(error: impl ToString) -> SeparationError {
    SeparationError::Input(error.to_string())
}

/// Reads a PCM or float WAV (the library's `audio.wav`) as stereo at
/// [`MODEL_RATE`]: mono is duplicated, a third channel and beyond dropped.
pub fn read_song(path: &Path, cancel: &AtomicBool) -> Result<SongAudio, SeparationError> {
    let mut reader = hound::WavReader::open(path).map_err(input_error)?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels);
    if channels == 0 || spec.sample_rate == 0 {
        return Err(input_error("the file has no audio"));
    }
    let frames = reader.duration() as usize;
    if frames == 0 {
        return Err(input_error("the file has no audio"));
    }
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(if channels > 1 { frames } else { 0 });
    let mut push = |index: usize, value: f32| match index % channels {
        0 => left.push(value),
        1 => right.push(value),
        _ => {}
    };
    match spec.sample_format {
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1_u64 << (spec.bits_per_sample.clamp(1, 32) - 1)) as f32;
            for (index, sample) in reader.samples::<i32>().enumerate() {
                if index % READ_BATCH == 0 && cancel.load(Ordering::Relaxed) {
                    return Err(SeparationError::Cancelled);
                }
                push(index, sample.map_err(input_error)? as f32 * scale);
            }
        }
        hound::SampleFormat::Float => {
            for (index, sample) in reader.samples::<f32>().enumerate() {
                if index % READ_BATCH == 0 && cancel.load(Ordering::Relaxed) {
                    return Err(SeparationError::Cancelled);
                }
                push(index, sample.map_err(input_error)?);
            }
        }
    }
    let frames = left.len();
    if channels == 1 {
        right = left.clone();
    }
    right.truncate(frames);
    right.resize(frames, 0.0);
    let (left, right) = if spec.sample_rate == MODEL_RATE {
        (left, right)
    } else {
        resample_clip([left, right], spec.sample_rate, MODEL_RATE)?
    };
    Ok(SongAudio {
        left,
        right,
        rate: spec.sample_rate,
        channels: spec.channels.min(2),
        frames: frames as u64,
    })
}

/// Resamples a whole two-channel clip, delay trimmed.
fn resample_clip(
    channels: [Vec<f32>; 2],
    from: u32,
    to: u32,
) -> Result<(Vec<f32>, Vec<f32>), SeparationError> {
    let frames = channels[0].len();
    let mut resampler = Fft::<f32>::new(
        from as usize,
        to as usize,
        RESAMPLE_CHUNK,
        2,
        FixedSync::Input,
    )
    .map_err(input_error)?;
    let capacity = resampler.process_all_needed_output_len(frames);
    let mut out = vec![vec![0.0_f32; capacity]; 2];
    let produced = {
        let input = SequentialSliceOfVecs::new(&channels[..], 2, frames).map_err(input_error)?;
        let mut output =
            SequentialSliceOfVecs::new_mut(&mut out[..], 2, capacity).map_err(input_error)?;
        resampler
            .process_all_into_buffer(&input, &mut output, frames, None)
            .map_err(input_error)?
            .1
    };
    drop(channels);
    let [mut left, mut right]: [Vec<f32>; 2] = out.try_into().expect("two channels");
    left.truncate(produced);
    right.truncate(produced);
    Ok((left, right))
}

/// Streams model-rate stem planes into six 16-bit WAV files at the song rate.
pub struct StemWriters {
    /// One per source, in model order ([`MODEL_STEMS`]).
    writers: Vec<hound::WavWriter<BufWriter<File>>>,
    channels: usize,
    target: u64,
    written: u64,
    resampler: Option<OutputResampler>,
    noise: u32,
}

/// 44.1 kHz → song rate for all planes, fed in arbitrary block sizes.
struct OutputResampler {
    fft: Fft<f32>,
    /// Model-rate frames not resampled yet, per plane.
    pending: Vec<Vec<f32>>,
    out: Vec<Vec<f32>>,
    /// Output frames still to drop for the resampler delay.
    skip: usize,
}

impl StemWriters {
    /// Creates `<dir>/<name>.wav` for every stem.
    pub fn create(
        dir: &Path,
        rate: u32,
        channels: u16,
        frames: u64,
    ) -> Result<Self, SeparationError> {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let writers = MODEL_STEMS
            .iter()
            .map(|name| hound::WavWriter::create(dir.join(format!("{}.wav", name.as_str())), spec))
            .collect::<Result<Vec<_>, _>>()?;
        let resampler = if rate == MODEL_RATE {
            None
        } else {
            let fft = Fft::<f32>::new(
                MODEL_RATE as usize,
                rate as usize,
                RESAMPLE_CHUNK,
                PLANES,
                FixedSync::Input,
            )
            .map_err(input_error)?;
            let out_frames = fft.output_frames_max();
            Some(OutputResampler {
                skip: fft.output_delay(),
                fft,
                pending: vec![Vec::new(); PLANES],
                out: vec![vec![0.0; out_frames]; PLANES],
            })
        };
        Ok(Self {
            writers,
            channels: usize::from(channels.clamp(1, 2)),
            target: frames,
            written: 0,
            resampler,
            noise: 0x9E37_79B9,
        })
    }

    /// Adds `len` model-rate frames given as `[PLANES][len]`.
    pub fn push(&mut self, planes: &[f32], len: usize) -> Result<(), SeparationError> {
        let Some(mut resampler) = self.resampler.take() else {
            return self.write(|plane, i| planes[plane * len + i], len);
        };
        for (plane, pending) in resampler.pending.iter_mut().enumerate() {
            pending.extend_from_slice(&planes[plane * len..(plane + 1) * len]);
        }
        let result = self.drain(&mut resampler, false);
        self.resampler = Some(resampler);
        result
    }

    /// Resamples whole chunks out of `pending`; with `flush`, also the rest
    /// and enough silence to reach the target length.
    fn drain(&mut self, r: &mut OutputResampler, flush: bool) -> Result<(), SeparationError> {
        let available = r.pending[0].len();
        let mut offset = 0;
        loop {
            let need = r.fft.input_frames_next();
            let partial = if available - offset >= need {
                None
            } else if flush && self.written < self.target {
                // Pads the remainder (then plain silence) with zeros.
                Some(available - offset)
            } else {
                break;
            };
            let produced = {
                let input = SequentialSliceOfVecs::new(&r.pending[..], PLANES, available)
                    .map_err(input_error)?;
                let frames = r.out[0].len();
                let mut output = SequentialSliceOfVecs::new_mut(&mut r.out[..], PLANES, frames)
                    .map_err(input_error)?;
                let indexing = Indexing {
                    input_offset: offset,
                    output_offset: 0,
                    partial_len: partial,
                    active_channels_mask: None,
                };
                r.fft
                    .process_into_buffer(&input, &mut output, Some(&indexing))
                    .map_err(input_error)?
                    .1
            };
            offset = match partial {
                Some(_) => available,
                None => offset + need,
            };
            let skip = r.skip.min(produced);
            r.skip -= skip;
            let out = &r.out;
            self.write(|plane, i| out[plane][skip + i], produced - skip)?;
        }
        for pending in &mut r.pending {
            pending.drain(..offset);
        }
        Ok(())
    }

    fn random(&mut self) -> f32 {
        // xorshift32: dither only needs to be noise-like.
        self.noise ^= self.noise << 13;
        self.noise ^= self.noise >> 17;
        self.noise ^= self.noise << 5;
        self.noise as f32 / u32::MAX as f32
    }

    fn quantize(&mut self, value: f32) -> i16 {
        let dither = self.random() - self.random();
        (value * 32_767.0 + dither)
            .round()
            .clamp(-32_768.0, 32_767.0) as i16
    }

    /// Writes up to `len` frames; `sample(plane, i)` reads plane `s * 2 + c`.
    /// Frames past the target length are dropped.
    fn write(
        &mut self,
        sample: impl Fn(usize, usize) -> f32,
        len: usize,
    ) -> Result<(), SeparationError> {
        let len = len.min((self.target - self.written) as usize);
        if len == 0 {
            return Ok(());
        }
        let channels = self.channels;
        let mut writers = std::mem::take(&mut self.writers);
        let result = (|| {
            for (source, writer) in writers.iter_mut().enumerate() {
                let mut samples = writer.get_i16_writer((len * channels) as u32);
                for i in 0..len {
                    let left = sample(source * 2, i);
                    let right = sample(source * 2 + 1, i);
                    if channels == 2 {
                        samples.write_sample(self.quantize(left));
                        samples.write_sample(self.quantize(right));
                    } else {
                        samples.write_sample(self.quantize((left + right) * 0.5));
                    }
                }
                samples.flush()?;
            }
            Ok(())
        })();
        self.writers = writers;
        self.written += len as u64;
        result
    }

    /// Flushes the resampler, pads to the target length with silence and
    /// finalises the files.
    pub fn finish(mut self) -> Result<(), SeparationError> {
        if let Some(mut resampler) = self.resampler.take() {
            self.drain(&mut resampler, true)?;
        }
        let missing = (self.target - self.written) as usize;
        self.write(|_, _| 0.0, missing)?;
        for writer in self.writers {
            writer.finalize()?;
        }
        Ok(())
    }
}

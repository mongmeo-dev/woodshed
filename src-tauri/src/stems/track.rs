//! Track material for the stem player: random-access readers of stereo
//! frames at the song's sample rate. They run on the feeder thread (never the
//! audio thread), so they may block on disk I/O.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// Reads stereo frames from one track. Mono material plays on both
/// channels, extra channels are ignored, and frames past the end are silent.
pub trait TrackReader: Send {
    /// Length in frames.
    fn frames(&self) -> u64;

    /// Sample rate of the material.
    fn sample_rate(&self) -> u32;

    /// Writes `frames` stereo frames starting at `pos` into `out`: frame `i`
    /// goes to `out[i * stride + offset]` (left) and `out[i * stride + offset + 1]`
    /// (right). Frames past the end are written as silence.
    fn read(&mut self, pos: u64, frames: usize, out: &mut [f32], stride: usize, offset: usize);
}

/// A track held in memory (interleaved stereo). Used by tests and short material.
pub struct MemoryTrack {
    sample_rate: u32,
    stereo: Vec<f32>,
}

impl MemoryTrack {
    /// `stereo` is interleaved left/right.
    pub fn new(sample_rate: u32, stereo: Vec<f32>) -> Self {
        assert!(
            stereo.len().is_multiple_of(2),
            "stereo material needs whole frames"
        );
        Self {
            sample_rate,
            stereo,
        }
    }

    /// The same samples on both channels.
    pub fn mono(sample_rate: u32, samples: &[f32]) -> Self {
        let stereo = samples.iter().flat_map(|&s| [s, s]).collect();
        Self::new(sample_rate, stereo)
    }
}

impl TrackReader for MemoryTrack {
    fn frames(&self) -> u64 {
        (self.stereo.len() / 2) as u64
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn read(&mut self, pos: u64, frames: usize, out: &mut [f32], stride: usize, offset: usize) {
        let total = self.frames();
        for i in 0..frames {
            let frame = pos + i as u64;
            let (left, right) = if frame < total {
                let at = frame as usize * 2;
                (self.stereo[at], self.stereo[at + 1])
            } else {
                (0.0, 0.0)
            };
            out[i * stride + offset] = left;
            out[i * stride + offset + 1] = right;
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TrackError {
    #[error("cannot open track: {0}")]
    Open(#[from] hound::Error),
    #[error("unsupported WAV sample format")]
    Format,
}

#[derive(Clone, Copy)]
enum WavSamples {
    Float,
    Int(f32),
}

/// A WAV file streamed from disk (16/24/32-bit integer or 32-bit float PCM).
pub struct WavTrack {
    reader: hound::WavReader<BufReader<File>>,
    samples: WavSamples,
    channels: usize,
    sample_rate: u32,
    frames: u64,
    /// Frame the reader will return next.
    cursor: u64,
    scratch: Vec<f32>,
}

/// Frames read from disk per call to the WAV reader.
const WAV_CHUNK_FRAMES: usize = 1024;

impl WavTrack {
    pub fn open(path: &Path) -> Result<Self, TrackError> {
        let reader = hound::WavReader::open(path)?;
        let spec = reader.spec();
        let samples = match (spec.sample_format, spec.bits_per_sample) {
            (hound::SampleFormat::Float, 32) => WavSamples::Float,
            (hound::SampleFormat::Int, bits @ (8 | 16 | 24 | 32)) => {
                WavSamples::Int(1.0 / (1_u64 << (bits - 1)) as f32)
            }
            _ => return Err(TrackError::Format),
        };
        let channels = usize::from(spec.channels.max(1));
        Ok(Self {
            frames: u64::from(reader.duration()),
            reader,
            samples,
            channels,
            sample_rate: spec.sample_rate,
            cursor: 0,
            scratch: vec![0.0; WAV_CHUNK_FRAMES * channels],
        })
    }

    /// Reads up to `frames` frames at the cursor into `scratch`; returns how many.
    fn read_chunk(&mut self, frames: usize) -> usize {
        let wanted = frames * self.channels;
        let mut got = 0;
        match self.samples {
            WavSamples::Float => {
                for sample in self.reader.samples::<f32>().take(wanted) {
                    let Ok(sample) = sample else { break };
                    self.scratch[got] = sample;
                    got += 1;
                }
            }
            WavSamples::Int(scale) => {
                for sample in self.reader.samples::<i32>().take(wanted) {
                    let Ok(sample) = sample else { break };
                    self.scratch[got] = sample as f32 * scale;
                    got += 1;
                }
            }
        }
        let read = got / self.channels;
        self.cursor += read as u64;
        read
    }
}

impl TrackReader for WavTrack {
    fn frames(&self) -> u64 {
        self.frames
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn read(&mut self, pos: u64, frames: usize, out: &mut [f32], stride: usize, offset: usize) {
        let mut done = 0;
        if pos < self.frames && pos != self.cursor {
            match u32::try_from(pos).map(|at| self.reader.seek(at)) {
                Ok(Ok(())) => self.cursor = pos,
                _ => self.cursor = u64::MAX,
            }
        }
        while done < frames && pos + (done as u64) < self.frames && self.cursor != u64::MAX {
            let chunk = (frames - done).min(WAV_CHUNK_FRAMES);
            let read = self.read_chunk(chunk);
            for i in 0..read {
                let src = &self.scratch[i * self.channels..];
                let left = src[0];
                let right = if self.channels > 1 { src[1] } else { left };
                let at = (done + i) * stride + offset;
                out[at] = left;
                out[at + 1] = right;
            }
            done += read;
            if read < chunk {
                break;
            }
        }
        for i in done..frames {
            out[i * stride + offset] = 0.0;
            out[i * stride + offset + 1] = 0.0;
        }
    }
}

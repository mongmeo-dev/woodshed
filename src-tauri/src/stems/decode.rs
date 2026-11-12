//! Decodes an imported file (mp3, wav, flac, m4a/aac/alac, ogg/vorbis) with
//! Symphonia into the library's playback format: 16-bit PCM WAV at the
//! song's own sample rate, mono or stereo. Also measures the waveform
//! overview. Runs on a blocking worker thread.

use std::fs::File;
use std::io::{BufWriter, ErrorKind};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Bins in the stored waveform overview.
pub const WAVEFORM_BINS: usize = 2_000;
/// Frames per fine peak before reducing to [`WAVEFORM_BINS`].
const PEAK_FRAMES: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("cannot read the file: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported or damaged audio: {0}")]
    Unsupported(String),
    #[error("the file has no audio")]
    NoAudio,
    #[error("cannot write the playback file: {0}")]
    Write(#[from] hound::Error),
    #[error("cancelled")]
    Cancelled,
}

impl From<SymphoniaError> for DecodeError {
    fn from(error: SymphoniaError) -> Self {
        match error {
            SymphoniaError::IoError(error) => Self::Io(error),
            other => Self::Unsupported(other.to_string()),
        }
    }
}

/// What the decoded playback file holds.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedAudio {
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    /// Peak level per bin, 0..=255 for 0.0..=1.0.
    pub waveform: Vec<u8>,
}

/// Writes 16-bit PCM with triangular dither.
struct PcmWriter {
    writer: hound::WavWriter<BufWriter<File>>,
    channels: usize,
    frames: u64,
    peaks: Vec<f32>,
    peak: f32,
    peak_fill: usize,
    noise: u32,
}

impl PcmWriter {
    fn create(path: &Path, sample_rate: u32, channels: u16) -> Result<Self, DecodeError> {
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        Ok(Self {
            writer: hound::WavWriter::create(path, spec)?,
            channels: usize::from(channels),
            frames: 0,
            peaks: Vec::new(),
            peak: 0.0,
            peak_fill: 0,
            noise: 0x9E37_79B9,
        })
    }

    fn random(&mut self) -> f32 {
        // xorshift32: cheap, and dither only needs to be noise-like.
        self.noise ^= self.noise << 13;
        self.noise ^= self.noise >> 17;
        self.noise ^= self.noise << 5;
        self.noise as f32 / u32::MAX as f32
    }

    /// `samples` is interleaved with `source_channels` channels.
    fn write(&mut self, samples: &[f32], source_channels: usize) -> Result<(), DecodeError> {
        for frame in samples.chunks_exact(source_channels.max(1)) {
            for channel in 0..self.channels {
                let value = frame[channel.min(frame.len() - 1)];
                self.peak = self.peak.max(value.abs());
                let dither = self.random() - self.random();
                let scaled = (value * 32_767.0 + dither)
                    .round()
                    .clamp(-32_768.0, 32_767.0);
                self.writer.write_sample(scaled as i16)?;
            }
            self.frames += 1;
            self.peak_fill += 1;
            if self.peak_fill == PEAK_FRAMES {
                self.peaks.push(self.peak);
                self.peak = 0.0;
                self.peak_fill = 0;
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<(u64, Vec<u8>), DecodeError> {
        if self.peak_fill > 0 {
            self.peaks.push(self.peak);
        }
        self.writer.finalize()?;
        Ok((self.frames, reduce_peaks(&self.peaks, WAVEFORM_BINS)))
    }
}

/// Reduces fine peaks to `bins` bins (max per bin), scaled to 0..=255.
pub fn reduce_peaks(peaks: &[f32], bins: usize) -> Vec<u8> {
    if peaks.is_empty() {
        return Vec::new();
    }
    let bins = bins.min(peaks.len());
    (0..bins)
        .map(|bin| {
            let from = bin * peaks.len() / bins;
            let to = ((bin + 1) * peaks.len() / bins).max(from + 1);
            let peak = peaks[from..to].iter().copied().fold(0.0_f32, f32::max);
            (peak.clamp(0.0, 1.0) * 255.0).round() as u8
        })
        .collect()
}

/// Decodes `source` into a 16-bit PCM WAV at `target`. `progress` gets
/// 0.0..=1.0 when the length is known.
pub fn decode_to_wav(
    source: &Path,
    target: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<DecodedAudio, DecodeError> {
    let file = File::open(source)?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = source.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }
    let mut format = symphonia::default::get_probe().probe(
        &hint,
        stream,
        FormatOptions::default(),
        MetadataOptions::default(),
    )?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or(DecodeError::NoAudio)?;
    let track_id = track.id;
    let total_frames = track.num_frames;
    let Some(CodecParameters::Audio(params)) = track.codec_params.clone() else {
        return Err(DecodeError::NoAudio);
    };
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())?;

    let mut writer: Option<PcmWriter> = None;
    let mut samples: Vec<f32> = Vec::new();
    let mut last_report = 0.0_f32;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(DecodeError::Cancelled);
        }
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if packet.track_id != track_id {
            continue;
        }
        let buffer = match decoder.decode(&packet) {
            Ok(buffer) => buffer,
            // A damaged packet is skipped, as players do.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(error.into()),
        };
        let source_channels = buffer.spec().channels().count().max(1);
        let rate = buffer.spec().rate();
        buffer.copy_to_vec_interleaved::<f32>(&mut samples);
        if writer.is_none() {
            let channels = source_channels.min(2) as u16;
            writer = Some(PcmWriter::create(target, rate, channels)?);
        }
        if let Some(writer) = writer.as_mut() {
            writer.write(&samples, source_channels)?;
            if let Some(total) = total_frames.filter(|&total| total > 0) {
                let done = (writer.frames as f32 / total as f32).min(1.0);
                if done - last_report >= 0.01 {
                    last_report = done;
                    progress(done);
                }
            }
        }
    }

    let writer = writer.ok_or(DecodeError::NoAudio)?;
    let sample_rate = writer.writer.spec().sample_rate;
    let channels = writer.writer.spec().channels;
    let (frames, waveform) = writer.finish()?;
    if frames == 0 {
        return Err(DecodeError::NoAudio);
    }
    progress(1.0);
    Ok(DecodedAudio {
        sample_rate,
        channels,
        frames,
        waveform,
    })
}

/// Measures the waveform overview of an existing WAV (e.g. one ffmpeg wrote).
pub fn wav_waveform(path: &Path) -> Result<DecodedAudio, DecodeError> {
    use super::track::{TrackReader, WavTrack};
    let mut track = WavTrack::open(path).map_err(|e| DecodeError::Unsupported(e.to_string()))?;
    let reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    drop(reader);
    let frames = track.frames();
    let mut buffer = vec![0.0; PEAK_FRAMES * 2];
    let mut peaks = Vec::with_capacity((frames as usize).div_ceil(PEAK_FRAMES));
    let mut pos = 0;
    while pos < frames {
        let n = PEAK_FRAMES.min((frames - pos) as usize);
        track.read(pos, n, &mut buffer, 2, 0);
        peaks.push(buffer[..n * 2].iter().fold(0.0_f32, |m, s| m.max(s.abs())));
        pos += n as u64;
    }
    Ok(DecodedAudio {
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        frames,
        waveform: reduce_peaks(&peaks, WAVEFORM_BINS),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stems::track::{TrackReader, WavTrack};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("woodshed-decode-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_float_wav(path: &Path, rate: u32, channels: u16, samples: &[f32]) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for &sample in samples {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn decodes_to_16_bit_pcm_at_the_source_rate() {
        let dir = temp_dir("pcm");
        let source = dir.join("in.wav");
        let frames = 44_100;
        let samples: Vec<f32> = (0..frames * 2)
            .map(|i| ((i / 2) as f32 * 0.01).sin() * if i % 2 == 0 { 0.5 } else { 0.25 })
            .collect();
        write_float_wav(&source, 44_100, 2, &samples);

        let target = dir.join("audio.wav");
        let mut reports = Vec::new();
        let decoded = decode_to_wav(&source, &target, &AtomicBool::new(false), &mut |p| {
            reports.push(p)
        })
        .unwrap();
        assert_eq!(decoded.sample_rate, 44_100);
        assert_eq!(decoded.channels, 2);
        assert_eq!(decoded.frames, frames as u64);
        assert_eq!(reports.last(), Some(&1.0));

        let spec = hound::WavReader::open(&target).unwrap().spec();
        assert_eq!(
            (spec.bits_per_sample, spec.sample_format),
            (16, hound::SampleFormat::Int)
        );
        let mut track = WavTrack::open(&target).unwrap();
        let mut out = vec![0.0; 200];
        track.read(1_000, 100, &mut out, 2, 0);
        for (i, frame) in out.as_chunks::<2>().0.iter().enumerate() {
            let at = (1_000 + i) * 2;
            assert!((frame[0] - samples[at]).abs() < 1e-3);
            assert!((frame[1] - samples[at + 1]).abs() < 1e-3);
        }
        // Waveform: the louder (left) channel peaks near 0.5.
        let peak = *decoded.waveform.iter().max().unwrap();
        assert!((126..=129).contains(&peak), "{peak}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_files_without_audio() {
        let dir = temp_dir("bad");
        let source = dir.join("notes.mp3");
        std::fs::write(&source, b"not audio at all").unwrap();
        let result = decode_to_wav(
            &source,
            &dir.join("audio.wav"),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert!(matches!(
            result,
            Err(DecodeError::Unsupported(_) | DecodeError::NoAudio)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_stops_decoding() {
        let dir = temp_dir("cancel");
        let source = dir.join("in.wav");
        write_float_wav(&source, 48_000, 1, &vec![0.1; 48_000]);
        let result = decode_to_wav(
            &source,
            &dir.join("audio.wav"),
            &AtomicBool::new(true),
            &mut |_| {},
        );
        assert!(matches!(result, Err(DecodeError::Cancelled)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

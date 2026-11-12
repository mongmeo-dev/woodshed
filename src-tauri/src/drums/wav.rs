//! Minimal RIFF/WAVE reader for the bundled kit: integer PCM (8/16/24/32-bit)
//! and 32-bit float, any channel count, mixed down to mono `f32`.

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WavError {
    #[error("not a RIFF/WAVE file")]
    NotWave,
    #[error("the file has no `{0}` chunk")]
    MissingChunk(&'static str),
    #[error("unsupported sample format (tag {tag}, {bits} bits)")]
    Unsupported { tag: u16, bits: u16 },
}

/// A decoded sound: mono samples at their recorded rate.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

const FORMAT_PCM: u16 = 1;
const FORMAT_FLOAT: u16 = 3;
const FORMAT_EXTENSIBLE: u16 = 0xFFFE;

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

struct Format {
    tag: u16,
    channels: usize,
    sample_rate: u32,
    bits: u16,
}

/// Decodes a WAV file held in memory.
pub fn decode_wav(bytes: &[u8]) -> Result<Decoded, WavError> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err(WavError::NotWave);
    }
    let mut format = None;
    let mut data = None;
    let mut at = 12;
    while let (Some(id), Some(size)) = (bytes.get(at..at + 4), u32_at(bytes, at + 4)) {
        let body_start = at + 8;
        let body_end = body_start.saturating_add(size as usize).min(bytes.len());
        let body = &bytes[body_start..body_end];
        match id {
            b"fmt " => {
                let mut tag = u16_at(body, 0).ok_or(WavError::MissingChunk("fmt "))?;
                if tag == FORMAT_EXTENSIBLE {
                    // The real format tag is the first two bytes of the sub-format GUID.
                    tag = u16_at(body, 24).unwrap_or(0);
                }
                format = Some(Format {
                    tag,
                    channels: usize::from(u16_at(body, 2).unwrap_or(0)),
                    sample_rate: u32_at(body, 4).unwrap_or(0),
                    bits: u16_at(body, 14).unwrap_or(0),
                });
            }
            b"data" => data = Some(body),
            _ => {}
        }
        // Chunks are padded to an even size.
        at = body_start.saturating_add(size as usize + (size as usize & 1));
    }
    let format = format.ok_or(WavError::MissingChunk("fmt "))?;
    let data = data.ok_or(WavError::MissingChunk("data"))?;
    let unsupported = WavError::Unsupported {
        tag: format.tag,
        bits: format.bits,
    };
    if format.channels == 0 || format.sample_rate == 0 {
        return Err(unsupported);
    }
    let width = usize::from(format.bits / 8);
    let read: fn(&[u8]) -> f32 = match (format.tag, format.bits) {
        (FORMAT_PCM, 8) => |b| (f32::from(b[0]) - 128.0) / 128.0,
        (FORMAT_PCM, 16) => |b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32_768.0,
        (FORMAT_PCM, 24) => {
            |b| (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0
        }
        (FORMAT_PCM, 32) => {
            |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0
        }
        (FORMAT_FLOAT, 32) => |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        _ => return Err(unsupported),
    };
    let frame_bytes = width * format.channels;
    let scale = 1.0 / format.channels as f32;
    let samples = data
        .chunks_exact(frame_bytes)
        .map(|frame| frame.chunks_exact(width).map(read).sum::<f32>() * scale)
        .collect();
    Ok(Decoded {
        sample_rate: format.sample_rate,
        samples,
    })
}

/// Encodes mono 16-bit PCM. Used by tests to build kits.
#[cfg(test)]
pub fn encode_wav_16(sample_rate: u32, samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&FORMAT_PCM.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * 32_767.0).round() as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_16_bit_mono() {
        let decoded = decode_wav(&encode_wav_16(44_100, &[0.0, 0.5, -0.5, 1.0])).unwrap();
        assert_eq!(decoded.sample_rate, 44_100);
        let expected = [
            0.0,
            16_384.0 / 32_768.0,
            -16_384.0 / 32_768.0,
            32_767.0 / 32_768.0,
        ];
        for (got, want) in decoded.samples.iter().zip(expected) {
            assert!((got - want).abs() < 1.0e-4, "{got} vs {want}");
        }
    }

    #[test]
    fn mixes_24_bit_stereo_to_mono_and_skips_unknown_chunks() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF\0\0\0\0WAVE");
        bytes.extend_from_slice(b"LIST\x03\0\0\0abc\0"); // odd size, padded
        bytes.extend_from_slice(b"fmt \x10\0\0\0");
        bytes.extend_from_slice(&FORMAT_PCM.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&48_000_u32.to_le_bytes());
        bytes.extend_from_slice(&(48_000_u32 * 6).to_le_bytes());
        bytes.extend_from_slice(&6_u16.to_le_bytes());
        bytes.extend_from_slice(&24_u16.to_le_bytes());
        bytes.extend_from_slice(b"data\x06\0\0\0");
        // left = +0.5 (0x400000), right = -0.25 (0xE00000)
        bytes.extend_from_slice(&[0x00, 0x00, 0x40, 0x00, 0x00, 0xE0]);
        let decoded = decode_wav(&bytes).unwrap();
        assert_eq!(decoded.sample_rate, 48_000);
        assert_eq!(decoded.samples, vec![0.125]);
    }

    #[test]
    fn rejects_other_files() {
        assert_eq!(decode_wav(b"not a wav file"), Err(WavError::NotWave));
        let mut float64 = encode_wav_16(48_000, &[0.0]);
        float64[20..22].copy_from_slice(&FORMAT_FLOAT.to_le_bytes());
        assert!(matches!(
            decode_wav(&float64),
            Err(WavError::Unsupported { tag: 3, bits: 16 })
        ));
    }
}

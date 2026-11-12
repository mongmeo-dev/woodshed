//! Reads every track of a song in lockstep, at the song's sample rate, with
//! an optional A-B loop. Runs on the feeder thread.

use std::f32::consts::FRAC_PI_2;

use super::track::TrackReader;

/// A loop region in song frames: `start` inclusive, `end` exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoopRange {
    pub start: u64,
    pub end: u64,
}

impl LoopRange {
    /// `None` when the region is empty.
    pub fn new(start: u64, end: u64) -> Option<Self> {
        (end > start).then_some(Self { start, end })
    }

    pub fn len(&self) -> u64 {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

/// Interleaved multi-track reader: frame `i` holds `[t0 L, t0 R, t1 L, t1 R, ...]`.
///
/// With a loop set and the position before its end, reading wraps from `end`
/// back to `start`. The last `crossfade` frames before `end` are blended
/// (equal power) with the frames just before `start`, so the material flows
/// into `start` without a click and every lap is exactly `end - start` frames.
pub struct SongReader {
    tracks: Vec<Box<dyn TrackReader>>,
    frames: u64,
    pos: u64,
    looping: Option<LoopRange>,
    crossfade: u64,
    scratch: Vec<f32>,
}

impl SongReader {
    pub fn new(tracks: Vec<Box<dyn TrackReader>>, crossfade: u64) -> Self {
        let frames = tracks.iter().map(|t| t.frames()).max().unwrap_or(0);
        Self {
            tracks,
            frames,
            pos: 0,
            looping: None,
            crossfade,
            scratch: Vec::new(),
        }
    }

    /// Interleaved samples per frame (two per track).
    pub fn channels(&self) -> usize {
        self.tracks.len() * 2
    }

    /// Song length in frames (the longest track).
    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn seek(&mut self, pos: u64) {
        self.pos = pos;
    }

    pub fn set_loop(&mut self, looping: Option<LoopRange>) {
        self.looping = looping.filter(|range| !range.is_empty());
    }

    /// Loop in effect at the current position, with its crossfade length.
    fn active_loop(&self) -> Option<(LoopRange, u64)> {
        let range = self.looping?;
        if self.pos >= range.end {
            return None;
        }
        let crossfade = self.crossfade.min(range.start).min(range.len() / 2);
        Some((range, crossfade))
    }

    fn read_plain(tracks: &mut [Box<dyn TrackReader>], pos: u64, frames: usize, out: &mut [f32]) {
        let stride = tracks.len() * 2;
        for (index, track) in tracks.iter_mut().enumerate() {
            track.read(pos, frames, out, stride, index * 2);
        }
    }

    /// Fills `frames` interleaved frames into `out` and advances.
    pub fn read(&mut self, out: &mut [f32], frames: usize) {
        let channels = self.channels();
        if channels == 0 {
            return;
        }
        let mut done = 0;
        while done < frames {
            let remaining = frames - done;
            let seg = &mut out[done * channels..];
            let Some((range, crossfade)) = self.active_loop() else {
                Self::read_plain(&mut self.tracks, self.pos, remaining, seg);
                self.pos += remaining as u64;
                done = frames;
                continue;
            };
            let fade_start = range.end - crossfade;
            if self.pos < fade_start {
                let n = remaining.min((fade_start - self.pos) as usize);
                Self::read_plain(&mut self.tracks, self.pos, n, seg);
                self.pos += n as u64;
                done += n;
                if crossfade == 0 && self.pos == range.end {
                    self.pos = range.start;
                }
                continue;
            }
            // Inside the crossfade: blend the tail before `end` with the
            // material just before `start`.
            let n = remaining.min((range.end - self.pos) as usize);
            Self::read_plain(&mut self.tracks, self.pos, n, seg);
            if self.scratch.len() < n * channels {
                self.scratch.resize(n * channels, 0.0);
            }
            let into = self.pos - fade_start;
            let lead_in = range.start - crossfade + into;
            Self::read_plain(&mut self.tracks, lead_in, n, &mut self.scratch);
            for i in 0..n {
                let t = ((into + i as u64) as f32 + 0.5) / crossfade as f32;
                let (fade_out, fade_in) = ((t * FRAC_PI_2).cos(), (t * FRAC_PI_2).sin());
                let frame = &mut seg[i * channels..(i + 1) * channels];
                let lead = &self.scratch[i * channels..(i + 1) * channels];
                for (dst, &src) in frame.iter_mut().zip(lead) {
                    *dst = *dst * fade_out + src * fade_in;
                }
            }
            self.pos += n as u64;
            done += n;
            if self.pos == range.end {
                self.pos = range.start;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stems::track::MemoryTrack;

    fn ramp(frames: usize) -> Box<dyn TrackReader> {
        let samples: Vec<f32> = (0..frames).map(|i| i as f32).collect();
        Box::new(MemoryTrack::mono(48_000, &samples))
    }

    fn read_left(reader: &mut SongReader, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * reader.channels()];
        reader.read(&mut out, frames);
        out.chunks_exact(reader.channels()).map(|f| f[0]).collect()
    }

    #[test]
    fn plays_past_the_end_as_silence() {
        let mut reader = SongReader::new(vec![ramp(4)], 0);
        assert_eq!(
            read_left(&mut reader, 6),
            vec![0.0, 1.0, 2.0, 3.0, 0.0, 0.0]
        );
    }

    #[test]
    fn loop_without_crossfade_wraps_exactly() {
        let mut reader = SongReader::new(vec![ramp(100)], 0);
        reader.set_loop(LoopRange::new(10, 13));
        reader.seek(8);
        assert_eq!(
            read_left(&mut reader, 9),
            vec![8.0, 9.0, 10.0, 11.0, 12.0, 10.0, 11.0, 12.0, 10.0]
        );
    }

    #[test]
    fn loop_crossfade_blends_into_the_start() {
        let mut reader = SongReader::new(vec![ramp(1_000)], 4);
        reader.set_loop(LoopRange::new(100, 200));
        reader.seek(190);
        let out = read_left(&mut reader, 20);
        // Before the fade: untouched.
        assert_eq!(&out[..6], &[190.0, 191.0, 192.0, 193.0, 194.0, 195.0]);
        // Fade: equal-power blend of the tail (196..200) and the lead-in (96..100).
        for (i, &value) in out[6..10].iter().enumerate() {
            let t = (i as f32 + 0.5) / 4.0 * FRAC_PI_2;
            let expected = (196.0 + i as f32) * t.cos() + (96.0 + i as f32) * t.sin();
            assert!((value - expected).abs() < 1e-3, "{value} vs {expected}");
        }
        // Lap restarts exactly at the loop start.
        assert_eq!(&out[10..14], &[100.0, 101.0, 102.0, 103.0]);
    }

    #[test]
    fn position_after_the_loop_end_plays_through() {
        let mut reader = SongReader::new(vec![ramp(100)], 2);
        reader.set_loop(LoopRange::new(10, 20));
        reader.seek(30);
        assert_eq!(read_left(&mut reader, 3), vec![30.0, 31.0, 32.0]);
    }

    #[test]
    fn tracks_stay_in_lockstep() {
        let mut reader = SongReader::new(
            vec![
                ramp(50),
                Box::new(MemoryTrack::mono(48_000, &[-1.0; 50])) as Box<dyn TrackReader>,
            ],
            0,
        );
        reader.seek(5);
        let mut out = vec![0.0; 3 * 4];
        reader.read(&mut out, 3);
        assert_eq!(
            out,
            vec![
                5.0, 5.0, -1.0, -1.0, 6.0, 6.0, -1.0, -1.0, 7.0, 7.0, -1.0, -1.0
            ]
        );
    }
}

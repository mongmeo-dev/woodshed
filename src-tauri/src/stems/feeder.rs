//! The feeder: a normal thread that turns song material into ready-to-mix
//! blocks for the audio thread.
//!
//! Pipeline: [`SongReader`] (all tracks in lockstep, A-B loop) → time stretch
//! when slower than 100 % (Signalsmith Stretch, pitch kept) → resampling from
//! the song rate to the output rate (rubato FFT) → fixed-size [`Block`]s.
//!
//! Blocks are allocated once and travel between the threads through two
//! lock-free queues (filled → audio, used → feeder), so the audio thread never
//! allocates or frees. The audio thread asks for restarts (seek, loop change,
//! output rate change, stretch on/off) with [`FeederRequest`]s carrying a new
//! epoch; blocks of older epochs are discarded on its side.

use rtrb::{Consumer, Producer};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use signalsmith_stretch::Stretch;

use super::reader::{LoopRange, SongReader};

/// Most tracks one song can play (six stems, with room to spare).
pub const MAX_TRACKS: usize = 8;
/// Output frames per block.
pub const BLOCK_FRAMES: usize = 256;
/// Blocks in flight. Bounds how far the feeder runs ahead of playback
/// (`POOL_BLOCKS * BLOCK_FRAMES` frames, about 85 ms at 48 kHz) and so how
/// late a seamless speed change is heard.
pub const POOL_BLOCKS: usize = 16;
/// Capacity of the audio → feeder request queue.
pub const REQUEST_CAPACITY: usize = 64;
/// Speeds at or above this play the material directly (no stretching).
pub const BYPASS_SPEED: f32 = 0.9995;
/// Slowest speed.
pub const MIN_SPEED: f32 = 0.5;
/// Frames the stretcher processes per call.
const STRETCH_CHUNK: usize = 1024;

/// True when `speed` plays without time stretching.
pub fn is_bypass(speed: f32) -> bool {
    speed >= BYPASS_SPEED
}

/// Ready-to-mix output frames for every track (two channels per track).
pub struct Block {
    pub(crate) epoch: u32,
    /// Song position (in song frames) of the block's first frame.
    pub(crate) pos: f64,
    /// Song frames advanced per output frame.
    pub(crate) step: f64,
    /// Loop that wraps the position inside this block, if any.
    pub(crate) looping: Option<LoopRange>,
    pub(crate) frames: usize,
    /// The song ends after this block.
    pub(crate) end: bool,
    /// `BLOCK_FRAMES * channels` samples, interleaved `[t0 L, t0 R, t1 L, ...]`.
    pub(crate) data: Box<[f32]>,
}

impl Block {
    pub(crate) fn new(channels: usize) -> Self {
        Self {
            epoch: 0,
            pos: 0.0,
            step: 1.0,
            looping: None,
            frames: 0,
            end: false,
            data: vec![0.0; BLOCK_FRAMES * channels].into_boxed_slice(),
        }
    }

    /// Song position after `frames` frames of this block.
    pub(crate) fn position_at(&self, frames: usize) -> f64 {
        let pos = self.pos + frames as f64 * self.step;
        match self.looping {
            Some(range) if self.pos < range.end as f64 && pos >= range.end as f64 => {
                pos - range.len() as f64
            }
            _ => pos,
        }
    }
}

/// What the feeder should produce. A request with a new `epoch` restarts
/// production at `position`; one with the current epoch only changes the
/// speed, seamlessly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeederRequest {
    pub epoch: u32,
    /// Song frame to restart at.
    pub position: u64,
    pub out_rate: u32,
    pub speed: f32,
    pub looping: Option<LoopRange>,
}

pub struct Feeder {
    reader: SongReader,
    song_rate: u32,
    channels: usize,
    requests: Consumer<FeederRequest>,
    filled: Producer<Box<Block>>,
    used: Consumer<Box<Block>>,
    #[expect(
        clippy::vec_box,
        reason = "blocks move between queues as boxes so the audio thread never allocates"
    )]
    idle: Vec<Box<Block>>,
    config: Option<FeederRequest>,
    finished: bool,
    /// Song position of the next output frame (aligned with what is heard).
    out_pos: f64,
    stretch: Option<Stretch>,
    stretching: bool,
    stretch_discard: usize,
    stretch_carry: f64,
    resampler: Option<(u32, Fft<f32>)>,
    resampling: bool,
    resample_discard: usize,
    staging: Vec<f32>,
    staging_read: usize,
    song_buf: Vec<f32>,
    stretch_in: Vec<f32>,
    stretch_out: Vec<f32>,
    resample_out: Vec<f32>,
}

impl Feeder {
    pub(crate) fn new(
        reader: SongReader,
        song_rate: u32,
        requests: Consumer<FeederRequest>,
        filled: Producer<Box<Block>>,
        used: Consumer<Box<Block>>,
    ) -> Self {
        let channels = reader.channels();
        Self {
            reader,
            song_rate,
            channels,
            requests,
            filled,
            used,
            idle: Vec::with_capacity(POOL_BLOCKS),
            config: None,
            finished: false,
            out_pos: 0.0,
            stretch: None,
            stretching: false,
            stretch_discard: 0,
            stretch_carry: 0.0,
            resampler: None,
            resampling: false,
            resample_discard: 0,
            staging: Vec::new(),
            staging_read: 0,
            song_buf: Vec::new(),
            stretch_in: Vec::new(),
            stretch_out: Vec::new(),
            resample_out: Vec::new(),
        }
    }

    /// Applies pending requests and fills every free block. Returns how many
    /// blocks it produced. Called in a loop by the feeder thread, or directly
    /// by tests between render callbacks.
    pub fn fill(&mut self) -> usize {
        self.apply_requests();
        while let Ok(block) = self.used.pop() {
            self.idle.push(block);
        }
        if self.config.is_none() {
            return 0;
        }
        let mut produced = 0;
        while !self.finished {
            let Some(mut block) = self.idle.pop() else {
                break;
            };
            self.produce(&mut block);
            let end = block.end;
            if let Err(rtrb::PushError::Full(block)) = self.filled.push(block) {
                // Unreachable: the queue holds every block of the pool.
                self.idle.push(block);
                break;
            }
            produced += 1;
            self.finished = end;
        }
        produced
    }

    fn apply_requests(&mut self) {
        let mut latest = None;
        while let Ok(request) = self.requests.pop() {
            latest = Some(request);
        }
        let Some(request) = latest else { return };
        match self.config {
            Some(current)
                if current.epoch == request.epoch
                    && current.out_rate == request.out_rate
                    && is_bypass(current.speed) == is_bypass(request.speed) =>
            {
                self.config = Some(FeederRequest {
                    speed: request.speed,
                    ..current
                });
            }
            _ => self.restart(request),
        }
    }

    fn restart(&mut self, request: FeederRequest) {
        let request = FeederRequest {
            speed: request.speed.clamp(MIN_SPEED, 1.0),
            ..request
        };
        self.config = Some(request);
        self.finished = false;
        self.staging.clear();
        self.staging_read = 0;
        self.reader.set_loop(request.looping);
        self.reader.seek(request.position);
        self.out_pos = request.position as f64;

        self.stretching = !is_bypass(request.speed);
        if self.stretching {
            let channels = self.channels;
            let stretch = self
                .stretch
                .get_or_insert_with(|| Stretch::preset_default(channels as u32, self.song_rate));
            stretch.reset();
            // The processing time starts at the first pre-roll frame, so the
            // output lines up with `position` once `output_latency` pre-roll
            // frames are dropped.
            let pre_roll = stretch.input_latency();
            let output_latency = stretch.output_latency();
            self.stretch_in.resize(pre_roll * channels, 0.0);
            self.reader.read(&mut self.stretch_in, pre_roll);
            if let Some(stretch) = self.stretch.as_mut() {
                stretch.seek(
                    &self.stretch_in[..pre_roll * channels],
                    f64::from(request.speed),
                );
            }
            self.stretch_discard = output_latency;
            self.stretch_carry = 0.0;
        }

        self.resampling = request.out_rate != self.song_rate;
        if self.resampling {
            let stale = self
                .resampler
                .as_ref()
                .is_none_or(|(rate, _)| *rate != request.out_rate);
            if stale {
                self.resampler = Fft::<f32>::new(
                    self.song_rate as usize,
                    request.out_rate as usize,
                    BLOCK_FRAMES,
                    self.channels,
                    FixedSync::Output,
                )
                .map_err(|error| log::error!("stem resampler: {error}"))
                .ok()
                .map(|fft| (request.out_rate, fft));
            }
            match self.resampler.as_mut() {
                Some((_, fft)) => {
                    fft.reset();
                    self.resample_discard = fft.output_delay();
                }
                None => self.resampling = false,
            }
        }
    }

    /// Song frames per output frame.
    fn step(&self, config: &FeederRequest) -> f64 {
        let speed = if self.stretching {
            f64::from(config.speed)
        } else {
            1.0
        };
        let rate = if self.resampling {
            f64::from(self.song_rate) / f64::from(config.out_rate)
        } else {
            1.0
        };
        speed * rate
    }

    fn produce(&mut self, block: &mut Block) {
        let Some(config) = self.config else { return };
        let step = self.step(&config);
        let looping = config
            .looping
            .filter(|range| self.out_pos < range.end as f64);
        let mut frames = BLOCK_FRAMES;
        let mut end = false;
        if looping.is_none() {
            let left = (self.reader.frames() as f64 - self.out_pos).max(0.0);
            let until_end = (left / step).ceil() as usize;
            if until_end <= BLOCK_FRAMES {
                frames = until_end;
                end = true;
            }
        }
        while self.staged_frames() < frames {
            self.produce_chunk();
        }
        let channels = self.channels;
        let from = self.staging_read;
        block.data[..frames * channels]
            .copy_from_slice(&self.staging[from..from + frames * channels]);
        self.staging_read += frames * channels;
        if self.staging_read * 2 > self.staging.len() {
            self.staging.drain(..self.staging_read);
            self.staging_read = 0;
        }

        block.epoch = config.epoch;
        block.pos = self.out_pos;
        block.step = step;
        block.looping = looping;
        block.frames = frames;
        block.end = end;
        self.out_pos = block.position_at(frames);
    }

    fn staged_frames(&self) -> usize {
        (self.staging.len() - self.staging_read) / self.channels.max(1)
    }

    /// Appends one chunk of output-rate frames to `staging`.
    fn produce_chunk(&mut self) {
        let channels = self.channels;
        let mut song = std::mem::take(&mut self.song_buf);
        if self.resampling
            && let Some((_, fft)) = self.resampler.as_ref()
        {
            let needed = fft.input_frames_next();
            let out_frames = fft.output_frames_next();
            song.resize(needed * channels, 0.0);
            self.pull_song(&mut song, needed);
            self.resample_out.resize(out_frames * channels, 0.0);
            let produced = match (
                self.resampler.as_mut(),
                InterleavedSlice::new(&song[..], channels, needed),
                InterleavedSlice::new_mut(&mut self.resample_out[..], channels, out_frames),
            ) {
                (Some((_, fft)), Ok(input), Ok(mut output)) => fft
                    .process_into_buffer(&input, &mut output, None)
                    .map(|(_, produced)| produced)
                    .unwrap_or(0),
                _ => 0,
            };
            if produced == 0 {
                // Keep the pipeline moving even if the resampler refuses.
                self.staging
                    .resize(self.staging.len() + out_frames * channels, 0.0);
            } else {
                let skip = self.resample_discard.min(produced);
                self.resample_discard -= skip;
                self.staging
                    .extend_from_slice(&self.resample_out[skip * channels..produced * channels]);
            }
        } else {
            song.resize(BLOCK_FRAMES * channels, 0.0);
            self.pull_song(&mut song, BLOCK_FRAMES);
            self.staging.extend_from_slice(&song);
        }
        self.song_buf = song;
    }

    /// Fills `frames` song-rate frames into `out`, stretched when slower than 100 %.
    fn pull_song(&mut self, out: &mut [f32], frames: usize) {
        if !self.stretching {
            self.reader.read(out, frames);
            return;
        }
        let channels = self.channels;
        let speed = f64::from(self.config.map_or(1.0, |config| config.speed));
        let mut filled = 0;
        while filled < frames {
            let want = if self.stretch_discard > 0 {
                self.stretch_discard.min(STRETCH_CHUNK)
            } else {
                (frames - filled).min(STRETCH_CHUNK)
            };
            self.stretch_carry += want as f64 * speed;
            let take = self.stretch_carry.floor() as usize;
            self.stretch_carry -= take as f64;
            self.stretch_in.resize(take * channels, 0.0);
            self.reader.read(&mut self.stretch_in, take);
            self.stretch_out.resize(want * channels, 0.0);
            if let Some(stretch) = self.stretch.as_mut() {
                stretch.process(
                    &self.stretch_in[..take * channels],
                    &mut self.stretch_out[..want * channels],
                );
            }
            if self.stretch_discard > 0 {
                self.stretch_discard -= want;
            } else {
                out[filled * channels..(filled + want) * channels]
                    .copy_from_slice(&self.stretch_out[..want * channels]);
                filled += want;
            }
        }
    }
}

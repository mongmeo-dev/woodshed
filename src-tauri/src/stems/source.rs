//! The stem player's engine [`Source`]: mixes the feeder's blocks with
//! per-track volume, mute and solo. Runs on the audio thread: no allocation,
//! locks, I/O or system calls. Every control arrives as a scheduled engine
//! event, so it takes effect on an exact frame.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rtrb::{Consumer, Producer};

use crate::audio::{EventKind, Source, StreamFormat};

use super::feeder::{Block, FeederRequest, MAX_TRACKS, MIN_SPEED, is_bypass};
use super::reader::LoopRange;

/// Ids of [`EventKind::Param`] events understood by [`StemSource`].
pub mod param {
    use super::MAX_TRACKS;

    /// Playback speed, `0.5..=1.0` (pitch is kept).
    pub const SPEED: u16 = 1;
    /// Loop bounds in song frames, split into `frames >> 16` (HI) and
    /// `frames & 0xFFFF` (LO) so each part is exact in an `f32`. Send them
    /// before [`LOOP_APPLY`] in the same frame.
    pub const LOOP_START_HI: u16 = 2;
    pub const LOOP_START_LO: u16 = 3;
    pub const LOOP_END_HI: u16 = 4;
    pub const LOOP_END_LO: u16 = 5;
    /// `1.0` turns the loop on with the bounds above, `0.0` turns it off.
    pub const LOOP_APPLY: u16 = 6;

    const TRACK_BASE: u16 = 16;
    const TRACK_STRIDE: u16 = 4;
    /// Track volume, linear `0.0..=1.0`.
    pub const VOLUME: u16 = 0;
    /// Track mute, `1.0` = muted.
    pub const MUTE: u16 = 1;
    /// Track solo, `1.0` = soloed.
    pub const SOLO: u16 = 2;

    /// Param id of `kind` ([`VOLUME`], [`MUTE`], [`SOLO`]) for `track`.
    pub fn track(track: usize, kind: u16) -> u16 {
        assert!(track < MAX_TRACKS, "track {track} out of range");
        TRACK_BASE + track as u16 * TRACK_STRIDE + kind
    }

    pub(super) fn decode_track(id: u16) -> Option<(usize, u16)> {
        let offset = id.checked_sub(TRACK_BASE)?;
        let track = usize::from(offset / TRACK_STRIDE);
        (track < MAX_TRACKS).then_some((track, offset % TRACK_STRIDE))
    }
}

/// Events that set (or clear, with `None`) the A-B loop. Schedule them in
/// order on the same frame.
pub fn loop_events(range: Option<LoopRange>) -> Vec<EventKind> {
    let param = |id, value: u64| EventKind::Param {
        id,
        value: value as f32,
    };
    match range {
        Some(range) => vec![
            param(param::LOOP_START_HI, range.start >> 16),
            param(param::LOOP_START_LO, range.start & 0xFFFF),
            param(param::LOOP_END_HI, range.end >> 16),
            param(param::LOOP_END_LO, range.end & 0xFFFF),
            EventKind::Param {
                id: param::LOOP_APPLY,
                value: 1.0,
            },
        ],
        None => vec![EventKind::Param {
            id: param::LOOP_APPLY,
            value: 0.0,
        }],
    }
}

/// Playback state the audio thread publishes for the UI side.
#[derive(Default)]
pub struct PlayerShared {
    position: AtomicU64,
    playing: AtomicBool,
    ended: AtomicBool,
    underruns: AtomicU64,
}

impl PlayerShared {
    /// Song frame after the last rendered frame.
    pub fn position(&self) -> f64 {
        f64::from_bits(self.position.load(Ordering::Acquire))
    }

    pub fn playing(&self) -> bool {
        self.playing.load(Ordering::Acquire)
    }

    /// Playback reached the end of the song (and stopped).
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }

    /// Render calls that found no material while playing.
    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }
}

/// Fade applied when playback starts, stops or jumps.
const FADE_SECS: f32 = 0.003;
/// Track gain changes glide over roughly this long.
const GAIN_SMOOTHING_SECS: f32 = 0.005;

#[derive(Clone, Copy)]
struct TrackMix {
    volume: f32,
    mute: bool,
    solo: bool,
    gain: f32,
    target: f32,
}

impl Default for TrackMix {
    fn default() -> Self {
        Self {
            volume: 1.0,
            mute: false,
            solo: false,
            gain: 1.0,
            target: 1.0,
        }
    }
}

pub struct StemSource {
    tracks: usize,
    song_frames: u64,
    filled: Consumer<Box<Block>>,
    used: Producer<Box<Block>>,
    requests: Producer<FeederRequest>,
    pending_request: Option<FeederRequest>,
    shared: Arc<PlayerShared>,
    current: Option<Box<Block>>,
    offset: usize,

    playing: bool,
    ended: bool,
    epoch: u32,
    /// Fading out material of an older epoch before dropping it.
    draining: bool,
    /// The current epoch has delivered material (a later gap is an underrun).
    primed: bool,
    position: f64,
    speed: f32,
    looping: Option<LoopRange>,
    loop_parts: [u64; 4],

    out_rate: u32,
    envelope: f32,
    envelope_step: f32,
    gain_coeff: f32,
    mix: [TrackMix; MAX_TRACKS],
}

impl StemSource {
    pub(crate) fn new(
        tracks: usize,
        song_frames: u64,
        start: u64,
        filled: Consumer<Box<Block>>,
        used: Producer<Box<Block>>,
        requests: Producer<FeederRequest>,
        shared: Arc<PlayerShared>,
    ) -> Self {
        shared
            .position
            .store((start as f64).to_bits(), Ordering::Release);
        Self {
            tracks,
            song_frames,
            filled,
            used,
            requests,
            pending_request: None,
            shared,
            current: None,
            offset: 0,
            playing: false,
            ended: false,
            epoch: 0,
            draining: false,
            primed: false,
            position: start as f64,
            speed: 1.0,
            looping: None,
            loop_parts: [0; 4],
            out_rate: 48_000,
            envelope: 0.0,
            envelope_step: 1.0,
            gain_coeff: 1.0,
            mix: [TrackMix::default(); MAX_TRACKS],
        }
    }

    fn send_request(&mut self) {
        let request = FeederRequest {
            epoch: self.epoch,
            position: self.position.max(0.0).round() as u64,
            out_rate: self.out_rate,
            speed: self.speed,
            looping: self.looping,
        };
        if self.requests.push(request).is_err() {
            // Retried at the next render; the newest request wins.
            self.pending_request = Some(request);
        } else {
            self.pending_request = None;
        }
    }

    fn retry_request(&mut self) {
        if let Some(request) = self.pending_request
            && self.requests.push(request).is_ok()
        {
            self.pending_request = None;
        }
    }

    /// Starts a new epoch at song frame `position`.
    fn restart(&mut self, position: f64) {
        self.epoch = self.epoch.wrapping_add(1);
        self.position = position.clamp(0.0, self.song_frames as f64);
        self.ended = false;
        self.primed = false;
        self.draining = self.envelope > 0.0;
        self.send_request();
    }

    fn update_targets(&mut self) {
        let any_solo = self.mix[..self.tracks].iter().any(|track| track.solo);
        for track in &mut self.mix[..self.tracks] {
            let audible = !track.mute && (!any_solo || track.solo);
            track.target = if audible { track.volume } else { 0.0 };
        }
    }

    fn set_param(&mut self, id: u16, value: f32) {
        match id {
            param::SPEED => {
                let speed = value.clamp(MIN_SPEED, 1.0);
                let crosses = is_bypass(speed) != is_bypass(self.speed);
                self.speed = speed;
                if crosses {
                    self.restart(self.position);
                } else {
                    self.send_request();
                }
            }
            param::LOOP_START_HI..=param::LOOP_END_LO => {
                self.loop_parts[usize::from(id - param::LOOP_START_HI)] = value.max(0.0) as u64;
            }
            param::LOOP_APPLY => {
                let [start_hi, start_lo, end_hi, end_lo] = self.loop_parts;
                self.looping = if value > 0.5 {
                    LoopRange::new(
                        (start_hi << 16) | start_lo,
                        ((end_hi << 16) | end_lo).min(self.song_frames),
                    )
                } else {
                    None
                };
                self.restart(self.position);
            }
            _ => {
                let Some((track, kind)) = param::decode_track(id) else {
                    return;
                };
                let Some(mix) = self.mix.get_mut(track) else {
                    return;
                };
                match kind {
                    param::VOLUME => mix.volume = value.clamp(0.0, 1.0),
                    param::MUTE => mix.mute = value > 0.5,
                    param::SOLO => mix.solo = value > 0.5,
                    _ => return,
                }
                self.update_targets();
            }
        }
    }

    fn recycle(&mut self, block: Box<Block>) {
        if let Err(rtrb::PushError::Full(block)) = self.used.push(block) {
            // Unreachable: the queue holds every block of the pool. Never
            // free on the audio thread.
            std::mem::forget(block);
        }
    }

    /// Drops queued material of older epochs so the feeder can refill.
    fn discard_stale(&mut self) {
        if let Some(block) = self.current.take() {
            if block.epoch == self.epoch {
                self.current = Some(block);
            } else {
                self.recycle(block);
                self.offset = 0;
            }
        }
        while let Ok(block) = self.filled.peek() {
            if block.epoch == self.epoch {
                break;
            }
            if let Ok(block) = self.filled.pop() {
                self.recycle(block);
            }
        }
    }

    /// Makes `current` a block with frames left. While draining it takes
    /// blocks of older epochs only; otherwise current-epoch blocks only.
    fn ensure_block(&mut self) -> bool {
        loop {
            if let Some(block) = self.current.as_ref() {
                let fresh = block.epoch == self.epoch;
                if self.offset < block.frames && fresh != self.draining {
                    return true;
                }
                if self.draining && fresh {
                    // Old material ran out; the new epoch is already here.
                    return false;
                }
                let finished = fresh && self.offset >= block.frames && block.end;
                if let Some(block) = self.current.take() {
                    self.recycle(block);
                }
                self.offset = 0;
                if finished {
                    self.ended = true;
                    self.playing = false;
                    self.envelope = 0.0;
                    self.position = self.song_frames as f64;
                    return false;
                }
            }
            let Ok(block) = self.filled.pop() else {
                return false;
            };
            if !self.draining && block.epoch != self.epoch {
                self.recycle(block);
                continue;
            }
            if !self.draining {
                self.primed = true;
            }
            self.current = Some(block);
            self.offset = 0;
        }
    }

    /// Mixes frames from `current` into `out[from..]` while moving the
    /// envelope toward `target`. Stops early once a fade-out completes.
    /// Returns the frames written.
    fn mix_frames(&mut self, out: &mut [f32], channels: usize, from: usize, target: f32) -> usize {
        let Some(block) = self.current.as_ref() else {
            return 0;
        };
        let stride = self.tracks * 2;
        let total = out.len() / channels;
        let n = (total - from).min(block.frames - self.offset);
        let mut written = 0;
        for i in 0..n {
            if self.envelope < target {
                self.envelope = (self.envelope + self.envelope_step).min(target);
            } else if self.envelope > target {
                self.envelope = (self.envelope - self.envelope_step).max(target);
            }
            let frame = &block.data[(self.offset + i) * stride..(self.offset + i + 1) * stride];
            let (mut left, mut right) = (0.0, 0.0);
            for (track, mix) in self.mix[..self.tracks].iter_mut().enumerate() {
                if mix.gain != mix.target {
                    mix.gain += (mix.target - mix.gain) * self.gain_coeff;
                    if (mix.target - mix.gain).abs() < 1.0e-4 {
                        mix.gain = mix.target;
                    }
                }
                left += frame[track * 2] * mix.gain;
                right += frame[track * 2 + 1] * mix.gain;
            }
            let dst = &mut out[(from + i) * channels..(from + i + 1) * channels];
            if channels == 1 {
                dst[0] = (left + right) * 0.5 * self.envelope;
            } else {
                dst[0] = left * self.envelope;
                dst[1] = right * self.envelope;
            }
            written = i + 1;
            if target == 0.0 && self.envelope == 0.0 {
                break;
            }
        }
        self.offset += written;
        if !self.draining {
            self.position = block.position_at(self.offset);
        }
        written
    }

    fn publish(&self) {
        self.shared
            .position
            .store(self.position.to_bits(), Ordering::Release);
        self.shared
            .playing
            .store(self.playing && !self.ended, Ordering::Release);
        self.shared.ended.store(self.ended, Ordering::Release);
    }
}

impl Source for StemSource {
    fn prepare(&mut self, format: StreamFormat) {
        let rate = format.sample_rate.max(1) as f32;
        self.out_rate = format.sample_rate.max(1);
        self.envelope_step = 1.0 / (FADE_SECS * rate).max(1.0);
        self.gain_coeff = 1.0 - (-1.0 / (GAIN_SMOOTHING_SECS * rate)).exp();
        // Material queued for the old rate is dropped; start over here.
        self.envelope = 0.0;
        self.restart(self.position);
        self.draining = false;
    }

    fn on_event(&mut self, kind: &EventKind) {
        match *kind {
            EventKind::Start => {
                let at_end = self.ended
                    || (self.looping.is_none() && self.position >= self.song_frames as f64);
                if at_end {
                    self.restart(0.0);
                }
                self.playing = true;
            }
            EventKind::Stop => self.playing = false,
            EventKind::Seek { frame } => self.restart(frame as f64),
            EventKind::Param { id, value } => self.set_param(id, value),
            EventKind::Trigger { .. } => {}
        }
    }

    fn render(&mut self, out: &mut [f32], channels: usize) {
        self.retry_request();
        let frames = out.len() / channels;
        let mut at = 0;
        while at < frames {
            if self.draining {
                if self.envelope > 0.0 && self.ensure_block() {
                    at += self.mix_frames(out, channels, at, 0.0);
                    continue;
                }
                self.envelope = 0.0;
                self.draining = false;
                continue;
            }
            self.discard_stale();
            let audible = self.playing && !self.ended;
            if !audible && self.envelope == 0.0 {
                break;
            }
            if !self.ensure_block() {
                if audible && self.primed && !self.ended {
                    self.shared.underruns.fetch_add(1, Ordering::Relaxed);
                }
                // Nothing to fade with; the next material fades back in.
                self.envelope = 0.0;
                break;
            }
            let target = if audible { 1.0 } else { 0.0 };
            at += self.mix_frames(out, channels, at, target);
        }
        if !self.draining {
            self.discard_stale();
        }
        self.publish();
    }
}

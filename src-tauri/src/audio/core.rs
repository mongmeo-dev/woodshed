//! The device-independent render core: mixer plus sample-accurate scheduler.
//!
//! [`EngineCore::process`] runs on the audio thread (or in an offline render).
//! Everything it touches is allocated up front; commands arrive through a
//! lock-free SPSC queue and removed sources leave through another one, so the
//! audio thread never allocates, frees, locks or blocks.

use std::sync::Arc;

use cpal::{FromSample, Sample};
use rtrb::{Consumer, Producer, RingBuffer};

use super::handle::EngineHandle;
use super::shared::SharedState;
use super::source::Source;
use super::types::{
    CallbackTiming, FramePos, MAX_BLOCK_FRAMES, MAX_CHANNELS, RenderedEvent, ScheduledEvent,
    SourceId, StreamFormat,
};

/// Capacities fixed when the engine is created.
#[derive(Clone, Copy, Debug)]
pub struct EngineConfig {
    /// Sources the mixer can hold at once.
    pub max_sources: usize,
    /// Events that can wait on the schedule at once.
    pub max_events: usize,
    /// Commands that can wait in the UI → audio queue.
    pub command_capacity: usize,
    /// Rendered-event reports that can wait in the audio → UI queue.
    pub notify_capacity: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_sources: 64,
            max_events: 4096,
            command_capacity: 4096,
            notify_capacity: 1024,
        }
    }
}

/// UI → audio thread messages.
pub(crate) enum Command {
    AddSource {
        id: SourceId,
        source: Box<dyn Source>,
        gain: f32,
    },
    RemoveSource(SourceId),
    SetGain(SourceId, f32),
    SetMasterGain(f32),
    Schedule(ScheduledEvent),
    /// Drops every pending event for `target` at or after `from_frame`.
    Cancel {
        target: SourceId,
        from_frame: FramePos,
    },
}

struct Slot {
    id: SourceId,
    source: Box<dyn Source>,
    gain: f32,
    target_gain: f32,
}

/// Gain changes glide over roughly this long to avoid clicks.
const GAIN_SMOOTHING_SECS: f32 = 0.005;

pub struct EngineCore {
    format: StreamFormat,
    channels: usize,
    frame: FramePos,
    commands: Consumer<Command>,
    /// Removed (or rejected) sources travel back here so they are freed off
    /// the audio thread. Its capacity is `max_sources`, and the handle never
    /// lets more than `max_sources` sources exist on the audio side, so every
    /// source always has a way back.
    retired: Producer<Box<dyn Source>>,
    notify: Producer<RenderedEvent>,
    shared: Arc<SharedState>,
    sources: Vec<Slot>,
    /// Sources waiting for a free slot in `retired` (bounded by the same
    /// `max_sources` invariant, so pushing here never reallocates).
    parked: Vec<Box<dyn Source>>,
    /// Pending events sorted by frame, latest first, so the next one is `last()`.
    events: Vec<ScheduledEvent>,
    mix: Vec<f32>,
    scratch: Vec<f32>,
    master_gain: f32,
    gain_coeff: f32,
    /// Output generation this core renders for (mirrors `SharedState`).
    generation: u64,
    /// Frame and timing at the start of the current callback, used to stamp
    /// rendered-event reports with their playback time.
    callback_frame: FramePos,
    callback_timing: CallbackTiming,
}

/// Creates an engine: the UI-side handle, the render core, and the queue of
/// rendered-event reports.
pub fn create_engine(
    format: StreamFormat,
    config: EngineConfig,
) -> (EngineHandle, EngineCore, Consumer<RenderedEvent>) {
    assert!(
        (1..=MAX_CHANNELS).contains(&usize::from(format.channels)),
        "channel count {} is outside 1..={MAX_CHANNELS}",
        format.channels
    );
    let shared = Arc::new(SharedState::new(format.sample_rate));
    let (command_tx, command_rx) = RingBuffer::new(config.command_capacity);
    let (retired_tx, retired_rx) = RingBuffer::new(config.max_sources);
    let (notify_tx, notify_rx) = RingBuffer::new(config.notify_capacity);

    let core = EngineCore {
        format,
        channels: usize::from(format.channels),
        frame: 0,
        commands: command_rx,
        retired: retired_tx,
        notify: notify_tx,
        shared: Arc::clone(&shared),
        sources: Vec::with_capacity(config.max_sources),
        parked: Vec::with_capacity(config.max_sources),
        events: Vec::with_capacity(config.max_events),
        mix: vec![0.0; MAX_BLOCK_FRAMES * MAX_CHANNELS],
        scratch: vec![0.0; MAX_BLOCK_FRAMES * MAX_CHANNELS],
        master_gain: 1.0,
        gain_coeff: gain_coeff(format.sample_rate),
        generation: shared.generation(),
        callback_frame: 0,
        callback_timing: CallbackTiming {
            callback_ns: 0,
            playback_ns: 0,
        },
    };
    let handle = EngineHandle::new(command_tx, retired_rx, shared, format, config.max_sources);
    (handle, core, notify_rx)
}

fn gain_coeff(sample_rate: u32) -> f32 {
    1.0 - (-1.0 / (GAIN_SMOOTHING_SECS * sample_rate as f32)).exp()
}

impl EngineCore {
    pub fn format(&self) -> StreamFormat {
        self.format
    }

    /// Frames rendered so far.
    pub fn frame(&self) -> FramePos {
        self.frame
    }

    /// Changes the render format (e.g. after switching output devices). Calls
    /// [`Source::prepare`] on every source, so it must run off the audio
    /// thread, while no stream is rendering this core.
    pub fn reformat(&mut self, format: StreamFormat) {
        assert!((1..=MAX_CHANNELS).contains(&usize::from(format.channels)));
        // Apply commands that were queued for the old format first, so sources
        // still waiting in the queue are prepared for the new one below.
        self.drain_commands();
        self.retry_parked();
        self.format = format;
        self.channels = usize::from(format.channels);
        self.gain_coeff = gain_coeff(format.sample_rate);
        for slot in &mut self.sources {
            slot.source.prepare(format);
        }
        self.generation = self.shared.begin_generation(format.sample_rate);
    }

    /// Output generation this core renders for.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Renders one device callback into `out` (interleaved, `format.channels`
    /// channels). Real-time safe.
    pub fn process<T>(&mut self, out: &mut [T], timing: CallbackTiming)
    where
        T: Sample + FromSample<f32>,
    {
        let channels = self.channels;
        let total_frames = out.len() / channels;
        self.shared
            .publish_timing(self.generation, self.format.sample_rate, self.frame, timing);
        self.callback_frame = self.frame;
        self.callback_timing = timing;
        self.drain_commands();

        for chunk in out.chunks_mut(MAX_BLOCK_FRAMES * channels) {
            let frames = chunk.len() / channels;
            self.render_block(frames);
            let master = self.master_gain;
            for (dst, &src) in chunk.iter_mut().zip(&self.mix[..frames * channels]) {
                *dst = T::from_sample((src * master).clamp(-1.0, 1.0));
            }
        }

        self.retry_parked();
        self.shared
            .publish_rendered(self.frame, u32::try_from(total_frames).unwrap_or(u32::MAX));
    }

    fn drain_commands(&mut self) {
        while let Ok(command) = self.commands.pop() {
            match command {
                Command::AddSource { id, source, gain } => {
                    if self.sources.len() < self.sources.capacity() {
                        self.sources.push(Slot {
                            id,
                            source,
                            gain,
                            target_gain: gain,
                        });
                    } else {
                        // Unreachable while the handle's admission check holds;
                        // hand the source back rather than dropping it here.
                        self.retire(source);
                    }
                }
                Command::RemoveSource(id) => {
                    if let Some(index) = self.sources.iter().position(|slot| slot.id == id) {
                        let slot = self.sources.swap_remove(index);
                        self.retire(slot.source);
                    }
                }
                Command::SetGain(id, gain) => {
                    if let Some(slot) = self.sources.iter_mut().find(|slot| slot.id == id) {
                        slot.target_gain = gain.max(0.0);
                    }
                }
                Command::SetMasterGain(gain) => self.master_gain = gain.max(0.0),
                Command::Schedule(event) => self.insert_event(event),
                Command::Cancel { target, from_frame } => self
                    .events
                    .retain(|event| !(event.target == target && event.frame >= from_frame)),
            }
        }
    }

    fn insert_event(&mut self, event: ScheduledEvent) {
        if self.events.len() == self.events.capacity() {
            self.shared.count_dropped_event();
            return;
        }
        // Latest first. Inserting before existing events of the same frame
        // makes those (scheduled earlier) render first.
        let index = self.events.partition_point(|e| e.frame > event.frame);
        self.events.insert(index, event);
    }

    fn retire(&mut self, source: Box<dyn Source>) {
        if let Err(rtrb::PushError::Full(source)) = self.retired.push(source) {
            // At most `max_sources` sources exist on this side (handle
            // admission), so `parked` never grows past its capacity.
            debug_assert!(self.parked.len() < self.parked.capacity());
            self.parked.push(source);
        }
    }

    fn retry_parked(&mut self) {
        while let Some(source) = self.parked.pop() {
            if let Err(rtrb::PushError::Full(source)) = self.retired.push(source) {
                self.parked.push(source);
                break;
            }
        }
    }

    /// Renders `frames` frames into `self.mix`, applying every event whose
    /// frame falls inside the block exactly at that frame.
    fn render_block(&mut self, frames: usize) {
        let channels = self.channels;
        let start = self.frame;
        let end = start + frames as FramePos;
        self.mix[..frames * channels].fill(0.0);

        let mut cursor = start;
        loop {
            let next_due = self
                .events
                .last()
                .copied()
                .filter(|event| event.frame < end);
            let segment_end = next_due.map_or(end, |event| event.frame.max(cursor));
            if segment_end > cursor {
                self.render_segment((cursor - start) as usize, (segment_end - start) as usize);
                cursor = segment_end;
            }
            let Some(event) = next_due else { break };
            self.events.pop();
            self.deliver(event, cursor);
        }
        self.frame = end;
    }

    fn deliver(&mut self, event: ScheduledEvent, rendered_frame: FramePos) {
        if event.frame < rendered_frame {
            self.shared.count_late();
        }
        let Some(slot) = self.sources.iter_mut().find(|slot| slot.id == event.target) else {
            return;
        };
        slot.source.on_event(&event.kind);
        if event.notify {
            // A full report queue only costs the UI a notification.
            let since_callback = rendered_frame.saturating_sub(self.callback_frame);
            let offset_ns = since_callback.saturating_mul(1_000_000_000)
                / u64::from(self.format.sample_rate.max(1));
            let _ = self.notify.push(RenderedEvent {
                scheduled_frame: event.frame,
                rendered_frame,
                target: event.target,
                kind: event.kind,
                generation: self.generation,
                playback_ns: self.callback_timing.playback_ns.saturating_add(offset_ns),
            });
        }
    }

    /// Renders every source for block-relative frames `from..to` and mixes them.
    fn render_segment(&mut self, from: usize, to: usize) {
        let channels = self.channels;
        let range = from * channels..to * channels;
        let coeff = self.gain_coeff;
        for slot in &mut self.sources {
            let scratch = &mut self.scratch[range.clone()];
            scratch.fill(0.0);
            slot.source.render(scratch, channels);
            let mix = &mut self.mix[range.clone()];
            if slot.gain == slot.target_gain {
                let gain = slot.gain;
                for (dst, &src) in mix.iter_mut().zip(scratch.iter()) {
                    *dst += src * gain;
                }
            } else {
                for (dst_frame, src_frame) in mix
                    .chunks_exact_mut(channels)
                    .zip(scratch.chunks_exact(channels))
                {
                    slot.gain += (slot.target_gain - slot.gain) * coeff;
                    if (slot.target_gain - slot.gain).abs() < 1.0e-4 {
                        slot.gain = slot.target_gain;
                    }
                    for (dst, &src) in dst_frame.iter_mut().zip(src_frame) {
                        *dst += src * slot.gain;
                    }
                }
            }
        }
    }
}

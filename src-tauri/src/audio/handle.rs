//! UI-side handle: the only way other threads talk to the render core.

use std::sync::Arc;

use rtrb::{Consumer, Producer};

use super::core::Command;
use super::shared::{EngineStats, SharedState, TimingSnapshot};
use super::source::Source;
use super::types::{EventKind, FramePos, ScheduledEvent, SourceId, StreamFormat};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EngineError {
    #[error("the engine command queue is full")]
    QueueFull,
}

/// Sends commands to the audio thread. Lives on a normal thread (behind a
/// mutex in the app); every method is non-blocking.
pub struct EngineHandle {
    commands: Producer<Command>,
    retired: Consumer<Box<dyn Source>>,
    shared: Arc<SharedState>,
    format: StreamFormat,
    next_id: u32,
}

impl EngineHandle {
    pub(crate) fn new(
        commands: Producer<Command>,
        retired: Consumer<Box<dyn Source>>,
        shared: Arc<SharedState>,
        format: StreamFormat,
    ) -> Self {
        Self {
            commands,
            retired,
            shared,
            format,
            next_id: 1,
        }
    }

    pub fn format(&self) -> StreamFormat {
        self.format
    }

    pub(crate) fn set_format(&mut self, format: StreamFormat) {
        self.format = format;
    }

    pub fn shared(&self) -> &Arc<SharedState> {
        &self.shared
    }

    fn send(&mut self, command: Command) -> Result<(), EngineError> {
        self.collect_retired();
        self.commands
            .push(command)
            .map_err(|_| EngineError::QueueFull)
    }

    /// Frees sources the audio thread has detached. Called by every command;
    /// call it yourself if you go a long time without sending commands.
    pub fn collect_retired(&mut self) {
        while let Ok(source) = self.retired.pop() {
            drop(source);
        }
    }

    /// Prepares `source` for the current format (on this thread) and attaches
    /// it to the mixer with `gain` (linear, 1.0 = unity).
    pub fn add_source(
        &mut self,
        mut source: Box<dyn Source>,
        gain: f32,
    ) -> Result<SourceId, EngineError> {
        source.prepare(self.format);
        let id = SourceId(self.next_id);
        self.next_id += 1;
        self.send(Command::AddSource { id, source, gain })?;
        Ok(id)
    }

    /// Detaches a source. It is freed on this thread at a later command.
    pub fn remove_source(&mut self, id: SourceId) -> Result<(), EngineError> {
        self.send(Command::RemoveSource(id))
    }

    /// Sets a source's gain (linear). Changes glide over ~5 ms.
    pub fn set_gain(&mut self, id: SourceId, gain: f32) -> Result<(), EngineError> {
        self.send(Command::SetGain(id, gain))
    }

    pub fn set_master_gain(&mut self, gain: f32) -> Result<(), EngineError> {
        self.send(Command::SetMasterGain(gain))
    }

    /// Places `kind` for `target` at absolute `frame`. It renders exactly at
    /// that frame if it reaches the audio thread before the frame is rendered
    /// (schedule at least one callback ahead of [`Self::frames_rendered`]).
    pub fn schedule(
        &mut self,
        frame: FramePos,
        target: SourceId,
        kind: EventKind,
    ) -> Result<(), EngineError> {
        self.schedule_event(ScheduledEvent {
            frame,
            target,
            kind,
            notify: false,
        })
    }

    /// Like [`Self::schedule`], and the engine reports the event once rendered
    /// so the UI can learn when it becomes audible.
    pub fn schedule_notify(
        &mut self,
        frame: FramePos,
        target: SourceId,
        kind: EventKind,
    ) -> Result<(), EngineError> {
        self.schedule_event(ScheduledEvent {
            frame,
            target,
            kind,
            notify: true,
        })
    }

    pub fn schedule_event(&mut self, event: ScheduledEvent) -> Result<(), EngineError> {
        self.send(Command::Schedule(event))
    }

    /// Drops pending events for `target` at or after `from_frame`.
    pub fn cancel(&mut self, target: SourceId, from_frame: FramePos) -> Result<(), EngineError> {
        self.send(Command::Cancel { target, from_frame })
    }

    /// Frames the audio thread has rendered so far.
    pub fn frames_rendered(&self) -> FramePos {
        self.shared.frames_rendered()
    }

    /// Earliest frame that is safe to schedule with `lead_secs` of margin past
    /// what the audio thread has already rendered.
    pub fn frame_after(&self, lead_secs: f64) -> FramePos {
        self.frames_rendered() + self.format.frames_for_secs(lead_secs)
    }

    pub fn timing(&self) -> Option<TimingSnapshot> {
        self.shared.timing()
    }

    pub fn stats(&self) -> EngineStats {
        self.shared.stats()
    }
}

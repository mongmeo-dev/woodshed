//! Step sequencer: places the pattern's hits on the engine's frame clock.
//!
//! The sequencer keeps a timeline that maps frames to positions (in straight
//! sixteenths) and schedules each step at the frame of its swung position.
//! The caller calls [`Sequencer::fill`] regularly with a horizon a little
//! ahead of the audio thread. Changes while playing (pattern, tempo, swing,
//! length, mix, a new output rate) cancel what was scheduled from a frame
//! just ahead of the audio thread and reschedule from there, so playback
//! never stops and the timeline stays continuous.

use std::collections::VecDeque;

use crate::audio::{EngineError, EngineHandle, EventKind, FramePos, SourceId};

use super::kit::Track;
use super::pattern::{Pattern, level_velocity};

/// A step handed to the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScheduledStep {
    /// Absolute step, counted from the start of playback.
    pub step: u64,
    /// Step within the pattern (what the grid highlights).
    pub index: u32,
    pub frame: FramePos,
}

pub struct Sequencer {
    source: SourceId,
    pattern: Pattern,
    playing: bool,
    /// Timeline: `anchor_position` (straight sixteenths) is at `anchor_frame`.
    anchor_frame: f64,
    anchor_position: f64,
    frames_per_step: f64,
    /// Next absolute step (counted from the start of playback) to schedule.
    next_step: u64,
    /// Steps handed to the engine whose frames may not be rendered yet.
    scheduled: VecDeque<ScheduledStep>,
    /// No event goes before this frame: the latest start or reschedule point.
    floor: FramePos,
    /// Output generation the schedule was computed for.
    generation: u64,
}

impl Sequencer {
    pub fn new(source: SourceId, pattern: Pattern) -> Self {
        Self {
            source,
            pattern,
            playing: false,
            anchor_frame: 0.0,
            anchor_position: 0.0,
            frames_per_step: 1.0,
            next_step: 0,
            scheduled: VecDeque::new(),
            floor: 0,
            generation: 0,
        }
    }

    pub fn source(&self) -> SourceId {
        self.source
    }

    pub fn pattern(&self) -> &Pattern {
        &self.pattern
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Output generation the current schedule belongs to.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Steps scheduled and possibly not yet rendered, earliest first.
    pub fn scheduled(&self) -> impl DoubleEndedIterator<Item = &ScheduledStep> {
        self.scheduled.iter()
    }

    /// Frame at which absolute step `step` plays on the current timeline.
    pub fn step_frame(&self, step: u64) -> f64 {
        self.anchor_frame
            + (self.pattern.swung_position(step) - self.anchor_position) * self.frames_per_step
    }

    /// Starts the pattern from its first step at frame `at` (ahead of the
    /// audio thread). Call [`Self::fill`] afterwards.
    pub fn start(&mut self, handle: &mut EngineHandle, at: FramePos) -> Result<(), EngineError> {
        handle.cancel(self.source, 0)?;
        self.playing = true;
        self.anchor_frame = at as f64;
        self.anchor_position = 0.0;
        self.frames_per_step = self.pattern.frames_per_step(handle.format().sample_rate);
        self.next_step = 0;
        self.scheduled.clear();
        self.floor = at;
        self.generation = handle.generation();
        self.send_gains(handle, at)
    }

    /// Stops playback and drops everything still scheduled. Sounds already
    /// started ring out.
    pub fn stop(&mut self, handle: &mut EngineHandle) -> Result<(), EngineError> {
        self.playing = false;
        self.scheduled.clear();
        handle.cancel(self.source, 0)
    }

    /// Replaces the pattern. While playing, events from `from` on are
    /// rescheduled with it (`from` must be ahead of the audio thread); call
    /// [`Self::fill`] afterwards.
    pub fn set_pattern(
        &mut self,
        handle: &mut EngineHandle,
        pattern: Pattern,
        from: FramePos,
    ) -> Result<(), EngineError> {
        if !self.playing {
            self.pattern = pattern;
            return Ok(());
        }
        self.reschedule(handle, from, |sequencer| sequencer.pattern = pattern)
    }

    /// Re-anchors the schedule after the output changed: cancels from `from`
    /// and continues at the new sample rate. Call [`Self::fill`] afterwards.
    pub fn retime(&mut self, handle: &mut EngineHandle, from: FramePos) -> Result<(), EngineError> {
        if !self.playing {
            self.generation = handle.generation();
            return Ok(());
        }
        self.reschedule(handle, from, |_| {})
    }

    /// Schedules every step that starts before frame `until` and returns how
    /// many it added (the last entries of [`Self::scheduled`]).
    pub fn fill(
        &mut self,
        handle: &mut EngineHandle,
        until: FramePos,
    ) -> Result<usize, EngineError> {
        if !self.playing {
            return Ok(0);
        }
        let rendered = handle.frames_rendered();
        while self
            .scheduled
            .front()
            .is_some_and(|scheduled| scheduled.frame < rendered)
        {
            self.scheduled.pop_front();
        }
        let mut added = 0;
        let length = u64::from(self.pattern.length);
        let floor = self.floor as f64;
        loop {
            let step = self.next_step;
            let exact = self.step_frame(step).round();
            // Every step from `next_step` on is still to be heard. A change
            // (less swing) can move one behind the reschedule point before
            // it played; it then plays at the point instead of vanishing.
            let frame = if exact >= floor {
                exact as FramePos
            } else {
                self.floor
            };
            if frame >= until {
                return Ok(added);
            }
            let index = (step % length) as usize;
            for track in Track::ALL {
                let level = self.pattern.level(track, index);
                if level > 0 && self.pattern.is_audible(track) {
                    handle.schedule(
                        frame,
                        self.source,
                        EventKind::Trigger {
                            note: track.index() as u8,
                            velocity: level_velocity(level),
                        },
                    )?;
                }
            }
            self.scheduled.push_back(ScheduledStep {
                step,
                index: index as u32,
                frame,
            });
            self.next_step += 1;
            added += 1;
        }
    }

    fn reschedule(
        &mut self,
        handle: &mut EngineHandle,
        from: FramePos,
        update: impl FnOnce(&mut Self),
    ) -> Result<(), EngineError> {
        // Where the old timeline stands at `from`; the new one continues there.
        let position =
            self.anchor_position + (from as f64 - self.anchor_frame) / self.frames_per_step;
        handle.cancel(self.source, from)?;
        while let Some(&scheduled) = self.scheduled.back() {
            if scheduled.frame < from {
                break;
            }
            self.next_step = scheduled.step;
            self.scheduled.pop_back();
        }
        update(self);
        self.anchor_frame = from as f64;
        self.anchor_position = position;
        self.frames_per_step = self.pattern.frames_per_step(handle.format().sample_rate);
        self.floor = from;
        self.generation = handle.generation();
        self.send_gains(handle, from)
    }

    /// Sets every track's gain at `at`, before the hits of that frame.
    fn send_gains(&self, handle: &mut EngineHandle, at: FramePos) -> Result<(), EngineError> {
        for track in Track::ALL {
            handle.schedule(
                at,
                self.source,
                EventKind::Param {
                    id: track.index() as u16,
                    value: self.pattern.track_gain(track),
                },
            )?;
        }
        Ok(())
    }
}

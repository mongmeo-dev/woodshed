//! Feeds the timeline into the engine ahead of the render position.
//!
//! The scheduler runs on a normal thread (or in a test loop). Each
//! [`Scheduler::pump`] schedules every beat that starts before
//! `now + horizon`, remembering a cursor per beat it planned. A settings
//! change or an output switch cancels the planned beats from the first one
//! that is still safely ahead and regenerates from its cursor, so changes
//! land on the next beat without stopping playback.

use std::collections::VecDeque;

use crate::audio::{EngineError, EngineHandle, EventKind, FramePos, SourceId};

use super::pattern::MetronomeSettings;
use super::timeline::{BeatMark, BeatOutput, Cursor, PulseMark, Timeline};

/// How far ahead of the render position beats are scheduled.
pub const HORIZON_SECS: f64 = 0.25;
/// A change (or a restart after an output switch) never moves beats closer
/// than this to the render position, so it always reaches the audio thread
/// before their frame.
pub const CHANGE_LEAD_SECS: f64 = 0.04;
/// Delay from "start" to the first beat.
pub const START_LEAD_SECS: f64 = 0.06;

/// What the screen needs to know, in the order it happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Announcement {
    Beat(BeatMark),
    Pulse(PulseMark),
    /// Beats and pulses announced at or after `from_frame` will not play.
    Reset {
        from_frame: FramePos,
    },
}

#[derive(Clone, Debug)]
struct Planned {
    /// Timeline state just before this beat was generated.
    cursor: Cursor,
    mark: BeatMark,
    pulses: Vec<PulseMark>,
}

pub struct Scheduler {
    source: SourceId,
    timeline: Timeline,
    /// Beats scheduled and not yet passed, oldest first.
    planned: VecDeque<Planned>,
    output: BeatOutput,
}

fn secs_to_frames(secs: f64, sample_rate: u32) -> FramePos {
    (secs * f64::from(sample_rate)).round() as FramePos
}

impl Scheduler {
    /// Plays `settings` on `source`, with the first beat on `start_frame`.
    pub fn new(
        source: SourceId,
        settings: &MetronomeSettings,
        sample_rate: u32,
        start_frame: FramePos,
    ) -> Self {
        Self {
            source,
            timeline: Timeline::new(settings, sample_rate, start_frame),
            planned: VecDeque::new(),
            output: BeatOutput::default(),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.timeline.sample_rate()
    }

    /// True once the trainer has stopped playback and `now` is past the end.
    pub fn is_done(&self, now: FramePos) -> bool {
        self.timeline.is_finished() && now >= self.timeline.next_frame()
    }

    /// Beats and pulses already planned that are still ahead of `now`, for a
    /// screen that subscribes while playing.
    pub fn upcoming(&self, now: FramePos) -> Vec<Announcement> {
        self.planned
            .iter()
            .filter(|planned| planned.mark.frame >= now)
            .flat_map(|planned| {
                std::iter::once(Announcement::Beat(planned.mark))
                    .chain(planned.pulses.iter().copied().map(Announcement::Pulse))
            })
            .collect()
    }

    /// Schedules every beat that starts before `now + horizon`.
    pub fn pump(
        &mut self,
        handle: &mut EngineHandle,
        now: FramePos,
        announce: &mut Vec<Announcement>,
    ) -> Result<(), EngineError> {
        while self
            .planned
            .front()
            .is_some_and(|planned| planned.mark.frame < now)
        {
            self.planned.pop_front();
        }
        let horizon = now + secs_to_frames(HORIZON_SECS, self.sample_rate());
        while !self.timeline.is_finished() && self.timeline.next_frame() < horizon {
            let cursor = self.timeline.cursor();
            let Some(mark) = self.timeline.generate(&mut self.output) else {
                break;
            };
            for click in &self.output.clicks {
                let sent = handle.schedule(
                    click.frame,
                    self.source,
                    EventKind::Trigger {
                        note: click.note,
                        velocity: click.velocity,
                    },
                );
                if let Err(error) = sent {
                    // Undo this beat and retry it on the next pump.
                    self.timeline.restore(cursor);
                    let _ = handle.cancel(self.source, mark.frame);
                    return Err(error);
                }
            }
            announce.push(Announcement::Beat(mark));
            announce.extend(self.output.pulses.iter().copied().map(Announcement::Pulse));
            self.planned.push_back(Planned {
                cursor,
                mark,
                pulses: self.output.pulses.clone(),
            });
        }
        Ok(())
    }

    /// Applies new settings from the next beat that is at least
    /// [`CHANGE_LEAD_SECS`] ahead of `now`. Beats before it play as planned.
    pub fn update(
        &mut self,
        handle: &mut EngineHandle,
        now: FramePos,
        settings: &MetronomeSettings,
        announce: &mut Vec<Announcement>,
    ) -> Result<(), EngineError> {
        let cut = now + secs_to_frames(CHANGE_LEAD_SECS, self.sample_rate());
        if let Some(index) = self
            .planned
            .iter()
            .position(|planned| planned.mark.frame >= cut)
        {
            let from = self.planned[index].clone();
            handle.cancel(self.source, from.mark.frame)?;
            self.planned.truncate(index);
            self.timeline.restore(from.cursor);
            announce.push(Announcement::Reset {
                from_frame: from.mark.frame,
            });
        }
        self.timeline.apply(settings);
        Ok(())
    }

    /// Re-times playback after the output was reopened at `sample_rate`:
    /// every pending event (placed at the old rate) is cancelled and the
    /// beat grid continues from the first beat that, mapped to the new rate,
    /// is still safely ahead.
    pub fn retime(
        &mut self,
        handle: &mut EngineHandle,
        now: FramePos,
        sample_rate: u32,
        announce: &mut Vec<Announcement>,
    ) -> Result<(), EngineError> {
        let old_rate = self.sample_rate();
        handle.cancel(self.source, now)?;
        let cut = now + secs_to_frames(CHANGE_LEAD_SECS, sample_rate);
        // Same distance from `now` in seconds, counted at the new rate.
        let remap = |frame: FramePos| -> FramePos {
            let ahead = u128::from(frame.saturating_sub(now));
            let rate = u128::from(old_rate.max(1));
            now + u64::try_from((ahead * u128::from(sample_rate) * 2 + rate) / (2 * rate))
                .unwrap_or(u64::MAX)
        };
        let restart = self
            .planned
            .iter()
            .find(|planned| remap(planned.mark.frame) >= cut)
            .map(|planned| (planned.cursor, planned.mark.frame));
        let (cursor, old_frame) =
            restart.unwrap_or((self.timeline.cursor(), self.timeline.next_frame()));
        self.timeline.restore(cursor);
        self.timeline.rebase(sample_rate, remap(old_frame).max(cut));
        self.planned.clear();
        announce.push(Announcement::Reset { from_frame: 0 });
        Ok(())
    }

    /// Cancels everything still pending from `now` on.
    pub fn stop(&mut self, handle: &mut EngineHandle, now: FramePos) -> Result<(), EngineError> {
        self.planned.clear();
        handle.cancel(self.source, now)
    }
}

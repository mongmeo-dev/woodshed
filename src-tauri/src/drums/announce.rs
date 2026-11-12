//! Tells the screen ahead of time when each scheduled step is heard, so the
//! grid can light the step in step with the sound (no wait for a
//! rendered-event report).

use serde::Serialize;

use crate::audio::{AudioEngine, FramePos};

use super::sequencer::ScheduledStep;

/// A step the screen highlights when it is heard.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeardStep {
    /// Step within the pattern.
    pub index: u32,
    pub frame: FramePos,
    /// Unix-epoch ms; `heardAtMs - performance.timeOrigin` is the webview's
    /// `performance.now()` at which the step sounds.
    pub heard_at_ms: f64,
}

/// Sent to the screen's step channel.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum StepEvent {
    /// Steps just scheduled, with the time each is heard.
    Steps {
        generation: u64,
        steps: Vec<HeardStep>,
    },
    /// Forget announced steps at or after `from_frame`: they were rescheduled
    /// (and are announced again right after) or playback stopped.
    Clear { from_frame: FramePos },
}

/// Maps engine frames to the wall-clock time they are heard.
pub trait HeardClock {
    /// `None` until the current output has run its first callback.
    fn heard_at_ms(&self, frame: FramePos) -> Option<f64>;
    fn generation(&self) -> u64;
}

impl HeardClock for AudioEngine {
    fn heard_at_ms(&self, frame: FramePos) -> Option<f64> {
        self.heard_at_epoch_ms(frame)
    }

    fn generation(&self) -> u64 {
        AudioEngine::generation(self)
    }
}

/// Holds steps that could not be timed yet (a new output has no timing
/// before its first callback) and turns steps into [`StepEvent`]s.
#[derive(Debug, Default)]
pub struct Announcer {
    waiting: Vec<ScheduledStep>,
}

impl Announcer {
    /// Withdraws announced and waiting steps at or after `from`.
    pub fn clear(&mut self, from: FramePos, outbox: &mut Vec<StepEvent>) {
        self.waiting.retain(|step| step.frame < from);
        outbox.push(StepEvent::Clear { from_frame: from });
    }

    /// Announces `steps`, plus any still waiting, once the clock can time them.
    pub fn announce(
        &mut self,
        clock: &impl HeardClock,
        steps: impl IntoIterator<Item = ScheduledStep>,
        outbox: &mut Vec<StepEvent>,
    ) {
        self.waiting.extend(steps);
        let Some(first) = self.waiting.first() else {
            return;
        };
        if clock.heard_at_ms(first.frame).is_none() {
            return;
        }
        let steps = self
            .waiting
            .drain(..)
            .filter_map(|step| {
                Some(HeardStep {
                    index: step.index,
                    frame: step.frame,
                    heard_at_ms: clock.heard_at_ms(step.frame)?,
                })
            })
            .collect();
        outbox.push(StepEvent::Steps {
            generation: clock.generation(),
            steps,
        });
    }

    /// Replaces everything announced with `steps` (after an output change
    /// every earlier time is stale).
    pub fn announce_all(
        &mut self,
        clock: &impl HeardClock,
        steps: impl IntoIterator<Item = ScheduledStep>,
        outbox: &mut Vec<StepEvent>,
    ) {
        self.clear(0, outbox);
        self.announce(clock, steps, outbox);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeClock {
        timed: bool,
        generation: u64,
    }

    impl HeardClock for FakeClock {
        fn heard_at_ms(&self, frame: FramePos) -> Option<f64> {
            self.timed.then(|| 1_000.0 + frame as f64 / 48.0)
        }

        fn generation(&self) -> u64 {
            self.generation
        }
    }

    fn step(index: u32, frame: FramePos) -> ScheduledStep {
        ScheduledStep {
            step: u64::from(index),
            index,
            frame,
        }
    }

    #[test]
    fn steps_are_announced_with_their_heard_time() {
        let clock = FakeClock {
            timed: true,
            generation: 3,
        };
        let mut announcer = Announcer::default();
        let mut outbox = Vec::new();
        announcer.announce(&clock, [step(0, 4_800), step(1, 9_600)], &mut outbox);
        assert_eq!(
            outbox,
            vec![StepEvent::Steps {
                generation: 3,
                steps: vec![
                    HeardStep {
                        index: 0,
                        frame: 4_800,
                        heard_at_ms: 1_100.0
                    },
                    HeardStep {
                        index: 1,
                        frame: 9_600,
                        heard_at_ms: 1_200.0
                    },
                ],
            }]
        );
    }

    #[test]
    fn steps_wait_until_the_new_output_can_time_them() {
        let mut clock = FakeClock {
            timed: false,
            generation: 2,
        };
        let mut announcer = Announcer::default();
        let mut outbox = Vec::new();
        announcer.announce_all(&clock, [step(0, 100), step(1, 200)], &mut outbox);
        assert_eq!(outbox, vec![StepEvent::Clear { from_frame: 0 }]);

        // A reschedule from frame 200 withdraws the waiting step at 200.
        outbox.clear();
        announcer.clear(200, &mut outbox);
        clock.timed = true;
        announcer.announce(&clock, [step(1, 250)], &mut outbox);
        let StepEvent::Steps { steps, .. } = &outbox[1] else {
            panic!("expected steps, got {outbox:?}");
        };
        let frames: Vec<FramePos> = steps.iter().map(|s| s.frame).collect();
        assert_eq!(frames, vec![100, 250]);
    }

    #[test]
    fn events_serialize_for_the_webview() {
        let json = serde_json::to_string(&StepEvent::Clear { from_frame: 7 }).unwrap();
        assert_eq!(json, r#"{"type":"clear","fromFrame":7}"#);
        let json = serde_json::to_string(&StepEvent::Steps {
            generation: 1,
            steps: vec![HeardStep {
                index: 2,
                frame: 3,
                heard_at_ms: 4.5,
            }],
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"type":"steps","generation":1,"steps":[{"index":2,"frame":3,"heardAtMs":4.5}]}"#
        );
    }
}

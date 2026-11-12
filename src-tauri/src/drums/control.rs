//! Drum machine control: the sequencer plus the step announcements for the
//! screen, driven against any engine that offers a handle and a heard-time
//! clock (the live [`AudioEngine`], or an offline one in tests).
//!
//! Every entry point first follows an output change ([`DrumControl::tick`],
//! [`DrumControl::set_pattern`], [`DrumControl::announce_all`]), so whichever
//! reaches the engine first after a new generation reschedules at the new
//! sample rate and withdraws every heard time computed for the old output.

use crate::audio::{AudioEngine, EngineError, EngineHandle};

use super::announce::{Announcer, HeardClock, StepEvent};
use super::pattern::Pattern;
use super::sequencer::{ScheduledStep, Sequencer};

/// How far ahead of the audio thread playback starts.
pub const START_LEAD_SECS: f64 = 0.025;
/// How far ahead of the audio thread a change takes effect while playing.
pub const EDIT_LEAD_SECS: f64 = 0.02;
/// How far ahead steps are kept scheduled.
pub const LOOKAHEAD_SECS: f64 = 0.12;

/// What the drum control needs from an engine.
pub trait DrumEngine: HeardClock {
    fn handle(&mut self) -> &mut EngineHandle;
}

impl DrumEngine for AudioEngine {
    fn handle(&mut self) -> &mut EngineHandle {
        AudioEngine::handle(self)
    }
}

pub struct DrumControl {
    sequencer: Sequencer,
    announcer: Announcer,
}

impl DrumControl {
    pub fn new(sequencer: Sequencer) -> Self {
        Self {
            sequencer,
            announcer: Announcer::default(),
        }
    }

    pub fn sequencer(&self) -> &Sequencer {
        &self.sequencer
    }

    /// If the output changed since the schedule was made, reschedules for
    /// the new sample rate and replaces every announcement (`clear(0)`, then
    /// all scheduled steps with the new output's heard times, or as soon as
    /// it has them). Returns whether it did.
    fn follow_output(
        &mut self,
        engine: &mut impl DrumEngine,
        outbox: &mut Vec<StepEvent>,
    ) -> Result<bool, EngineError> {
        let handle = engine.handle();
        if handle.generation() == self.sequencer.generation() {
            return Ok(false);
        }
        let from = handle.frame_after(EDIT_LEAD_SECS);
        self.sequencer.retime(handle, from)?;
        if self.sequencer.is_playing() {
            fill(&mut self.sequencer, handle)?;
        }
        self.replace_announcements(engine, outbox);
        Ok(true)
    }

    fn replace_announcements(&mut self, clock: &impl HeardClock, outbox: &mut Vec<StepEvent>) {
        let steps: Vec<ScheduledStep> = self.sequencer.scheduled().copied().collect();
        self.announcer.announce_all(clock, steps, outbox);
    }

    /// One scheduler tick: follows an output change, tops up the schedule
    /// and announces the new steps (and any waiting for the output's timing).
    pub fn tick(
        &mut self,
        engine: &mut impl DrumEngine,
        outbox: &mut Vec<StepEvent>,
    ) -> Result<(), EngineError> {
        if !self.sequencer.is_playing() || self.follow_output(engine, outbox)? {
            return Ok(());
        }
        let added = fill(&mut self.sequencer, engine.handle())?;
        let mut fresh: Vec<ScheduledStep> = self
            .sequencer
            .scheduled()
            .rev()
            .take(added)
            .copied()
            .collect();
        fresh.reverse();
        self.announcer.announce(engine, fresh, outbox);
        Ok(())
    }

    pub fn set_playing(
        &mut self,
        engine: &mut impl DrumEngine,
        playing: bool,
        outbox: &mut Vec<StepEvent>,
    ) -> Result<(), EngineError> {
        if self.sequencer.is_playing() == playing {
            return Ok(());
        }
        let handle = engine.handle();
        if playing {
            let at = handle.frame_after(START_LEAD_SECS);
            self.sequencer.start(handle, at)?;
            fill(&mut self.sequencer, handle)?;
            self.replace_announcements(engine, outbox);
        } else {
            self.sequencer.stop(handle)?;
            self.announcer.clear(0, outbox);
        }
        Ok(())
    }

    /// Applies an edited pattern; while playing it is heard from a moment
    /// ahead without stopping.
    pub fn set_pattern(
        &mut self,
        engine: &mut impl DrumEngine,
        pattern: Pattern,
        outbox: &mut Vec<StepEvent>,
    ) -> Result<(), EngineError> {
        self.follow_output(engine, outbox)?;
        let handle = engine.handle();
        let from = handle.frame_after(EDIT_LEAD_SECS);
        self.sequencer.set_pattern(handle, pattern, from)?;
        if !self.sequencer.is_playing() {
            return Ok(());
        }
        fill(&mut self.sequencer, handle)?;
        let rescheduled: Vec<ScheduledStep> = self
            .sequencer
            .scheduled()
            .filter(|step| step.frame >= from)
            .copied()
            .collect();
        self.announcer.clear(from, outbox);
        self.announcer.announce(engine, rescheduled, outbox);
        Ok(())
    }

    /// Announces everything scheduled from scratch (a new screen channel).
    pub fn announce_all(
        &mut self,
        engine: &mut impl DrumEngine,
        outbox: &mut Vec<StepEvent>,
    ) -> Result<(), EngineError> {
        if !self.follow_output(engine, outbox)? {
            self.replace_announcements(engine, outbox);
        }
        Ok(())
    }
}

fn fill(sequencer: &mut Sequencer, handle: &mut EngineHandle) -> Result<usize, EngineError> {
    let until = handle.frame_after(LOOKAHEAD_SECS);
    sequencer.fill(handle, until)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::audio::testing::ImpulseSource;
    use crate::audio::{FramePos, OfflineEngine, StreamFormat};
    use crate::drums::{MAX_LEVEL, Track};

    /// An offline engine with a controllable heard-time clock: `timed` is
    /// false until the current output's first callback.
    struct FakeEngine {
        offline: OfflineEngine,
        timed: bool,
        latency_ms: f64,
    }

    impl FakeEngine {
        fn new(rate: u32) -> Self {
            Self {
                offline: OfflineEngine::new(StreamFormat::new(rate, 2)),
                timed: true,
                latency_ms: 5.0,
            }
        }

        fn clock(&self, frame: FramePos) -> f64 {
            1_000.0
                + frame as f64 * 1_000.0 / f64::from(self.offline.format().sample_rate)
                + self.latency_ms
        }

        fn render(&mut self, frames: usize) {
            self.offline.render(frames, 64);
        }
    }

    impl HeardClock for FakeEngine {
        fn heard_at_ms(&self, frame: FramePos) -> Option<f64> {
            self.timed.then(|| self.clock(frame))
        }

        fn generation(&self) -> u64 {
            self.offline.handle.generation()
        }
    }

    impl DrumEngine for FakeEngine {
        fn handle(&mut self) -> &mut EngineHandle {
            &mut self.offline.handle
        }
    }

    /// The screen's queue of upcoming steps: frame → heard time.
    #[derive(Default)]
    struct Screen(BTreeMap<FramePos, f64>);

    impl Screen {
        fn apply(&mut self, outbox: &mut Vec<StepEvent>) {
            for event in outbox.drain(..) {
                match event {
                    StepEvent::Clear { from_frame } => self.0.retain(|&f, _| f < from_frame),
                    StepEvent::Steps { steps, .. } => {
                        for step in steps {
                            self.0.insert(step.frame, step.heard_at_ms);
                        }
                    }
                }
            }
        }
    }

    fn groove(steps: &[usize]) -> Pattern {
        let mut pattern = Pattern {
            bpm: 120,
            ..Pattern::default()
        }
        .normalized();
        let hats = pattern.tracks.get_mut(&Track::ClosedHat).unwrap();
        for &step in steps {
            hats.steps[step] = MAX_LEVEL;
        }
        pattern
    }

    fn control(engine: &mut FakeEngine, pattern: Pattern) -> DrumControl {
        let source = engine
            .offline
            .handle
            .add_source(Box::new(ImpulseSource::default()), 1.0)
            .unwrap();
        DrumControl::new(Sequencer::new(source, pattern))
    }

    /// Every step still to be heard is on screen with the current output's
    /// time, and nothing else is (steps already heard have left the screen).
    fn assert_screen_matches(screen: &Screen, control: &DrumControl, engine: &FakeEngine) {
        let rendered = engine.offline.handle.frames_rendered();
        let want: BTreeMap<FramePos, f64> = control
            .sequencer()
            .scheduled()
            .filter(|step| step.frame >= rendered)
            .map(|step| (step.frame, engine.clock(step.frame)))
            .collect();
        let upcoming: BTreeMap<FramePos, f64> = screen
            .0
            .iter()
            .filter(|&(&frame, _)| frame >= rendered)
            .map(|(&frame, &at)| (frame, at))
            .collect();
        assert!(!want.is_empty());
        assert_eq!(upcoming, want);
    }

    fn change_output(engine: &mut FakeEngine, rate: u32, timed: bool) {
        engine.offline.reformat(StreamFormat::new(rate, 2));
        engine.timed = timed;
        engine.latency_ms = 12.0;
    }

    #[test]
    fn an_edit_right_after_an_output_change_replaces_every_old_heard_time() {
        let mut engine = FakeEngine::new(48_000);
        let mut control = control(&mut engine, groove(&[0, 2, 4, 6, 8, 10, 12, 14]));
        let mut screen = Screen::default();
        let mut outbox = Vec::new();
        control.set_playing(&mut engine, true, &mut outbox).unwrap();
        screen.apply(&mut outbox);
        engine.render(3_000);
        control.tick(&mut engine, &mut outbox).unwrap();
        screen.apply(&mut outbox);
        assert_screen_matches(&screen, &control, &engine);

        // New output, already timed; an edit arrives before the next tick.
        change_output(&mut engine, 44_100, true);
        control
            .set_pattern(&mut engine, groove(&[0, 4, 8, 12]), &mut outbox)
            .unwrap();
        assert!(outbox.contains(&StepEvent::Clear { from_frame: 0 }));
        screen.apply(&mut outbox);
        assert_eq!(control.sequencer().generation(), engine.generation());
        assert_screen_matches(&screen, &control, &engine);

        // The next tick has nothing stale left to fix.
        control.tick(&mut engine, &mut outbox).unwrap();
        assert!(!outbox.contains(&StepEvent::Clear { from_frame: 0 }));
        screen.apply(&mut outbox);
        assert_screen_matches(&screen, &control, &engine);
    }

    #[test]
    fn before_the_new_output_has_timing_nothing_old_stays_on_screen() {
        let mut engine = FakeEngine::new(48_000);
        let mut control = control(&mut engine, groove(&[0, 2, 4, 6, 8, 10, 12, 14]));
        let mut screen = Screen::default();
        let mut outbox = Vec::new();
        control.set_playing(&mut engine, true, &mut outbox).unwrap();
        screen.apply(&mut outbox);
        engine.render(3_000);
        control.tick(&mut engine, &mut outbox).unwrap();
        screen.apply(&mut outbox);

        // New output before its first callback: no timing yet.
        change_output(&mut engine, 44_100, false);
        control
            .set_pattern(&mut engine, groove(&[0, 1, 4, 8, 12]), &mut outbox)
            .unwrap();
        screen.apply(&mut outbox);
        assert!(
            screen.0.is_empty(),
            "old heard times withdrawn: {:?}",
            screen.0
        );

        // A tick still without timing announces nothing.
        control.tick(&mut engine, &mut outbox).unwrap();
        screen.apply(&mut outbox);
        assert!(screen.0.is_empty());

        // First callback: the next tick sends every scheduled step's new time.
        engine.render(64);
        engine.timed = true;
        control.tick(&mut engine, &mut outbox).unwrap();
        screen.apply(&mut outbox);
        assert_screen_matches(&screen, &control, &engine);
    }

    #[test]
    fn a_tick_alone_follows_an_output_change() {
        let mut engine = FakeEngine::new(48_000);
        let mut control = control(&mut engine, groove(&[0, 4, 8, 12]));
        let mut screen = Screen::default();
        let mut outbox = Vec::new();
        control.set_playing(&mut engine, true, &mut outbox).unwrap();
        screen.apply(&mut outbox);
        engine.render(10_000);
        change_output(&mut engine, 96_000, true);
        control.tick(&mut engine, &mut outbox).unwrap();
        screen.apply(&mut outbox);
        assert_eq!(control.sequencer().generation(), engine.generation());
        assert_screen_matches(&screen, &control, &engine);
    }
}

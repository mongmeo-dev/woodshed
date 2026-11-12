//! The single owner of the output stream.
//!
//! Every decision to open, replace or recover the output — the user picking a
//! device, the system default changing, a stream error, a device going away —
//! is made and carried out here, one input at a time, on one thread. Because
//! nothing else opens outputs, a decision can never act on stale state.
//!
//! The manager talks to the outside through [`OutputEnv`], so the state
//! machine is tested with a fake environment (see the tests below).

use std::time::{Duration, Instant};

use super::device::StreamErrorKind;
use super::monitor::DeviceSnapshot;

/// The output that is playing after a successful open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenedOutput {
    pub device_id: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_frames: Option<u32>,
    /// Engine generation started by this open.
    pub generation: u64,
}

/// What the manager needs from the world.
pub trait OutputEnv {
    /// Replaces the running output with `device` (`None` = system default).
    /// On error the running output must keep playing unchanged.
    fn open(&mut self, device: Option<&str>) -> Result<OpenedOutput, String>;
    /// The output playing now.
    fn current(&self) -> Option<OpenedOutput>;
    /// Pending backend error on the running stream.
    fn stream_error(&self) -> Option<StreamErrorKind>;
    /// The sample rate `device_id` runs at right now.
    fn device_rate(&self, device_id: &str) -> Option<u32>;
    /// Which devices exist right now.
    fn snapshot(&mut self) -> DeviceSnapshot;
    /// Persists the preferred device.
    fn save_preferred(&mut self, preferred: Option<&str>) -> Result<(), String>;
    /// Called exactly once per successful open, right where it succeeds.
    fn output_changed(&mut self, output: &OpenedOutput);
    /// Called when the device list differs from the previous tick.
    fn devices_changed(&mut self, snapshot: &DeviceSnapshot);
}

/// Spaces out retries: the wait doubles after each failure up to `max`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryBackoff {
    delay: Duration,
    next_attempt: Option<Instant>,
}

pub const RETRY_MIN: Duration = Duration::from_secs(1);
pub const RETRY_MAX: Duration = Duration::from_secs(30);

impl Default for RetryBackoff {
    fn default() -> Self {
        Self {
            delay: RETRY_MIN,
            next_attempt: None,
        }
    }
}

impl RetryBackoff {
    /// A backoff whose first attempt is due at `now + RETRY_MIN`.
    fn after_failure(now: Instant) -> Self {
        let mut backoff = Self::default();
        backoff.failed(now);
        backoff
    }

    pub fn ready(&self, now: Instant) -> bool {
        self.next_attempt.is_none_or(|at| now >= at)
    }

    pub fn failed(&mut self, now: Instant) {
        self.next_attempt = Some(now + self.delay);
        self.delay = (self.delay * 2).min(RETRY_MAX);
    }

    /// Time until the next attempt is allowed.
    pub fn delay(&self) -> Duration {
        self.delay
    }
}

/// Where the manager stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputMode {
    /// Nothing has been tried yet.
    Starting,
    /// Playing on the target: the preferred device, or the system default
    /// when no device is preferred.
    OnTarget,
    /// A device is preferred but cannot be used; playing on the system
    /// default. Returning to the preferred device is attempted only when
    /// `preferred_retry` allows it, so a healthy fallback is not disturbed on
    /// every tick.
    Fallback { preferred_retry: RetryBackoff },
    /// Nothing could be opened.
    Failed { retry: RetryBackoff },
}

/// Why a running output is unusable or stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputProblem {
    NoOutput,
    StreamError(StreamErrorKind),
    DeviceGone,
    /// It should follow the system default and the default moved.
    DefaultChanged,
    /// The device now runs at another rate than the stream.
    SampleRateChanged,
}

/// Checks the running output. `follow_default` is true when the output is
/// supposed to be the system default (no preferred device, or fallback).
pub fn output_problem(
    current: Option<&OpenedOutput>,
    stream_error: Option<StreamErrorKind>,
    device_rate_now: Option<u32>,
    snapshot: &DeviceSnapshot,
    follow_default: bool,
) -> Option<OutputProblem> {
    let Some(current) = current else {
        return Some(OutputProblem::NoOutput);
    };
    if let Some(error) = stream_error.filter(|error| error.needs_reopen()) {
        return Some(OutputProblem::StreamError(error));
    }
    if !snapshot.outputs.contains(&current.device_id) {
        return Some(OutputProblem::DeviceGone);
    }
    if follow_default
        && snapshot
            .default_output
            .as_deref()
            .is_some_and(|default| default != current.device_id)
    {
        return Some(OutputProblem::DefaultChanged);
    }
    match device_rate_now {
        Some(rate) if rate != current.sample_rate => Some(OutputProblem::SampleRateChanged),
        _ => None,
    }
}

/// Result of a user's device choice.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SelectError {
    /// The chosen device could not be opened. The choice is kept (and
    /// retried with backoff); the system default plays meanwhile, if it can.
    #[error("cannot open the chosen output: {0}")]
    OpenFailed(String),
    /// The output switched, but the choice could not be saved to disk. The
    /// running preference is the new choice until the app quits.
    #[error("cannot save the output choice: {0}")]
    SaveFailed(String),
}

/// One unit of work for the manager, processed strictly in arrival order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputInput {
    Tick,
    Select(Option<String>),
}

pub struct OutputManager<E: OutputEnv> {
    env: E,
    preferred: Option<String>,
    mode: OutputMode,
    last_snapshot: Option<DeviceSnapshot>,
}

impl<E: OutputEnv> OutputManager<E> {
    pub fn new(env: E, preferred: Option<String>) -> Self {
        Self {
            env,
            preferred,
            mode: OutputMode::Starting,
            last_snapshot: None,
        }
    }

    pub fn mode(&self) -> &OutputMode {
        &self.mode
    }

    /// The preference in effect (it may be newer than what is on disk).
    pub fn preferred(&self) -> Option<&str> {
        self.preferred.as_deref()
    }

    pub fn env(&self) -> &E {
        &self.env
    }

    pub fn env_mut(&mut self) -> &mut E {
        &mut self.env
    }

    /// Processes one input. `Select` returns the outcome for its caller.
    pub fn handle(&mut self, input: OutputInput, now: Instant) -> Option<Result<(), SelectError>> {
        match input {
            OutputInput::Tick => {
                self.tick(now);
                None
            }
            OutputInput::Select(device) => Some(self.select(device, now)),
        }
    }

    /// Opens through the environment and announces every success.
    fn open(&mut self, device: Option<&str>) -> Result<OpenedOutput, String> {
        let opened = self.env.open(device);
        match &opened {
            Ok(output) => self.env.output_changed(output),
            Err(error) => log::warn!(
                "audio output: cannot open {}: {error}",
                device.unwrap_or("the system default")
            ),
        }
        opened
    }

    fn healthy_default(&self, snapshot: &DeviceSnapshot) -> bool {
        let current = self.env.current();
        let rate = current
            .as_ref()
            .and_then(|output| self.env.device_rate(&output.device_id));
        output_problem(
            current.as_ref(),
            self.env.stream_error(),
            rate,
            snapshot,
            true,
        )
        .is_none()
    }

    /// Opens the target from scratch. `retry` carries an existing backoff
    /// forward when this is itself a retry (so failures keep doubling).
    /// Returns whether the target (not a fallback) is playing.
    fn settle(
        &mut self,
        now: Instant,
        snapshot: &DeviceSnapshot,
        retry: Option<RetryBackoff>,
    ) -> bool {
        let Some(preferred) = self.preferred.clone() else {
            match self.open(None) {
                Ok(_) => self.mode = OutputMode::OnTarget,
                Err(_) => self.fail(now, retry),
            }
            return matches!(self.mode, OutputMode::OnTarget);
        };
        if self.open(Some(&preferred)).is_ok() {
            self.mode = OutputMode::OnTarget;
            return true;
        }
        let preferred_retry = match retry {
            Some(mut backoff) => {
                backoff.failed(now);
                backoff
            }
            None => RetryBackoff::after_failure(now),
        };
        self.fall_back(snapshot, preferred_retry);
        false
    }

    /// Plays on the system default while waiting to return to the preferred
    /// device. Keeps the running output if it already is a healthy default.
    fn fall_back(&mut self, snapshot: &DeviceSnapshot, preferred_retry: RetryBackoff) {
        if self.healthy_default(snapshot) || self.open(None).is_ok() {
            self.mode = OutputMode::Fallback { preferred_retry };
        } else {
            self.mode = OutputMode::Failed {
                retry: preferred_retry,
            };
        }
    }

    fn fail(&mut self, now: Instant, retry: Option<RetryBackoff>) {
        let retry = match retry {
            Some(mut backoff) => {
                backoff.failed(now);
                backoff
            }
            None => RetryBackoff::after_failure(now),
        };
        self.mode = OutputMode::Failed { retry };
    }

    /// Periodic check: forwards device list changes and keeps the output on
    /// target, within the retry limits.
    pub fn tick(&mut self, now: Instant) {
        let snapshot = self.env.snapshot();
        let devices_changed = self.last_snapshot.as_ref() != Some(&snapshot);
        if devices_changed && self.last_snapshot.is_some() {
            self.env.devices_changed(&snapshot);
        }
        self.last_snapshot = Some(snapshot.clone());

        match self.mode.clone() {
            OutputMode::Starting => {
                self.settle(now, &snapshot, None);
            }
            OutputMode::OnTarget => {
                let current = self.env.current();
                let rate = current
                    .as_ref()
                    .and_then(|output| self.env.device_rate(&output.device_id));
                let problem = output_problem(
                    current.as_ref(),
                    self.env.stream_error(),
                    rate,
                    &snapshot,
                    self.preferred.is_none(),
                );
                if let Some(problem) = problem {
                    log::info!("audio output: {problem:?}, reopening");
                    self.settle(now, &snapshot, None);
                }
            }
            OutputMode::Fallback {
                mut preferred_retry,
            } => {
                // A changed device list is new information: retry right away.
                if devices_changed {
                    preferred_retry = RetryBackoff::default();
                }
                let preferred = self.preferred.clone().unwrap_or_default();
                if snapshot.outputs.contains(&preferred) && preferred_retry.ready(now) {
                    if self.open(Some(&preferred)).is_ok() {
                        self.mode = OutputMode::OnTarget;
                        return;
                    }
                    preferred_retry.failed(now);
                }
                self.fall_back(&snapshot, preferred_retry);
            }
            OutputMode::Failed { mut retry } => {
                if devices_changed {
                    retry = RetryBackoff::default();
                }
                if retry.ready(now) {
                    self.settle(now, &snapshot, Some(retry));
                } else {
                    self.mode = OutputMode::Failed { retry };
                }
            }
        }
    }

    /// The user picked a device (`None` = system default). The new choice is
    /// in effect immediately, even if it cannot be opened or saved.
    pub fn select(&mut self, device: Option<String>, now: Instant) -> Result<(), SelectError> {
        self.preferred = device;
        let snapshot = self.env.snapshot();
        self.last_snapshot = Some(snapshot.clone());
        let on_target = self.settle(now, &snapshot, None);
        let saved = self.env.save_preferred(self.preferred.as_deref());
        if !on_target {
            return Err(SelectError::OpenFailed(
                self.preferred
                    .clone()
                    .unwrap_or_else(|| "system default".into()),
            ));
        }
        saved.map_err(SelectError::SaveFailed)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;

    const A: &str = "coreaudio:A";
    const B: &str = "coreaudio:B";
    const C: &str = "coreaudio:C";

    /// A scripted world: which devices are listed, which can open, which is
    /// the default; records every open and announcement.
    #[derive(Default)]
    struct FakeEnv {
        listed: Vec<String>,
        default: Option<String>,
        openable: HashSet<String>,
        rates: HashMap<String, u32>,
        current: Option<OpenedOutput>,
        error: Option<StreamErrorKind>,
        generation: u64,
        save_fails: bool,
        open_attempts: Vec<String>,
        announced: Vec<OpenedOutput>,
        saved: Vec<Option<String>>,
        device_events: usize,
    }

    impl FakeEnv {
        fn new(listed: &[&str], default: &str, openable: &[&str]) -> Self {
            Self {
                listed: listed.iter().map(|s| (*s).to_owned()).collect(),
                default: Some(default.to_owned()),
                openable: openable.iter().map(|s| (*s).to_owned()).collect(),
                ..Self::default()
            }
        }

        fn current_id(&self) -> Option<&str> {
            self.current
                .as_ref()
                .map(|output| output.device_id.as_str())
        }

        fn attempts_on(&self, id: &str) -> usize {
            self.open_attempts.iter().filter(|a| *a == id).count()
        }
    }

    impl OutputEnv for FakeEnv {
        fn open(&mut self, device: Option<&str>) -> Result<OpenedOutput, String> {
            let id = device
                .map(str::to_owned)
                .or_else(|| self.default.clone())
                .ok_or("no default")?;
            self.open_attempts.push(id.clone());
            if !self.listed.contains(&id) || !self.openable.contains(&id) {
                return Err(format!("{id} failed"));
            }
            self.generation += 1;
            let output = OpenedOutput {
                sample_rate: *self.rates.get(&id).unwrap_or(&48_000),
                device_id: id,
                channels: 2,
                buffer_frames: Some(64),
                generation: self.generation,
            };
            self.current = Some(output.clone());
            self.error = None;
            Ok(output)
        }

        fn current(&self) -> Option<OpenedOutput> {
            self.current.clone()
        }

        fn stream_error(&self) -> Option<StreamErrorKind> {
            self.error
        }

        fn device_rate(&self, device_id: &str) -> Option<u32> {
            Some(*self.rates.get(device_id).unwrap_or(&48_000))
        }

        fn snapshot(&mut self) -> DeviceSnapshot {
            DeviceSnapshot {
                outputs: self.listed.clone(),
                default_output: self.default.clone(),
                ..DeviceSnapshot::default()
            }
        }

        fn save_preferred(&mut self, preferred: Option<&str>) -> Result<(), String> {
            if self.save_fails {
                return Err("disk full".into());
            }
            self.saved.push(preferred.map(str::to_owned));
            Ok(())
        }

        fn output_changed(&mut self, output: &OpenedOutput) {
            self.announced.push(output.clone());
        }

        fn devices_changed(&mut self, _: &DeviceSnapshot) {
            self.device_events += 1;
        }
    }

    fn secs(start: Instant, s: u64) -> Instant {
        start + Duration::from_secs(s)
    }

    /// Runs one tick per second for `seconds` seconds starting at `from`.
    fn run_ticks(manager: &mut OutputManager<FakeEnv>, start: Instant, from: u64, seconds: u64) {
        for s in from..from + seconds {
            manager.handle(OutputInput::Tick, secs(start, s));
        }
    }

    #[test]
    fn preferred_device_plays_when_it_opens() {
        let env = FakeEnv::new(&[A, B], B, &[A, B]);
        let mut manager = OutputManager::new(env, Some(A.into()));
        manager.tick(Instant::now());
        assert_eq!(manager.mode(), &OutputMode::OnTarget);
        assert_eq!(manager.env().current_id(), Some(A));
        assert_eq!(manager.env().announced.len(), 1);
    }

    #[test]
    fn unopenable_preferred_falls_back_without_reopening_the_fallback_every_tick() {
        // A is listed but always fails; the default B is healthy.
        let start = Instant::now();
        let env = FakeEnv::new(&[A, B], B, &[B]);
        let mut manager = OutputManager::new(env, Some(A.into()));
        run_ticks(&mut manager, start, 0, 64);

        let env = manager.env();
        assert_eq!(env.current_id(), Some(B));
        assert_eq!(env.attempts_on(B), 1, "B opened once and left alone");
        assert_eq!(env.announced.len(), 1, "one generation, one announcement");
        // Retries on A at t = 0 (start), then 1, 3, 7, 15, 31, 61 s (1→30 s doubling).
        assert_eq!(env.attempts_on(A), 7);
        match manager.mode() {
            OutputMode::Fallback { preferred_retry } => {
                assert_eq!(preferred_retry.delay(), RETRY_MAX);
            }
            other => panic!("expected fallback, got {other:?}"),
        }
    }

    #[test]
    fn preferred_device_is_resumed_when_it_starts_working() {
        let start = Instant::now();
        let env = FakeEnv::new(&[A, B], B, &[B]);
        let mut manager = OutputManager::new(env, Some(A.into()));
        run_ticks(&mut manager, start, 0, 10);
        manager.env_mut().openable.insert(A.into());
        run_ticks(&mut manager, start, 10, 10); // next retry at t = 15
        assert_eq!(manager.mode(), &OutputMode::OnTarget);
        assert_eq!(manager.env().current_id(), Some(A));
        assert_eq!(manager.env().announced.last().unwrap().device_id, A);
    }

    #[test]
    fn a_device_list_change_resets_the_preferred_retry() {
        let start = Instant::now();
        let env = FakeEnv::new(&[A, B], B, &[B]);
        let mut manager = OutputManager::new(env, Some(A.into()));
        run_ticks(&mut manager, start, 0, 20); // backoff is now 16 s, next at 31
        let before = manager.env().attempts_on(A);
        manager.env_mut().listed.push(C.into());
        manager.handle(OutputInput::Tick, secs(start, 20));
        assert_eq!(manager.env().attempts_on(A), before + 1, "retried at once");
        assert_eq!(manager.env().device_events, 1);
    }

    #[test]
    fn unplugged_preferred_falls_back_and_returns_when_plugged_back() {
        let start = Instant::now();
        let env = FakeEnv::new(&[A, B], B, &[A, B]);
        let mut manager = OutputManager::new(env, Some(A.into()));
        manager.handle(OutputInput::Tick, start);
        manager.env_mut().listed.retain(|id| id != A);
        manager.handle(OutputInput::Tick, secs(start, 1));
        assert_eq!(manager.env().current_id(), Some(B));
        assert!(matches!(manager.mode(), OutputMode::Fallback { .. }));

        manager.env_mut().listed.push(A.into());
        manager.handle(OutputInput::Tick, secs(start, 2));
        assert_eq!(manager.env().current_id(), Some(A));
        assert_eq!(manager.mode(), &OutputMode::OnTarget);
    }

    #[test]
    fn system_default_mode_follows_the_default_while_both_stay_connected() {
        let start = Instant::now();
        let env = FakeEnv::new(&[A, B], A, &[A, B]);
        let mut manager = OutputManager::new(env, None);
        manager.handle(OutputInput::Tick, start);
        assert_eq!(manager.env().current_id(), Some(A));
        manager.env_mut().default = Some(B.into());
        manager.handle(OutputInput::Tick, secs(start, 1));
        assert_eq!(manager.env().current_id(), Some(B));
        // A chosen device ignores default changes.
        manager.select(Some(A.into()), secs(start, 2)).unwrap();
        manager.env_mut().default = Some(C.into());
        manager.env_mut().listed.push(C.into());
        manager.env_mut().openable.insert(C.into());
        run_ticks(&mut manager, start, 3, 5);
        assert_eq!(manager.env().current_id(), Some(A));
    }

    #[test]
    fn stream_error_on_an_unchanged_device_list_reopens() {
        let start = Instant::now();
        let env = FakeEnv::new(&[A], A, &[A]);
        let mut manager = OutputManager::new(env, None);
        manager.handle(OutputInput::Tick, start);
        manager.env_mut().error = Some(StreamErrorKind::StreamInvalidated);
        manager.handle(OutputInput::Tick, secs(start, 1));
        assert_eq!(manager.env().attempts_on(A), 2);
        assert_eq!(manager.env().announced.len(), 2);
        // An xrun is not a reason to reopen.
        manager.env_mut().error = Some(StreamErrorKind::Xrun);
        manager.handle(OutputInput::Tick, secs(start, 2));
        assert_eq!(manager.env().attempts_on(A), 2);
    }

    #[test]
    fn sample_rate_change_on_the_same_device_reopens() {
        let start = Instant::now();
        let env = FakeEnv::new(&[A], A, &[A]);
        let mut manager = OutputManager::new(env, None);
        manager.handle(OutputInput::Tick, start);
        manager.env_mut().rates.insert(A.into(), 44_100);
        manager.handle(OutputInput::Tick, secs(start, 1));
        let last = manager.env().announced.last().unwrap();
        assert_eq!(last.sample_rate, 44_100);
        assert_eq!(last.generation, 2);
    }

    #[test]
    fn nothing_opens_retries_with_growing_backoff_then_recovers() {
        let start = Instant::now();
        let env = FakeEnv::new(&[A], A, &[]);
        let mut manager = OutputManager::new(env, None);
        run_ticks(&mut manager, start, 0, 32);
        // t = 0, 1, 3, 7, 15, 31
        assert_eq!(manager.env().attempts_on(A), 6);
        assert!(matches!(manager.mode(), OutputMode::Failed { .. }));
        manager.env_mut().openable.insert(A.into());
        run_ticks(&mut manager, start, 32, 32); // next at 63
        assert_eq!(manager.mode(), &OutputMode::OnTarget);
    }

    #[test]
    fn save_failure_keeps_the_new_output_and_preference_and_still_announces() {
        let start = Instant::now();
        let env = FakeEnv::new(&[A, C], A, &[A, C]);
        let mut manager = OutputManager::new(env, None);
        manager.handle(OutputInput::Tick, start);
        manager.env_mut().save_fails = true;

        let result = manager.handle(OutputInput::Select(Some(C.into())), secs(start, 1));
        assert!(matches!(result, Some(Err(SelectError::SaveFailed(_)))));
        assert_eq!(manager.env().current_id(), Some(C));
        assert_eq!(manager.preferred(), Some(C));
        let last = manager.env().announced.last().unwrap();
        assert_eq!((last.device_id.as_str(), last.generation), (C, 2));
        // Later ticks keep C (the running preference), not the old one.
        run_ticks(&mut manager, start, 2, 5);
        assert_eq!(manager.env().current_id(), Some(C));
    }

    #[test]
    fn choosing_an_unopenable_device_keeps_the_choice_and_plays_the_default() {
        let start = Instant::now();
        let env = FakeEnv::new(&[A, C], A, &[A]);
        let mut manager = OutputManager::new(env, None);
        manager.handle(OutputInput::Tick, start);
        let result = manager.select(Some(C.into()), secs(start, 1));
        assert!(matches!(result, Err(SelectError::OpenFailed(_))));
        assert_eq!(manager.preferred(), Some(C));
        assert_eq!(
            manager.env().current_id(),
            Some(A),
            "the default keeps playing"
        );
        assert_eq!(manager.env().saved, vec![Some(C.to_owned())]);
        assert!(matches!(manager.mode(), OutputMode::Fallback { .. }));
    }

    #[test]
    fn a_manual_choice_queued_before_a_tick_is_never_undone_by_it() {
        // Fallback on B while preferred A is back; the user picks C at the same
        // moment. Inputs are handled in queue order against the latest state.
        let start = Instant::now();
        let env = FakeEnv::new(&[B, C], B, &[A, B, C]);
        let mut manager = OutputManager::new(env, Some(A.into()));
        manager.handle(OutputInput::Tick, start);
        assert!(matches!(manager.mode(), OutputMode::Fallback { .. }));
        manager.env_mut().listed.push(A.into());

        for input in [OutputInput::Select(Some(C.into())), OutputInput::Tick] {
            manager.handle(input, secs(start, 1));
        }
        assert_eq!(manager.env().current_id(), Some(C));
        assert_eq!(manager.preferred(), Some(C));
        assert_eq!(manager.env().attempts_on(A), 1, "A only tried at start");
    }

    #[test]
    fn a_tick_queued_before_a_manual_choice_runs_first_then_the_choice_wins() {
        let start = Instant::now();
        let env = FakeEnv::new(&[B, C], B, &[A, B, C]);
        let mut manager = OutputManager::new(env, Some(A.into()));
        manager.handle(OutputInput::Tick, start);
        manager.env_mut().listed.push(A.into());

        let order: Vec<String> = [OutputInput::Tick, OutputInput::Select(Some(C.into()))]
            .into_iter()
            .map(|input| {
                manager.handle(input, secs(start, 1));
                manager.env().current_id().unwrap().to_owned()
            })
            .collect();
        assert_eq!(order, vec![A.to_owned(), C.to_owned()]);
        run_ticks(&mut manager, start, 2, 5);
        assert_eq!(manager.env().current_id(), Some(C));
    }

    #[test]
    fn every_successful_open_is_announced_once_with_its_generation() {
        let start = Instant::now();
        let env = FakeEnv::new(&[A, B], B, &[B]);
        let mut manager = OutputManager::new(env, Some(A.into()));
        run_ticks(&mut manager, start, 0, 10);
        manager.select(None, secs(start, 10)).unwrap();
        manager.env_mut().error = Some(StreamErrorKind::DeviceChanged);
        manager.handle(OutputInput::Tick, secs(start, 11));
        let generations: Vec<u64> = manager
            .env()
            .announced
            .iter()
            .map(|o| o.generation)
            .collect();
        let expected: Vec<u64> = (1..=manager.env().generation).collect();
        assert_eq!(generations, expected);
    }
}

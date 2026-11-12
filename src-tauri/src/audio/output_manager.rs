//! The single owner of the output stream.
//!
//! Every decision to open, replace or recover the output — the user picking a
//! device, the system default changing, a stream error, a device going away —
//! is made and carried out here, one input at a time, on one thread. Because
//! nothing else opens outputs, a decision can never act on stale state.
//!
//! The manager talks to the outside through [`OutputEnv`], so the state
//! machine is tested with a fake environment (see the tests below).

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
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
    /// Whether the user has answered the system's microphone prompt, either
    /// way. Until then macOS holds the creation of an output stream until the
    /// answer whenever the device — or the system default output, which the
    /// backend connects first — has inputs.
    fn microphone_answered(&mut self) -> bool;
    /// The manager starts (`true`) or stops (`false`) holding opens back for
    /// the microphone answer. On `true` the environment shows the prompt; the
    /// answer should wake the manager with [`ManagerRequest::Recheck`].
    fn waiting_for_microphone(&mut self, waiting: bool);
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
    /// The request's deadline had passed before the manager could start it;
    /// nothing was changed.
    #[error("the output request expired before it started")]
    Expired,
}

/// State right after a request was handled, so callers need not query the
/// engine again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectOutcome {
    pub output: Option<OpenedOutput>,
    pub preferred: Option<String>,
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
    /// Opens are held back until the microphone prompt is answered.
    microphone_wait: bool,
}

impl<E: OutputEnv> OutputManager<E> {
    pub fn new(env: E, preferred: Option<String>) -> Self {
        Self {
            env,
            preferred,
            mode: OutputMode::Starting,
            last_snapshot: None,
            microphone_wait: false,
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

    /// True while the target cannot be opened before the user answers the
    /// microphone prompt.
    pub fn waiting_for_microphone(&self) -> bool {
        self.microphone_wait
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

    /// Whether opening `device` now would make macOS hold the stream until
    /// the microphone prompt is answered: the device or the system default
    /// output has inputs, and the prompt has not been answered.
    fn held_for_microphone(&mut self, device: Option<&str>) -> bool {
        let Some(snapshot) = self.last_snapshot.as_ref() else {
            return false;
        };
        let has_inputs = |id: &str| snapshot.inputs.iter().any(|input| input == id);
        let affected = snapshot.default_output.as_deref().is_some_and(has_inputs)
            || device.is_some_and(has_inputs);
        affected && !self.env.microphone_answered()
    }

    /// Starts or stops waiting for the microphone answer (the environment
    /// shows the prompt when the wait starts).
    fn set_microphone_wait(&mut self, waiting: bool) {
        if self.microphone_wait == waiting {
            return;
        }
        self.microphone_wait = waiting;
        if waiting {
            log::info!("audio output: waiting for the microphone answer before opening");
        }
        self.env.waiting_for_microphone(waiting);
    }

    /// Updates the wait after an input: it lasts while the target is not
    /// playing and opening it would be held back.
    fn update_microphone_wait(&mut self) {
        let preferred = self.preferred.clone();
        let waiting =
            self.mode != OutputMode::OnTarget && self.held_for_microphone(preferred.as_deref());
        self.set_microphone_wait(waiting);
    }

    /// Opens through the environment and announces every success. An open
    /// that macOS would hold for the microphone answer is not started: it
    /// fails at once, so this thread never waits inside the backend.
    fn open(&mut self, device: Option<&str>) -> Result<OpenedOutput, String> {
        if self.held_for_microphone(device) {
            return Err("waiting for the microphone answer".into());
        }
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

    /// Reads the device list, announces it if it differs from the last one
    /// seen, and makes it the new reference. Every input goes through here,
    /// so no change is swallowed by whichever input happens to see it first.
    fn observe_devices(&mut self) -> (DeviceSnapshot, bool) {
        let snapshot = self.env.snapshot();
        let changed = self.last_snapshot.as_ref() != Some(&snapshot);
        if changed && self.last_snapshot.is_some() {
            self.env.devices_changed(&snapshot);
        }
        self.last_snapshot = Some(snapshot.clone());
        (snapshot, changed)
    }

    /// The output and preference in effect now.
    pub fn outcome(&self) -> SelectOutcome {
        SelectOutcome {
            output: self.env.current(),
            preferred: self.preferred.clone(),
        }
    }

    /// Periodic check: forwards device list changes and keeps the output on
    /// target, within the retry limits.
    pub fn tick(&mut self, now: Instant) {
        let (snapshot, devices_changed) = self.observe_devices();
        // The prompt was answered: open the target now, whatever retry
        // backoff the held opens left behind.
        if self.microphone_wait && self.env.microphone_answered() {
            self.mode = OutputMode::Starting;
        }
        self.step(now, &snapshot, devices_changed);
        self.update_microphone_wait();
    }

    fn step(&mut self, now: Instant, snapshot: &DeviceSnapshot, devices_changed: bool) {
        match self.mode.clone() {
            OutputMode::Starting => {
                self.settle(now, snapshot, None);
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
                    snapshot,
                    self.preferred.is_none(),
                );
                if let Some(problem) = problem {
                    log::info!("audio output: {problem:?}, reopening");
                    self.settle(now, snapshot, None);
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
                self.fall_back(snapshot, preferred_retry);
            }
            OutputMode::Failed { mut retry } => {
                if devices_changed {
                    retry = RetryBackoff::default();
                }
                if retry.ready(now) {
                    self.settle(now, snapshot, Some(retry));
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
        let (snapshot, _) = self.observe_devices();
        let on_target = self.settle(now, &snapshot, None);
        self.update_microphone_wait();
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

/// Messages for the output manager thread.
pub enum ManagerRequest {
    /// Switch to `device_id` (`None` = system default) unless `deadline` has
    /// already passed when the manager gets to it.
    Select {
        device_id: Option<String>,
        deadline: Instant,
        reply: Sender<Result<SelectOutcome, SelectError>>,
    },
    /// Check again now, without waiting for the next tick (the microphone
    /// prompt was answered).
    Recheck,
    /// Stop the loop.
    Shutdown,
}

fn publish<E: OutputEnv>(manager: &OutputManager<E>, preferred: &Mutex<Option<String>>) {
    *preferred.lock().unwrap_or_else(PoisonError::into_inner) =
        manager.preferred().map(str::to_owned);
}

/// Handles one request; returns false on shutdown.
fn serve<E: OutputEnv>(manager: &mut OutputManager<E>, request: ManagerRequest) -> bool {
    match request {
        ManagerRequest::Shutdown => false,
        ManagerRequest::Recheck => {
            manager.tick(Instant::now());
            true
        }
        ManagerRequest::Select {
            device_id,
            deadline,
            reply,
        } => {
            let now = Instant::now();
            let result = if now > deadline {
                Err(SelectError::Expired)
            } else {
                manager.select(device_id, now).map(|()| manager.outcome())
            };
            // The caller may have stopped waiting; the result is applied and
            // announced (output-changed, published preference) either way.
            let _ = reply.send(result);
            true
        }
    }
}

/// The output manager thread's loop. Requests already waiting are handled
/// before the next tick, and the next tick is due `poll` after the previous
/// one finished, so a slow tick cannot starve requests. The preference in
/// effect is published to `preferred` after every input. Returns the manager
/// on `Shutdown` or when every sender is gone.
pub fn run_output_manager<E: OutputEnv>(
    mut manager: OutputManager<E>,
    inbox: Receiver<ManagerRequest>,
    poll: Duration,
    preferred: Arc<Mutex<Option<String>>>,
) -> OutputManager<E> {
    publish(&manager, &preferred);
    let mut next_tick = Instant::now();
    loop {
        loop {
            match inbox.try_recv() {
                Ok(request) => {
                    if !serve(&mut manager, request) {
                        return manager;
                    }
                    publish(&manager, &preferred);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return manager,
            }
        }
        let now = Instant::now();
        if now >= next_tick {
            manager.tick(now);
            publish(&manager, &preferred);
            next_tick = Instant::now() + poll;
            continue;
        }
        match inbox.recv_timeout(next_tick - now) {
            Ok(request) => {
                if !serve(&mut manager, request) {
                    return manager;
                }
                publish(&manager, &preferred);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return manager,
        }
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
        inputs: Vec<String>,
        snapshots_started: Arc<std::sync::atomic::AtomicUsize>,
        snapshot_delay: Duration,
        open_delay: Duration,
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
        /// The microphone prompt is showing (not answered yet).
        prompt_pending: Arc<std::sync::atomic::AtomicBool>,
        /// Every start (`true`) and end (`false`) of the microphone wait.
        microphone_waits: Vec<bool>,
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
            std::thread::sleep(self.open_delay);
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
            self.snapshots_started
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(self.snapshot_delay);
            DeviceSnapshot {
                outputs: self.listed.clone(),
                inputs: self.inputs.clone(),
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

        fn microphone_answered(&mut self) -> bool {
            !self
                .prompt_pending
                .load(std::sync::atomic::Ordering::SeqCst)
        }

        fn waiting_for_microphone(&mut self, waiting: bool) {
            self.microphone_waits.push(waiting);
        }
    }

    /// `A` has inputs (an audio interface) and is the default; `B` and `C`
    /// are output-only. The microphone prompt has not been answered.
    fn prompt_pending_env(default: &str) -> FakeEnv {
        let mut env = FakeEnv::new(&[A, B, C], default, &[A, B, C]);
        env.inputs = vec![A.into()];
        env.prompt_pending
            .store(true, std::sync::atomic::Ordering::SeqCst);
        env
    }

    fn answer_prompt(manager: &mut OutputManager<FakeEnv>) {
        manager
            .env_mut()
            .prompt_pending
            .store(false, std::sync::atomic::Ordering::SeqCst);
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

    #[test]
    fn an_input_change_seen_first_by_a_selection_is_still_announced_once() {
        let start = Instant::now();
        let env = FakeEnv::new(&[A, C], A, &[A, C]);
        let mut manager = OutputManager::new(env, None);
        manager.handle(OutputInput::Tick, start);
        manager.env_mut().inputs.push("coreaudio:mic".into());
        manager.handle(OutputInput::Select(Some(C.into())), secs(start, 1));
        manager.handle(OutputInput::Tick, secs(start, 2));
        assert_eq!(manager.env().device_events, 1);
    }

    #[test]
    fn a_default_output_with_inputs_waits_for_the_microphone_answer_then_opens_at_once() {
        let start = Instant::now();
        let mut manager = OutputManager::new(prompt_pending_env(A), None);
        run_ticks(&mut manager, start, 0, 40);
        assert!(
            manager.env().open_attempts.is_empty(),
            "nothing may reach the backend while macOS would hold it"
        );
        assert!(manager.waiting_for_microphone());
        assert_eq!(manager.env().microphone_waits, vec![true], "asked once");

        // Answered after 40 s of held retries: open on the next check, not
        // after the grown backoff.
        answer_prompt(&mut manager);
        manager.handle(OutputInput::Tick, secs(start, 40));
        assert_eq!(manager.mode(), &OutputMode::OnTarget);
        assert_eq!(manager.env().current_id(), Some(A));
        assert_eq!(manager.env().announced.len(), 1);
        assert!(!manager.waiting_for_microphone());
        assert_eq!(manager.env().microphone_waits, vec![true, false]);
    }

    #[test]
    fn a_default_output_with_inputs_holds_back_an_output_only_choice_too() {
        // The backend connects the system default before the chosen device,
        // so an output-only preference waits as well.
        let start = Instant::now();
        let mut manager = OutputManager::new(prompt_pending_env(A), Some(B.into()));
        run_ticks(&mut manager, start, 0, 5);
        assert!(manager.env().open_attempts.is_empty());
        assert!(manager.waiting_for_microphone());

        answer_prompt(&mut manager);
        manager.handle(OutputInput::Tick, secs(start, 5));
        assert_eq!(manager.mode(), &OutputMode::OnTarget);
        assert_eq!(manager.env().current_id(), Some(B));
    }

    #[test]
    fn output_only_devices_open_while_the_prompt_is_pending() {
        let start = Instant::now();
        let mut manager = OutputManager::new(prompt_pending_env(B), None);
        run_ticks(&mut manager, start, 0, 3);
        assert_eq!(manager.mode(), &OutputMode::OnTarget);
        assert_eq!(manager.env().current_id(), Some(B));
        assert!(!manager.waiting_for_microphone());
        assert!(
            manager.env().microphone_waits.is_empty(),
            "no prompt for playback alone"
        );
    }

    #[test]
    fn choosing_a_device_with_inputs_keeps_the_output_only_device_playing_until_the_answer() {
        let start = Instant::now();
        let mut manager = OutputManager::new(prompt_pending_env(B), None);
        manager.handle(OutputInput::Tick, start);
        assert_eq!(manager.env().current_id(), Some(B));

        let result = manager.handle(OutputInput::Select(Some(A.into())), secs(start, 1));
        assert_eq!(result, Some(Err(SelectError::OpenFailed(A.into()))));
        assert_eq!(manager.preferred(), Some(A), "the choice is kept");
        assert_eq!(manager.env().attempts_on(A), 0);
        assert_eq!(manager.env().current_id(), Some(B), "B keeps playing");
        assert!(manager.waiting_for_microphone());
        run_ticks(&mut manager, start, 2, 10);
        assert_eq!(manager.env().attempts_on(A), 0);

        answer_prompt(&mut manager);
        manager.handle(OutputInput::Tick, secs(start, 12));
        assert_eq!(manager.mode(), &OutputMode::OnTarget);
        assert_eq!(manager.env().current_id(), Some(A));
        assert!(!manager.waiting_for_microphone());
    }

    #[test]
    fn an_answered_prompt_never_holds_anything_back() {
        let env = prompt_pending_env(A);
        env.prompt_pending
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let mut manager = OutputManager::new(env, None);
        manager.tick(Instant::now());
        assert_eq!(manager.env().current_id(), Some(A));
        assert!(manager.env().microphone_waits.is_empty());
    }

    type LoopHandles = (
        Sender<ManagerRequest>,
        Arc<Mutex<Option<String>>>,
        std::thread::JoinHandle<OutputManager<FakeEnv>>,
    );

    fn spawn_loop(manager: OutputManager<FakeEnv>, poll: Duration) -> LoopHandles {
        let (requests, inbox) = std::sync::mpsc::channel();
        let preferred = Arc::new(Mutex::new(None));
        let published = Arc::clone(&preferred);
        let thread =
            std::thread::spawn(move || run_output_manager(manager, inbox, poll, published));
        (requests, preferred, thread)
    }

    fn select_request(
        device: &str,
        deadline: Instant,
    ) -> (
        ManagerRequest,
        std::sync::mpsc::Receiver<Result<SelectOutcome, SelectError>>,
    ) {
        let (reply, response) = std::sync::mpsc::channel();
        (
            ManagerRequest::Select {
                device_id: Some(device.to_owned()),
                deadline,
                reply,
            },
            response,
        )
    }

    #[test]
    fn a_request_that_expires_behind_a_slow_tick_is_dropped_unexecuted() {
        let mut env = FakeEnv::new(&[A, C], A, &[A, C]);
        env.snapshot_delay = Duration::from_millis(300);
        let tick_started = Arc::clone(&env.snapshots_started);
        let (requests, preferred, thread) =
            spawn_loop(OutputManager::new(env, None), Duration::from_secs(60));
        // Wait until the first tick is inside its slow device scan.
        while tick_started.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            std::thread::yield_now();
        }
        let (request, response) = select_request(C, Instant::now() + Duration::from_millis(50));
        requests.send(request).unwrap();
        requests.send(ManagerRequest::Shutdown).unwrap();
        let manager = thread.join().unwrap();

        assert_eq!(response.recv().unwrap(), Err(SelectError::Expired));
        assert_eq!(manager.env().current_id(), Some(A));
        assert_eq!(manager.env().attempts_on(C), 0);
        assert_eq!(manager.env().saved, Vec::<Option<String>>::new());
        assert_eq!(*preferred.lock().unwrap(), None);
    }

    #[test]
    fn a_request_whose_deadline_passes_mid_switch_completes_and_is_published() {
        let mut env = FakeEnv::new(&[A, C], A, &[A, C]);
        env.open_delay = Duration::from_millis(150);
        let manager = OutputManager::new(env, None);
        let (request, response) = select_request(C, Instant::now() + Duration::from_millis(50));
        // Queued before the loop starts: handled before the first tick.
        let (requests, inbox) = std::sync::mpsc::channel();
        requests.send(request).unwrap();
        requests.send(ManagerRequest::Shutdown).unwrap();
        let preferred = Arc::new(Mutex::new(None));
        let manager = run_output_manager(
            manager,
            inbox,
            Duration::from_secs(60),
            Arc::clone(&preferred),
        );

        let outcome = response.recv().unwrap().unwrap();
        assert_eq!(outcome.output.unwrap().device_id, C);
        assert_eq!(outcome.preferred.as_deref(), Some(C));
        assert_eq!(preferred.lock().unwrap().as_deref(), Some(C));
        assert_eq!(manager.env().announced.last().unwrap().device_id, C);
        assert_eq!(
            manager.env().attempts_on(A),
            0,
            "the request ran before any tick"
        );
    }

    #[test]
    fn waiting_requests_run_before_the_next_tick() {
        let env = FakeEnv::new(&[A, B, C], A, &[A, B, C]);
        let (requests, inbox) = std::sync::mpsc::channel();
        let far = Instant::now() + Duration::from_secs(10);
        let (first, first_reply) = select_request(B, far);
        let (second, second_reply) = select_request(C, far);
        requests.send(first).unwrap();
        requests.send(second).unwrap();
        requests.send(ManagerRequest::Shutdown).unwrap();
        let manager = run_output_manager(
            OutputManager::new(env, None),
            inbox,
            Duration::ZERO,
            Arc::default(),
        );
        assert!(first_reply.recv().unwrap().is_ok());
        assert!(second_reply.recv().unwrap().is_ok());
        let opened: Vec<&str> = manager
            .env()
            .open_attempts
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(
            opened,
            vec![B, C],
            "both requests, in order, before any tick"
        );
    }

    #[test]
    fn while_the_prompt_is_pending_the_loop_serves_requests_and_the_answer_opens_at_once() {
        // A backend open takes 1.5 s here, standing in for macOS holding it
        // until the answer; the loop must never be inside one while waiting.
        let mut env = prompt_pending_env(A);
        env.open_delay = Duration::from_millis(1_500);
        let prompt = Arc::clone(&env.prompt_pending);
        let ticks = Arc::clone(&env.snapshots_started);
        let (requests, _preferred, thread) =
            spawn_loop(OutputManager::new(env, None), Duration::from_secs(60));
        while ticks.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            std::thread::yield_now();
        }
        // A choice made meanwhile is answered right away (kept for later).
        let (request, response) = select_request(B, Instant::now() + Duration::from_secs(10));
        let asked = Instant::now();
        requests.send(request).unwrap();
        assert_eq!(
            response.recv().unwrap(),
            Err(SelectError::OpenFailed(B.into()))
        );
        assert!(asked.elapsed() < Duration::from_secs(1));

        // The answer wakes the loop before its next tick (60 s away).
        prompt.store(false, std::sync::atomic::Ordering::SeqCst);
        requests.send(ManagerRequest::Recheck).unwrap();
        requests.send(ManagerRequest::Shutdown).unwrap();
        let manager = thread.join().unwrap();
        assert_eq!(manager.env().current_id(), Some(B));
        assert_eq!(manager.env().open_attempts, vec![B.to_owned()]);
    }

    #[test]
    fn shutdown_is_prompt_while_the_prompt_is_pending() {
        let mut env = prompt_pending_env(A);
        env.open_delay = Duration::from_secs(5);
        let ticks = Arc::clone(&env.snapshots_started);
        let (requests, _preferred, thread) =
            spawn_loop(OutputManager::new(env, None), Duration::from_millis(10));
        while ticks.load(std::sync::atomic::Ordering::SeqCst) < 3 {
            std::thread::yield_now();
        }
        let started = Instant::now();
        requests.send(ManagerRequest::Shutdown).unwrap();
        thread.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn shutdown_stops_the_loop_promptly() {
        let env = FakeEnv::new(&[A], A, &[A]);
        let (requests, _preferred, thread) =
            spawn_loop(OutputManager::new(env, None), Duration::from_secs(60));
        let started = Instant::now();
        requests.send(ManagerRequest::Shutdown).unwrap();
        thread.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}

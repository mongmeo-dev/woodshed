//! The separation worker: one thread that owns the model (Core ML objects
//! stay on the thread that made them), runs queued songs one at a time,
//! reports progress per chunk as [`SeparationEvent`]s, warms the model up
//! after the app starts and drops it again when idle. Nothing here runs on
//! the audio thread.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::stems::{Library, LibraryError, SeparationEvent, StemStatus};

use super::dsp::Stft;
use super::engine::Workspace;
use super::job::{self, Abandon};
use super::model::{SeparationError, StemModel};

/// `SeparationEvent::state` values.
pub const STATE_RUNNING: &str = "running";
pub const STATE_DONE: &str = "done";
pub const STATE_FAILED: &str = "failed";
pub const STATE_CANCELLED: &str = "cancelled";

/// How long `shutdown` waits for a running job to clean up.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(3);

/// Receives every event (the app emits them as `stems://separation`).
pub type Emit = Arc<dyn Fn(SeparationEvent) + Send + Sync>;
/// Makes the model on the worker thread; called again after an unload.
pub type ModelLoader = Box<dyn Fn() -> Result<Box<dyn StemModel>, SeparationError> + Send>;

#[derive(Clone, Copy, Debug)]
pub struct ServiceConfig {
    /// Load the model and run one silent chunk this long after start, so the
    /// first separation does not wait for GPU kernel compilation.
    pub warm_up_after: Option<Duration>,
    /// Drop the model and buffers after this long without work.
    pub unload_after: Duration,
}

impl ServiceConfig {
    /// The app: warm up shortly after launch, release about 1 GB when idle.
    pub const APP: Self = Self {
        warm_up_after: Some(Duration::from_secs(5)),
        unload_after: Duration::from_secs(120),
    };
}

fn event(song_id: &str, state: &str, progress: Option<f32>) -> SeparationEvent {
    SeparationEvent {
        song_id: song_id.to_owned(),
        state: state.to_owned(),
        progress,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Running {
    song_id: String,
    cancel: Arc<AtomicBool>,
    progress: Option<f32>,
}

#[derive(Default)]
struct State {
    queue: VecDeque<String>,
    running: Option<Running>,
    shutdown: bool,
    stopped: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

/// Lock order: `Shared::state` → the library's manifest lock.
pub struct SeparationService {
    library: Library,
    shared: Arc<Shared>,
    emit: Emit,
}

impl SeparationService {
    /// Recovers songs a previous run left mid-separation, then starts the
    /// worker thread.
    pub fn start(library: Library, loader: ModelLoader, emit: Emit, config: ServiceConfig) -> Self {
        job::recover(&library);
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        });
        let worker = Worker {
            shared: Arc::clone(&shared),
            library: library.clone(),
            emit: Arc::clone(&emit),
            loader,
            config,
        };
        let spawned = std::thread::Builder::new()
            .name("stem-separation".into())
            .spawn(move || worker.run());
        if let Err(error) = spawned {
            log::error!("cannot start the separation thread: {error}");
            let mut state = lock(&shared.state);
            state.shutdown = true;
            state.stopped = true;
        }
        Self {
            library,
            shared,
            emit,
        }
    }

    /// Queues song `id`. A song already queued or running is left alone.
    pub fn separate(&self, id: &str) -> Result<(), SeparationError> {
        self.library.get(id)?;
        let mut state = lock(&self.shared.state);
        if state.shutdown {
            return Err(SeparationError::Cancelled);
        }
        let queued = state.queue.iter().any(|queued| queued == id);
        if queued || state.running.as_ref().is_some_and(|r| r.song_id == id) {
            return Ok(());
        }
        job::mark(&self.library, id, StemStatus::Pending)?;
        (self.emit)(event(id, STATE_RUNNING, None));
        state.queue.push_back(id.to_owned());
        drop(state);
        self.shared.wake.notify_all();
        Ok(())
    }

    /// Cancels song `id`: a queued song leaves the queue now, a running one
    /// stops at its next chunk. Either way a `cancelled` event follows.
    pub fn cancel(&self, id: &str) -> Result<(), SeparationError> {
        let mut state = lock(&self.shared.state);
        if let Some(index) = state.queue.iter().position(|queued| queued == id) {
            state.queue.remove(index);
            let result = job::abandon(&self.library, id, Abandon::Cancelled);
            (self.emit)(event(id, STATE_CANCELLED, None));
            return result;
        }
        if let Some(running) = state.running.as_ref().filter(|r| r.song_id == id) {
            running.cancel.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Separations not finished yet: the running one with its progress, then
    /// the queue. Lets a screen that was closed show them again.
    pub fn status(&self) -> Vec<SeparationEvent> {
        let state = lock(&self.shared.state);
        state
            .running
            .iter()
            .map(|r| event(&r.song_id, STATE_RUNNING, r.progress))
            .chain(state.queue.iter().map(|id| event(id, STATE_RUNNING, None)))
            .collect()
    }

    /// Stops the worker (app exit): the queue is dropped, a running job
    /// stops at its next chunk and cleans up. Waits a few seconds at most;
    /// whatever is left is recovered at the next start.
    pub fn shutdown(&self) {
        let mut state = lock(&self.shared.state);
        state.shutdown = true;
        state.queue.clear();
        if let Some(running) = &state.running {
            running.cancel.store(true, Ordering::Relaxed);
        }
        self.shared.wake.notify_all();
        let deadline = Instant::now() + SHUTDOWN_WAIT;
        while !state.stopped {
            let now = Instant::now();
            if now >= deadline {
                log::warn!("the separation thread is still busy at exit");
                return;
            }
            state = self
                .shared
                .wake
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

enum Task {
    Job {
        song_id: String,
        cancel: Arc<AtomicBool>,
    },
    WarmUp,
    Unload,
    Stop,
}

struct Worker {
    shared: Arc<Shared>,
    library: Library,
    emit: Emit,
    loader: ModelLoader,
    config: ServiceConfig,
}

/// What the worker holds between jobs.
struct Loaded {
    model: Option<Box<dyn StemModel>>,
    work: Option<Workspace>,
    idle_since: Instant,
}

impl Worker {
    fn run(self) {
        let stft = Stft::new();
        let mut loaded = Loaded {
            model: None,
            work: None,
            idle_since: Instant::now(),
        };
        let mut warm_up_at = self
            .config
            .warm_up_after
            .map(|after| Instant::now() + after);
        loop {
            let resident = loaded.model.is_some() || loaded.work.is_some();
            match self.next_task(&mut warm_up_at, resident, loaded.idle_since) {
                Task::Stop => break,
                Task::Unload => {
                    loaded.model = None;
                    loaded.work = None;
                    log::info!("stem model unloaded after idling");
                }
                Task::WarmUp => {
                    let started = Instant::now();
                    match self.warm_up(&stft, &mut loaded) {
                        Ok(()) => log::info!(
                            "stem model warmed up in {:.1}s",
                            started.elapsed().as_secs_f64()
                        ),
                        Err(error) => log::warn!("stem model warm-up skipped: {error}"),
                    }
                    loaded.idle_since = Instant::now();
                }
                Task::Job { song_id, cancel } => {
                    self.run_job(&song_id, &cancel, &stft, &mut loaded);
                    loaded.idle_since = Instant::now();
                }
            }
        }
        let mut state = lock(&self.shared.state);
        state.stopped = true;
        state.running = None;
        self.shared.wake.notify_all();
    }

    fn next_task(
        &self,
        warm_up_at: &mut Option<Instant>,
        resident: bool,
        idle_since: Instant,
    ) -> Task {
        let mut state = lock(&self.shared.state);
        loop {
            if state.shutdown {
                return Task::Stop;
            }
            if let Some(song_id) = state.queue.pop_front() {
                let cancel = Arc::new(AtomicBool::new(false));
                state.running = Some(Running {
                    song_id: song_id.clone(),
                    cancel: Arc::clone(&cancel),
                    progress: None,
                });
                return Task::Job { song_id, cancel };
            }
            let now = Instant::now();
            let mut deadline: Option<Instant> = None;
            if let Some(at) = *warm_up_at {
                if now >= at {
                    *warm_up_at = None;
                    return Task::WarmUp;
                }
                deadline = Some(at);
            }
            if resident {
                let at = idle_since + self.config.unload_after;
                if now >= at {
                    return Task::Unload;
                }
                deadline = Some(deadline.map_or(at, |d| d.min(at)));
            }
            state = match deadline {
                Some(at) => {
                    self.shared
                        .wake
                        .wait_timeout(state, at - now)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
                None => self
                    .shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner),
            };
        }
    }

    fn ensure_loaded<'a>(
        &self,
        loaded: &'a mut Loaded,
    ) -> Result<(&'a mut dyn StemModel, &'a mut Workspace), SeparationError> {
        if loaded.model.is_none() {
            let started = Instant::now();
            loaded.model = Some((self.loader)()?);
            log::info!(
                "stem model loaded in {:.2}s",
                started.elapsed().as_secs_f64()
            );
        }
        let work = loaded.work.get_or_insert_with(Workspace::new);
        let model = loaded.model.as_deref_mut().expect("loaded above");
        Ok((model, work))
    }

    fn warm_up(&self, stft: &Stft, loaded: &mut Loaded) -> Result<(), SeparationError> {
        let (model, work) = self.ensure_loaded(loaded)?;
        work.warm_up(stft, model)
    }

    fn set_progress(&self, song_id: &str, progress: Option<f32>) {
        if let Some(running) = lock(&self.shared.state).running.as_mut() {
            running.progress = progress;
        }
        (self.emit)(event(song_id, STATE_RUNNING, progress));
    }

    fn run_job(&self, song_id: &str, cancel: &AtomicBool, stft: &Stft, loaded: &mut Loaded) {
        let started = Instant::now();
        let result = (|| {
            // `separate` already sent `running` with no progress.
            job::mark(&self.library, song_id, StemStatus::Running)?;
            let (model, work) = self.ensure_loaded(loaded)?;
            if cancel.load(Ordering::Relaxed) {
                return Err(SeparationError::Cancelled);
            }
            job::run(
                &self.library,
                song_id,
                stft,
                model,
                work,
                cancel,
                &mut |progress| self.set_progress(song_id, Some(progress)),
            )
        })();
        let (state, progress) = match result {
            Ok(()) => {
                log::info!(
                    "separated {song_id} in {:.1}s",
                    started.elapsed().as_secs_f64()
                );
                (STATE_DONE, Some(1.0))
            }
            Err(SeparationError::Cancelled | SeparationError::Library(LibraryError::NotFound)) => {
                if let Err(error) = job::abandon(&self.library, song_id, Abandon::Cancelled) {
                    log::debug!("cleanup after cancelling {song_id}: {error}");
                }
                (STATE_CANCELLED, None)
            }
            Err(error) => {
                log::error!("separation of {song_id} failed: {error}");
                if let Err(error) = job::abandon(&self.library, song_id, Abandon::Failed) {
                    log::warn!("cleanup after the failure of {song_id}: {error}");
                }
                (STATE_FAILED, None)
            }
        };
        // Under the lock, so a new request for the song queues after this event.
        let mut shared = lock(&self.shared.state);
        shared.running = None;
        (self.emit)(event(song_id, state, progress));
    }
}

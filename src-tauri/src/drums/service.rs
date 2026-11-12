//! Tauri side of the drum machine: owns the sequencer, keeps the schedule
//! filled from a helper thread, tells the screen ahead of time when each step
//! is heard, and stores patterns.
//!
//! Lock order: the service's own mutex first, then the engine's.

use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;

use crate::audio::{AudioEngine, EngineError};

use super::announce::StepEvent;
use super::control::DrumControl;
use super::kit::Kit;
use super::pattern::Pattern;
use super::sequencer::Sequencer;
use super::source::DrumSource;
use super::store::{PatternStore, SavedPattern, StoreError};

/// How often the scheduler thread tops up the schedule while playing.
const TICK: Duration = Duration::from_millis(10);
/// Edits within this window are written to disk once.
const PERSIST_DEBOUNCE: Duration = Duration::from_millis(400);
/// Mixer gain of the drum source.
const SOURCE_GAIN: f32 = 0.8;
/// Output rates converted at startup, so switching devices rarely converts
/// the kit while the engine is locked.
const WARM_RATES: [u32; 2] = [44_100, 48_000];

/// What the screen needs when it opens.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DrumState {
    pub playing: bool,
    pub pattern: Pattern,
    pub saved: Vec<SavedPattern>,
}

struct Inner {
    control: DrumControl,
    /// The screen's step channel and its subscription token.
    channel: Option<(u64, Channel<StepEvent>)>,
}

impl Inner {
    fn send(&mut self, outbox: Vec<StepEvent>) {
        let Some((_, channel)) = &self.channel else {
            return;
        };
        for event in outbox {
            if channel.send(event).is_err() {
                self.channel = None;
                return;
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct DrumService {
    inner: Arc<Mutex<Inner>>,
    engine: Arc<Mutex<AudioEngine>>,
    store: Arc<PatternStore>,
    wake: mpsc::Sender<()>,
    persist: mpsc::Sender<Pattern>,
}

impl DrumService {
    /// Decodes the kit, attaches the drum source to the engine and starts
    /// the scheduler and persistence threads.
    pub fn start(engine: &Arc<Mutex<AudioEngine>>, data_dir: &Path) -> Result<Self, EngineError> {
        let kit = Arc::new(Kit::bundled().unwrap_or_else(|error| {
            log::error!("drum kit cannot be decoded, drums are silent: {error}");
            Kit::silent()
        }));
        let store = Arc::new(PatternStore::load(data_dir));
        let pattern = store.current().unwrap_or_default().normalized();
        let source = lock(engine)
            .handle()
            .add_source(Box::new(DrumSource::new(Arc::clone(&kit))), SOURCE_GAIN)?;
        {
            let kit = Arc::clone(&kit);
            std::thread::Builder::new()
                .name("drums-kit-warmup".into())
                .spawn(move || {
                    for rate in WARM_RATES {
                        kit.at_rate(rate);
                    }
                })
                .expect("spawn drum kit warmup");
        }

        let inner = Arc::new(Mutex::new(Inner {
            control: DrumControl::new(Sequencer::new(source, pattern)),
            channel: None,
        }));
        let (wake, wakes) = mpsc::channel();
        spawn_scheduler(Arc::clone(&inner), Arc::clone(engine), wakes);
        let (persist, edits) = mpsc::channel();
        spawn_persister(Arc::clone(&store), edits);
        Ok(Self {
            inner,
            engine: Arc::clone(engine),
            store,
            wake,
            persist,
        })
    }

    pub fn state(&self) -> DrumState {
        let inner = lock(&self.inner);
        DrumState {
            playing: inner.control.sequencer().is_playing(),
            pattern: inner.control.sequencer().pattern().clone(),
            saved: self.store.saved(),
        }
    }

    pub fn set_playing(&self, playing: bool) -> Result<bool, EngineError> {
        let mut inner = lock(&self.inner);
        let mut outbox = Vec::new();
        let result = inner
            .control
            .set_playing(&mut *lock(&self.engine), playing, &mut outbox);
        inner.send(outbox);
        drop(inner);
        let _ = self.wake.send(());
        result.map(|()| playing)
    }

    /// Applies an edited pattern; while playing it is heard from a moment
    /// ahead without stopping.
    pub fn set_pattern(&self, pattern: Pattern) -> Result<Pattern, EngineError> {
        let pattern = pattern.normalized();
        let mut inner = lock(&self.inner);
        let mut outbox = Vec::new();
        let result =
            inner
                .control
                .set_pattern(&mut *lock(&self.engine), pattern.clone(), &mut outbox);
        inner.send(outbox);
        result?;
        let _ = self.persist.send(pattern.clone());
        Ok(pattern)
    }

    /// Makes `channel` the screen's step channel and announces what is
    /// already scheduled. `token` increases with each subscription on the
    /// screen side, so a late-arriving older subscription cannot replace a
    /// newer one.
    pub fn subscribe(&self, channel: Channel<StepEvent>, token: u64) {
        let mut inner = lock(&self.inner);
        if inner
            .channel
            .as_ref()
            .is_some_and(|(current, _)| *current > token)
        {
            return;
        }
        inner.channel = Some((token, channel));
        let mut outbox = Vec::new();
        let result = inner
            .control
            .announce_all(&mut *lock(&self.engine), &mut outbox);
        inner.send(outbox);
        if let Err(error) = result {
            log::warn!("drum step channel: {error}");
        }
    }

    pub fn save_named(
        &self,
        name: &str,
        pattern: Pattern,
    ) -> Result<Vec<SavedPattern>, StoreError> {
        self.store.save_named(name, &pattern.normalized())
    }

    pub fn delete_named(&self, name: &str) -> Result<Vec<SavedPattern>, StoreError> {
        self.store.delete(name)
    }
}

fn spawn_scheduler(
    inner: Arc<Mutex<Inner>>,
    engine: Arc<Mutex<AudioEngine>>,
    wakes: mpsc::Receiver<()>,
) {
    std::thread::Builder::new()
        .name("drums-scheduler".into())
        .spawn(move || {
            loop {
                let playing = lock(&inner).control.sequencer().is_playing();
                let woke = if playing {
                    wakes.recv_timeout(TICK)
                } else {
                    wakes.recv().map_err(|_| RecvTimeoutError::Disconnected)
                };
                if woke == Err(RecvTimeoutError::Disconnected) {
                    return;
                }
                let mut inner = lock(&inner);
                if !inner.control.sequencer().is_playing() {
                    continue;
                }
                let mut outbox = Vec::new();
                let result = inner.control.tick(&mut *lock(&engine), &mut outbox);
                if let Err(error) = result {
                    log::warn!("drum scheduler: {error}");
                }
                inner.send(outbox);
            }
        })
        .expect("spawn drum scheduler");
}

fn spawn_persister(store: Arc<PatternStore>, edits: mpsc::Receiver<Pattern>) {
    std::thread::Builder::new()
        .name("drums-persist".into())
        .spawn(move || {
            while let Ok(mut latest) = edits.recv() {
                let mut open = true;
                loop {
                    match edits.recv_timeout(PERSIST_DEBOUNCE) {
                        Ok(pattern) => latest = pattern,
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => {
                            open = false;
                            break;
                        }
                    }
                }
                if let Err(error) = store.save_current(&latest) {
                    log::warn!("cannot save the drum pattern: {error}");
                }
                if !open {
                    return;
                }
            }
        })
        .expect("spawn drum persistence");
}

// ---- commands ------------------------------------------------------------------

/// Why saving or deleting a pattern failed.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum PatternStoreError {
    EmptyName,
    SaveFailed { message: String },
}

impl From<StoreError> for PatternStoreError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::EmptyName => Self::EmptyName,
            StoreError::Io(error) => Self::SaveFailed {
                message: error.to_string(),
            },
        }
    }
}

#[tauri::command]
pub async fn drums_state(drums: State<'_, DrumService>) -> Result<DrumState, String> {
    Ok(drums.state())
}

#[tauri::command]
pub async fn drums_set_playing(
    drums: State<'_, DrumService>,
    playing: bool,
) -> Result<bool, String> {
    drums
        .set_playing(playing)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn drums_set_pattern(
    drums: State<'_, DrumService>,
    pattern: Pattern,
) -> Result<Pattern, String> {
    drums
        .set_pattern(pattern)
        .map_err(|error| error.to_string())
}

/// Registers the screen's channel for [`StepEvent`]s, replacing one with a
/// lower `token`.
#[tauri::command]
pub async fn drums_subscribe(
    drums: State<'_, DrumService>,
    on_event: Channel<StepEvent>,
    token: u64,
) -> Result<(), String> {
    drums.subscribe(on_event, token);
    Ok(())
}

#[tauri::command]
pub async fn drums_save_pattern(
    drums: State<'_, DrumService>,
    name: String,
    pattern: Pattern,
) -> Result<Vec<SavedPattern>, PatternStoreError> {
    Ok(drums.save_named(&name, pattern)?)
}

#[tauri::command]
pub async fn drums_delete_pattern(
    drums: State<'_, DrumService>,
    name: String,
) -> Result<Vec<SavedPattern>, PatternStoreError> {
    Ok(drums.delete_named(&name)?)
}

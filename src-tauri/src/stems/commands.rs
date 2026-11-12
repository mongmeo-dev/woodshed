//! Tauri commands of the stem splitter screen. Every command is async: the
//! engine lock, disk work and external tools never run on the webview's IPC
//! thread. Lock order: `StemService::player` → engine.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::app_audio::AudioService;
use crate::audio::{AudioEngine, EventKind, SourceId};

use super::import::{ImportError, ImportProgress, import_file, import_link};
use super::library::{Library, LibraryError, Manifest, SongSource, StemEntry, TrackName};
use super::player::{FeederThread, StemPlayer};
use super::reader::LoopRange;
use super::source::{PlayerShared, loop_events, param};
use super::track::{TrackReader, WavTrack};
use super::youtube::{ProcessControl, Tools};
use super::{MAX_TRACKS, MIN_SPEED};

/// Event the stem engine emits while separating (payload [`SeparationEvent`]).
pub const SEPARATION_EVENT: &str = "stems://separation";

/// Lead for transport events: a few callbacks ahead of the render position.
const EVENT_LEAD_SECS: f64 = 0.01;

/// Progress of a separation, for the screen's status slot.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeparationEvent {
    pub song_id: String,
    /// `running`, `done` or `failed`.
    pub state: String,
    /// 0.0..=1.0 while running.
    pub progress: Option<f32>,
}

struct ActivePlayer {
    song_id: String,
    source: SourceId,
    shared: Arc<PlayerShared>,
    song_rate: u32,
    frames: u64,
    tracks: usize,
    _feeder: FeederThread,
}

pub struct StemService {
    library: Library,
    cache_dir: PathBuf,
    player: Mutex<Option<ActivePlayer>>,
    import: Mutex<Option<Arc<ProcessControl>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl StemService {
    /// The library lives in `app_data/songs`; yt-dlp caches in `cache_dir`.
    pub fn new(app_data: &std::path::Path, cache_dir: PathBuf) -> Self {
        let library = Library::new(app_data);
        library.clean_incoming();
        Self {
            library,
            cache_dir,
            player: Mutex::new(None),
            import: Mutex::new(None),
        }
    }

    /// Stops an import in progress (app exit).
    pub fn shutdown(&self) {
        if let Some(control) = lock(&self.import).take() {
            control.cancel();
        }
    }

    fn close_player(&self, audio: &AudioService, player: &mut Option<ActivePlayer>) {
        if let Some(active) = player.take() {
            let _ = lock(audio.engine()).handle().remove_source(active.source);
            // Dropping `active` stops and joins the feeder thread.
        }
    }
}

/// A song as the list shows it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongSummary {
    pub id: String,
    pub title: String,
    pub source: SongSource,
    pub duration_secs: f64,
    pub sample_rate: u32,
    pub added_at_ms: u64,
    pub stems: Vec<StemEntry>,
}

impl From<&Manifest> for SongSummary {
    fn from(manifest: &Manifest) -> Self {
        Self {
            id: manifest.id.clone(),
            title: manifest.title.clone(),
            source: manifest.source.clone(),
            duration_secs: manifest.duration_secs(),
            sample_rate: manifest.sample_rate,
            added_at_ms: manifest.added_at_ms,
            stems: manifest.stems.clone(),
        }
    }
}

/// The song now loaded in the player.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedSong {
    pub song: SongSummary,
    pub tracks: Vec<TrackName>,
    pub waveform: Vec<u8>,
}

/// Playback state for the screen. `position_secs` is heard at `heard_at_ms`
/// (Unix epoch ms; `None` before the output's first callback).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transport {
    pub song_id: String,
    pub playing: bool,
    pub ended: bool,
    pub position_secs: f64,
    pub heard_at_ms: Option<f64>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopSeconds {
    pub start: f64,
    pub end: f64,
}

fn library_error(error: LibraryError) -> String {
    error.to_string()
}

fn run_blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> impl std::future::Future<Output = Option<T>> {
    let task = tauri::async_runtime::spawn_blocking(work);
    async move { task.await.ok() }
}

/// Schedules `kinds` for the open player's source, in order, just ahead of
/// the render position.
fn schedule(engine: &mut AudioEngine, source: SourceId, kinds: &[EventKind]) -> Result<(), String> {
    let handle = engine.handle();
    let frame = handle.frame_after(EVENT_LEAD_SECS);
    for kind in kinds {
        handle
            .schedule(frame, source, *kind)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn with_player<T>(
    stems: &StemService,
    audio: &AudioService,
    action: impl FnOnce(&ActivePlayer, &mut AudioEngine) -> Result<T, String>,
) -> Result<T, String> {
    let player = lock(&stems.player);
    let active = player
        .as_ref()
        .ok_or_else(|| "no song is open".to_owned())?;
    let mut engine = lock(audio.engine());
    action(active, &mut engine)
}

// ---- library and import ----------------------------------------------------

#[tauri::command]
pub async fn stems_list(stems: State<'_, StemService>) -> Result<Vec<SongSummary>, String> {
    Ok(stems.library.list().iter().map(SongSummary::from).collect())
}

/// Shows the system open panel for audio files; `None` when dismissed.
#[tauri::command]
pub async fn stems_pick_file(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (reply, response) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .add_filter("Audio", super::AUDIO_EXTENSIONS)
        .pick_file(move |path| {
            let _ = reply.send(path.and_then(|path| path.into_path().ok()));
        });
    let picked = run_blocking(move || response.recv().ok().flatten())
        .await
        .flatten();
    Ok(picked.map(|path| path.to_string_lossy().into_owned()))
}

/// The one import that may run at a time. Dropping it (the command
/// finished, or its future was dropped) cancels anything still running and
/// frees the slot.
struct ImportSlot<'a> {
    slot: &'a Mutex<Option<Arc<ProcessControl>>>,
    control: Arc<ProcessControl>,
}

impl Drop for ImportSlot<'_> {
    fn drop(&mut self) {
        self.control.cancel();
        lock(self.slot).take();
    }
}

fn begin_import(stems: &StemService) -> Result<ImportSlot<'_>, ImportError> {
    let mut slot = lock(&stems.import);
    if slot.is_some() {
        return Err(ImportError::Busy);
    }
    let control = Arc::new(ProcessControl::default());
    *slot = Some(Arc::clone(&control));
    Ok(ImportSlot {
        slot: &stems.import,
        control,
    })
}

#[tauri::command]
pub async fn stems_import_file(
    stems: State<'_, StemService>,
    path: String,
    on_progress: Channel<ImportProgress>,
) -> Result<SongSummary, ImportError> {
    let slot = begin_import(&stems)?;
    let library = stems.library.clone();
    let job = Arc::clone(&slot.control);
    let result = run_blocking(move || {
        import_file(
            &library,
            std::path::Path::new(&path),
            &job,
            &mut |progress| {
                let _ = on_progress.send(progress);
            },
        )
    })
    .await
    .unwrap_or(Err(ImportError::Cancelled));
    drop(slot);
    result.map(|manifest| SongSummary::from(&manifest))
}

#[tauri::command]
pub async fn stems_import_link(
    stems: State<'_, StemService>,
    url: String,
    on_progress: Channel<ImportProgress>,
) -> Result<SongSummary, ImportError> {
    let slot = begin_import(&stems)?;
    let library = stems.library.clone();
    let cache = stems.cache_dir.join("yt-dlp");
    let job = Arc::clone(&slot.control);
    let result = run_blocking(move || {
        import_link(
            &library,
            &url,
            Tools::locate(),
            &cache,
            &job,
            &mut |progress| {
                let _ = on_progress.send(progress);
            },
        )
    })
    .await
    .unwrap_or(Err(ImportError::Cancelled));
    drop(slot);
    result.map(|manifest| SongSummary::from(&manifest))
}

#[tauri::command]
pub async fn stems_cancel_import(stems: State<'_, StemService>) -> Result<(), String> {
    if let Some(control) = lock(&stems.import).as_ref() {
        control.cancel();
    }
    Ok(())
}

#[tauri::command]
pub async fn stems_delete(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
    id: String,
) -> Result<(), String> {
    let mut player = lock(&stems.player);
    if player.as_ref().is_some_and(|active| active.song_id == id) {
        stems.close_player(&audio, &mut player);
    }
    drop(player);
    stems.library.delete(&id).map_err(library_error)
}

// ---- player ------------------------------------------------------------------

#[tauri::command]
pub async fn stems_open(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
    id: String,
) -> Result<OpenedSong, String> {
    let manifest = stems.library.get(&id).map_err(library_error)?;
    let dir = stems.library.song_dir(&id).map_err(library_error)?;
    let waveform = stems.library.waveform(&id).unwrap_or_default();
    let tracks = manifest.tracks();
    let mut readers: Vec<Box<dyn TrackReader>> = Vec::with_capacity(tracks.len());
    for (_, path) in &tracks {
        let track = WavTrack::open(&dir.join(path)).map_err(|error| error.to_string())?;
        readers.push(Box::new(track));
    }
    let StemPlayer {
        source,
        feeder,
        shared,
        song_rate,
        frames,
    } = StemPlayer::new(readers, 0).map_err(|error| error.to_string())?;

    let mut player = lock(&stems.player);
    stems.close_player(&audio, &mut player);
    let source = lock(audio.engine())
        .handle()
        .add_source(Box::new(source), 1.0)
        .map_err(|error| error.to_string())?;
    let feeder = FeederThread::spawn(feeder).map_err(|error| error.to_string())?;
    *player = Some(ActivePlayer {
        song_id: id,
        source,
        shared,
        song_rate,
        frames,
        tracks: tracks.len(),
        _feeder: feeder,
    });
    Ok(OpenedSong {
        song: SongSummary::from(&manifest),
        tracks: tracks.into_iter().map(|(name, _)| name).collect(),
        waveform,
    })
}

#[tauri::command]
pub async fn stems_close(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
) -> Result<(), String> {
    let mut player = lock(&stems.player);
    stems.close_player(&audio, &mut player);
    Ok(())
}

#[tauri::command]
pub async fn stems_play(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
) -> Result<(), String> {
    with_player(&stems, &audio, |active, engine| {
        schedule(engine, active.source, &[EventKind::Start])
    })
}

#[tauri::command]
pub async fn stems_pause(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
) -> Result<(), String> {
    with_player(&stems, &audio, |active, engine| {
        schedule(engine, active.source, &[EventKind::Stop])
    })
}

#[tauri::command]
pub async fn stems_seek(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
    seconds: f64,
) -> Result<(), String> {
    with_player(&stems, &audio, |active, engine| {
        let frame = (seconds.max(0.0) * f64::from(active.song_rate)).round() as u64;
        schedule(
            engine,
            active.source,
            &[EventKind::Seek {
                frame: frame.min(active.frames),
            }],
        )
    })
}

#[tauri::command]
pub async fn stems_set_speed(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
    speed: f32,
) -> Result<(), String> {
    with_player(&stems, &audio, |active, engine| {
        let value = speed.clamp(MIN_SPEED, 1.0);
        schedule(
            engine,
            active.source,
            &[EventKind::Param {
                id: param::SPEED,
                value,
            }],
        )
    })
}

/// Sets the A-B loop in seconds, or clears it with `null`.
#[tauri::command]
pub async fn stems_set_loop(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
    range: Option<LoopSeconds>,
) -> Result<(), String> {
    with_player(&stems, &audio, |active, engine| {
        let rate = f64::from(active.song_rate);
        let range = range.and_then(|range| {
            let start = (range.start.max(0.0) * rate).round() as u64;
            let end = ((range.end.max(0.0) * rate).round() as u64).min(active.frames);
            LoopRange::new(start, end)
        });
        schedule(engine, active.source, &loop_events(range))
    })
}

#[tauri::command]
pub async fn stems_set_track(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
    index: usize,
    volume: f32,
    muted: bool,
    solo: bool,
) -> Result<(), String> {
    with_player(&stems, &audio, |active, engine| {
        if index >= active.tracks.min(MAX_TRACKS) {
            return Err("no such track".into());
        }
        let flag = |on: bool| if on { 1.0 } else { 0.0 };
        schedule(
            engine,
            active.source,
            &[
                EventKind::Param {
                    id: param::track(index, param::VOLUME),
                    value: volume.clamp(0.0, 1.0),
                },
                EventKind::Param {
                    id: param::track(index, param::MUTE),
                    value: flag(muted),
                },
                EventKind::Param {
                    id: param::track(index, param::SOLO),
                    value: flag(solo),
                },
            ],
        )
    })
}

/// The stem player's master volume (its source gain; ~5 ms glide).
#[tauri::command]
pub async fn stems_set_master(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
    volume: f32,
) -> Result<(), String> {
    with_player(&stems, &audio, |active, engine| {
        engine
            .handle()
            .set_gain(active.source, volume.clamp(0.0, 1.0))
            .map_err(|error| error.to_string())
    })
}

/// Current playback position and when it is heard. `None` without a song.
#[tauri::command]
pub async fn stems_transport(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
) -> Result<Option<Transport>, String> {
    let player = lock(&stems.player);
    let Some(active) = player.as_ref() else {
        return Ok(None);
    };
    let engine = lock(audio.engine());
    // The source publishes the position after its last rendered frame,
    // which is the engine's rendered frame count (within one callback).
    let heard_at_ms = engine.heard_at_epoch_ms(engine.stats().frames_rendered);
    Ok(Some(Transport {
        song_id: active.song_id.clone(),
        playing: active.shared.playing(),
        ended: active.shared.ended(),
        position_secs: active.shared.position() / f64::from(active.song_rate.max(1)),
        heard_at_ms,
    }))
}

/// After `audio://output-changed`: drops events still pending for the old
/// output. The player itself restarts at the new rate (`Source::prepare`).
#[tauri::command]
pub async fn stems_resync(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
) -> Result<(), String> {
    let player = lock(&stems.player);
    if let Some(active) = player.as_ref() {
        let mut engine = lock(audio.engine());
        let handle = engine.handle();
        let from = handle.frames_rendered();
        handle
            .cancel(active.source, from)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

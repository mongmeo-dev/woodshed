//! Tauri commands of the stem splitter screen. Every command is async: the
//! engine lock, disk work and external tools never run on the webview's IPC
//! thread. Lock order: `StemService::player` → engine.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::app_audio::AudioService;
use crate::audio::{AudioEngine, EventKind, FramePos, SourceId};

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
    /// `None` only in tests, which drive the feeder by hand.
    _feeder: Option<FeederThread>,
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
    /// Engine frames rendered when sampled; events scheduled at or before
    /// this frame are reflected.
    pub rendered_frame: FramePos,
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
fn schedule(
    engine: &mut AudioEngine,
    source: SourceId,
    kinds: &[EventKind],
) -> Result<Scheduled, String> {
    let handle = engine.handle();
    let frame = handle.frame_after(EVENT_LEAD_SECS);
    for kind in kinds {
        handle
            .schedule(frame, source, *kind)
            .map_err(|error| error.to_string())?;
    }
    Ok(Scheduled { frame })
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
        _feeder: Some(feeder),
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

/// Where a command's events sit on the engine clock: a transport sample
/// with `rendered_frame >= frame` already reflects them.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scheduled {
    pub frame: FramePos,
}

/// Mix of one track as the screen holds it.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackMixIntent {
    pub volume: f32,
    pub muted: bool,
    pub solo: bool,
}

/// Everything the screen wants the player to be, for re-applying it on a new
/// output (`stems_restore`).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerIntent {
    pub playing: bool,
    /// A seek the screen sent that may not have been applied yet.
    pub seek: Option<f64>,
    pub speed: f32,
    #[serde(rename = "loop")]
    pub looping: Option<LoopSeconds>,
    pub tracks: Vec<TrackMixIntent>,
    pub master: f32,
}

fn seek_event(active: &ActivePlayer, seconds: f64) -> EventKind {
    let frame = (seconds.max(0.0) * f64::from(active.song_rate)).round() as u64;
    EventKind::Seek {
        frame: frame.min(active.frames),
    }
}

fn speed_event(speed: f32) -> EventKind {
    EventKind::Param {
        id: param::SPEED,
        value: speed.clamp(MIN_SPEED, 1.0),
    }
}

fn loop_range(active: &ActivePlayer, range: Option<LoopSeconds>) -> Option<LoopRange> {
    let rate = f64::from(active.song_rate);
    range.and_then(|range| {
        let start = (range.start.max(0.0) * rate).round() as u64;
        let end = ((range.end.max(0.0) * rate).round() as u64).min(active.frames);
        LoopRange::new(start, end)
    })
}

fn track_events(index: usize, mix: TrackMixIntent) -> [EventKind; 3] {
    let flag = |on: bool| if on { 1.0 } else { 0.0 };
    [
        EventKind::Param {
            id: param::track(index, param::VOLUME),
            value: mix.volume.clamp(0.0, 1.0),
        },
        EventKind::Param {
            id: param::track(index, param::MUTE),
            value: flag(mix.muted),
        },
        EventKind::Param {
            id: param::track(index, param::SOLO),
            value: flag(mix.solo),
        },
    ]
}

#[tauri::command]
pub async fn stems_play(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
) -> Result<Scheduled, String> {
    with_player(&stems, &audio, |active, engine| {
        schedule(engine, active.source, &[EventKind::Start])
    })
}

#[tauri::command]
pub async fn stems_pause(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
) -> Result<Scheduled, String> {
    with_player(&stems, &audio, |active, engine| {
        schedule(engine, active.source, &[EventKind::Stop])
    })
}

#[tauri::command]
pub async fn stems_seek(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
    seconds: f64,
) -> Result<Scheduled, String> {
    with_player(&stems, &audio, |active, engine| {
        schedule(engine, active.source, &[seek_event(active, seconds)])
    })
}

#[tauri::command]
pub async fn stems_set_speed(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
    speed: f32,
) -> Result<Scheduled, String> {
    with_player(&stems, &audio, |active, engine| {
        schedule(engine, active.source, &[speed_event(speed)])
    })
}

/// Sets the A-B loop in seconds, or clears it with `null`.
#[tauri::command]
pub async fn stems_set_loop(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
    range: Option<LoopSeconds>,
) -> Result<Scheduled, String> {
    with_player(&stems, &audio, |active, engine| {
        schedule(
            engine,
            active.source,
            &loop_events(loop_range(active, range)),
        )
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
) -> Result<Scheduled, String> {
    with_player(&stems, &audio, |active, engine| {
        if index >= active.tracks.min(MAX_TRACKS) {
            return Err("no such track".into());
        }
        let mix = TrackMixIntent {
            volume,
            muted,
            solo,
        };
        schedule(engine, active.source, &track_events(index, mix))
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
    let rendered_frame = engine.stats().frames_rendered;
    let heard_at_ms = engine.heard_at_epoch_ms(rendered_frame);
    Ok(Some(Transport {
        song_id: active.song_id.clone(),
        playing: active.shared.playing(),
        ended: active.shared.ended(),
        position_secs: active.shared.position() / f64::from(active.song_rate.max(1)),
        heard_at_ms,
        rendered_frame,
    }))
}

/// Events that put the player where `intent` says, in the order the source
/// must apply them on one frame: seek, speed, loop, track mixes, then
/// start or stop.
fn restore_events(active: &ActivePlayer, intent: &PlayerIntent) -> Vec<EventKind> {
    let mut events = Vec::new();
    if let Some(seconds) = intent.seek {
        events.push(seek_event(active, seconds));
    }
    events.push(speed_event(intent.speed));
    events.extend(loop_events(loop_range(active, intent.looping)));
    for (index, mix) in intent.tracks.iter().enumerate().take(active.tracks) {
        events.extend(track_events(index, *mix));
    }
    events.push(if intent.playing {
        EventKind::Start
    } else {
        EventKind::Stop
    });
    events
}

/// After `audio://output-changed`: drops the events still pending for the
/// old output and schedules the screen's whole intent again, under one
/// engine lock so no other command lands in between. The player itself
/// restarts at the new rate in `Source::prepare`.
#[tauri::command]
pub async fn stems_restore(
    stems: State<'_, StemService>,
    audio: State<'_, AudioService>,
    intent: PlayerIntent,
) -> Result<Option<Scheduled>, String> {
    let player = lock(&stems.player);
    let Some(active) = player.as_ref() else {
        return Ok(None);
    };
    let mut engine = lock(audio.engine());
    let from = engine.handle().frames_rendered();
    engine
        .handle()
        .cancel(active.source, from)
        .map_err(|error| error.to_string())?;
    engine
        .handle()
        .set_gain(active.source, intent.master.clamp(0.0, 1.0))
        .map_err(|error| error.to_string())?;
    schedule(&mut engine, active.source, &restore_events(active, &intent)).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{OfflineEngine, StreamFormat};
    use crate::stems::{MemoryTrack, StemPlayer};

    fn ramp_player() -> (OfflineEngine, ActivePlayer, crate::stems::Feeder) {
        let ramp: Vec<f32> = (0..96_000).map(|i| i as f32 * 1e-6).collect();
        let player = StemPlayer::new(vec![Box::new(MemoryTrack::mono(48_000, &ramp))], 0).unwrap();
        let mut engine = OfflineEngine::new(StreamFormat::new(48_000, 2));
        let source = engine
            .handle
            .add_source(Box::new(player.source), 1.0)
            .unwrap();
        let active = ActivePlayer {
            song_id: "s1".into(),
            source,
            shared: player.shared,
            song_rate: player.song_rate,
            frames: player.frames,
            tracks: 1,
            _feeder: None,
        };
        (engine, active, player.feeder)
    }

    fn render(
        engine: &mut OfflineEngine,
        feeder: &mut crate::stems::Feeder,
        frames: usize,
    ) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        for chunk in out.chunks_mut(256) {
            feeder.fill();
            engine.render_callback(chunk);
        }
        out.as_chunks::<2>().0.iter().map(|f| f[0]).collect()
    }

    fn schedule_now(engine: &mut OfflineEngine, active: &ActivePlayer, events: &[EventKind]) {
        let frame = engine.handle.frames_rendered() + 64;
        for event in events {
            engine
                .handle
                .schedule(frame, active.source, *event)
                .unwrap();
        }
    }

    fn intent(playing: bool) -> PlayerIntent {
        PlayerIntent {
            playing,
            seek: None,
            speed: 1.0,
            looping: None,
            tracks: vec![TrackMixIntent {
                volume: 1.0,
                muted: false,
                solo: false,
            }],
            master: 1.0,
        }
    }

    /// Cancels like the command, then schedules the intent.
    fn restore(engine: &mut OfflineEngine, active: &ActivePlayer, intent: &PlayerIntent) {
        let from = engine.handle.frames_rendered();
        engine.handle.cancel(active.source, from).unwrap();
        schedule_now(engine, active, &restore_events(active, intent));
    }

    #[test]
    fn restore_reapplies_a_cancelled_stop() {
        let (mut engine, active, mut feeder) = ramp_player();
        schedule_now(&mut engine, &active, &[EventKind::Start]);
        render(&mut engine, &mut feeder, 4_800);
        // Pause is pending when the output changes; the restore sends Stop.
        let pause_at = engine.handle.frames_rendered() + 2_000;
        engine
            .handle
            .schedule(pause_at, active.source, EventKind::Stop)
            .unwrap();
        restore(&mut engine, &active, &intent(false));
        render(&mut engine, &mut feeder, 4_800);
        assert!(!active.shared.playing());
    }

    #[test]
    fn restore_reapplies_start_seek_speed_loop_and_mute() {
        let (mut engine, active, mut feeder) = ramp_player();
        let mut wanted = intent(true);
        wanted.seek = Some(1.0);
        wanted.speed = 0.75;
        wanted.looping = Some(LoopSeconds {
            start: 1.0,
            end: 1.5,
        });
        wanted.tracks[0].muted = true;
        restore(&mut engine, &active, &wanted);
        let muted = render(&mut engine, &mut feeder, 9_600);
        assert!(active.shared.playing(), "started");
        // The mute glides in over ~5 ms; after that the track is silent.
        assert!(muted[4_800..].iter().all(|s| s.abs() < 1e-6), "muted");
        let position = active.shared.position();
        assert!(
            (48_000.0..72_000.0).contains(&position),
            "inside the loop: {position}"
        );

        // Unmuted, the speed shows in how far the position moves.
        wanted.seek = None;
        wanted.looping = None;
        wanted.tracks[0].muted = false;
        restore(&mut engine, &active, &wanted);
        render(&mut engine, &mut feeder, 4_800);
        let before = active.shared.position();
        let heard = render(&mut engine, &mut feeder, 9_600);
        let moved = active.shared.position() - before;
        assert!((moved - 0.75 * 9_600.0).abs() < 300.0, "{moved}");
        assert!(heard.iter().any(|s| s.abs() > 0.0), "audible again");
    }
}

//! Tauri-facing metronome: one worker thread that owns playback, the
//! commands the screen calls, and the channel that tells the screen when
//! each beat is heard.
//!
//! Beats are announced as soon as they are scheduled (about 250 ms early),
//! with the time they will be heard from `AudioEngine::heard_at_epoch_ms`,
//! so the screen can light them on time instead of reacting late.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;

use crate::app_audio::AudioService;
use crate::audio::{AudioEngine, FramePos, SourceId};

use super::pattern::{Accent, MetronomeSettings, tenths_to_bpm};
use super::scheduler::{Announcement, START_LEAD_SECS, Scheduler};
use super::source::MetronomeSource;
use super::store::{MetronomeStore, Preset, PresetError};

/// How often the worker tops up the schedule while playing.
const TICK: Duration = Duration::from_millis(10);
/// Last settings are written this long after the latest change.
const SAVE_DELAY: Duration = Duration::from_millis(500);
/// Mixer gain of the metronome source.
const SOURCE_GAIN: f32 = 0.9;

/// Sent to the screen over the channel from `metronome_subscribe`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MetronomeMessage {
    /// A main beat, heard at `heard_at_ms` (Unix epoch ms).
    #[serde(rename_all = "camelCase")]
    Beat {
        frame: FramePos,
        heard_at_ms: f64,
        bar: u64,
        beat: u8,
        beats: u8,
        accent: Accent,
        bpm: f64,
    },
    /// A polyrhythm click.
    #[serde(rename_all = "camelCase")]
    Pulse {
        frame: FramePos,
        heard_at_ms: f64,
        pulse: u8,
        pulses: u8,
    },
    /// Beats and pulses announced at or after `from_frame` will not play.
    #[serde(rename_all = "camelCase")]
    Reset { from_frame: FramePos },
    /// Playback ended by itself (the tempo trainer finished, or the click
    /// source is unavailable).
    Stopped,
}

/// Saved settings, presets and whether the metronome is playing.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetronomeState {
    pub settings: MetronomeSettings,
    pub presets: Vec<Preset>,
    pub playing: bool,
}

enum Command {
    Start(MetronomeSettings),
    Update(MetronomeSettings),
    Stop,
    Subscribe(Channel<MetronomeMessage>),
}

pub struct MetronomeService {
    commands: Mutex<mpsc::Sender<Command>>,
    store: Arc<MetronomeStore>,
    playing: Arc<AtomicBool>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl MetronomeService {
    /// Loads the saved data from `data_dir`, attaches the click source to
    /// the engine and starts the worker thread.
    pub fn start(data_dir: &Path, audio: &AudioService) -> Self {
        let store = Arc::new(MetronomeStore::load(data_dir));
        let engine = Arc::clone(audio.engine());
        let source = lock(&engine)
            .handle()
            .add_source(Box::new(MetronomeSource::new()), SOURCE_GAIN)
            .map_err(|error| log::error!("metronome: cannot attach the click source: {error}"))
            .ok();
        let playing = Arc::new(AtomicBool::new(false));
        let (commands, inbox) = mpsc::channel();
        let worker = Worker {
            engine,
            source,
            store: Arc::clone(&store),
            playing: Arc::clone(&playing),
            subscriber: None,
            session: None,
            save_due: None,
        };
        std::thread::Builder::new()
            .name("metronome".into())
            .spawn(move || worker.run(inbox))
            .expect("spawn metronome worker");
        Self {
            commands: Mutex::new(commands),
            store,
            playing,
        }
    }

    fn send(&self, command: Command) -> Result<(), String> {
        lock(&self.commands)
            .send(command)
            .map_err(|_| "the metronome worker stopped".to_owned())
    }
}

struct Session {
    scheduler: Scheduler,
    /// Output generation the schedule was computed for.
    generation: u64,
    /// Announcements waiting for their heard time (no timing yet).
    pending: Vec<Announcement>,
}

struct Worker {
    engine: Arc<Mutex<AudioEngine>>,
    source: Option<SourceId>,
    store: Arc<MetronomeStore>,
    playing: Arc<AtomicBool>,
    subscriber: Option<Channel<MetronomeMessage>>,
    session: Option<Session>,
    save_due: Option<Instant>,
}

fn announcement_frame(announcement: &Announcement) -> FramePos {
    match announcement {
        Announcement::Beat(mark) => mark.frame,
        Announcement::Pulse(mark) => mark.frame,
        Announcement::Reset { from_frame } => *from_frame,
    }
}

fn to_message(announcement: &Announcement, heard_at_ms: f64) -> MetronomeMessage {
    match *announcement {
        Announcement::Beat(mark) => MetronomeMessage::Beat {
            frame: mark.frame,
            heard_at_ms,
            bar: mark.bar,
            beat: mark.beat,
            beats: mark.beats,
            accent: mark.accent,
            bpm: tenths_to_bpm(mark.bpm_tenths),
        },
        Announcement::Pulse(mark) => MetronomeMessage::Pulse {
            frame: mark.frame,
            heard_at_ms,
            pulse: mark.pulse,
            pulses: mark.pulses,
        },
        Announcement::Reset { from_frame } => MetronomeMessage::Reset { from_frame },
    }
}

impl Worker {
    fn run(mut self, inbox: mpsc::Receiver<Command>) {
        loop {
            let timeout = if self.session.is_some() {
                TICK
            } else if let Some(due) = self.save_due {
                due.saturating_duration_since(Instant::now())
            } else {
                Duration::from_secs(3_600)
            };
            match inbox.recv_timeout(timeout) {
                Ok(command) => self.handle(command),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.save();
                    return;
                }
            }
            while let Ok(command) = inbox.try_recv() {
                self.handle(command);
            }
            self.tick();
            if self.save_due.is_some_and(|due| Instant::now() >= due) {
                self.save();
            }
        }
    }

    fn remember(&mut self, settings: &MetronomeSettings) {
        self.store.set_settings(settings.clone());
        self.save_due = Some(Instant::now() + SAVE_DELAY);
    }

    fn save(&mut self) {
        self.save_due = None;
        if let Err(error) = self.store.save() {
            log::warn!("metronome: cannot save the last settings: {error}");
        }
    }

    fn send(&mut self, messages: Vec<MetronomeMessage>) {
        let Some(channel) = &self.subscriber else {
            return;
        };
        if messages
            .into_iter()
            .any(|message| channel.send(message).is_err())
        {
            self.subscriber = None;
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Start(settings) => {
                let settings = settings.normalized();
                self.remember(&settings);
                self.stop_session();
                let Some(source) = self.source else {
                    self.send(vec![MetronomeMessage::Stopped]);
                    return;
                };
                let mut engine = lock(&self.engine);
                let generation = engine.generation();
                let handle = engine.handle();
                let sample_rate = handle.format().sample_rate;
                let start = handle.frame_after(START_LEAD_SECS);
                self.session = Some(Session {
                    scheduler: Scheduler::new(source, &settings, sample_rate, start),
                    generation,
                    pending: Vec::new(),
                });
                self.playing.store(true, Ordering::Release);
            }
            Command::Update(settings) => {
                let settings = settings.normalized();
                self.remember(&settings);
                let Some(session) = &mut self.session else {
                    return;
                };
                let mut announced = Vec::new();
                {
                    let mut engine = lock(&self.engine);
                    let handle = engine.handle();
                    let now = handle.frames_rendered();
                    if let Err(error) =
                        session
                            .scheduler
                            .update(handle, now, &settings, &mut announced)
                    {
                        log::warn!("metronome: cannot apply the change: {error}");
                    }
                }
                let messages = Self::queue(session, announced);
                self.send(messages);
            }
            Command::Stop => {
                self.stop_session();
                self.save();
            }
            Command::Subscribe(channel) => {
                self.subscriber = Some(channel);
                if let Some(session) = &mut self.session {
                    let now = lock(&self.engine).handle().frames_rendered();
                    session.pending = session.scheduler.upcoming(now);
                }
            }
        }
    }

    /// Ends playback the screen asked to end (no `Stopped` message: the
    /// screen already knows, and a late one could undo a quick restart).
    fn stop_session(&mut self) {
        let Some(mut session) = self.session.take() else {
            return;
        };
        {
            let mut engine = lock(&self.engine);
            let handle = engine.handle();
            let now = handle.frames_rendered();
            if let Err(error) = session.scheduler.stop(handle, now) {
                log::warn!("metronome: cannot cancel the schedule: {error}");
            }
        }
        self.playing.store(false, Ordering::Release);
    }

    /// Moves announcements into the session's pending list; resets go out
    /// at once and drop what they cancel.
    fn queue(session: &mut Session, announced: Vec<Announcement>) -> Vec<MetronomeMessage> {
        let mut messages = Vec::new();
        for announcement in announced {
            if let Announcement::Reset { from_frame } = announcement {
                session
                    .pending
                    .retain(|pending| announcement_frame(pending) < from_frame);
                messages.push(to_message(&announcement, 0.0));
            } else {
                session.pending.push(announcement);
            }
        }
        messages
    }

    fn tick(&mut self) {
        let Some(session) = &mut self.session else {
            return;
        };
        let mut messages;
        let done;
        {
            let mut engine = lock(&self.engine);
            let generation = engine.generation();
            let sample_rate = engine.format().sample_rate;
            let now = engine.handle().frames_rendered();
            let mut announced = Vec::new();
            if generation != session.generation {
                // The output was reopened (`audio://output-changed`): what is
                // pending was placed for the old stream and sample rate.
                session.pending.clear();
                match session
                    .scheduler
                    .retime(engine.handle(), now, sample_rate, &mut announced)
                {
                    Ok(()) => session.generation = generation,
                    Err(error) => log::warn!("metronome: cannot reschedule: {error}"),
                }
            }
            if let Err(error) = session.scheduler.pump(engine.handle(), now, &mut announced) {
                log::warn!("metronome: cannot schedule: {error}");
            }
            messages = Self::queue(session, announced);
            // Heard times are known once the current output has rendered.
            session.pending.retain(|pending| {
                match engine.heard_at_epoch_ms(announcement_frame(pending)) {
                    Some(heard_at_ms) => {
                        messages.push(to_message(pending, heard_at_ms));
                        false
                    }
                    None => true,
                }
            });
            done = session.scheduler.is_done(now);
        }
        self.send(messages);
        if done {
            self.session = None;
            self.playing.store(false, Ordering::Release);
            self.send(vec![MetronomeMessage::Stopped]);
        }
    }
}

// ---- commands ------------------------------------------------------------------

#[tauri::command]
pub async fn metronome_load(
    metronome: State<'_, MetronomeService>,
) -> Result<MetronomeState, String> {
    let data = metronome.store.get();
    Ok(MetronomeState {
        settings: data.settings,
        presets: data.presets,
        playing: metronome.playing.load(Ordering::Acquire),
    })
}

/// Sends beats, pulses, resets and stops to `on_event` (replaces the
/// previous subscriber; beats already scheduled are sent again).
#[tauri::command]
pub async fn metronome_subscribe(
    metronome: State<'_, MetronomeService>,
    on_event: Channel<MetronomeMessage>,
) -> Result<(), String> {
    metronome.send(Command::Subscribe(on_event))
}

#[tauri::command]
pub async fn metronome_start(
    metronome: State<'_, MetronomeService>,
    settings: MetronomeSettings,
) -> Result<(), String> {
    metronome.send(Command::Start(settings))
}

/// Applies `settings` from the next beat while playing, and remembers them.
#[tauri::command]
pub async fn metronome_update(
    metronome: State<'_, MetronomeService>,
    settings: MetronomeSettings,
) -> Result<(), String> {
    metronome.send(Command::Update(settings))
}

#[tauri::command]
pub async fn metronome_stop(metronome: State<'_, MetronomeService>) -> Result<(), String> {
    metronome.send(Command::Stop)
}

#[tauri::command]
pub async fn metronome_save_preset(
    metronome: State<'_, MetronomeService>,
    name: String,
    settings: MetronomeSettings,
) -> Result<Vec<Preset>, PresetError> {
    metronome.store.save_preset(&name, settings)
}

#[tauri::command]
pub async fn metronome_delete_preset(
    metronome: State<'_, MetronomeService>,
    name: String,
) -> Result<Vec<Preset>, PresetError> {
    metronome.store.delete_preset(&name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metronome::timeline::{BeatMark, PulseMark};

    #[test]
    fn messages_use_the_screen_field_names() {
        let beat = to_message(
            &Announcement::Beat(BeatMark {
                frame: 48_000,
                index: 4,
                bar: 1,
                beat: 0,
                beats: 4,
                accent: Accent::Strong,
                bpm_tenths: 1_205,
            }),
            1_000.5,
        );
        assert_eq!(
            serde_json::to_value(beat).unwrap(),
            serde_json::json!({
                "type": "beat", "frame": 48_000, "heardAtMs": 1_000.5, "bar": 1,
                "beat": 0, "beats": 4, "accent": "strong", "bpm": 120.5
            })
        );
        let pulse = to_message(
            &Announcement::Pulse(PulseMark {
                frame: 10,
                pulse: 2,
                pulses: 3,
            }),
            5.0,
        );
        assert_eq!(
            serde_json::to_value(pulse).unwrap(),
            serde_json::json!({ "type": "pulse", "frame": 10, "heardAtMs": 5.0, "pulse": 2, "pulses": 3 })
        );
        assert_eq!(
            serde_json::to_value(MetronomeMessage::Reset { from_frame: 7 }).unwrap(),
            serde_json::json!({ "type": "reset", "fromFrame": 7 })
        );
        assert_eq!(
            serde_json::to_value(MetronomeMessage::Stopped).unwrap(),
            serde_json::json!({ "type": "stopped" })
        );
    }
}

//! Tauri-facing audio service: owns the live engine, exposes commands, and
//! forwards "this sound is heard at" reports to the webview over channels.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::{Duration, Instant};

use rtrb::Consumer;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::audio::{
    AudioEngine, ClickSource, DeviceInfo, DeviceSnapshot, Direction, EngineConfig, EngineStats,
    EventKind, FramePos, OpenedOutput, OutputEnv, OutputInput, OutputManager, OutputRequest,
    OutputStreamInfo, RenderedEvent, SelectError, SourceId, StreamErrorKind, list_devices,
    output_device_sample_rate,
};
use crate::settings::SettingsStore;

/// Event name for device list changes (devices plugged/unplugged, default changed).
pub const DEVICES_CHANGED_EVENT: &str = "audio://devices-changed";
/// Event name for every successful output (re)open: manual switch, following
/// the system default, or recovery. Payload: [`OutputChanged`].
pub const OUTPUT_CHANGED_EVENT: &str = "audio://output-changed";

/// Buffer size the app asks every output device for.
const TARGET_BUFFER_FRAMES: u32 = 64;
const NOTIFY_POLL: Duration = Duration::from_millis(2);
const DEVICE_POLL: Duration = Duration::from_secs(1);
const SELECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Lead time for the test sound so it is never late.
const TEST_SOUND_LEAD_SECS: f64 = 0.03;

/// A rendered `notify` event, with the wall-clock time it is heard.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackEvent {
    pub source_id: SourceId,
    pub scheduled_frame: FramePos,
    pub rendered_frame: FramePos,
    pub kind: EventKind,
    /// Output generation it was rendered in (always the current one when sent).
    pub generation: u64,
    /// Unix-epoch ms. In the webview: `heardAtMs - performance.timeOrigin`
    /// is the `performance.now()` value at which the sound is heard.
    pub heard_at_ms: f64,
}

/// Payload of [`OUTPUT_CHANGED_EVENT`]. Frames scheduled for an older
/// generation were computed at the old sample rate and should be rescheduled.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputChanged {
    pub generation: u64,
    pub device_id: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_frames: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStatus {
    pub output: Option<OutputStreamInfo>,
    pub output_latency_ms: Option<f64>,
    pub stats: EngineStats,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledSound {
    pub frame: FramePos,
    pub heard_at_ms: Option<f64>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceDirection {
    Input,
    Output,
}

type Subscribers = Arc<Mutex<Vec<Channel<PlaybackEvent>>>>;

/// Work sent to the output manager thread.
enum ManagerRequest {
    Select {
        device_id: Option<String>,
        reply: mpsc::Sender<Result<(), SelectError>>,
    },
}

pub struct AudioService {
    engine: Arc<Mutex<AudioEngine>>,
    subscribers: Subscribers,
    test_sound: SourceId,
    /// The only way to change the output: requests to the manager thread.
    requests: mpsc::Sender<ManagerRequest>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl AudioService {
    /// Creates the engine and starts the helper threads. The output manager
    /// thread opens the preferred output (or the system default) on its
    /// first tick and owns every later open.
    pub fn start(app: &AppHandle, settings: &SettingsStore) -> Self {
        let (mut engine, notifications) = AudioEngine::new(EngineConfig::default());
        let test_sound = engine
            .handle()
            .add_source(Box::new(ClickSource::new(1_000.0, 0.04)), 0.4)
            .expect("fresh engine queue has room");

        let engine = Arc::new(Mutex::new(engine));
        let subscribers: Subscribers = Arc::default();
        spawn_notifier(Arc::clone(&engine), Arc::clone(&subscribers), notifications);
        let requests = spawn_output_manager(
            app.clone(),
            Arc::clone(&engine),
            settings.get().output_device_id,
        );

        Self {
            engine,
            subscribers,
            test_sound,
            requests,
        }
    }

    pub fn engine(&self) -> &Arc<Mutex<AudioEngine>> {
        &self.engine
    }

    fn status(&self) -> AudioStatus {
        let engine = lock(&self.engine);
        AudioStatus {
            output: engine.output_info().cloned(),
            output_latency_ms: engine.output_latency().map(|latency| latency.millis()),
            stats: engine.stats(),
        }
    }
}

fn opened_output(info: &OutputStreamInfo, generation: u64) -> OpenedOutput {
    OpenedOutput {
        device_id: info.device_id.clone(),
        sample_rate: info.sample_rate,
        channels: info.channels,
        buffer_frames: info.buffer_frames,
        generation,
    }
}

/// The real world for [`OutputManager`]: the live engine, cpal devices, the
/// settings file and webview events.
struct TauriOutputEnv {
    app: AppHandle,
    engine: Arc<Mutex<AudioEngine>>,
}

impl OutputEnv for TauriOutputEnv {
    fn open(&mut self, device: Option<&str>) -> Result<OpenedOutput, String> {
        let mut engine = lock(&self.engine);
        let info = engine
            .open_output(&OutputRequest {
                device_id: device.map(str::to_owned),
                target_buffer_frames: TARGET_BUFFER_FRAMES,
            })
            .map_err(|error| error.to_string())?;
        Ok(opened_output(&info, engine.generation()))
    }

    fn current(&self) -> Option<OpenedOutput> {
        let engine = lock(&self.engine);
        engine
            .output_info()
            .map(|info| opened_output(info, engine.generation()))
    }

    fn stream_error(&self) -> Option<StreamErrorKind> {
        lock(&self.engine).stream_error()
    }

    fn device_rate(&self, device_id: &str) -> Option<u32> {
        output_device_sample_rate(device_id)
    }

    fn snapshot(&mut self) -> DeviceSnapshot {
        DeviceSnapshot::capture()
    }

    fn save_preferred(&mut self, preferred: Option<&str>) -> Result<(), String> {
        let store = self.app.state::<SettingsStore>();
        let mut settings = store.get();
        settings.output_device_id = preferred.map(str::to_owned);
        store
            .save(settings)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn output_changed(&mut self, output: &OpenedOutput) {
        let _ = self.app.emit(
            OUTPUT_CHANGED_EVENT,
            OutputChanged {
                generation: output.generation,
                device_id: output.device_id.clone(),
                sample_rate: output.sample_rate,
                channels: output.channels,
                buffer_frames: output.buffer_frames,
            },
        );
        log_latency_when_ready(Arc::clone(&self.engine));
    }

    fn devices_changed(&mut self, snapshot: &DeviceSnapshot) {
        let _ = self.app.emit(DEVICES_CHANGED_EVENT, snapshot);
    }
}

/// Starts the thread that owns the output. It handles requests in arrival
/// order and ticks once per [`DEVICE_POLL`]; both run on this one thread, so
/// a tick can never act on a state that a request has already replaced.
fn spawn_output_manager(
    app: AppHandle,
    engine: Arc<Mutex<AudioEngine>>,
    preferred: Option<String>,
) -> mpsc::Sender<ManagerRequest> {
    let (requests, inbox) = mpsc::channel::<ManagerRequest>();
    std::thread::Builder::new()
        .name("audio-output".into())
        .spawn(move || {
            let mut manager = OutputManager::new(TauriOutputEnv { app, engine }, preferred);
            let mut next_tick = Instant::now();
            loop {
                let now = Instant::now();
                if now >= next_tick {
                    manager.handle(OutputInput::Tick, now);
                    next_tick = now + DEVICE_POLL;
                    continue;
                }
                match inbox.recv_timeout(next_tick - now) {
                    Ok(ManagerRequest::Select { device_id, reply }) => {
                        let result = manager.select(device_id, Instant::now());
                        let _ = reply.send(result);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        })
        .expect("spawn audio output manager");
    requests
}

/// Logs the stream format and the measured output latency once the stream
/// has run a few callbacks. Polls with short locks so the UI never waits.
fn log_latency_when_ready(engine: Arc<Mutex<AudioEngine>>) {
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            let engine = lock(&engine);
            let Some(info) = engine.output_info() else {
                return;
            };
            if engine.stats().callbacks < 4 {
                continue;
            }
            if let Some(latency) = engine.output_latency() {
                log::info!(
                    "audio output: device=\"{}\" sample_rate={} channels={} buffer_requested={:?} buffer_reported={:?} callback_frames={} output_latency_ms={:.2}",
                    info.device_name,
                    info.sample_rate,
                    info.channels,
                    info.requested_buffer_frames,
                    info.buffer_frames,
                    latency.callback_frames,
                    latency.millis()
                );
            }
            return;
        }
        log::warn!("audio output: no callbacks within 3 s");
    });
}

fn spawn_notifier(
    engine: Arc<Mutex<AudioEngine>>,
    subscribers: Subscribers,
    mut notifications: Consumer<RenderedEvent>,
) {
    std::thread::Builder::new()
        .name("audio-notifier".into())
        .spawn(move || {
            let mut batch = Vec::with_capacity(64);
            loop {
                std::thread::sleep(NOTIFY_POLL);
                while let Ok(event) = notifications.pop() {
                    batch.push(event);
                }
                if batch.is_empty() {
                    if notifications.is_abandoned() {
                        return;
                    }
                    continue;
                }
                let events: Vec<PlaybackEvent> = {
                    let engine = lock(&engine);
                    // Reports from an older output generation describe a stream
                    // that is gone; their heard time cannot be known, so they
                    // are dropped instead of being mapped onto the new clock.
                    batch
                        .drain(..)
                        .filter_map(|event| {
                            let heard_at_ms = engine.event_heard_at_epoch_ms(&event)?;
                            Some(PlaybackEvent {
                                source_id: event.target,
                                scheduled_frame: event.scheduled_frame,
                                rendered_frame: event.rendered_frame,
                                kind: event.kind,
                                generation: event.generation,
                                heard_at_ms,
                            })
                        })
                        .collect()
                };
                if events.is_empty() {
                    continue;
                }
                let mut subscribers = lock(&subscribers);
                subscribers
                    .retain(|channel| events.iter().all(|e| channel.send(e.clone()).is_ok()));
            }
        })
        .expect("spawn audio notifier");
}

// ---- commands ------------------------------------------------------------------

#[tauri::command]
pub fn audio_status(audio: State<'_, AudioService>) -> AudioStatus {
    audio.status()
}

#[tauri::command]
pub fn audio_list_devices(direction: DeviceDirection) -> Result<Vec<DeviceInfo>, String> {
    let direction = match direction {
        DeviceDirection::Input => Direction::Input,
        DeviceDirection::Output => Direction::Output,
    };
    list_devices(direction).map_err(|error| error.to_string())
}

/// Why choosing an output failed. The choice is in effect either way.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum OutputSelectError {
    /// The device could not be opened; the system default plays meanwhile
    /// and the device is retried.
    OpenFailed { message: String },
    /// The output switched but the choice could not be saved to disk.
    SaveFailed { message: String },
    /// The output manager did not answer.
    Unavailable,
}

/// Asks the output manager to switch to `device_id` (`None` = system
/// default) and to save the choice; returns the new status.
#[tauri::command]
pub fn audio_set_output_device(
    audio: State<'_, AudioService>,
    device_id: Option<String>,
) -> Result<AudioStatus, OutputSelectError> {
    let (reply, response) = mpsc::channel();
    audio
        .requests
        .send(ManagerRequest::Select { device_id, reply })
        .map_err(|_| OutputSelectError::Unavailable)?;
    match response.recv_timeout(SELECT_TIMEOUT) {
        Ok(Ok(())) => Ok(audio.status()),
        Ok(Err(SelectError::OpenFailed(message))) => Err(OutputSelectError::OpenFailed { message }),
        Ok(Err(SelectError::SaveFailed(message))) => Err(OutputSelectError::SaveFailed { message }),
        Err(_) => Err(OutputSelectError::Unavailable),
    }
}

/// Registers a channel that receives every rendered `notify` event.
#[tauri::command]
pub fn audio_subscribe(audio: State<'_, AudioService>, on_event: Channel<PlaybackEvent>) {
    lock(&audio.subscribers).push(on_event);
}

/// Unix-epoch ms at which `frame` will be heard on the current output.
#[tauri::command]
pub fn audio_heard_at(audio: State<'_, AudioService>, frame: FramePos) -> Option<f64> {
    lock(audio.engine()).heard_at_epoch_ms(frame)
}

/// Plays the output test sound shortly from now. Subscribers get a
/// [`PlaybackEvent`] for it once rendered.
#[tauri::command]
pub fn audio_play_test_sound(audio: State<'_, AudioService>) -> Result<ScheduledSound, String> {
    let mut engine = lock(audio.engine());
    let frame = engine.handle().frame_after(TEST_SOUND_LEAD_SECS);
    engine
        .handle()
        .schedule_notify(
            frame,
            audio.test_sound,
            EventKind::Trigger {
                note: 0,
                velocity: 1.0,
            },
        )
        .map_err(|error| error.to_string())?;
    Ok(ScheduledSound {
        frame,
        heard_at_ms: engine.heard_at_epoch_ms(frame),
    })
}

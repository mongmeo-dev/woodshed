//! Tauri-facing audio service: owns the live engine, exposes commands, and
//! forwards "this sound is heard at" reports to the webview over channels.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rtrb::Consumer;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::audio::{
    AudioEngine, ClickSource, DeviceInfo, DeviceMonitor, DeviceSnapshot, Direction, EngineConfig,
    EngineStats, EventKind, FramePos, OutputRequest, OutputStreamInfo, RenderedEvent, SourceId,
    list_devices, spawn_device_monitor,
};
use crate::settings::SettingsStore;

/// Event name for device list changes.
pub const DEVICES_CHANGED_EVENT: &str = "audio://devices-changed";

/// Buffer size the app asks every output device for.
const TARGET_BUFFER_FRAMES: u32 = 64;
const NOTIFY_POLL: Duration = Duration::from_millis(2);
const DEVICE_POLL: Duration = Duration::from_secs(2);
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
    /// Unix-epoch ms. In the webview: `heardAtMs - performance.timeOrigin`
    /// is the `performance.now()` value at which the sound is heard.
    pub heard_at_ms: Option<f64>,
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

pub struct AudioService {
    engine: Arc<Mutex<AudioEngine>>,
    subscribers: Subscribers,
    test_sound: SourceId,
    _monitor: DeviceMonitor,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl AudioService {
    /// Creates the engine, opens the preferred output (falling back to the
    /// system default) and starts the helper threads.
    pub fn start(app: &AppHandle, settings: &SettingsStore) -> Self {
        let (mut engine, notifications) = AudioEngine::new(EngineConfig::default());
        let test_sound = engine
            .handle()
            .add_source(Box::new(ClickSource::new(1_000.0, 0.04)), 0.4)
            .expect("fresh engine queue has room");
        open_preferred(&mut engine, settings.get().output_device_id.as_deref());

        let engine = Arc::new(Mutex::new(engine));
        let subscribers: Subscribers = Arc::default();
        log_latency_when_ready(Arc::clone(&engine));
        spawn_notifier(Arc::clone(&engine), Arc::clone(&subscribers), notifications);
        let monitor = watch_devices(app.clone(), Arc::clone(&engine));

        Self {
            engine,
            subscribers,
            test_sound,
            _monitor: monitor,
        }
    }

    pub fn engine(&self) -> &Arc<Mutex<AudioEngine>> {
        &self.engine
    }

    fn status(&self) -> AudioStatus {
        let engine = lock(&self.engine);
        status_of(&engine)
    }
}

fn status_of(engine: &AudioEngine) -> AudioStatus {
    AudioStatus {
        output: engine.output_info().cloned(),
        output_latency_ms: engine.output_latency().map(|latency| latency.millis()),
        stats: engine.stats(),
    }
}

/// Opens `preferred`, or the default output if that fails.
fn open_preferred(engine: &mut AudioEngine, preferred: Option<&str>) {
    let request = |device_id: Option<&str>| OutputRequest {
        device_id: device_id.map(str::to_owned),
        target_buffer_frames: TARGET_BUFFER_FRAMES,
    };
    if let Some(id) = preferred {
        match engine.open_output(&request(Some(id))) {
            Ok(_) => return,
            Err(error) => log::warn!("cannot open output {id}, using the default: {error}"),
        }
    }
    if let Err(error) = engine.open_output(&request(None)) {
        log::error!("cannot open the default output: {error}");
    }
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
                    batch
                        .drain(..)
                        .map(|event| PlaybackEvent {
                            source_id: event.target,
                            scheduled_frame: event.scheduled_frame,
                            rendered_frame: event.rendered_frame,
                            kind: event.kind,
                            heard_at_ms: engine.heard_at_epoch_ms(event.rendered_frame),
                        })
                        .collect()
                };
                let mut subscribers = lock(&subscribers);
                subscribers
                    .retain(|channel| events.iter().all(|e| channel.send(e.clone()).is_ok()));
            }
        })
        .expect("spawn audio notifier");
}

/// Emits device list changes to the webview and reopens the output when the
/// current device disappeared or the stream reported a fatal error.
fn watch_devices(app: AppHandle, engine: Arc<Mutex<AudioEngine>>) -> DeviceMonitor {
    spawn_device_monitor(DEVICE_POLL, move |snapshot: &DeviceSnapshot| {
        let _ = app.emit(DEVICES_CHANGED_EVENT, snapshot);
        let reopened = {
            let mut engine = lock(&engine);
            let stream_failed = engine
                .take_stream_error()
                .is_some_and(|error| error.needs_reopen());
            let device_gone = engine
                .output_info()
                .is_none_or(|info| !snapshot.outputs.contains(&info.device_id));
            if stream_failed || device_gone {
                log::info!("audio output device changed, reopening");
                let preferred = app.state::<SettingsStore>().get().output_device_id;
                open_preferred(&mut engine, preferred.as_deref());
            }
            stream_failed || device_gone
        };
        if reopened {
            log_latency_when_ready(Arc::clone(&engine));
        }
    })
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

/// Switches the output device (`None` = system default) and returns the new
/// status. The caller persists the choice in settings.
#[tauri::command]
pub fn audio_set_output_device(
    audio: State<'_, AudioService>,
    device_id: Option<String>,
) -> Result<AudioStatus, String> {
    {
        let mut engine = lock(audio.engine());
        engine
            .open_output(&OutputRequest {
                device_id,
                target_buffer_frames: TARGET_BUFFER_FRAMES,
            })
            .map_err(|error| error.to_string())?;
    }
    log_latency_when_ready(Arc::clone(audio.engine()));
    Ok(audio.status())
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

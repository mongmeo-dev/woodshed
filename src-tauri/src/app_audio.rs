//! Tauri-facing audio service: owns the live engine, exposes commands, and
//! forwards "this sound is heard at" reports to the webview over channels.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rtrb::Consumer;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::audio::{
    AudioEngine, AudioError, ClickSource, DeviceInfo, DeviceMonitor, DeviceSnapshot, Direction,
    EngineConfig, EngineStats, EventKind, FramePos, OutputRequest, OutputState, OutputStreamInfo,
    RenderedEvent, RetryBackoff, SourceId, list_devices, output_device_sample_rate, reopen_reason,
    spawn_device_monitor,
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
const RETRY_MIN: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);
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
        if open_preferred(&mut engine, settings.get().output_device_id.as_deref()).is_ok() {
            announce_output(app, &engine);
        }

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
fn open_preferred(
    engine: &mut AudioEngine,
    preferred: Option<&str>,
) -> Result<OutputStreamInfo, AudioError> {
    let request = |device_id: Option<&str>| OutputRequest {
        device_id: device_id.map(str::to_owned),
        target_buffer_frames: TARGET_BUFFER_FRAMES,
    };
    if let Some(id) = preferred {
        match engine.open_output(&request(Some(id))) {
            Ok(info) => return Ok(info),
            Err(error) => log::warn!("cannot open output {id}, using the default: {error}"),
        }
    }
    engine.open_output(&request(None)).inspect_err(|error| {
        log::error!("cannot open the default output: {error}");
    })
}

/// Emits [`OUTPUT_CHANGED_EVENT`] for the stream that is open now.
fn announce_output(app: &AppHandle, engine: &AudioEngine) {
    let Some(info) = engine.output_info() else {
        return;
    };
    let _ = app.emit(
        OUTPUT_CHANGED_EVENT,
        OutputChanged {
            generation: engine.generation(),
            device_id: info.device_id.clone(),
            sample_rate: info.sample_rate,
            channels: info.channels,
            buffer_frames: info.buffer_frames,
        },
    );
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

/// On every monitor tick: forwards device list changes to the webview, and
/// reopens the output when it is missing, failed, unplugged, no longer the
/// system default (in default mode), not the preferred device although that
/// is back, or running at a rate the device no longer uses. Failed attempts
/// back off (1 s doubling to 30 s).
fn watch_devices(app: AppHandle, engine: Arc<Mutex<AudioEngine>>) -> DeviceMonitor {
    let mut backoff = RetryBackoff::new(RETRY_MIN, RETRY_MAX);
    spawn_device_monitor(DEVICE_POLL, move |snapshot: &DeviceSnapshot, changed| {
        if changed {
            let _ = app.emit(DEVICES_CHANGED_EVENT, snapshot);
        }
        let (info, stream_error) = {
            let engine = lock(&engine);
            (engine.output_info().cloned(), engine.stream_error())
        };
        let preferred = app.state::<SettingsStore>().get().output_device_id;
        // Read outside the engine lock: it queries the device.
        let device_rate_now = info
            .as_ref()
            .and_then(|info| output_device_sample_rate(&info.device_id));
        let state = OutputState {
            current_device: info.as_ref().map(|info| info.device_id.as_str()),
            current_rate: info.as_ref().map(|info| info.sample_rate),
            device_rate_now,
            stream_error,
            preferred: preferred.as_deref(),
        };
        let Some(reason) = reopen_reason(&state, snapshot) else {
            return;
        };
        let now = Instant::now();
        if !backoff.ready(now) {
            return;
        }
        log::info!("audio output: reopening ({reason:?})");
        let mut guard = lock(&engine);
        match open_preferred(&mut guard, preferred.as_deref()) {
            Ok(_) => {
                backoff.succeeded(now);
                announce_output(&app, &guard);
                drop(guard);
                log_latency_when_ready(Arc::clone(&engine));
            }
            Err(_) => backoff.failed(now),
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

/// Switches the output device (`None` = system default), saves the choice in
/// settings and returns the new status.
#[tauri::command]
pub fn audio_set_output_device(
    app: AppHandle,
    audio: State<'_, AudioService>,
    settings: State<'_, SettingsStore>,
    device_id: Option<String>,
) -> Result<AudioStatus, String> {
    {
        let mut engine = lock(audio.engine());
        engine
            .open_output(&OutputRequest {
                device_id: device_id.clone(),
                target_buffer_frames: TARGET_BUFFER_FRAMES,
            })
            .map_err(|error| error.to_string())?;
        // Persist before releasing the engine, so the device watcher never
        // sees the new stream with the old preference and switches back.
        let mut saved = settings.get();
        saved.output_device_id = device_id;
        settings.save(saved).map_err(|error| error.to_string())?;
        announce_output(&app, &engine);
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

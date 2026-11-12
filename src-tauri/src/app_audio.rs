//! Tauri-facing audio service: owns the live engine, exposes commands, and
//! forwards "this sound is heard at" reports to the webview over channels.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rtrb::Consumer;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::audio::{
    AudioEngine, ClickSource, DeviceInfo, DeviceSnapshot, Direction, EngineConfig, EngineStats,
    EventKind, FramePos, ManagerRequest, OpenedOutput, OutputEnv, OutputManager, OutputRequest,
    OutputStreamInfo, RenderedEvent, SelectError, SourceId, StreamErrorKind, list_devices,
    output_device_sample_rate, run_output_manager,
};
use crate::settings::SettingsStore;
use crate::tuner::permission::{self, MicPermission};

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
    /// Output device chosen by the user and in effect (`None` = system
    /// default). It can differ from the device playing (fallback) and from
    /// the settings file (if saving failed).
    pub preferred_device_id: Option<String>,
    /// No output yet: macOS holds it until the user answers the microphone
    /// prompt (the output device, or the system default, has inputs).
    pub waiting_for_microphone: bool,
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
    /// The only way to change the output: requests to the manager thread.
    requests: mpsc::Sender<ManagerRequest>,
    /// Preference in effect, published by the manager after every input.
    preferred: Arc<Mutex<Option<String>>>,
    /// The manager holds the output back until the microphone prompt is
    /// answered (see [`OutputEnv::microphone_answered`]).
    waiting_for_microphone: Arc<AtomicBool>,
    manager_thread: Mutex<Option<JoinHandle<()>>>,
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
        let preferred = Arc::new(Mutex::new(settings.get().output_device_id));
        let waiting_for_microphone = Arc::new(AtomicBool::new(false));
        let (requests, manager_thread) = spawn_output_manager(
            app.clone(),
            Arc::clone(&engine),
            settings.get().output_device_id,
            Arc::clone(&preferred),
            Arc::clone(&waiting_for_microphone),
        );

        Self {
            engine,
            subscribers,
            test_sound,
            requests,
            preferred,
            waiting_for_microphone,
            manager_thread: Mutex::new(Some(manager_thread)),
        }
    }

    /// Stops the output manager thread and waits for it (app exit).
    pub fn shutdown(&self) {
        let _ = self.requests.send(ManagerRequest::Shutdown);
        if let Some(thread) = lock(&self.manager_thread).take() {
            let _ = thread.join();
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
            preferred_device_id: lock(&self.preferred).clone(),
            waiting_for_microphone: self.waiting_for_microphone.load(Ordering::Relaxed),
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
    /// The manager's own inbox, to wake it when the microphone prompt is
    /// answered.
    wake: mpsc::Sender<ManagerRequest>,
    waiting_for_microphone: Arc<AtomicBool>,
}

impl OutputEnv for TauriOutputEnv {
    fn open(&mut self, device: Option<&str>) -> Result<OpenedOutput, String> {
        // Build without holding the engine (device lookup and stream creation
        // can be slow); only the swap itself takes the lock.
        crate::startup::mark("first output open started");
        let starter = lock(&self.engine).output_starter();
        let pending = starter
            .build(&OutputRequest {
                device_id: device.map(str::to_owned),
                target_buffer_frames: TARGET_BUFFER_FRAMES,
            })
            .map_err(|error| error.to_string())?;
        let mut engine = lock(&self.engine);
        let info = engine
            .commit_output(pending)
            .map_err(|error| error.to_string())?;
        crate::startup::mark("first output open finished");
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
        crate::startup::mark("first device list started");
        let snapshot = DeviceSnapshot::capture();
        crate::startup::mark("first device list finished");
        snapshot
    }

    fn save_preferred(&mut self, preferred: Option<&str>) -> Result<(), String> {
        let store = self.app.state::<SettingsStore>();
        store
            .update(|settings| settings.output_device_id = preferred.map(str::to_owned))
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

    fn microphone_answered(&mut self) -> bool {
        permission::status() != MicPermission::Undetermined
    }

    fn waiting_for_microphone(&mut self, waiting: bool) {
        self.waiting_for_microphone
            .store(waiting, Ordering::Relaxed);
        if waiting {
            crate::startup::mark("output asked for microphone access");
            // The same system prompt the tuner shows; whichever asks first
            // shows it, and the answer wakes the manager right away.
            let wake = self.wake.clone();
            permission::request(move |_| {
                crate::startup::mark("microphone prompt answered");
                let _ = wake.send(ManagerRequest::Recheck);
            });
        }
    }
}

/// Starts the thread that owns the output (see [`run_output_manager`]).
fn spawn_output_manager(
    app: AppHandle,
    engine: Arc<Mutex<AudioEngine>>,
    preferred: Option<String>,
    published: Arc<Mutex<Option<String>>>,
    waiting_for_microphone: Arc<AtomicBool>,
) -> (mpsc::Sender<ManagerRequest>, JoinHandle<()>) {
    let (requests, inbox) = mpsc::channel::<ManagerRequest>();
    let wake = requests.clone();
    let thread = std::thread::Builder::new()
        .name("audio-output".into())
        .spawn(move || {
            let env = TauriOutputEnv {
                app,
                engine,
                wake,
                waiting_for_microphone,
            };
            let manager = OutputManager::new(env, preferred);
            run_output_manager(manager, inbox, DEVICE_POLL, published);
        })
        .expect("spawn audio output manager");
    (requests, thread)
}

/// Waits for a reply on the blocking pool, so the async command (and the
/// webview's IPC thread) never blocks on the manager.
async fn await_reply<T: Send + 'static>(
    response: mpsc::Receiver<T>,
    timeout: Duration,
) -> Option<T> {
    tauri::async_runtime::spawn_blocking(move || response.recv_timeout(timeout).ok())
        .await
        .ok()
        .flatten()
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
                crate::startup::mark("first output playing");
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

// Commands that touch the engine or devices are async so they never run on
// the webview's IPC (main) thread.

#[tauri::command]
pub async fn audio_status(audio: State<'_, AudioService>) -> Result<AudioStatus, String> {
    Ok(audio.status())
}

#[tauri::command]
pub async fn audio_list_devices(direction: DeviceDirection) -> Result<Vec<DeviceInfo>, String> {
    let direction = match direction {
        DeviceDirection::Input => Direction::Input,
        DeviceDirection::Output => Direction::Output,
    };
    list_devices(direction).map_err(|error| error.to_string())
}

/// Why choosing an output failed.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum OutputSelectError {
    /// The device could not be opened; the choice is in effect, the system
    /// default plays meanwhile and the device is retried.
    OpenFailed { message: String },
    /// The output switched but the choice could not be saved to disk; the
    /// choice is in effect until the app quits.
    SaveFailed { message: String },
    /// No answer in time. The request is dropped if it had not started; if it
    /// had, its result arrives through `audio://output-changed` and
    /// `audio_status().preferredDeviceId`.
    Unavailable,
}

/// The output and preference right after the manager handled the choice.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputSelection {
    pub output: Option<OutputChanged>,
    pub preferred_device_id: Option<String>,
}

/// Asks the output manager to switch to `device_id` (`None` = system
/// default) and to save the choice. Async: the wait runs on the blocking
/// pool, never on the webview's IPC thread.
#[tauri::command]
pub async fn audio_set_output_device(
    audio: State<'_, AudioService>,
    device_id: Option<String>,
) -> Result<OutputSelection, OutputSelectError> {
    let (reply, response) = mpsc::channel();
    let deadline = Instant::now() + SELECT_TIMEOUT;
    audio
        .requests
        .send(ManagerRequest::Select {
            device_id,
            deadline,
            reply,
        })
        .map_err(|_| OutputSelectError::Unavailable)?;
    match await_reply(response, SELECT_TIMEOUT).await {
        Some(Ok(outcome)) => Ok(OutputSelection {
            output: outcome.output.map(|output| OutputChanged {
                generation: output.generation,
                device_id: output.device_id,
                sample_rate: output.sample_rate,
                channels: output.channels,
                buffer_frames: output.buffer_frames,
            }),
            preferred_device_id: outcome.preferred,
        }),
        Some(Err(SelectError::OpenFailed(message))) => {
            Err(OutputSelectError::OpenFailed { message })
        }
        Some(Err(SelectError::SaveFailed(message))) => {
            Err(OutputSelectError::SaveFailed { message })
        }
        Some(Err(SelectError::Expired)) | None => Err(OutputSelectError::Unavailable),
    }
}

/// Registers a channel that receives every rendered `notify` event.
#[tauri::command]
pub fn audio_subscribe(audio: State<'_, AudioService>, on_event: Channel<PlaybackEvent>) {
    lock(&audio.subscribers).push(on_event);
}

/// Unix-epoch ms at which `frame` will be heard on the current output.
#[tauri::command]
pub async fn audio_heard_at(
    audio: State<'_, AudioService>,
    frame: FramePos,
) -> Result<Option<f64>, String> {
    Ok(lock(audio.engine()).heard_at_epoch_ms(frame))
}

/// Plays the output test sound shortly from now. Subscribers get a
/// [`PlaybackEvent`] for it once rendered.
#[tauri::command]
pub async fn audio_play_test_sound(
    audio: State<'_, AudioService>,
) -> Result<ScheduledSound, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_for_the_manager_does_not_block_other_async_work() {
        let (keep_open, response) = mpsc::channel::<u32>();
        tauri::async_runtime::block_on(async move {
            let started = Instant::now();
            let waiting =
                tauri::async_runtime::spawn(await_reply(response, Duration::from_millis(400)));
            let other = tauri::async_runtime::spawn(async { 7 });
            assert_eq!(other.await.unwrap(), 7);
            assert!(
                started.elapsed() < Duration::from_millis(200),
                "other work ran while the reply was pending"
            );
            assert_eq!(waiting.await.unwrap(), None, "timed out without a reply");
        });
        drop(keep_open);
    }

    #[test]
    fn a_reply_that_arrives_is_returned() {
        let (reply, response) = mpsc::channel::<u32>();
        reply.send(5).unwrap();
        let value = tauri::async_runtime::block_on(await_reply(response, Duration::from_secs(1)));
        assert_eq!(value, Some(5));
    }
}

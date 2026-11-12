//! Tauri side of the tuner: the `tuner-analysis` thread that owns the input
//! stream and the analyzer, and the commands the screen calls.
//!
//! The input callback only writes samples into the ring buffer of
//! [`crate::audio::open_input`]; this thread reads them, analyses them and
//! sends frames to the webview over a [`Channel`].

use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Listener, State};

use super::TunerSettings;
use super::analyzer::{Analyzer, Reading};
use super::input::{InputDevices, InputEnv, InputSupervisor, ListedInput, OpenedInput};
use super::notes::clamp_reference;
use super::permission::{self, MicPermission};
use crate::app_audio::DEVICES_CHANGED_EVENT;
use crate::audio::{Direction, InputCapture, InputChannel, InputRequest, list_devices, open_input};
use crate::settings::SettingsStore;

/// Input buffer the tuner asks for (frames).
const INPUT_BUFFER_FRAMES: u32 = 64;
/// Ring between the input callback and this thread: over a second of audio,
/// so a stalled analysis loses nothing.
const RING_FRAMES: usize = 1 << 16;
/// Polling interval of the analysis thread (the callback delivers ~1.3 ms).
const POLL: Duration = Duration::from_millis(1);
/// Frames go to the screen at most this often, except when the note changes.
const FRAME_INTERVAL: Duration = Duration::from_millis(8);
const PERMISSION_POLL: Duration = Duration::from_secs(1);
/// A stream that delivers nothing for this long is reopened.
const STALL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TunerState {
    /// Not running (screen closed).
    Stopped,
    /// Waiting for the user to answer the system's microphone prompt.
    PermissionPending,
    /// Microphone access is off in System Settings.
    PermissionDenied,
    /// No input device could be opened (retrying).
    NoInput,
    Listening,
}

/// What the tuner listens to right now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunerStatus {
    pub state: TunerState,
    /// The running input, when listening.
    pub input: Option<OpenedInput>,
    /// The chosen device is gone or failed; the system default plays.
    pub fallback: bool,
    /// The user's choice (`None` = system default).
    pub preferred_device_id: Option<String>,
    pub channel: InputChannel,
    pub reference_hz: u16,
}

/// Sent to the screen over the channel given to `tuner_start`.
#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TunerEvent {
    Status(TunerStatus),
    /// One analysis step: input level and the pitch shown (if any).
    Frame {
        level_db: f32,
        pitch: Option<Reading>,
    },
}

/// Why a tuner choice could not be stored; the choice still applies.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum TunerError {
    SaveFailed { message: String },
}

enum Command {
    Select {
        device_id: Option<String>,
        channel: InputChannel,
    },
    Stop,
}

struct Shared {
    sink: Mutex<Option<Channel<TunerEvent>>>,
    status: Mutex<TunerStatus>,
    reference_hz: AtomicU16,
    devices_changed: AtomicBool,
    /// Set by the system's answer to the microphone prompt.
    permission_answered: Arc<AtomicBool>,
}

struct Worker {
    commands: mpsc::Sender<Command>,
    thread: JoinHandle<()>,
}

pub struct TunerService {
    shared: Arc<Shared>,
    worker: Mutex<Option<Worker>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn stopped_status(settings: &TunerSettings) -> TunerStatus {
    TunerStatus {
        state: TunerState::Stopped,
        input: None,
        fallback: false,
        preferred_device_id: settings.input_device_id.clone(),
        channel: settings.input_channel,
        reference_hz: clamp_reference(settings.reference_hz),
    }
}

impl TunerService {
    /// Creates the service and follows `audio://devices-changed`. Nothing is
    /// opened until the screen calls `tuner_start`.
    pub fn new(app: &AppHandle, settings: &SettingsStore) -> Self {
        let tuner = settings.get().tuner;
        let shared = Arc::new(Shared {
            sink: Mutex::new(None),
            status: Mutex::new(stopped_status(&tuner)),
            reference_hz: AtomicU16::new(clamp_reference(tuner.reference_hz)),
            devices_changed: AtomicBool::new(false),
            permission_answered: Arc::new(AtomicBool::new(false)),
        });
        let listener = Arc::clone(&shared);
        app.listen_any(DEVICES_CHANGED_EVENT, move |_| {
            listener.devices_changed.store(true, Ordering::Relaxed);
        });
        Self {
            shared,
            worker: Mutex::new(None),
        }
    }

    fn status(&self) -> TunerStatus {
        lock(&self.shared.status).clone()
    }

    fn start(&self, settings: TunerSettings, sink: Channel<TunerEvent>) {
        *lock(&self.shared.sink) = Some(sink);
        let mut worker = lock(&self.worker);
        if worker.as_ref().is_some_and(|w| !w.thread.is_finished()) {
            return;
        }
        let (commands, inbox) = mpsc::channel();
        let shared = Arc::clone(&self.shared);
        let thread = std::thread::Builder::new()
            .name("tuner-analysis".into())
            .spawn(move || run(&shared, &settings, &inbox))
            .expect("spawn tuner analysis thread");
        *worker = Some(Worker { commands, thread });
    }

    /// Stops listening and waits for the thread (closes the microphone).
    pub fn shutdown(&self) {
        let worker = lock(&self.worker).take();
        if let Some(worker) = worker {
            let _ = worker.commands.send(Command::Stop);
            let _ = worker.thread.join();
        }
    }

    fn send(&self, command: Command) {
        if let Some(worker) = lock(&self.worker).as_ref() {
            let _ = worker.commands.send(command);
        }
    }
}

/// Real input devices through cpal.
struct CpalInputEnv;

impl InputEnv for CpalInputEnv {
    type Capture = InputCapture;

    fn devices(&mut self) -> InputDevices {
        let listed = list_devices(Direction::Input).unwrap_or_default();
        InputDevices {
            default: listed.iter().find(|d| d.is_default).map(|d| d.id.clone()),
            devices: listed
                .into_iter()
                .map(|d| ListedInput {
                    id: d.id,
                    channels: d.channels,
                })
                .collect(),
        }
    }

    fn open(
        &mut self,
        device: Option<&str>,
        channel: InputChannel,
    ) -> Result<(InputCapture, OpenedInput), String> {
        let capture = open_input(&InputRequest {
            device_id: device.map(str::to_owned),
            channel,
            target_buffer_frames: INPUT_BUFFER_FRAMES,
            ring_frames: RING_FRAMES,
        })
        .map_err(|error| error.to_string())?;
        let opened = OpenedInput {
            device_id: capture.info.device_id.clone(),
            device_name: capture.info.device_name.clone(),
            sample_rate: capture.info.sample_rate,
            channels: capture.info.channels,
            channel,
        };
        log::info!(
            "tuner input: device=\"{}\" sample_rate={} channels={} channel={:?} buffer={:?}",
            opened.device_name,
            opened.sample_rate,
            opened.channels,
            opened.channel,
            capture.info.buffer_frames,
        );
        Ok((capture, opened))
    }
}

/// Sends frames to the screen, at most every [`FRAME_INTERVAL`] unless the
/// note appears, changes or goes away.
struct FrameSender {
    last_sent: Option<Instant>,
    last_note: Option<i32>,
}

impl FrameSender {
    fn offer(&mut self, shared: &Shared, level_db: f32, pitch: Option<Reading>) {
        let now = Instant::now();
        let note = pitch.map(|p| p.midi);
        let due = self
            .last_sent
            .is_none_or(|at| now.duration_since(at) >= FRAME_INTERVAL);
        if !due && note == self.last_note {
            return;
        }
        self.last_sent = Some(now);
        self.last_note = note;
        if let Some(sink) = lock(&shared.sink).as_ref() {
            let _ = sink.send(TunerEvent::Frame { level_db, pitch });
        }
    }
}

fn publish(shared: &Shared, status: TunerStatus) {
    let mut current = lock(&shared.status);
    if *current == status {
        return;
    }
    *current = status.clone();
    drop(current);
    if let Some(sink) = lock(&shared.sink).as_ref() {
        let _ = sink.send(TunerEvent::Status(status));
    }
}

fn status_of(
    supervisor: &InputSupervisor<CpalInputEnv>,
    permission: MicPermission,
    reference_hz: u16,
) -> TunerStatus {
    let state = match permission {
        MicPermission::Denied => TunerState::PermissionDenied,
        MicPermission::Undetermined => TunerState::PermissionPending,
        MicPermission::Granted if supervisor.opened().is_some() => TunerState::Listening,
        MicPermission::Granted => TunerState::NoInput,
    };
    TunerStatus {
        state,
        input: supervisor.opened().cloned(),
        fallback: supervisor.is_fallback(),
        preferred_device_id: supervisor.preferred().map(str::to_owned),
        channel: supervisor.channel(),
        reference_hz,
    }
}

/// The analysis thread.
fn run(shared: &Shared, settings: &TunerSettings, inbox: &mpsc::Receiver<Command>) {
    let mut supervisor = InputSupervisor::new(
        CpalInputEnv,
        settings.input_device_id.clone(),
        settings.input_channel,
    );
    let mut permission = permission::status();
    let mut asked = false;
    let mut started = false;
    let mut next_permission_check = Instant::now();
    let mut analyzer: Option<Analyzer> = None;
    let mut generation = 0;
    let mut frames = FrameSender {
        last_sent: None,
        last_note: None,
    };
    let mut last_progress = (0_u64, Instant::now());

    loop {
        let now = Instant::now();
        loop {
            match inbox.try_recv() {
                Ok(Command::Select { device_id, channel }) => {
                    if started {
                        supervisor.select(now, device_id, channel);
                    } else {
                        supervisor.choose(device_id, channel);
                    }
                }
                Ok(Command::Stop) | Err(mpsc::TryRecvError::Disconnected) => {
                    drop(supervisor);
                    let mut status = lock(&shared.status).clone();
                    status.state = TunerState::Stopped;
                    status.input = None;
                    status.fallback = false;
                    publish(shared, status);
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }

        if now >= next_permission_check || shared.permission_answered.swap(false, Ordering::Relaxed)
        {
            next_permission_check = now + PERMISSION_POLL;
            permission = permission::status();
            if permission == MicPermission::Undetermined && !asked {
                asked = true;
                let answered = Arc::clone(&shared.permission_answered);
                permission::request(move |_| answered.store(true, Ordering::Relaxed));
            }
        }
        match (permission, started) {
            (MicPermission::Granted, false) => {
                shared.devices_changed.store(false, Ordering::Relaxed);
                supervisor.start(now);
                started = true;
            }
            (MicPermission::Denied, true) => {
                supervisor.stop();
                started = false;
            }
            _ => {}
        }

        let reference_hz = shared.reference_hz.load(Ordering::Relaxed);
        if started {
            if shared.devices_changed.swap(false, Ordering::Relaxed) {
                supervisor.devices_changed(now);
            }
            supervisor.tick(now);

            if supervisor.generation() != generation {
                generation = supervisor.generation();
                analyzer = supervisor
                    .opened()
                    .map(|opened| Analyzer::new(opened.sample_rate, f64::from(reference_hz)));
                last_progress = (0, now);
            }

            let mut failed = false;
            if let (Some(capture), Some(analyzer)) = (supervisor.capture_mut(), analyzer.as_mut()) {
                analyzer.set_reference(f64::from(reference_hz));
                let available = capture.samples.slots();
                if let Ok(chunk) = capture.samples.read_chunk(available) {
                    let (first, second) = chunk.as_slices();
                    for part in [first, second] {
                        analyzer.push(part, |frame| {
                            frames.offer(shared, frame.level_db, frame.pitch);
                        });
                    }
                    chunk.commit_all();
                }
                if capture
                    .shared
                    .take_error()
                    .is_some_and(|error| error.needs_reopen())
                {
                    failed = true;
                }
                let captured = capture.shared.frames();
                if captured != last_progress.0 {
                    last_progress = (captured, now);
                } else if now.duration_since(last_progress.1) > STALL {
                    failed = true;
                }
            }
            if failed {
                log::warn!("tuner input: stream failed, reopening");
                supervisor.stream_failed(now);
            }
        }

        publish(shared, status_of(&supervisor, permission, reference_hz));
        std::thread::sleep(POLL);
    }
}

fn save(store: &SettingsStore, update: impl FnOnce(&mut TunerSettings)) -> Result<(), TunerError> {
    let mut settings = store.get();
    update(&mut settings.tuner);
    store
        .save(settings)
        .map(|_| ())
        .map_err(|error| TunerError::SaveFailed {
            message: error.to_string(),
        })
}

// ---- commands ------------------------------------------------------------------

/// Starts listening (if not yet) and sends events to `on_event` from now on.
#[tauri::command]
pub async fn tuner_start(
    tuner: State<'_, TunerService>,
    settings: State<'_, SettingsStore>,
    on_event: Channel<TunerEvent>,
) -> Result<TunerStatus, String> {
    tuner.start(settings.get().tuner, on_event);
    Ok(tuner.status())
}

/// Stops listening and releases the microphone.
#[tauri::command]
pub async fn tuner_stop(tuner: State<'_, TunerService>) -> Result<(), String> {
    tuner.shutdown();
    Ok(())
}

/// Chooses the input device (`None` = system default) and channel, and saves
/// the choice. On `saveFailed` the choice still applies until the app quits.
#[tauri::command]
pub async fn tuner_select_input(
    tuner: State<'_, TunerService>,
    settings: State<'_, SettingsStore>,
    device_id: Option<String>,
    channel: InputChannel,
) -> Result<(), TunerError> {
    tuner.send(Command::Select {
        device_id: device_id.clone(),
        channel,
    });
    {
        let mut status = lock(&tuner.shared.status);
        if status.state == TunerState::Stopped {
            status.preferred_device_id.clone_from(&device_id);
            status.channel = channel;
        }
    }
    save(&settings, |tuner| {
        tuner.input_device_id = device_id;
        tuner.input_channel = channel;
    })
}

/// Sets A4 (clamped to 430–450 Hz) and saves it. Returns the value in effect.
#[tauri::command]
pub async fn tuner_set_reference(
    tuner: State<'_, TunerService>,
    settings: State<'_, SettingsStore>,
    hz: u16,
) -> Result<u16, TunerError> {
    let hz = clamp_reference(hz);
    tuner.shared.reference_hz.store(hz, Ordering::Relaxed);
    lock(&tuner.shared.status).reference_hz = hz;
    save(&settings, |tuner| tuner.reference_hz = hz).map(|()| hz)
}

/// Opens the microphone privacy page of System Settings.
#[tauri::command]
pub async fn tuner_open_microphone_settings() -> Result<(), String> {
    permission::open_settings().map_err(|error| error.to_string())
}

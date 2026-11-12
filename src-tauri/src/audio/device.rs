//! cpal glue: device listing, choosing the smallest buffer (≥ the 64-frame floor) a device allows,
//! and the output stream that renders an [`EngineCore`].

use std::cell::UnsafeCell;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Device, DeviceId, ErrorKind, FromSample, Sample, SampleFormat, SizedSample,
    StreamConfig, SupportedBufferSize, SupportedStreamConfig,
};
use serde::Serialize;

use super::core::EngineCore;
use super::types::{CallbackTiming, MAX_CHANNELS, StreamFormat};

/// Buffer sizes tried after the device minimum, in order.
const FALLBACK_BUFFER_FRAMES: [u32; 3] = [128, 256, 512];
const STREAM_OPEN_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("audio device not found: {0}")]
    DeviceNotFound(String),
    #[error("no default {0} device")]
    NoDefaultDevice(&'static str),
    #[error("unsupported stream configuration: {0}")]
    Unsupported(String),
    #[error(transparent)]
    Backend(#[from] cpal::Error),
}

/// Direction of a device in listings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Input,
    Output,
}

/// A device as shown in device pickers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    /// Stable id (`"<host>:<device>"`) to persist in settings.
    pub id: String,
    pub name: String,
    pub is_default: bool,
    /// Channels of the device's default configuration.
    pub channels: u16,
    pub sample_rate: u32,
}

pub(crate) fn host() -> cpal::Host {
    cpal::default_host()
}

pub(crate) fn device_id_string(device: &Device) -> Option<String> {
    device.id().ok().map(|id| id.to_string())
}

pub(crate) fn device_name(device: &Device) -> String {
    device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| device.to_string())
}

/// Lists devices that can play (`Output`) or record (`Input`).
pub fn list_devices(direction: Direction) -> Result<Vec<DeviceInfo>, AudioError> {
    let host = host();
    let (devices, default) = match direction {
        Direction::Output => (
            host.output_devices()?.collect::<Vec<_>>(),
            host.default_output_device(),
        ),
        Direction::Input => (
            host.input_devices()?.collect::<Vec<_>>(),
            host.default_input_device(),
        ),
    };
    let default_id = default.as_ref().and_then(device_id_string);
    let mut infos = Vec::with_capacity(devices.len());
    for device in devices {
        let Some(id) = device_id_string(&device) else {
            continue;
        };
        let config = match direction {
            Direction::Output => device.default_output_config(),
            Direction::Input => device.default_input_config(),
        };
        let Ok(config) = config else { continue };
        infos.push(DeviceInfo {
            is_default: default_id.as_deref() == Some(id.as_str()),
            name: device_name(&device),
            id,
            channels: config.channels(),
            sample_rate: config.sample_rate(),
        });
    }
    Ok(infos)
}

/// Finds a device by its persisted id, or the default one when `id` is `None`.
pub(crate) fn find_device(id: Option<&str>, direction: Direction) -> Result<Device, AudioError> {
    let host = host();
    match id {
        Some(id) => {
            let parsed =
                DeviceId::from_str(id).map_err(|_| AudioError::DeviceNotFound(id.to_owned()))?;
            host.device_by_id(&parsed)
                .ok_or_else(|| AudioError::DeviceNotFound(id.to_owned()))
        }
        None => match direction {
            Direction::Output => host
                .default_output_device()
                .ok_or(AudioError::NoDefaultDevice("output")),
            Direction::Input => host
                .default_input_device()
                .ok_or(AudioError::NoDefaultDevice("input")),
        },
    }
}

/// Sample rate an output device runs at right now (its current default
/// configuration), or `None` if it cannot be read.
pub fn output_device_sample_rate(id: &str) -> Option<u32> {
    let device = find_device(Some(id), Direction::Output).ok()?;
    device.default_output_config().ok().map(|c| c.sample_rate())
}

/// Buffer sizes to try, smallest first. `target` is a floor, not a wish: the
/// first candidate is the smallest size the device allows that is at least
/// `target` (the app uses 64 frames — about 1.3 ms at 48 kHz — because smaller
/// sizes risk dropouts on some devices). Then common fallbacks inside the
/// device range, then `None` for the backend default.
pub(crate) fn buffer_candidates(range: SupportedBufferSize, target: u32) -> Vec<Option<u32>> {
    let mut candidates = Vec::new();
    match range {
        SupportedBufferSize::Range { min, max } => {
            let first = target.max(min).min(max);
            candidates.push(Some(first));
            for size in FALLBACK_BUFFER_FRAMES {
                if size > first && size <= max {
                    candidates.push(Some(size));
                }
            }
        }
        SupportedBufferSize::Unknown => {
            candidates.push(Some(target));
            candidates.extend(FALLBACK_BUFFER_FRAMES.iter().map(|&size| Some(size)));
        }
    }
    candidates.push(None);
    candidates
}

/// Picks the device's default config, narrowed to at most [`MAX_CHANNELS`]
/// channels (multi-channel interfaces get a stereo config when they offer one).
pub(crate) fn pick_config(
    device: &Device,
    direction: Direction,
) -> Result<SupportedStreamConfig, AudioError> {
    let default = match direction {
        Direction::Output => device.default_output_config()?,
        Direction::Input => device.default_input_config()?,
    };
    if direction == Direction::Input || usize::from(default.channels()) <= MAX_CHANNELS {
        return Ok(default);
    }
    let rate = default.sample_rate();
    device
        .supported_output_configs()?
        .find(|range| {
            range.channels() == 2
                && range.sample_format() == default.sample_format()
                && range.contains_rate(rate)
        })
        .map(|range| range.with_sample_rate(rate))
        .ok_or_else(|| {
            AudioError::Unsupported(format!(
                "{} output channels and no stereo configuration",
                default.channels()
            ))
        })
}

/// Holds the render core so both the audio callback and the control thread
/// can reach it. The audio thread only ever *tries* to take it: if the control
/// thread holds it (switching devices), the callback outputs silence instead
/// of waiting. Only the control thread ever spins, for at most one callback.
pub(crate) struct CoreCell {
    busy: AtomicBool,
    core: UnsafeCell<EngineCore>,
}

// SAFETY: access to `core` is serialized by the `busy` flag (acquire/release),
// and `EngineCore` is `Send`.
unsafe impl Sync for CoreCell {}

impl CoreCell {
    pub(crate) fn new(core: EngineCore) -> Self {
        Self {
            busy: AtomicBool::new(false),
            core: UnsafeCell::new(core),
        }
    }

    /// Audio thread: renders if the core is free, otherwise returns false.
    fn try_process<T>(&self, out: &mut [T], timing: CallbackTiming) -> bool
    where
        T: Sample + FromSample<f32>,
    {
        if self
            .busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return false;
        }
        // SAFETY: `busy` is held, so no other reference to the core exists.
        unsafe { (*self.core.get()).process(out, timing) };
        self.busy.store(false, Ordering::Release);
        true
    }

    /// Control thread: runs `f` with exclusive access to the core.
    pub(crate) fn with_core<R>(&self, f: impl FnOnce(&mut EngineCore) -> R) -> R {
        while self
            .busy
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            std::thread::yield_now();
        }
        // SAFETY: `busy` is held, so no other reference to the core exists.
        let result = f(unsafe { &mut *self.core.get() });
        self.busy.store(false, Ordering::Release);
        result
    }
}

/// Stream error reported by the backend, kept as a code so the error callback
/// stays allocation-free.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamErrorKind {
    DeviceNotAvailable,
    DeviceChanged,
    StreamInvalidated,
    Xrun,
    Other,
}

impl StreamErrorKind {
    pub(crate) fn code(kind: ErrorKind) -> u32 {
        match kind {
            ErrorKind::DeviceNotAvailable => 1,
            ErrorKind::DeviceChanged => 2,
            ErrorKind::StreamInvalidated => 3,
            ErrorKind::Xrun => 4,
            _ => 5,
        }
    }

    pub(crate) fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => None,
            1 => Some(Self::DeviceNotAvailable),
            2 => Some(Self::DeviceChanged),
            3 => Some(Self::StreamInvalidated),
            4 => Some(Self::Xrun),
            _ => Some(Self::Other),
        }
    }

    /// Whether the stream must be rebuilt to keep sounding.
    pub fn needs_reopen(self) -> bool {
        !matches!(self, Self::Xrun)
    }
}

pub(crate) fn error_callback(flag: Arc<AtomicU32>) -> impl FnMut(cpal::Error) + Send + 'static {
    move |error| flag.store(StreamErrorKind::code(error.kind()), Ordering::Relaxed)
}

/// What the device actually granted for an output stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputStreamInfo {
    pub device_id: String,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// Fixed size we asked for, `None` when the backend default was used.
    pub requested_buffer_frames: Option<u32>,
    /// Size the backend reports per callback.
    pub buffer_frames: Option<u32>,
}

pub(crate) struct OutputStream {
    pub(crate) stream: cpal::Stream,
    pub(crate) info: OutputStreamInfo,
    pub(crate) error: Arc<AtomicU32>,
}

/// Builds (but does not start) an output stream on `device` with the smallest
/// workable buffer of at least `target_buffer_frames` frames, wired to render
/// `cell`'s core. Nothing else changes, so on failure the running output keeps
/// playing; on success the caller stops the old stream, reformats the core to
/// the returned format and then calls `play`.
pub(crate) fn build_output_stream(
    device: &Device,
    cell: &Arc<CoreCell>,
    target_buffer_frames: u32,
) -> Result<(OutputStream, StreamFormat), AudioError> {
    let supported = pick_config(device, Direction::Output)?;
    let format = StreamFormat::new(supported.sample_rate(), supported.channels());

    let error = Arc::new(AtomicU32::new(0));
    let mut last_error = None;
    for buffer in buffer_candidates(*supported.buffer_size(), target_buffer_frames) {
        let config = StreamConfig {
            channels: supported.channels(),
            sample_rate: supported.sample_rate(),
            buffer_size: buffer.map_or(BufferSize::Default, BufferSize::Fixed),
        };
        let built = match supported.sample_format() {
            SampleFormat::F32 => build_output::<f32>(device, config, cell, &error),
            SampleFormat::I16 => build_output::<i16>(device, config, cell, &error),
            SampleFormat::I32 => build_output::<i32>(device, config, cell, &error),
            SampleFormat::U16 => build_output::<u16>(device, config, cell, &error),
            other => {
                return Err(AudioError::Unsupported(format!("sample format {other:?}")));
            }
        };
        match built {
            Ok(stream) => {
                let info = OutputStreamInfo {
                    device_id: device_id_string(device).unwrap_or_default(),
                    device_name: device_name(device),
                    sample_rate: format.sample_rate,
                    channels: format.channels,
                    requested_buffer_frames: buffer,
                    buffer_frames: stream.buffer_size().ok(),
                };
                return Ok((
                    OutputStream {
                        stream,
                        info,
                        error,
                    },
                    format,
                ));
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.map_or_else(
        || AudioError::Unsupported("no buffer size worked".into()),
        AudioError::Backend,
    ))
}

fn build_output<T>(
    device: &Device,
    config: StreamConfig,
    cell: &Arc<CoreCell>,
    error: &Arc<AtomicU32>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    let cell = Arc::clone(cell);
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], info| {
            let stamp = info.timestamp();
            let timing = CallbackTiming {
                callback_ns: nanos(stamp.callback.as_nanos()),
                playback_ns: nanos(stamp.playback.as_nanos()),
            };
            if !cell.try_process(data, timing) {
                data.fill(T::EQUILIBRIUM);
            }
        },
        error_callback(Arc::clone(error)),
        Some(STREAM_OPEN_TIMEOUT),
    )
}

pub(crate) fn nanos(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

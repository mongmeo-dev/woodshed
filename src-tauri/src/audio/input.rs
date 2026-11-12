//! Input capture: opens an input device and hands its samples to a normal
//! thread through a lock-free ring buffer (used by the tuner).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{BufferSize, Device, FromSample, SampleFormat, SizedSample, StreamConfig};
use rtrb::{Consumer, Producer, RingBuffer};
use serde::{Deserialize, Serialize};

use super::device::{
    AudioError, Direction, StreamErrorKind, buffer_candidates, device_id_string, device_name,
    find_device, nanos, pick_config,
};

/// Which part of the input signal to capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "channel", rename_all = "camelCase")]
pub enum InputChannel {
    /// One channel, zero-based (e.g. input 1 of an interface is `Single(0)`).
    Single(u16),
    /// Average of all channels.
    MixDown,
}

#[derive(Clone, Debug)]
pub struct InputRequest {
    /// Persisted device id, or `None` for the default input.
    pub device_id: Option<String>,
    pub channel: InputChannel,
    pub target_buffer_frames: u32,
    /// Capacity of the sample ring, in mono frames.
    pub ring_frames: usize,
}

impl Default for InputRequest {
    fn default() -> Self {
        Self {
            device_id: None,
            channel: InputChannel::Single(0),
            target_buffer_frames: 64,
            ring_frames: 48_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputStreamInfo {
    pub device_id: String,
    pub device_name: String,
    pub sample_rate: u32,
    /// Channels the device delivers (before channel selection).
    pub channels: u16,
    pub requested_buffer_frames: Option<u32>,
    pub buffer_frames: Option<u32>,
}

/// Counters written by the input callback.
#[derive(Debug, Default)]
pub struct InputShared {
    frames: AtomicU64,
    overflows: AtomicU64,
    capture_latency_ns: AtomicU64,
    error: AtomicU32,
}

impl InputShared {
    /// Mono frames captured so far.
    pub fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }

    /// Frames dropped because the reader fell behind and the ring was full.
    pub fn overflows(&self) -> u64 {
        self.overflows.load(Ordering::Relaxed)
    }

    /// Time from the ADC to the callback, as reported by the backend.
    pub fn capture_latency_ns(&self) -> u64 {
        self.capture_latency_ns.load(Ordering::Relaxed)
    }

    /// Last stream error, cleared on read.
    pub fn take_error(&self) -> Option<StreamErrorKind> {
        StreamErrorKind::from_code(self.error.swap(0, Ordering::Relaxed))
    }
}

/// A running input stream. Read mono `f32` samples from `samples`; dropping
/// the capture stops the stream.
pub struct InputCapture {
    pub samples: Consumer<f32>,
    pub info: InputStreamInfo,
    pub shared: Arc<InputShared>,
    _stream: cpal::Stream,
}

/// Opens an input stream with the smallest workable buffer.
pub fn open_input(request: &InputRequest) -> Result<InputCapture, AudioError> {
    let device = find_device(request.device_id.as_deref(), Direction::Input)?;
    let supported = pick_config(&device, Direction::Input)?;
    if let InputChannel::Single(channel) = request.channel
        && channel >= supported.channels()
    {
        return Err(AudioError::Unsupported(format!(
            "input channel {channel} of {}",
            supported.channels()
        )));
    }

    let shared = Arc::new(InputShared::default());
    let mut last_error = None;
    for buffer in buffer_candidates(*supported.buffer_size(), request.target_buffer_frames) {
        let config = StreamConfig {
            channels: supported.channels(),
            sample_rate: supported.sample_rate(),
            buffer_size: buffer.map_or(BufferSize::Default, BufferSize::Fixed),
        };
        let (producer, consumer) = RingBuffer::new(request.ring_frames.max(1));
        let built = match supported.sample_format() {
            SampleFormat::F32 => build_input::<f32>(&device, config, request, producer, &shared),
            SampleFormat::I16 => build_input::<i16>(&device, config, request, producer, &shared),
            SampleFormat::I32 => build_input::<i32>(&device, config, request, producer, &shared),
            SampleFormat::U16 => build_input::<u16>(&device, config, request, producer, &shared),
            other => {
                return Err(AudioError::Unsupported(format!("sample format {other:?}")));
            }
        };
        match built {
            Ok(stream) => {
                stream.play()?;
                let info = InputStreamInfo {
                    device_id: device_id_string(&device).unwrap_or_default(),
                    device_name: device_name(&device),
                    sample_rate: supported.sample_rate(),
                    channels: supported.channels(),
                    requested_buffer_frames: buffer,
                    buffer_frames: stream.buffer_size().ok(),
                };
                return Ok(InputCapture {
                    samples: consumer,
                    info,
                    shared,
                    _stream: stream,
                });
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.map_or_else(
        || AudioError::Unsupported("no buffer size worked".into()),
        AudioError::Backend,
    ))
}

fn build_input<T>(
    device: &Device,
    config: StreamConfig,
    request: &InputRequest,
    mut producer: Producer<f32>,
    shared: &Arc<InputShared>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels);
    let selection = request.channel;
    let callback_shared = Arc::clone(shared);
    let error_shared = Arc::clone(shared);
    device.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], info| {
            let stamp = info.timestamp();
            let latency =
                nanos(stamp.callback.as_nanos()).saturating_sub(nanos(stamp.capture.as_nanos()));
            callback_shared
                .capture_latency_ns
                .store(latency, Ordering::Relaxed);
            let mut written = 0_u64;
            let mut dropped = 0_u64;
            for frame in data.chunks_exact(channels) {
                let value = match selection {
                    InputChannel::Single(channel) => frame[usize::from(channel)].to_sample::<f32>(),
                    InputChannel::MixDown => {
                        frame.iter().map(|&s| s.to_sample::<f32>()).sum::<f32>() / channels as f32
                    }
                };
                if producer.push(value).is_ok() {
                    written += 1;
                } else {
                    dropped += 1;
                }
            }
            callback_shared.frames.fetch_add(written, Ordering::Relaxed);
            if dropped > 0 {
                callback_shared
                    .overflows
                    .fetch_add(dropped, Ordering::Relaxed);
            }
        },
        move |error: cpal::Error| {
            error_shared
                .error
                .store(StreamErrorKind::code(error.kind()), Ordering::Relaxed);
        },
        Some(std::time::Duration::from_secs(3)),
    )
}

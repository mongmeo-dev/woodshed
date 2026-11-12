//! Woodshed's low-latency audio engine.
//!
//! Layers, bottom up:
//! - [`EngineCore`]: device-independent mixer and sample-accurate scheduler.
//!   Runs on the audio thread; allocation-, lock- and syscall-free.
//! - [`EngineHandle`]: UI-side API. Talks to the core only through a
//!   lock-free SPSC queue (`rtrb`).
//! - [`AudioEngine`]: owns the core and the cpal output stream, picks the
//!   smallest buffer of at least 64 frames that the device allows (the 64-frame
//!   floor avoids per-device dropouts at tiny sizes) and switches devices
//!   without losing sources or the frame clock.
//! - [`OfflineEngine`]: renders the same core without a device, for tests.
//! - [`open_input`]: input capture for the tuner; [`spawn_device_monitor`]:
//!   device change hook.
//!
//! Tool nodes implement [`Source`], attach it with
//! [`EngineHandle::add_source`] and place [`EventKind`]s on the frame clock
//! with [`EngineHandle::schedule`].

mod clock;
mod core;
mod device;
mod engine;
mod handle;
mod input;
mod monitor;
mod offline;
mod output_manager;
mod shared;
mod source;
mod types;

#[cfg(test)]
mod tests;

pub use clock::{ClockPair, event_heard_at_epoch_ms, heard_at_epoch_ms};
pub use core::{EngineConfig, EngineCore, create_engine};
pub use device::{
    AudioError, DeviceInfo, Direction, OutputStreamInfo, StreamErrorKind, list_devices,
    output_device_sample_rate,
};
pub use engine::{AudioEngine, MeasuredLatency, OutputRequest, OutputStarter, PendingOutput};
pub use handle::{EngineError, EngineHandle};
pub use input::{
    InputCapture, InputChannel, InputRequest, InputShared, InputStreamInfo, open_input,
};
pub use monitor::{DeviceMonitor, DeviceSnapshot, spawn_device_monitor};
pub use offline::OfflineEngine;
pub use output_manager::{
    OpenedOutput, OutputEnv, OutputInput, OutputManager, OutputMode, OutputProblem, RetryBackoff,
    SelectError, output_problem,
};
pub use shared::{EngineStats, SharedState, TimingSnapshot};
pub use source::{ClickSource, Source, testing};
pub use types::{
    CallbackTiming, EventKind, FramePos, MAX_BLOCK_FRAMES, MAX_CHANNELS, RenderedEvent,
    ScheduledEvent, SourceId, StreamFormat,
};

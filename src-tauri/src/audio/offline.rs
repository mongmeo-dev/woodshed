//! Offline render mode: drive the engine without an audio device, so render
//! output can be inspected in tests.

use rtrb::Consumer;

use super::core::{EngineConfig, EngineCore, create_engine};
use super::handle::EngineHandle;
use super::types::{CallbackTiming, RenderedEvent, StreamFormat};

/// An engine whose clock advances only when [`OfflineEngine::render`] is
/// called. The synthetic stream clock starts at 0 ns with zero latency, so
/// frame `f` "plays" at `f / sample_rate` seconds.
///
/// ```ignore
/// let mut engine = OfflineEngine::new(StreamFormat::new(48_000, 2));
/// let id = engine.handle.add_source(Box::new(MySource::default()), 1.0)?;
/// engine.handle.schedule(1_000, id, EventKind::Start)?;
/// let buffer = engine.render(4_800, 128); // 4 800 frames in 128-frame callbacks
/// ```
pub struct OfflineEngine {
    pub handle: EngineHandle,
    core: EngineCore,
    notifications: Consumer<RenderedEvent>,
}

impl OfflineEngine {
    pub fn new(format: StreamFormat) -> Self {
        Self::with_config(format, EngineConfig::default())
    }

    pub fn with_config(format: StreamFormat, config: EngineConfig) -> Self {
        let (handle, core, notifications) = create_engine(format, config);
        Self {
            handle,
            core,
            notifications,
        }
    }

    pub fn format(&self) -> StreamFormat {
        self.core.format()
    }

    /// Renders `frames` frames as a series of `callback_frames`-sized device
    /// callbacks (the last one may be shorter) and returns them interleaved.
    pub fn render(&mut self, frames: usize, callback_frames: usize) -> Vec<f32> {
        let channels = usize::from(self.core.format().channels);
        let mut out = vec![0.0; frames * channels];
        for chunk in out.chunks_mut(callback_frames.max(1) * channels) {
            self.render_callback(chunk);
        }
        out
    }

    /// Renders exactly one callback into `out`.
    pub fn render_callback(&mut self, out: &mut [f32]) {
        let sample_rate = u128::from(self.core.format().sample_rate);
        let ns = u64::try_from(u128::from(self.core.frame()) * 1_000_000_000 / sample_rate)
            .unwrap_or(u64::MAX);
        self.core.process(
            out,
            CallbackTiming {
                callback_ns: ns,
                playback_ns: ns,
            },
        );
    }

    /// Reports of `notify` events rendered so far (drains the queue).
    pub fn take_notifications(&mut self) -> Vec<RenderedEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.notifications.pop() {
            events.push(event);
        }
        events
    }
}

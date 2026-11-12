//! The live engine: one render core, one output stream, device switching.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use rtrb::Consumer;

use super::clock::{ClockPair, event_heard_at_epoch_ms, heard_at_epoch_ms};
use super::core::{EngineConfig, create_engine};
use super::device::{
    AudioError, CoreCell, Direction, OutputStream, OutputStreamInfo, StreamErrorKind,
    build_output_stream, find_device,
};
use super::handle::EngineHandle;
use super::shared::EngineStats;
use super::types::{FramePos, RenderedEvent, StreamFormat};

/// How to open the output device.
#[derive(Clone, Debug)]
pub struct OutputRequest {
    /// Persisted device id, or `None` for the system default output.
    pub device_id: Option<String>,
    /// Smallest buffer to use (a floor); the device minimum wins if it is larger.
    pub target_buffer_frames: u32,
}

impl Default for OutputRequest {
    fn default() -> Self {
        Self {
            device_id: None,
            target_buffer_frames: 64,
        }
    }
}

/// Output latency measured from the first callbacks of a stream.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeasuredLatency {
    pub callback_frames: u32,
    pub output_latency_ns: u64,
}

impl MeasuredLatency {
    pub fn millis(&self) -> f64 {
        self.output_latency_ns as f64 / 1_000_000.0
    }
}

/// Builds a stopped output stream wired to the core. The real one uses cpal;
/// tests inject builders whose streams fail on purpose.
pub(crate) type OutputBuilder = Arc<
    dyn Fn(&OutputRequest, &Arc<CoreCell>) -> Result<(OutputStream, StreamFormat), AudioError>
        + Send
        + Sync,
>;

fn cpal_builder() -> OutputBuilder {
    Arc::new(|request: &OutputRequest, cell: &Arc<CoreCell>| {
        let device = find_device(request.device_id.as_deref(), Direction::Output)?;
        build_output_stream(&device, cell, request.target_buffer_frames)
    })
}

/// A built, not yet started output stream (see [`AudioEngine::commit_output`]).
pub struct PendingOutput {
    output: OutputStream,
    format: StreamFormat,
}

/// Builds output streams without holding the engine: the slow part of a
/// switch (device lookup, stream creation) runs outside the engine's mutex.
#[derive(Clone)]
pub struct OutputStarter {
    builder: OutputBuilder,
    cell: Arc<CoreCell>,
}

impl OutputStarter {
    /// Builds a stopped stream for `request`. The running output is untouched.
    pub fn build(&self, request: &OutputRequest) -> Result<PendingOutput, AudioError> {
        let (output, format) = (self.builder)(request, &self.cell)?;
        Ok(PendingOutput { output, format })
    }
}

/// Owns the render core and the output stream. Keep it behind a mutex on
/// normal threads; the audio thread never touches the mutex.
pub struct AudioEngine {
    handle: EngineHandle,
    cell: Arc<CoreCell>,
    output: Option<OutputStream>,
    builder: OutputBuilder,
}

impl AudioEngine {
    /// Creates the engine with no output open yet. The returned consumer
    /// receives reports of rendered `notify` events.
    pub fn new(config: EngineConfig) -> (Self, Consumer<RenderedEvent>) {
        Self::with_builder(config, cpal_builder())
    }

    pub(crate) fn with_builder(
        config: EngineConfig,
        builder: OutputBuilder,
    ) -> (Self, Consumer<RenderedEvent>) {
        let (handle, core, notifications) = create_engine(StreamFormat::new(48_000, 2), config);
        (
            Self {
                handle,
                cell: Arc::new(CoreCell::new(core)),
                output: None,
                builder,
            },
            notifications,
        )
    }

    /// A handle for building streams outside the engine's mutex.
    pub fn output_starter(&self) -> OutputStarter {
        OutputStarter {
            builder: Arc::clone(&self.builder),
            cell: Arc::clone(&self.cell),
        }
    }

    pub fn handle(&mut self) -> &mut EngineHandle {
        &mut self.handle
    }

    pub fn format(&self) -> StreamFormat {
        self.handle.format()
    }

    pub fn stats(&self) -> EngineStats {
        self.handle.stats()
    }

    pub fn output_info(&self) -> Option<&OutputStreamInfo> {
        self.output.as_ref().map(|output| &output.info)
    }

    /// Opens (or switches to) an output device: [`OutputStarter::build`]
    /// followed by [`Self::commit_output`].
    pub fn open_output(&mut self, request: &OutputRequest) -> Result<OutputStreamInfo, AudioError> {
        let pending = self.output_starter().build(request)?;
        self.commit_output(pending)
    }

    /// Switches to a built stream. Sources, scheduled events and the frame
    /// clock carry over; sources are re-prepared for the new format.
    ///
    /// The switch only becomes final once the new stream plays. Until then
    /// the previous stream is paused, not dropped: if the new stream fails to
    /// start, the core returns to the previous format and generation and the
    /// previous stream resumes, so a failed switch leaves output as it was.
    pub fn commit_output(
        &mut self,
        pending: PendingOutput,
    ) -> Result<OutputStreamInfo, AudioError> {
        let PendingOutput { output, format } = pending;
        let previous_format = self.handle.format();
        let previous_generation = self.generation();
        // Pause (not drop) the old stream so its callback releases the core
        // and it can resume if the new stream does not start.
        let previous = self.output.take().filter(|old| old.stream.pause().is_ok());
        self.cell.with_core(|core| core.reformat(format));
        self.handle.set_format(format);

        match output.stream.play() {
            Ok(()) => {
                drop(previous);
                let info = output.info.clone();
                self.output = Some(output);
                Ok(info)
            }
            Err(error) => {
                drop(output);
                self.cell
                    .with_core(|core| core.restore(previous_format, previous_generation));
                self.handle.set_format(previous_format);
                if let Some(old) = previous {
                    match old.stream.play() {
                        Ok(()) => self.output = Some(old),
                        Err(resume) => {
                            log::error!(
                                "audio output: cannot resume the previous stream: {resume}"
                            );
                        }
                    }
                }
                Err(error)
            }
        }
    }

    /// Stops output. The core stays alive, so a later `open_output` resumes it.
    pub fn close_output(&mut self) {
        // Dropping the stream drops the callback's clone of the cell.
        self.output = None;
    }

    /// Pending backend error on the output stream (not cleared; a reopened
    /// stream starts without one).
    pub fn stream_error(&self) -> Option<StreamErrorKind> {
        let output = self.output.as_ref()?;
        StreamErrorKind::from_code(output.error.load(Ordering::Relaxed))
    }

    /// Last backend error on the output stream, cleared on read.
    pub fn take_stream_error(&self) -> Option<StreamErrorKind> {
        let output = self.output.as_ref()?;
        StreamErrorKind::from_code(output.error.swap(0, Ordering::Relaxed))
    }

    /// Current output latency (callback → DAC) from the latest callback.
    pub fn output_latency(&self) -> Option<MeasuredLatency> {
        self.output.as_ref()?;
        let timing = self.handle.timing()?;
        Some(MeasuredLatency {
            callback_frames: self.handle.stats().max_callback_frames,
            output_latency_ns: timing.output_latency_ns(),
        })
    }

    /// Waits (polling) until the stream has run a few callbacks and returns
    /// the measured latency. Call off the audio thread, never while holding a
    /// lock the UI needs for long.
    pub fn wait_for_latency(&self, timeout: Duration) -> Option<MeasuredLatency> {
        let start_callbacks = self.handle.stats().callbacks;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.handle.stats().callbacks >= start_callbacks + 4 {
                return self.output_latency();
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    }

    /// Output generation; changes whenever an output stream is (re)opened.
    pub fn generation(&self) -> u64 {
        self.handle.generation()
    }

    /// Unix-epoch milliseconds at which a rendered event is heard. `None` for
    /// events from an older generation or when no output is running.
    pub fn event_heard_at_epoch_ms(&self, event: &RenderedEvent) -> Option<f64> {
        let output = self.output.as_ref()?;
        let clock = ClockPair::now(output.stream.now_ns());
        event_heard_at_epoch_ms(event, self.generation(), clock)
    }

    /// Unix-epoch milliseconds at which `frame` is heard, or `None` when no
    /// output is running yet.
    pub fn heard_at_epoch_ms(&self, frame: FramePos) -> Option<f64> {
        let output = self.output.as_ref()?;
        let snapshot = self.handle.timing()?;
        let clock = ClockPair::now(output.stream.now_ns());
        Some(heard_at_epoch_ms(&snapshot, clock, frame))
    }
}

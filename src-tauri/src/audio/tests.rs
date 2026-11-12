//! Engine tests, all on the offline renderer (no audio device needed).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cpal::SupportedBufferSize;

use super::device::buffer_candidates;
use super::testing::{ConstantSource, ImpulseSource};
use super::*;

// ---- allocation tracking -------------------------------------------------

struct CountingAllocator;

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn note_allocation() {
    let tracking = TRACKING.try_with(Cell::get).unwrap_or(false);
    if tracking {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
    }
}

// SAFETY: forwards to the system allocator; only adds thread-local counting.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note_allocation();
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note_allocation();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Runs `f` and returns how many allocations/frees it made on this thread.
fn count_allocations(f: impl FnOnce()) -> usize {
    ALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|tracking| tracking.set(true));
    f();
    TRACKING.with(|tracking| tracking.set(false));
    ALLOCATIONS.with(Cell::get)
}

// ---- helpers -------------------------------------------------------------

const RATE: u32 = 48_000;

fn engine(channels: u16) -> OfflineEngine {
    OfflineEngine::new(StreamFormat::new(RATE, channels))
}

fn trigger(velocity: f32) -> EventKind {
    EventKind::Trigger { note: 0, velocity }
}

/// Frames whose first channel is non-zero.
fn nonzero_frames(buffer: &[f32], channels: usize) -> Vec<usize> {
    buffer
        .chunks_exact(channels)
        .enumerate()
        .filter(|(_, frame)| frame[0] != 0.0)
        .map(|(index, _)| index)
        .collect()
}

// ---- scheduling ------------------------------------------------------------

#[test]
fn event_renders_at_the_requested_frame_with_zero_error() {
    let callback_sizes = [1, 7, 64, 100, 128, 333, 1024, 1500, 4096];
    let targets = [0_u64, 1, 63, 64, 127, 128, 1000, 1023, 1024, 4097, 12_345];
    for &callback in &callback_sizes {
        for &target in &targets {
            let mut engine = engine(2);
            let id = engine
                .handle
                .add_source(Box::new(ImpulseSource::default()), 1.0)
                .unwrap();
            engine.handle.schedule(target, id, trigger(1.0)).unwrap();
            let out = engine.render(16_000, callback);

            let hits = nonzero_frames(&out, 2);
            assert_eq!(
                hits,
                vec![target as usize],
                "callback {callback} frames, target frame {target}"
            );
            let frame = target as usize * 2;
            assert_eq!(&out[frame..frame + 2], &[1.0, 1.0], "both channels");
        }
    }
}

#[test]
fn many_events_render_at_their_frames_regardless_of_schedule_order() {
    let mut engine = engine(1);
    let id = engine
        .handle
        .add_source(Box::new(ImpulseSource::default()), 1.0)
        .unwrap();
    let frames = [9_000_u64, 5, 4_096, 128, 7_777, 129, 2_048];
    for &frame in &frames {
        engine.handle.schedule(frame, id, trigger(0.5)).unwrap();
    }
    let out = engine.render(10_000, 256);

    let mut expected: Vec<usize> = frames.iter().map(|&f| f as usize).collect();
    expected.sort_unstable();
    assert_eq!(nonzero_frames(&out, 1), expected);
}

#[test]
fn start_and_stop_switch_on_exact_frames() {
    let mut engine = engine(2);
    let id = engine
        .handle
        .add_source(Box::new(ConstantSource::new(0.25)), 1.0)
        .unwrap();
    engine.handle.schedule(300, id, EventKind::Start).unwrap();
    engine.handle.schedule(700, id, EventKind::Stop).unwrap();
    let out = engine.render(1_000, 64);

    let hits = nonzero_frames(&out, 2);
    assert_eq!(hits.first(), Some(&300));
    assert_eq!(hits.last(), Some(&699));
    assert_eq!(hits.len(), 400);
}

#[test]
fn events_on_the_same_frame_apply_in_schedule_order() {
    let mut engine = engine(1);
    let id = engine
        .handle
        .add_source(Box::new(ConstantSource::new(1.0)), 1.0)
        .unwrap();
    // Start then Stop on the same frame: the source must end up stopped.
    engine.handle.schedule(10, id, EventKind::Start).unwrap();
    engine.handle.schedule(10, id, EventKind::Stop).unwrap();
    let out = engine.render(100, 32);
    assert!(nonzero_frames(&out, 1).is_empty());
}

#[test]
fn late_event_renders_on_the_next_frame_and_is_counted() {
    let mut engine = engine(1);
    let id = engine
        .handle
        .add_source(Box::new(ImpulseSource::default()), 1.0)
        .unwrap();
    engine.render(1_000, 100);
    engine
        .handle
        .schedule_notify(500, id, trigger(1.0))
        .unwrap();
    let out = engine.render(200, 100);

    assert_eq!(
        nonzero_frames(&out, 1),
        vec![0],
        "first frame of next callback"
    );
    assert_eq!(engine.handle.stats().late_events, 1);
    let report = engine.take_notifications();
    assert_eq!(report.len(), 1);
    assert_eq!(report[0].scheduled_frame, 500);
    assert_eq!(report[0].rendered_frame, 1_000);
}

#[test]
fn notify_reports_the_rendered_frame() {
    let mut engine = engine(2);
    let id = engine
        .handle
        .add_source(Box::new(ImpulseSource::default()), 1.0)
        .unwrap();
    engine
        .handle
        .schedule_notify(2_000, id, trigger(1.0))
        .unwrap();
    engine.handle.schedule(2_100, id, trigger(1.0)).unwrap();
    engine.render(4_000, 128);

    let report = engine.take_notifications();
    assert_eq!(
        report,
        vec![RenderedEvent {
            scheduled_frame: 2_000,
            rendered_frame: 2_000,
            target: id,
            kind: trigger(1.0),
        }]
    );
    assert_eq!(engine.handle.stats().late_events, 0);
}

#[test]
fn cancel_drops_pending_events_from_a_frame_on() {
    let mut engine = engine(1);
    let id = engine
        .handle
        .add_source(Box::new(ImpulseSource::default()), 1.0)
        .unwrap();
    for frame in [100, 200, 300, 400] {
        engine.handle.schedule(frame, id, trigger(1.0)).unwrap();
    }
    engine.handle.cancel(id, 250).unwrap();
    let out = engine.render(1_000, 64);
    assert_eq!(nonzero_frames(&out, 1), vec![100, 200]);
}

#[test]
fn full_schedule_drops_and_counts_extra_events() {
    let config = EngineConfig {
        max_events: 4,
        ..EngineConfig::default()
    };
    let mut engine = OfflineEngine::with_config(StreamFormat::new(RATE, 1), config);
    let id = engine
        .handle
        .add_source(Box::new(ImpulseSource::default()), 1.0)
        .unwrap();
    for frame in 1..=6 {
        engine
            .handle
            .schedule(frame * 10, id, trigger(1.0))
            .unwrap();
    }
    let out = engine.render(100, 100);
    assert_eq!(nonzero_frames(&out, 1), vec![10, 20, 30, 40]);
    assert_eq!(engine.handle.stats().dropped_events, 2);
}

// ---- mixer -------------------------------------------------------------------

#[test]
fn sources_mix_with_their_gains() {
    let mut engine = engine(2);
    let a = engine
        .handle
        .add_source(Box::new(ConstantSource::new(0.5)), 0.5)
        .unwrap();
    let b = engine
        .handle
        .add_source(Box::new(ConstantSource::new(0.2)), 1.0)
        .unwrap();
    engine.handle.schedule(0, a, EventKind::Start).unwrap();
    engine.handle.schedule(0, b, EventKind::Start).unwrap();
    let out = engine.render(64, 64);
    assert!(out.iter().all(|&s| (s - 0.45).abs() < 1.0e-6));
}

#[test]
fn gain_change_glides_without_jumps() {
    let mut engine = engine(1);
    let id = engine
        .handle
        .add_source(Box::new(ConstantSource::new(1.0)), 1.0)
        .unwrap();
    engine.handle.schedule(0, id, EventKind::Start).unwrap();
    engine.render(128, 128);
    engine.handle.set_gain(id, 0.0).unwrap();
    let out = engine.render(RATE as usize / 10, 128);

    let largest_step = out
        .windows(2)
        .map(|pair| (pair[0] - pair[1]).abs())
        .fold(0.0_f32, f32::max);
    assert!(largest_step < 0.01, "largest step {largest_step}");
    assert_eq!(*out.last().unwrap(), 0.0, "reaches the target");
}

#[test]
fn output_is_clamped_to_full_scale() {
    let mut engine = engine(1);
    for _ in 0..3 {
        let id = engine
            .handle
            .add_source(Box::new(ConstantSource::new(0.9)), 1.0)
            .unwrap();
        engine.handle.schedule(0, id, EventKind::Start).unwrap();
    }
    let out = engine.render(16, 16);
    assert!(out.iter().all(|&s| s == 1.0));
}

struct DropFlag(Arc<AtomicBool>);

impl Source for DropFlag {
    fn on_event(&mut self, _: &EventKind) {}
    fn render(&mut self, _: &mut [f32], _: usize) {}
}

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[test]
fn removed_source_is_freed_by_the_handle_not_the_render_call() {
    let dropped = Arc::new(AtomicBool::new(false));
    let mut engine = engine(1);
    let id = engine
        .handle
        .add_source(Box::new(DropFlag(Arc::clone(&dropped))), 1.0)
        .unwrap();
    engine.render(64, 64);
    engine.handle.remove_source(id).unwrap();
    engine.render(64, 64);
    assert!(!dropped.load(Ordering::SeqCst), "render must not free it");
    engine.handle.collect_retired();
    assert!(dropped.load(Ordering::SeqCst));
}

#[test]
fn full_mixer_rejects_and_counts_extra_sources() {
    let config = EngineConfig {
        max_sources: 2,
        ..EngineConfig::default()
    };
    let mut engine = OfflineEngine::with_config(StreamFormat::new(RATE, 1), config);
    for _ in 0..3 {
        let id = engine
            .handle
            .add_source(Box::new(ConstantSource::new(0.1)), 1.0)
            .unwrap();
        engine.handle.schedule(0, id, EventKind::Start).unwrap();
    }
    let out = engine.render(8, 8);
    assert!((out[0] - 0.2).abs() < 1.0e-6);
    assert_eq!(engine.handle.stats().dropped_commands, 1);
}

// ---- real-time safety ----------------------------------------------------------

#[test]
fn allocation_counter_detects_allocations() {
    let allocations = count_allocations(|| {
        let boxed = std::hint::black_box(Box::new(42_u64));
        drop(boxed);
    });
    assert_eq!(allocations, 2, "one allocation and one free");
}

#[test]
fn render_callback_never_allocates_or_frees() {
    let mut engine = engine(2);
    let click = engine
        .handle
        .add_source(Box::new(ClickSource::new(1_000.0, 0.02)), 0.8)
        .unwrap();
    let constant = engine
        .handle
        .add_source(Box::new(ConstantSource::new(0.1)), 1.0)
        .unwrap();
    let impulse = engine
        .handle
        .add_source(Box::new(ImpulseSource::default()), 1.0)
        .unwrap();
    let mut buffer = vec![0.0_f32; 4_096 * 2];
    engine.render_callback(&mut buffer[..256]);

    for frame in (300..20_000).step_by(250) {
        engine
            .handle
            .schedule_notify(frame, click, trigger(1.0))
            .unwrap();
        engine
            .handle
            .schedule(frame + 3, impulse, trigger(1.0))
            .unwrap();
    }
    engine
        .handle
        .schedule(400, constant, EventKind::Start)
        .unwrap();
    engine.handle.set_gain(constant, 0.3).unwrap();
    engine.handle.remove_source(impulse).unwrap();
    engine.handle.cancel(click, 15_000).unwrap();

    let allocations = count_allocations(|| {
        for size in [64, 128, 1, 333, 4_096, 64, 2_000] {
            engine.render_callback(&mut buffer[..size * 2]);
        }
    });
    assert_eq!(allocations, 0);
}

// ---- clock -----------------------------------------------------------------------

#[test]
fn offline_timing_snapshot_tracks_the_callback_start() {
    let mut engine = engine(2);
    assert!(engine.handle.timing().is_none());
    engine.render(480, 480);
    engine.render(480, 480);
    let snapshot = engine.handle.timing().unwrap();
    assert_eq!(snapshot.block_frame, 480);
    assert_eq!(snapshot.playback_ns, 10_000_000);
    assert_eq!(snapshot.playback_ns_of(960), 20_000_000);
    assert_eq!(snapshot.playback_ns_of(0), 0);
}

#[test]
fn heard_time_adds_output_latency_and_frame_offset() {
    let snapshot = TimingSnapshot {
        block_frame: 48_000,
        callback_ns: 5_000_000_000,
        playback_ns: 5_004_000_000, // 4 ms output latency
        sample_rate: RATE,
    };
    assert_eq!(snapshot.output_latency_ns(), 4_000_000);
    let clock = ClockPair {
        stream_now_ns: 5_001_000_000,
        epoch_now_ms: 1_700_000_000_000.0,
    };
    // Frame 48 000 plays 3 ms after "now"; 480 frames later is 10 ms more.
    assert!((heard_at_epoch_ms(&snapshot, clock, 48_000) - 1_700_000_000_003.0).abs() < 1.0e-6);
    assert!((heard_at_epoch_ms(&snapshot, clock, 48_480) - 1_700_000_000_013.0).abs() < 1.0e-6);
}

// ---- device glue -------------------------------------------------------------------

#[test]
fn buffer_candidates_start_at_the_device_minimum() {
    let range = |min, max| SupportedBufferSize::Range { min, max };
    assert_eq!(
        buffer_candidates(range(14, 4_096), 64),
        vec![Some(64), Some(128), Some(256), Some(512), None]
    );
    assert_eq!(
        buffer_candidates(range(96, 4_096), 64),
        vec![Some(96), Some(128), Some(256), Some(512), None]
    );
    assert_eq!(
        buffer_candidates(range(256, 300), 64),
        vec![Some(256), None]
    );
    assert_eq!(
        buffer_candidates(SupportedBufferSize::Unknown, 64),
        vec![Some(64), Some(128), Some(256), Some(512), None]
    );
}

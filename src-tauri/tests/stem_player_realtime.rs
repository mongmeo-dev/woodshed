//! The stem player's render path never allocates or frees (audio thread
//! rule). Own test binary so it can install a counting global allocator.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use woodshed_lib::audio::{EventKind, OfflineEngine, StreamFormat};
use woodshed_lib::stems::{LoopRange, MemoryTrack, StemPlayer, TrackReader, loop_events, param};

struct CountingAllocator;

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn note_allocation() {
    if TRACKING.try_with(Cell::get).unwrap_or(false) {
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

fn count_allocations(f: impl FnOnce()) -> usize {
    ALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|tracking| tracking.set(true));
    f();
    TRACKING.with(|tracking| tracking.set(false));
    ALLOCATIONS.with(Cell::get)
}

fn sine(rate: u32, frames: usize, hz: f32) -> Box<dyn TrackReader> {
    let samples: Vec<f32> = (0..frames)
        .map(|i| (std::f32::consts::TAU * hz * i as f32 / rate as f32).sin() * 0.3)
        .collect();
    Box::new(MemoryTrack::mono(rate, &samples))
}

#[test]
fn stem_render_never_allocates_or_frees() {
    let tracks: Vec<Box<dyn TrackReader>> = (0..6)
        .map(|i| sine(44_100, 44_100 * 3, 110.0 * (i + 1) as f32))
        .collect();
    let player = StemPlayer::new(tracks, 0).unwrap();
    let mut feeder = player.feeder;
    let mut engine = OfflineEngine::new(StreamFormat::new(48_000, 2));
    let id = engine
        .handle
        .add_source(Box::new(player.source), 1.0)
        .unwrap();

    // Every kind of control, spread over the render.
    let mut events = vec![
        (0, EventKind::Start),
        (
            2_000,
            EventKind::Param {
                id: param::track(1, param::MUTE),
                value: 1.0,
            },
        ),
        (
            4_000,
            EventKind::Param {
                id: param::track(3, param::SOLO),
                value: 1.0,
            },
        ),
        (
            6_000,
            EventKind::Param {
                id: param::track(0, param::VOLUME),
                value: 0.3,
            },
        ),
        (8_000, EventKind::Seek { frame: 30_000 }),
        (
            14_000,
            EventKind::Param {
                id: param::SPEED,
                value: 0.7,
            },
        ),
        (
            20_000,
            EventKind::Param {
                id: param::SPEED,
                value: 0.55,
            },
        ),
        (
            26_000,
            EventKind::Param {
                id: param::SPEED,
                value: 1.0,
            },
        ),
        (36_000, EventKind::Stop),
        (40_000, EventKind::Start),
        (100_000, EventKind::Seek { frame: 120_000 }),
    ];
    for event in loop_events(LoopRange::new(40_000, 60_000)) {
        events.push((30_000, event));
    }
    for event in loop_events(None) {
        events.push((80_000, event));
    }
    for (frame, kind) in events {
        engine.handle.schedule(frame, id, kind).unwrap();
    }

    let mut buffer = vec![0.0_f32; 128 * 2];
    let mut allocations = 0;
    for _ in 0..(200_000 / 128) {
        feeder.fill();
        allocations += count_allocations(|| engine.render_callback(&mut buffer));
    }
    assert_eq!(allocations, 0);
    assert!(player.shared.ended(), "played through to the end");
}

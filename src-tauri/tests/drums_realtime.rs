//! The drum source must not allocate or free on the audio thread. Lives in
//! its own test binary so it can install its own counting allocator.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;

use woodshed_lib::audio::{OfflineEngine, StreamFormat};
use woodshed_lib::drums::{DrumSource, Kit, MAX_LEVEL, Pattern, Sequencer, SwingUnit, Track};

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

#[test]
fn rendering_the_drum_machine_never_allocates_or_frees() {
    let kit = Arc::new(Kit::bundled().expect("bundled kit"));
    let mut engine = OfflineEngine::new(StreamFormat::new(48_000, 2));
    let source = engine
        .handle
        .add_source(Box::new(DrumSource::new(Arc::clone(&kit))), 0.8)
        .unwrap();

    // Every track on every step: retriggers, open hats choked by closed
    // hats, gain changes and a reschedule all happen inside the render.
    let mut pattern = Pattern {
        bpm: 300,
        swing: 66,
        swing_unit: SwingUnit::Sixteenth,
        ..Pattern::default()
    }
    .normalized();
    for track in Track::ALL {
        let entry = pattern.tracks.get_mut(&track).unwrap();
        entry.steps.fill(MAX_LEVEL);
        entry.volume = 0.6;
    }
    let mut sequencer = Sequencer::new(source, pattern.clone());
    sequencer.start(&mut engine.handle, 256).unwrap();
    sequencer.fill(&mut engine.handle, 24_000).unwrap();
    pattern.tracks.get_mut(&Track::Snare).unwrap().muted = true;
    sequencer
        .set_pattern(&mut engine.handle, pattern, 12_000)
        .unwrap();
    sequencer.fill(&mut engine.handle, 48_000).unwrap();

    let mut buffer = vec![0.0_f32; 4_096 * 2];
    let allocations = count_allocations(|| {
        let mut rendered = 0;
        for size in [64, 128, 1, 333, 4_096, 64, 2_000].iter().cycle() {
            engine.render_callback(&mut buffer[..size * 2]);
            rendered += size;
            if rendered > 48_000 {
                break;
            }
        }
    });
    assert_eq!(allocations, 0);
    assert!(buffer.iter().any(|&v| v != 0.0), "the kit played");
}

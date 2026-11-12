//! Offline render tests for the metronome: the real scheduler driving the
//! real engine core, pumped every 10 ms of audio the way the worker thread
//! does, with no audio device.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::{Arc, Mutex};

use woodshed_lib::audio::{EventKind, OfflineEngine, Source, StreamFormat};
use woodshed_lib::metronome::scheduler::Announcement;
use woodshed_lib::metronome::source::{Variant, note, render_tables};
use woodshed_lib::metronome::timeline::{
    NORMAL_VELOCITY, POLY_VELOCITY, STRONG_VELOCITY, SUBDIVISION_VELOCITY, WEAK_VELOCITY,
};
use woodshed_lib::metronome::{
    Accent, MetronomeSettings, MetronomeSource, Polyrhythm, Scheduler, Sound, Trainer, TrainerEnd,
};

// ---- allocation tracking (this test binary's own allocator) -----------------

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

// ---- helpers -------------------------------------------------------------------

/// A trigger as the source received it: the frame it landed on, note, velocity.
type Hit = (u64, u8, f32);

/// Records every trigger at the exact frame the engine delivers it.
struct RecordingSource {
    frame: u64,
    hits: Arc<Mutex<Vec<Hit>>>,
}

impl Source for RecordingSource {
    fn on_event(&mut self, kind: &EventKind) {
        if let EventKind::Trigger { note, velocity } = *kind {
            self.hits.lock().unwrap().push((self.frame, note, velocity));
        }
    }

    fn render(&mut self, out: &mut [f32], channels: usize) {
        self.frame += (out.len() / channels) as u64;
    }
}

/// Simulates the worker: pumps every `TICK` frames, renders device callbacks.
struct Rig {
    engine: OfflineEngine,
    scheduler: Scheduler,
    generation: u64,
    announced: Vec<Announcement>,
    hits: Arc<Mutex<Vec<Hit>>>,
    since_pump: usize,
}

const TICK_SECS: f64 = 0.01;

impl Rig {
    fn recording(settings: &MetronomeSettings, rate: u32, start: u64) -> Self {
        let hits = Arc::new(Mutex::new(Vec::new()));
        let source = RecordingSource {
            frame: 0,
            hits: Arc::clone(&hits),
        };
        Self::with_source(Box::new(source), settings, rate, start, hits)
    }

    fn with_source(
        source: Box<dyn Source>,
        settings: &MetronomeSettings,
        rate: u32,
        start: u64,
        hits: Arc<Mutex<Vec<Hit>>>,
    ) -> Self {
        let mut engine = OfflineEngine::new(StreamFormat::new(rate, 1));
        let id = engine.handle.add_source(source, 1.0).unwrap();
        let generation = engine.handle.generation();
        Self {
            engine,
            scheduler: Scheduler::new(id, settings, rate, start),
            generation,
            announced: Vec::new(),
            hits,
            since_pump: usize::MAX,
        }
    }

    fn rate(&self) -> u32 {
        self.engine.format().sample_rate
    }

    fn now(&self) -> u64 {
        self.engine.handle.frames_rendered()
    }

    fn pump(&mut self) {
        let now = self.now();
        let generation = self.engine.handle.generation();
        if generation != self.generation {
            let rate = self.rate();
            self.scheduler
                .retime(&mut self.engine.handle, now, rate, &mut self.announced)
                .unwrap();
            self.generation = generation;
        }
        self.scheduler
            .pump(&mut self.engine.handle, now, &mut self.announced)
            .unwrap();
    }

    /// Renders `frames` in `callback`-frame callbacks, handing each rendered
    /// callback to `inspect` with its first frame.
    fn run_with(&mut self, frames: usize, callback: usize, mut inspect: impl FnMut(u64, &[f32])) {
        let tick = (TICK_SECS * f64::from(self.rate())) as usize;
        let mut buffer = vec![0.0_f32; callback];
        let mut done = 0;
        while done < frames {
            if self.since_pump >= tick {
                self.pump();
                self.since_pump = 0;
            }
            let n = callback.min(frames - done);
            let first = self.now();
            self.engine.render_callback(&mut buffer[..n]);
            inspect(first, &buffer[..n]);
            done += n;
            self.since_pump += n;
        }
    }

    fn run(&mut self, secs: f64) {
        let frames = (secs * f64::from(self.rate())) as usize;
        self.run_with(frames, 128, |_, _| {});
    }

    fn update(&mut self, settings: &MetronomeSettings) {
        let now = self.now();
        self.scheduler
            .update(&mut self.engine.handle, now, settings, &mut self.announced)
            .unwrap();
    }

    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().unwrap().clone()
    }

    fn hits_of(&self, note: u8) -> Vec<u64> {
        self.hits()
            .into_iter()
            .filter(|&(_, n, _)| n == note)
            .map(|(frame, _, _)| frame)
            .collect()
    }

    fn assert_on_time(&self) {
        let stats = self.engine.handle.stats();
        assert_eq!(stats.late_events, 0, "late events");
        assert_eq!(stats.dropped_events, 0, "dropped events");
    }
}

fn frames_for(beats: f64, rate: u32, bpm: f64) -> u64 {
    (beats * 60.0 * f64::from(rate) / bpm).round() as u64
}

fn settings(f: impl FnOnce(&mut MetronomeSettings)) -> MetronomeSettings {
    let mut settings = MetronomeSettings::default();
    f(&mut settings);
    settings
}

// ---- drift -----------------------------------------------------------------------

/// c1: at 120.0 BPM every click of a ten-minute render starts exactly on
/// `start + n × 24 000` frames (48 kHz), read from the rendered audio itself.
#[test]
fn ten_minutes_at_120_bpm_have_zero_drift_in_the_rendered_audio() {
    const RATE: u32 = 48_000;
    const START: u64 = 2_880;
    const SECS: usize = 600;
    let mut rig = Rig::with_source(
        Box::new(MetronomeSource::new()),
        &MetronomeSettings::default(),
        RATE,
        START,
        Arc::default(),
    );

    // An onset is a non-zero sample after at least 64 silent ones.
    let mut onsets: Vec<(u64, f32)> = Vec::new();
    let mut silent = usize::MAX;
    rig.run_with(SECS * RATE as usize, 128, |first, samples| {
        for (offset, &sample) in samples.iter().enumerate() {
            if sample == 0.0 {
                silent = silent.saturating_add(1);
            } else {
                if silent >= 64 {
                    onsets.push((first + offset as u64, sample));
                }
                silent = 0;
            }
        }
    });

    let tables = render_tables(RATE);
    let strong = tables[usize::from(note(Sound::Click, Variant::High))][0] * STRONG_VELOCITY;
    let normal = tables[usize::from(note(Sound::Click, Variant::Low))][0] * NORMAL_VELOCITY;
    let expected: Vec<(u64, f32)> = (0..)
        .map(|n: u64| {
            let first = if n.is_multiple_of(4) { strong } else { normal };
            (START + n * 24_000, first)
        })
        .take_while(|&(frame, _)| frame < (SECS as u64) * u64::from(RATE))
        .collect();
    assert_eq!(expected.len(), 1_200);
    assert_eq!(onsets.len(), expected.len(), "click count");
    for (n, (onset, want)) in onsets.iter().zip(&expected).enumerate() {
        assert_eq!(onset.0, want.0, "click {n} frame");
        assert_eq!(onset.1, want.1, "click {n} accent level");
    }
    rig.assert_on_time();
}

// ---- rules -----------------------------------------------------------------------

#[test]
fn meter_and_accents_render_per_beat() {
    let accents = vec![
        Accent::Strong,
        Accent::Weak,
        Accent::Normal,
        Accent::Mute,
        Accent::Normal,
        Accent::Weak,
        Accent::Weak,
    ];
    let s = settings(|s| {
        s.bpm = 210.0;
        s.beats = 7;
        s.note_value = 8;
        s.accents = accents.clone();
        s.sound = Sound::Woodblock;
    });
    let mut rig = Rig::recording(&s, 48_000, 1_000);
    rig.run(3.0 * 7.0 * 60.0 / 210.0 + 0.05); // three bars

    let high = note(Sound::Woodblock, Variant::High);
    let low = note(Sound::Woodblock, Variant::Low);
    let mut expected = Vec::new();
    for n in 0..21_u64 {
        let frame = 1_000 + frames_for(n as f64, 48_000, 210.0);
        match accents[(n % 7) as usize] {
            Accent::Strong => expected.push((frame, high, STRONG_VELOCITY)),
            Accent::Normal => expected.push((frame, low, NORMAL_VELOCITY)),
            Accent::Weak => expected.push((frame, low, WEAK_VELOCITY)),
            Accent::Mute => {}
        }
    }
    let hits = rig.hits();
    assert_eq!(&hits[..expected.len()], &expected[..]);
    assert_eq!(
        hits.len(),
        expected.len() + 1,
        "the next bar's downbeat follows"
    );

    let bars: Vec<_> = rig
        .announced
        .iter()
        .filter_map(|a| match a {
            Announcement::Beat(mark) => Some((mark.bar, mark.beat, mark.beats, mark.accent)),
            _ => None,
        })
        .take(8)
        .collect();
    assert_eq!(
        bars[3],
        (0, 3, 7, Accent::Mute),
        "muted beats are still announced"
    );
    assert_eq!(bars[7], (1, 0, 7, Accent::Strong));
    rig.assert_on_time();
}

#[test]
fn subdivisions_split_every_beat_evenly() {
    for subdivision in 1..=6_u8 {
        let s = settings(|s| s.subdivision = subdivision);
        let mut rig = Rig::recording(&s, 44_100, 500);
        rig.run(4.0 * 0.5 + 0.05); // four beats at 120 BPM

        let low = note(Sound::Click, Variant::Low);
        let high = note(Sound::Click, Variant::High);
        let sub = f64::from(subdivision);
        let mut expected = Vec::new();
        for beat in 0..4_u64 {
            let frame = 500 + frames_for(beat as f64, 44_100, 120.0);
            let accent = if beat == 0 {
                (high, STRONG_VELOCITY)
            } else {
                (low, NORMAL_VELOCITY)
            };
            expected.push((frame, accent.0, accent.1));
            for step in 1..subdivision {
                let at = beat as f64 + f64::from(step) / sub;
                expected.push((
                    500 + frames_for(at, 44_100, 120.0),
                    low,
                    SUBDIVISION_VELOCITY,
                ));
            }
        }
        let hits = rig.hits();
        assert_eq!(
            &hits[..expected.len()],
            &expected[..],
            "subdivision {subdivision}"
        );
        rig.assert_on_time();
    }
}

#[test]
fn polyrhythm_layer_plays_its_ratio_with_its_own_sound() {
    for (pulses, beats) in [(3_u8, 2_u8), (4, 3), (5, 4)] {
        let s = settings(|s| {
            s.polyrhythm = Polyrhythm {
                enabled: true,
                pulses,
                beats,
                sound: Sound::Cowbell,
            };
        });
        let mut rig = Rig::recording(&s, 48_000, 0);
        let cycles = 3_u64;
        rig.run(cycles as f64 * f64::from(beats) * 0.5 + 0.05);

        let poly = note(Sound::Cowbell, Variant::High);
        let expected: Vec<u64> = (0..cycles * u64::from(pulses))
            .map(|p| {
                let at = p as f64 * f64::from(beats) / f64::from(pulses);
                frames_for(at, 48_000, 120.0)
            })
            .collect();
        let got = rig.hits_of(poly);
        assert_eq!(&got[..expected.len()], &expected[..], "{pulses}:{beats}");
        assert!(
            rig.hits()
                .iter()
                .filter(|&&(_, n, _)| n == poly)
                .all(|&(_, _, velocity)| velocity == POLY_VELOCITY)
        );
        // The main layer is untouched.
        let main: Vec<u64> = rig
            .hits()
            .into_iter()
            .filter(|&(_, n, _)| n != poly)
            .map(|(frame, _, _)| frame)
            .take(4)
            .collect();
        assert_eq!(main, vec![0, 24_000, 48_000, 72_000]);
        rig.assert_on_time();
    }
}

/// Beat frames of a trainer run, computed independently: each tempo step
/// starts where the previous one's last bar ended.
fn trainer_frames(start: u64, rate: u32, tempos: &[(f64, u64)]) -> Vec<u64> {
    let mut frames = Vec::new();
    let mut segment = start;
    for &(bpm, beats) in tempos {
        for n in 0..beats {
            frames.push(segment + frames_for(n as f64, rate, bpm));
        }
        segment += frames_for(beats as f64, rate, bpm);
    }
    frames
}

#[test]
fn tempo_trainer_steps_up_every_n_bars_and_holds_at_max() {
    let s = settings(|s| {
        s.bpm = 100.0;
        s.trainer = Trainer {
            enabled: true,
            every_bars: 2,
            step_bpm: 7.5,
            max_bpm: 115.0,
            at_max: TrainerEnd::Hold,
        };
    });
    let mut rig = Rig::recording(&s, 48_000, 0);
    rig.run(25.0);

    // 100 → 107.5 → 115 (capped), then held: two bars (8 beats) per step.
    let expected = trainer_frames(0, 48_000, &[(100.0, 8), (107.5, 8), (115.0, 16)]);
    let got: Vec<u64> = rig.hits().iter().map(|&(frame, _, _)| frame).collect();
    assert_eq!(&got[..expected.len()], &expected[..]);
    let tempos: Vec<u32> = rig
        .announced
        .iter()
        .filter_map(|a| match a {
            Announcement::Beat(mark) if mark.beat == 0 => Some(mark.bpm_tenths),
            _ => None,
        })
        .take(8)
        .collect();
    assert_eq!(
        tempos,
        vec![1_000, 1_000, 1_075, 1_075, 1_150, 1_150, 1_150, 1_150]
    );
    rig.assert_on_time();
}

#[test]
fn tempo_trainer_can_stop_after_the_bars_at_max() {
    let s = settings(|s| {
        s.bpm = 200.0;
        s.trainer = Trainer {
            enabled: true,
            every_bars: 1,
            step_bpm: 50.0,
            max_bpm: 300.0,
            at_max: TrainerEnd::Stop,
        };
    });
    let mut rig = Rig::recording(&s, 48_000, 0);
    rig.run(10.0);

    let expected = trainer_frames(0, 48_000, &[(200.0, 4), (250.0, 4), (300.0, 4)]);
    let got: Vec<u64> = rig.hits().iter().map(|&(frame, _, _)| frame).collect();
    assert_eq!(got, expected, "nothing plays after the last bar at max");
    let end = 4 * (14_400 + 11_520 + 9_600);
    assert!(rig.scheduler.is_done(rig.now()));
    assert!(!rig.scheduler.is_done(end - 1));
    rig.assert_on_time();
}

// ---- live changes ------------------------------------------------------------------

fn main_frames(rig: &Rig) -> Vec<u64> {
    rig.hits()
        .into_iter()
        .filter(|&(_, n, _)| {
            n == note(Sound::Click, Variant::High) || n == note(Sound::Click, Variant::Low)
        })
        .map(|(frame, _, _)| frame)
        .collect()
}

#[test]
fn a_tempo_change_lands_on_the_next_beat_without_stopping() {
    // Changed 8 000 frames before beat 2 (48 000): it is already scheduled
    // but far enough ahead, so it is cancelled and becomes the first beat at
    // the new tempo.
    let mut rig = Rig::recording(&MetronomeSettings::default(), 48_000, 0);
    rig.run(40_000.0 / 48_000.0);
    assert!(
        rig.announced
            .iter()
            .any(|a| matches!(a, Announcement::Beat(mark) if mark.frame == 48_000))
    );
    rig.update(&settings(|s| s.bpm = 90.0));
    rig.run(3.0);
    assert_eq!(
        &main_frames(&rig)[..5],
        &[0, 24_000, 48_000, 80_000, 112_000]
    );
    assert!(
        rig.announced
            .contains(&Announcement::Reset { from_frame: 48_000 })
    );

    // Changed 1 000 frames before beat 2: too close, so beat 2 keeps the old
    // tempo and beat 3 starts the new one.
    let mut rig = Rig::recording(&MetronomeSettings::default(), 48_000, 0);
    rig.run(47_000.0 / 48_000.0);
    rig.update(&settings(|s| s.bpm = 90.0));
    rig.run(3.0);
    assert_eq!(
        &main_frames(&rig)[..5],
        &[0, 24_000, 48_000, 72_000, 104_000]
    );
    rig.assert_on_time();
}

#[test]
fn a_meter_change_takes_effect_on_the_next_beat() {
    let mut rig = Rig::recording(&MetronomeSettings::default(), 48_000, 0);
    rig.run(1.2); // beats 0, 1, 2 heard; beat 3 (72 000) is next
    rig.update(&settings(|s| {
        s.beats = 3;
        s.accents = vec![Accent::Strong, Accent::Weak, Accent::Weak];
    }));
    rig.run(2.0);
    let high = note(Sound::Click, Variant::High);
    let downbeats: Vec<u64> = rig.hits_of(high);
    // Beat 3 no longer fits a 3/4 bar, so a new bar starts there.
    assert_eq!(&downbeats[..3], &[0, 72_000, 144_000]);
    rig.assert_on_time();
}

#[test]
fn an_output_switch_reschedules_at_the_new_sample_rate() {
    let mut rig = Rig::recording(&MetronomeSettings::default(), 48_000, 0);
    rig.run(2.0);
    let before: Vec<u64> = main_frames(&rig);
    assert_eq!(before, vec![0, 24_000, 48_000, 72_000]);

    // The device changes to 44.1 kHz; the frame clock carries on and the
    // worker's next tick sees the new generation.
    rig.engine.reformat(StreamFormat::new(44_100, 1));
    let switched_at = rig.now();
    rig.pump();
    rig.run(4.0);

    let after: Vec<u64> = main_frames(&rig)[before.len()..].to_vec();
    assert!(after.len() >= 7);
    assert!(
        after[0] >= switched_at + 1_764,
        "first beat is safely ahead"
    );
    // From the restart on, beats are 0.5 s apart at the new rate.
    for pair in after.windows(2) {
        assert_eq!(pair[1] - pair[0], 22_050);
    }
    assert!(
        rig.announced
            .contains(&Announcement::Reset { from_frame: 0 })
    );
    rig.assert_on_time();
}

// ---- real-time safety ------------------------------------------------------------------

#[test]
fn the_click_source_renders_without_allocating() {
    let s = settings(|s| {
        s.bpm = 400.0;
        s.subdivision = 6;
        s.polyrhythm = Polyrhythm {
            enabled: true,
            pulses: 5,
            beats: 4,
            sound: Sound::Cowbell,
        };
    });
    let mut rig = Rig::with_source(
        Box::new(MetronomeSource::new()),
        &s,
        48_000,
        0,
        Arc::default(),
    );
    let mut buffer = vec![0.0_f32; 64];
    let mut heard = false;
    for _ in 0..200 {
        // Scheduling happens on the worker thread; only rendering is counted.
        rig.pump();
        let mut allocations = 0;
        for _ in 0..8 {
            allocations += count_allocations(|| rig.engine.render_callback(&mut buffer));
            heard |= buffer.iter().any(|&s| s != 0.0);
        }
        assert_eq!(allocations, 0);
    }
    assert!(heard);
    rig.assert_on_time();
}

//! Stem engine tests: the DSP against the numpy reference
//! (`testdata/dsp_reference.json`, from `tools/stem-model/make_dsp_fixture.py`),
//! the pipeline, library job and service with fake models, and a real
//! separation with the Core ML model when it is installed.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::dsp::{
    FRAMES, FREQS, MIX_LEN, Normalization, SEGMENT, SOURCES, SPEC_LEN, Stft, chunk_count,
};
use super::job::{OLD_STEMS_DIR, WORK_DIR};
use super::*;
use crate::stems::{
    AUDIO_FILE, Library, Manifest, ProcessControl, STEMS_DIR, SeparationEvent, StemPlayer,
    StemStatus, TrackName, TrackReader, WavTrack, import_file,
};

const PLANE: usize = FREQS * FRAMES;
const SR: f64 = 44_100.0;

/// Separation tests hold ~150 MB of buffers and up to four DSP threads each:
/// one at a time.
static HEAVY: Mutex<()> = Mutex::new(());

fn heavy() -> MutexGuard<'static, ()> {
    HEAVY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Writes straight to stderr, past the test harness's output capture, so
/// `cargo test` always shows it.
fn report(message: &str) {
    let _ = writeln!(std::io::stderr(), "{message}");
}

// ---- inputs shared with make_dsp_fixture.py (keep them identical) ----------

/// Integer-hash noise in [-0.05, 0.05).
fn noise(i: usize, channel: u32) -> f64 {
    let mut u = (i as u32)
        .wrapping_mul(2_654_435_761)
        .wrapping_add(channel.wrapping_mul(40_503));
    u ^= u >> 16;
    u = u.wrapping_mul(0x045D_9F3B);
    u ^= u >> 16;
    (f64::from(u) / 4_294_967_296.0 - 0.5) * 0.1
}

fn test_signal(n: usize) -> (Vec<f32>, Vec<f32>) {
    let tau = 2.0 * std::f64::consts::PI;
    (0..n)
        .map(|i| {
            let x = i as f64;
            let t = x / SR;
            let left = 0.5 * (tau * 220.0 * x / SR).sin()
                + 0.25 * (tau * 3150.7 * x / SR).sin()
                + 0.1 * (tau * 0.37 * x / SR).sin()
                + 0.2 * (tau * (50.0 * t + 750.0 * (t * t))).sin()
                + noise(i, 0);
            let right = 0.4 * (tau * 330.0 * x / SR + 1.0).sin()
                + 0.2 * (tau * 5021.3 * x / SR).sin()
                - 0.05
                + noise(i, 1);
            (left as f32, right as f32)
        })
        .unzip()
}

fn test_spectrum(channels: usize) -> Vec<f32> {
    let mut spec = vec![0.0_f32; channels * PLANE];
    for ch in 0..channels {
        for f in 0..FREQS {
            for k in 0..FRAMES {
                let (chf, ff, kf) = (ch as f64, f as f64, k as f64);
                spec[ch * PLANE + f * FRAMES + k] =
                    ((0.37 * ff + 1.13 * kf + 2.1 * chf).sin() * (-ff / 800.0).exp()) as f32;
            }
        }
    }
    spec
}

const NUMPY_G: [f32; SOURCES] = [0.1, 0.2, 0.3, 0.15, 0.15, 0.1];
const NUMPY_H: [f32; SOURCES] = [0.05, -0.02, 0.01, 0.0, 0.03, -0.07];

/// `fake_core` of make_dsp_fixture.py: a position-dependent stand-in for the
/// model that exercises both branches.
struct NumpyFake;

impl StemModel for NumpyFake {
    fn predict(
        &mut self,
        mix: &[f32],
        spec: &[f32],
        spec_out: &mut [f32],
        wave_out: &mut [f32],
    ) -> Result<(), SeparationError> {
        for (s, &g) in NUMPY_G.iter().enumerate() {
            for j in 0..4 {
                for f in 0..FREQS {
                    let tilt = 1.0_f32 - f as f32 / 4096.0;
                    let from = j * PLANE + f * FRAMES;
                    let to = (s * 4 + j) * PLANE + f * FRAMES;
                    for k in 0..FRAMES {
                        spec_out[to + k] = spec[from + k] * g * tilt;
                    }
                }
            }
        }
        for (s, &h) in NUMPY_H.iter().enumerate() {
            for c in 0..2 {
                for i in 0..SEGMENT {
                    let ramp = i as f32 / SEGMENT as f32;
                    wave_out[(s * 2 + c) * SEGMENT + i] = mix[c * SEGMENT + i] * h * ramp;
                }
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct StftReference {
    points: Vec<(usize, usize, usize, f64)>,
    energy: Vec<f64>,
}

#[derive(Deserialize)]
struct PlanesReference {
    points: Vec<(usize, usize, usize, f64)>,
    energy: Vec<Vec<f64>>,
}

#[derive(Deserialize)]
struct SeparateReference {
    frames: usize,
    mean: f64,
    std: f64,
    points: Vec<(usize, usize, usize, f64)>,
    energy: Vec<Vec<f64>>,
}

#[derive(Deserialize)]
struct Reference {
    stft: StftReference,
    istft: PlanesReference,
    separate: SeparateReference,
}

fn reference() -> Reference {
    serde_json::from_str(include_str!("testdata/dsp_reference.json")).expect("valid fixture")
}

fn energy(samples: &[f32]) -> f64 {
    samples.iter().map(|&s| f64::from(s).powi(2)).sum()
}

fn assert_close(actual: f32, expected: f64, abs: f64, rel: f64, what: &str) {
    let error = (f64::from(actual) - expected).abs();
    assert!(
        error <= abs + rel * expected.abs(),
        "{what}: {actual} vs numpy {expected} (error {error:e})"
    );
}

fn assert_energy(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= 1e-4 * expected.abs(),
        "{what} energy: {actual} vs numpy {expected}"
    );
}

#[test]
fn stft_matches_numpy_reference() {
    let reference = reference().stft;
    let (mut chunk, right) = test_signal(SEGMENT);
    chunk.extend_from_slice(&right);
    let mut spec = vec![0.0; SPEC_LEN];
    Stft::new().forward(&chunk, &mut spec);
    for &(c, f, k, expected) in &reference.points {
        let what = format!("stft[{c}][{f}][{k}]");
        assert_close(
            spec[c * PLANE + f * FRAMES + k],
            expected,
            1e-5,
            1e-4,
            &what,
        );
    }
    for (c, &expected) in reference.energy.iter().enumerate() {
        let plane = &spec[c * PLANE..(c + 1) * PLANE];
        assert_energy(energy(plane), expected, &format!("stft channel {c}"));
    }
}

#[test]
fn istft_matches_numpy_reference() {
    let reference = reference().istft;
    let spec = test_spectrum(8);
    let mut out = vec![0.0; 4 * SEGMENT];
    Stft::new().inverse(&spec, &mut out);
    for &(s, c, i, expected) in &reference.points {
        let what = format!("istft[{s}][{c}][{i}]");
        assert_close(out[(s * 2 + c) * SEGMENT + i], expected, 1e-6, 1e-4, &what);
    }
    for (s, channels) in reference.energy.iter().enumerate() {
        for (c, &expected) in channels.iter().enumerate() {
            let plane = &out[(s * 2 + c) * SEGMENT..(s * 2 + c + 1) * SEGMENT];
            assert_energy(energy(plane), expected, &format!("istft {s}/{c}"));
        }
    }
}

#[test]
fn istft_inverts_stft_away_from_the_chunk_edges() {
    // Band-limited (the Nyquist bin is dropped, as in Demucs).
    let tau = 2.0 * std::f64::consts::PI;
    let chunk: Vec<f32> = (0..MIX_LEN)
        .map(|n| {
            let (c, i) = ((n / SEGMENT) as f64, (n % SEGMENT) as f64);
            let t = i / SR;
            (0.5 * (tau * 220.0 * t + c).sin()
                + 0.25 * (tau * 3150.7 * t).sin()
                + 0.2 * (tau * (50.0 * t + 750.0 * (t * t))).sin()) as f32
        })
        .collect();
    let stft = Stft::new();
    let mut spec = vec![0.0; SPEC_LEN];
    stft.forward(&chunk, &mut spec);
    let mut back = vec![0.0; MIX_LEN];
    stft.inverse(&spec, &mut back);
    // The first and last frames are dropped as in Demucs, so the outer
    // ~1.5k samples of a chunk do not reconstruct; the overlap-add weights
    // make them negligible.
    for c in 0..2 {
        for i in 2048..SEGMENT - 2048 {
            let (x, y) = (chunk[c * SEGMENT + i], back[c * SEGMENT + i]);
            assert!((x - y).abs() < 2e-5, "channel {c} sample {i}: {x} vs {y}");
        }
    }
}

/// Collects every output plane in memory.
struct Collect {
    planes: Vec<Vec<f32>>,
    chunks: Vec<(usize, usize)>,
}

impl Collect {
    fn new() -> Self {
        Self {
            planes: vec![Vec::new(); PLANES],
            chunks: Vec::new(),
        }
    }
}

impl SeparationOutput for Collect {
    fn write(&mut self, planes: &[f32], len: usize) -> Result<(), SeparationError> {
        for (plane, out) in self.planes.iter_mut().enumerate() {
            out.extend_from_slice(&planes[plane * len..(plane + 1) * len]);
        }
        Ok(())
    }

    fn chunk_done(&mut self, done: usize, total: usize) {
        self.chunks.push((done, total));
    }
}

#[test]
fn separation_matches_numpy_reference_across_chunks() {
    let _heavy = heavy();
    let reference = reference().separate;
    let n = reference.frames;
    assert_eq!(chunk_count(n), 3, "two full chunks and a short centred one");
    let (left, right) = test_signal(n);
    let norm = Normalization::of(&left, &right);
    assert!((norm.mean - reference.mean).abs() < 1e-9, "{norm:?}");
    assert!(
        (norm.std - reference.std).abs() < 1e-9 * reference.std,
        "{norm:?}"
    );

    let mut output = Collect::new();
    let mut work = Workspace::new();
    work.separate(
        &left,
        &right,
        &Stft::new(),
        &mut NumpyFake,
        &AtomicBool::new(false),
        &mut output,
    )
    .unwrap();
    assert_eq!(output.chunks, vec![(1, 3), (2, 3), (3, 3)]);
    assert!(output.planes.iter().all(|plane| plane.len() == n));
    for &(s, c, i, expected) in &reference.points {
        let what = format!("stem {s} channel {c} sample {i}");
        assert_close(output.planes[s * 2 + c][i], expected, 2e-5, 1e-4, &what);
    }
    for (s, channels) in reference.energy.iter().enumerate() {
        for (c, &expected) in channels.iter().enumerate() {
            let what = format!("stem {s}/{c}");
            assert_energy(energy(&output.planes[s * 2 + c]), expected, &what);
        }
    }
}

// ---- fakes for the library job and the service ------------------------------

/// Lets a test hold a fake model inside `predict` and release it.
#[derive(Default)]
struct Gate {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}

impl Gate {
    /// Called by the model: marks the arrival and waits until opened.
    fn pass(&self) {
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        self.changed.notify_all();
        while !state.1 {
            state = self.changed.wait(state).unwrap();
        }
    }

    fn wait_arrived(&self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut state = self.state.lock().unwrap();
        while !state.0 {
            let now = Instant::now();
            assert!(now < deadline, "the model was never called");
            state = self.changed.wait_timeout(state, deadline - now).unwrap().0;
        }
    }

    fn open(&self) {
        self.state.lock().unwrap().1 = true;
        self.changed.notify_all();
    }
}

/// What the fake model does on its next calls.
#[derive(Clone, Default)]
struct Behavior {
    /// Time-branch gain per source in model order; the spectral branch is
    /// silent, so every stem is exactly `gain × input` and the stems sum to
    /// the input when the gains do.
    gains: [f32; SOURCES],
    fail: bool,
    hold: Option<Arc<Gate>>,
    delay: Duration,
}

/// Model order: drums, bass, other, vocals, guitar, piano.
const SPLIT: [f32; SOURCES] = [0.3, 0.25, 0.05, 0.2, 0.12, 0.08];
const OTHER_SPLIT: [f32; SOURCES] = [0.1, 0.1, 0.4, 0.2, 0.1, 0.1];

#[derive(Clone)]
struct FakeModel {
    behavior: Arc<Mutex<Behavior>>,
    calls: Arc<AtomicUsize>,
}

impl FakeModel {
    fn new(gains: [f32; SOURCES]) -> Self {
        Self {
            behavior: Arc::new(Mutex::new(Behavior {
                gains,
                ..Behavior::default()
            })),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn set(&self, change: impl FnOnce(&mut Behavior)) {
        change(&mut self.behavior.lock().unwrap());
    }

    fn loader(&self) -> ModelLoader {
        let model = self.clone();
        Box::new(move || Ok(Box::new(model.clone()) as Box<dyn StemModel>))
    }
}

impl StemModel for FakeModel {
    fn predict(
        &mut self,
        mix: &[f32],
        _spec: &[f32],
        spec_out: &mut [f32],
        wave_out: &mut [f32],
    ) -> Result<(), SeparationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let behavior = self.behavior.lock().unwrap().clone();
        if let Some(gate) = &behavior.hold {
            gate.pass();
        }
        std::thread::sleep(behavior.delay);
        if behavior.fail {
            return Err(SeparationError::Model("fake failure".into()));
        }
        spec_out.fill(0.0);
        for (s, &gain) in behavior.gains.iter().enumerate() {
            for c in 0..2 {
                let to = &mut wave_out[(s * 2 + c) * SEGMENT..(s * 2 + c + 1) * SEGMENT];
                for (o, &x) in to.iter_mut().zip(&mix[c * SEGMENT..(c + 1) * SEGMENT]) {
                    *o = x * gain;
                }
            }
        }
        Ok(())
    }
}

struct TempLibrary {
    dir: PathBuf,
    library: Library,
}

impl TempLibrary {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("woodshed-separation-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self {
            library: Library::new(&dir),
            dir,
        }
    }

    fn song_dir(&self, id: &str) -> PathBuf {
        self.library.song_dir(id).unwrap()
    }

    /// Imports a generated song through the real importer (16-bit `audio.wav`).
    fn import(
        &self,
        name: &str,
        rate: u32,
        channels: u16,
        frames: usize,
        signal: impl Fn(usize, usize) -> f32,
    ) -> Manifest {
        let path = self.dir.join(format!("{name}.wav"));
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..frames {
            for c in 0..usize::from(channels) {
                writer.write_sample(signal(i, c).clamp(-1.0, 1.0)).unwrap();
            }
        }
        writer.finalize().unwrap();
        import_file(
            &self.library,
            &path,
            &ProcessControl::default(),
            &mut |_| {},
        )
        .unwrap()
    }
}

impl Drop for TempLibrary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Zero-mean tones (the denormalisation adds the song mean to every stem).
fn tones(rate: u32) -> impl Fn(usize, usize) -> f32 {
    let rate = f64::from(rate);
    move |i, c| {
        let t = i as f64 / rate;
        let tau = 2.0 * std::f64::consts::PI;
        (0.4 * (tau * 440.0 * t).sin() + 0.2 * (tau * 1234.5 * t + c as f64).sin()) as f32
    }
}

/// Interleaved samples of a WAV file and its spec.
fn read_wav(path: &Path) -> (hound::WavSpec, Vec<f32>) {
    let mut reader = hound::WavReader::open(path).unwrap();
    let spec = reader.spec();
    let samples = reader
        .samples::<i16>()
        .map(|s| f32::from(s.unwrap()) / 32_768.0)
        .collect();
    (spec, samples)
}

fn snr_db(reference: &[f32], estimate: &[f32]) -> f64 {
    let signal: f64 = reference.iter().map(|&x| f64::from(x).powi(2)).sum();
    let error: f64 = reference
        .iter()
        .zip(estimate)
        .map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2))
        .sum();
    10.0 * (signal / error.max(1e-30)).log10()
}

fn stem_bytes(dir: &Path) -> Vec<Vec<u8>> {
    MODEL_STEMS
        .iter()
        .map(|name| fs::read(dir.join(name.relative_path())).unwrap())
        .collect()
}

fn manifest_text(dir: &Path) -> String {
    fs::read_to_string(dir.join("manifest.json")).unwrap()
}

/// Checks the stems of `song` against the contract and returns their sum and
/// the playback copy.
fn check_stems(lib: &TempLibrary, song: &Manifest, gains: &[f32; SOURCES]) -> f64 {
    let dir = lib.song_dir(&song.id);
    let (audio_spec, audio) = read_wav(&dir.join(AUDIO_FILE));
    let mut sum = vec![0.0_f32; audio.len()];
    for (source, name) in MODEL_STEMS.iter().enumerate() {
        let (spec, samples) = read_wav(&dir.join(name.relative_path()));
        assert_eq!(spec.sample_rate, song.sample_rate, "{name:?} rate");
        assert_eq!(spec.channels, song.channels, "{name:?} channels");
        assert_eq!(
            (spec.bits_per_sample, spec.sample_format),
            (16, hound::SampleFormat::Int),
            "{name:?} format"
        );
        assert_eq!(spec, audio_spec);
        assert_eq!(samples.len() as u64, song.frames * u64::from(song.channels));
        // Each stem carries its source's share: catches order mix-ups.
        let ratio = (energy(&samples) / energy(&audio)).sqrt();
        let gain = f64::from(gains[source]);
        assert!(
            (ratio - gain).abs() < 0.02 * gain + 1e-3,
            "{name:?}: level {ratio} vs gain {gain}"
        );
        for (s, &x) in sum.iter_mut().zip(&samples) {
            *s += x;
        }
    }
    snr_db(&audio, &sum)
}

#[test]
fn job_writes_six_stems_in_the_song_format() {
    let _heavy = heavy();
    let lib = TempLibrary::new("job");
    let stft = Stft::new();
    let mut work = Workspace::new();
    // A 48 kHz mono song (resampled both ways) and a 44.1 kHz stereo one.
    for (name, rate, channels, frames) in [
        ("mono48k", 48_000, 1, 480_000),
        ("stereo44k", 44_100, 2, 300_000),
    ] {
        let song = lib.import(name, rate, channels, frames, tones(rate));
        let mut progress = Vec::new();
        job::run(
            &lib.library,
            &song.id,
            &stft,
            &mut FakeModel::new(SPLIT),
            &mut work,
            &AtomicBool::new(false),
            &mut |p| progress.push(p),
        )
        .unwrap();
        let saved = lib.library.get(&song.id).unwrap();
        assert!(saved.stems_ready(), "{name}: {:?}", saved.stems);
        let tracks: Vec<TrackName> = saved.tracks().into_iter().map(|(t, _)| t).collect();
        assert_eq!(tracks.len(), 6, "{name}: the mixer gets six stems");
        assert!(!tracks.contains(&TrackName::Original));
        let model_frames = (frames as f64 * SR / f64::from(rate)).ceil() as usize;
        let chunks = chunk_count(model_frames);
        assert_eq!(progress.len(), chunks + 1, "{name}: {progress:?}");
        assert_eq!(progress.first(), Some(&0.0));
        assert_eq!(progress.last(), Some(&1.0));
        assert!(progress.windows(2).all(|w| w[0] < w[1]), "{progress:?}");
        let snr = check_stems(&lib, &saved, &SPLIT);
        assert!(snr > 40.0, "{name}: stems sum to the song at {snr:.1} dB");
        report(&format!(
            "separation job {name}: six stems sum to the song at {snr:.1} dB"
        ));
        let dir = lib.song_dir(&song.id);
        assert!(!dir.join(WORK_DIR).exists() && !dir.join(OLD_STEMS_DIR).exists());
    }
}

#[test]
fn stems_are_cut_or_padded_to_the_song_length() {
    let _heavy = heavy();
    let lib = TempLibrary::new("length");
    // Shorter than one chunk, at a rate that does not divide evenly.
    let song = lib.import("short", 22_050, 2, 12_345, tones(22_050));
    job::run(
        &lib.library,
        &song.id,
        &Stft::new(),
        &mut FakeModel::new(SPLIT),
        &mut Workspace::new(),
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    let snr = check_stems(&lib, &lib.library.get(&song.id).unwrap(), &SPLIT);
    assert!(snr > 30.0, "{snr:.1} dB");
}

// ---- the service: queue, cancel, failure, re-separation ---------------------

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<SeparationEvent>>>);

impl Events {
    fn emit(&self) -> Emit {
        let events = self.0.clone();
        Arc::new(move |event| events.lock().unwrap().push(event))
    }

    fn of(&self, id: &str) -> Vec<(String, Option<f32>)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.song_id == id)
            .map(|e| (e.state.clone(), e.progress))
            .collect()
    }

    /// Waits for the first `done`, `failed` or `cancelled` of song `id` after
    /// its first `seen` events, and returns its state.
    fn wait_end_after(&self, id: &str, seen: usize) -> String {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let end = self.of(id).into_iter().skip(seen).find(|(state, _)| {
                [STATE_DONE, STATE_FAILED, STATE_CANCELLED].contains(&state.as_str())
            });
            if let Some((state, _)) = end {
                return state;
            }
            assert!(
                Instant::now() < deadline,
                "no end for {id}: {:?}",
                self.of(id)
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Waits for a `state` event of song `id`.
    fn wait(&self, id: &str, state: &str) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.of(id).iter().any(|(s, _)| s == state) {
            assert!(
                Instant::now() < deadline,
                "no {state} for {id}: {:?}",
                self.of(id)
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

const TEST_CONFIG: ServiceConfig = ServiceConfig {
    warm_up_after: None,
    unload_after: Duration::from_secs(60),
};

fn start_service(lib: &TempLibrary, model: &FakeModel, events: &Events) -> SeparationService {
    SeparationService::start(
        lib.library.clone(),
        model.loader(),
        events.emit(),
        TEST_CONFIG,
    )
}

fn statuses(lib: &TempLibrary, id: &str) -> Vec<StemStatus> {
    lib.library
        .get(id)
        .unwrap()
        .stems
        .iter()
        .map(|s| s.status)
        .collect()
}

#[test]
fn service_runs_one_song_at_a_time_and_cancels_queued_and_running_songs() {
    let _heavy = heavy();
    let lib = TempLibrary::new("queue");
    let a = lib.import("a", 44_100, 1, 100_000, tones(44_100));
    let b = lib.import("b", 44_100, 1, 100_000, tones(44_100));
    let c = lib.import("c", 44_100, 1, 100_000, tones(44_100));
    let model = FakeModel::new(SPLIT);
    let gate = Arc::new(Gate::default());
    model.set(|b| b.hold = Some(gate.clone()));
    let events = Events::default();
    let service = start_service(&lib, &model, &events);

    service.separate(&a.id).unwrap();
    gate.wait_arrived();
    service.separate(&b.id).unwrap();
    service.separate(&b.id).unwrap(); // already queued: no second entry
    assert_eq!(statuses(&lib, &a.id), vec![StemStatus::Running; 6]);
    assert_eq!(statuses(&lib, &b.id), vec![StemStatus::Pending; 6]);
    let status: Vec<_> = service.status().into_iter().map(|e| e.song_id).collect();
    assert_eq!(status, vec![a.id.clone(), b.id.clone()]);

    // Cancelling the queued song takes it out at once.
    service.cancel(&b.id).unwrap();
    assert!(lib.library.get(&b.id).unwrap().stems.is_empty());
    assert_eq!(events.of(&b.id).last().unwrap().0, STATE_CANCELLED);

    gate.open();
    events.wait(&a.id, STATE_DONE);
    assert!(lib.library.get(&a.id).unwrap().stems_ready());
    let a_events = events.of(&a.id);
    assert_eq!(a_events.first().unwrap(), &(STATE_RUNNING.to_owned(), None));
    assert_eq!(
        a_events.last().unwrap(),
        &(STATE_DONE.to_owned(), Some(1.0))
    );
    assert!(a_events.contains(&(STATE_RUNNING.to_owned(), Some(0.0))));
    assert_eq!(model.calls.load(Ordering::SeqCst), 1, "b never ran");

    // Cancelling a running song stops it and leaves it as before.
    let gate = Arc::new(Gate::default());
    model.set(|b| b.hold = Some(gate.clone()));
    service.separate(&c.id).unwrap();
    gate.wait_arrived();
    assert!(lib.song_dir(&c.id).join(WORK_DIR).exists());
    service.cancel(&c.id).unwrap();
    gate.open();
    events.wait(&c.id, STATE_CANCELLED);
    assert!(lib.library.get(&c.id).unwrap().stems.is_empty());
    let dir = lib.song_dir(&c.id);
    assert!(!dir.join(WORK_DIR).exists() && !dir.join(STEMS_DIR).exists());
    assert!(service.status().is_empty());
    service.shutdown();
}

#[test]
fn service_marks_a_failed_separation_and_can_retry() {
    let _heavy = heavy();
    let lib = TempLibrary::new("failure");
    let song = lib.import("song", 44_100, 2, 100_000, tones(44_100));
    let model = FakeModel::new(SPLIT);
    model.set(|b| b.fail = true);
    let events = Events::default();
    let service = start_service(&lib, &model, &events);
    service.separate(&song.id).unwrap();
    events.wait(&song.id, STATE_FAILED);
    assert_eq!(statuses(&lib, &song.id), vec![StemStatus::Failed; 6]);
    assert!(!lib.song_dir(&song.id).join(WORK_DIR).exists());

    model.set(|b| b.fail = false);
    service.separate(&song.id).unwrap();
    events.wait(&song.id, STATE_DONE);
    assert!(lib.library.get(&song.id).unwrap().stems_ready());
    service.shutdown();
}

#[test]
fn service_reports_a_missing_model_as_a_failure() {
    let _heavy = heavy();
    let lib = TempLibrary::new("missing");
    let song = lib.import("song", 44_100, 1, 50_000, tones(44_100));
    let events = Events::default();
    let service = SeparationService::start(
        lib.library.clone(),
        Box::new(|| Err(SeparationError::ModelMissing)),
        events.emit(),
        TEST_CONFIG,
    );
    service.separate(&song.id).unwrap();
    events.wait(&song.id, STATE_FAILED);
    assert_eq!(statuses(&lib, &song.id), vec![StemStatus::Failed; 6]);
    assert!(service.separate("unknown1").is_err());
    service.shutdown();
}

/// The conductor's rule: re-separating a song whose stems are ready never
/// touches its stems or manifest until six new stems are complete, whether
/// it is cancelled, fails or the app stops in between.
#[test]
fn reseparating_a_ready_song_keeps_its_stems_until_new_ones_are_complete() {
    let _heavy = heavy();
    let lib = TempLibrary::new("reseparate");
    let song = lib.import("song", 48_000, 2, 150_000, tones(48_000));
    let model = FakeModel::new(SPLIT);
    let events = Events::default();
    let service = start_service(&lib, &model, &events);
    service.separate(&song.id).unwrap();
    events.wait(&song.id, STATE_DONE);
    let dir = lib.song_dir(&song.id);
    let manifest = manifest_text(&dir);
    let stems = stem_bytes(&dir);
    let unchanged = |when: &str| {
        assert_eq!(manifest_text(&dir), manifest, "manifest {when}");
        assert!(stem_bytes(&dir) == stems, "stem files {when}");
        assert!(lib.library.get(&song.id).unwrap().stems_ready());
    };

    // Cancelled while running: nothing changes, not even while it runs.
    let gate = Arc::new(Gate::default());
    model.set(|b| {
        b.gains = OTHER_SPLIT;
        b.hold = Some(gate.clone());
    });
    service.separate(&song.id).unwrap();
    gate.wait_arrived();
    unchanged("while re-separating");
    assert_eq!(lib.library.get(&song.id).unwrap().tracks().len(), 6);
    service.cancel(&song.id).unwrap();
    gate.open();
    events.wait(&song.id, STATE_CANCELLED);
    unchanged("after cancelling");
    assert!(!dir.join(WORK_DIR).exists());

    // Failed: the same.
    model.set(|b| {
        b.hold = None;
        b.fail = true;
    });
    service.separate(&song.id).unwrap();
    events.wait(&song.id, STATE_FAILED);
    unchanged("after a failure");
    assert!(!dir.join(WORK_DIR).exists());
    service.shutdown();

    // The app stopped mid-way: partial stems are dropped at the next start.
    fs::create_dir(dir.join(WORK_DIR)).unwrap();
    fs::write(dir.join(WORK_DIR).join("drums.wav"), b"partial").unwrap();
    job::recover(&lib.library);
    unchanged("after a restart mid-separation");
    assert!(!dir.join(WORK_DIR).exists());

    // ... or between moving the old stems aside and the new ones in.
    fs::rename(dir.join(STEMS_DIR), dir.join(OLD_STEMS_DIR)).unwrap();
    fs::create_dir(dir.join(WORK_DIR)).unwrap();
    job::recover(&lib.library);
    unchanged("after a restart mid-swap");
    assert!(!dir.join(OLD_STEMS_DIR).exists() && !dir.join(WORK_DIR).exists());

    // A complete re-separation replaces the stems.
    let model = FakeModel::new(OTHER_SPLIT);
    let events = Events::default();
    let service = start_service(&lib, &model, &events);
    service.separate(&song.id).unwrap();
    events.wait(&song.id, STATE_DONE);
    let saved = lib.library.get(&song.id).unwrap();
    assert!(saved.stems_ready());
    assert!(stem_bytes(&dir) != stems, "new stems in place");
    check_stems(&lib, &saved, &OTHER_SPLIT);
    assert!(!dir.join(OLD_STEMS_DIR).exists() && !dir.join(WORK_DIR).exists());
    service.shutdown();
}

/// Opens the song's tracks the way `stems_open` does.
fn open_player(lib: &TempLibrary, id: &str) -> usize {
    let manifest = lib.library.get(id).unwrap();
    let dir = lib.song_dir(id);
    let readers: Vec<Box<dyn TrackReader>> = manifest
        .tracks()
        .iter()
        .map(|(_, path)| Box::new(WavTrack::open(&dir.join(path)).unwrap()) as Box<dyn TrackReader>)
        .collect();
    let tracks = readers.len();
    StemPlayer::new(readers, 0).unwrap();
    tracks
}

/// Review R1: a step of installing new stems fails after the WAVs are
/// written. A song already separated keeps its six files and manifest byte
/// for byte, right after the failure and after the next start's recovery,
/// and the player still opens them. A song not separated yet rolls back
/// when its manifest update (its commit) fails.
#[test]
fn a_failed_install_keeps_the_previous_stems_now_and_after_a_restart() {
    let _heavy = heavy();
    let lib = TempLibrary::new("install-faults");
    let ready = lib.import("ready", 48_000, 2, 150_000, tones(48_000));
    let fresh = lib.import("fresh", 44_100, 1, 100_000, tones(44_100));
    let model = FakeModel::new(SPLIT);
    let events = Events::default();
    let service = start_service(&lib, &model, &events);
    service.separate(&ready.id).unwrap();
    events.wait(&ready.id, STATE_DONE);
    let dir = lib.song_dir(&ready.id);
    let manifest = manifest_text(&dir);
    let stems = stem_bytes(&dir);
    let unchanged = |when: &str| {
        assert_eq!(manifest_text(&dir), manifest, "manifest {when}");
        assert!(stem_bytes(&dir) == stems, "stem files {when}");
        assert!(!dir.join(WORK_DIR).exists(), ".separating {when}");
        assert!(!dir.join(OLD_STEMS_DIR).exists(), ".stems-old {when}");
        assert_eq!(open_player(&lib, &ready.id), 6, "player {when}");
    };

    model.set(|b| b.gains = OTHER_SPLIT);
    for step in ["finish", "move-old", "move-new"] {
        job::faults::inject(&ready.id, step);
        let seen = events.of(&ready.id).len();
        service.separate(&ready.id).unwrap();
        assert_eq!(
            events.wait_end_after(&ready.id, seen),
            STATE_FAILED,
            "{step}"
        );
        unchanged(&format!("after a fault at {step}"));
        job::recover(&lib.library);
        unchanged(&format!("after a restart following a fault at {step}"));
        job::faults::clear(&ready.id);
    }

    // The manifest of a separated song is not written when its stems are
    // swapped, so the rename is its commit and nothing can fail after it.
    job::faults::inject(&ready.id, "commit");
    let seen = events.of(&ready.id).len();
    service.separate(&ready.id).unwrap();
    assert_eq!(events.wait_end_after(&ready.id, seen), STATE_DONE);
    job::faults::clear(&ready.id);
    assert_eq!(manifest_text(&dir), manifest, "still ready, not rewritten");
    assert!(stem_bytes(&dir) != stems, "the new stems are in place");
    check_stems(&lib, &lib.library.get(&ready.id).unwrap(), &OTHER_SPLIT);

    // A song not separated yet, with an old stems/ left on disk: its
    // manifest update fails, so the swap is undone.
    let fresh_dir = lib.song_dir(&fresh.id);
    fs::create_dir(fresh_dir.join(STEMS_DIR)).unwrap();
    fs::write(fresh_dir.join(STEMS_DIR).join("left.txt"), b"older").unwrap();
    job::faults::inject(&fresh.id, "commit");
    service.separate(&fresh.id).unwrap();
    assert_eq!(events.wait_end_after(&fresh.id, 0), STATE_FAILED);
    assert_eq!(statuses(&lib, &fresh.id), vec![StemStatus::Failed; 6]);
    assert_eq!(
        fs::read(fresh_dir.join(STEMS_DIR).join("left.txt")).unwrap(),
        b"older"
    );
    assert!(!fresh_dir.join(STEMS_DIR).join("drums.wav").exists());
    assert!(!fresh_dir.join(WORK_DIR).exists() && !fresh_dir.join(OLD_STEMS_DIR).exists());
    job::recover(&lib.library);
    assert_eq!(statuses(&lib, &fresh.id), vec![StemStatus::Failed; 6]);
    assert_eq!(open_player(&lib, &fresh.id), 1, "the original track");
    job::faults::clear(&fresh.id);
    service.shutdown();
}

#[test]
fn startup_recovery_resolves_songs_left_mid_separation() {
    let _heavy = heavy();
    let lib = TempLibrary::new("recover");
    let running = lib.import("running", 44_100, 1, 50_000, tones(44_100));
    let swapped = lib.import("swapped", 44_100, 1, 50_000, tones(44_100));
    let idle = lib.import("idle", 44_100, 1, 50_000, tones(44_100));

    // Stopped while running: entries say running, partial stems on disk.
    job::mark(&lib.library, &running.id, StemStatus::Running).unwrap();
    let work = lib.song_dir(&running.id).join(WORK_DIR);
    fs::create_dir(&work).unwrap();
    fs::write(work.join("bass.wav"), b"partial").unwrap();

    // Stopped after the new stems moved in but before the manifest said so.
    job::run(
        &lib.library,
        &swapped.id,
        &Stft::new(),
        &mut FakeModel::new(SPLIT),
        &mut Workspace::new(),
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    lib.library
        .update(&swapped.id, |m| {
            for stem in &mut m.stems {
                stem.status = StemStatus::Running;
            }
        })
        .unwrap();
    let idle_manifest = manifest_text(&lib.song_dir(&idle.id));

    job::recover(&lib.library);
    assert!(lib.library.get(&running.id).unwrap().stems.is_empty());
    assert!(!work.exists());
    assert!(lib.library.get(&swapped.id).unwrap().stems_ready());
    assert_eq!(manifest_text(&lib.song_dir(&idle.id)), idle_manifest);
}

#[test]
fn separation_and_other_manifest_updates_keep_each_others_changes() {
    let _heavy = heavy();
    let lib = TempLibrary::new("race");
    let song = lib.import("song", 44_100, 1, 300_000, tones(44_100));
    let model = FakeModel::new(SPLIT);
    model.set(|b| b.delay = Duration::from_millis(30));
    let events = Events::default();
    let service = start_service(&lib, &model, &events);
    let stop = Arc::new(AtomicBool::new(false));
    let writer = {
        let library = lib.library.clone();
        let id = song.id.clone();
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut n = 0;
            while !stop.load(Ordering::Relaxed) {
                n += 1;
                library
                    .update(&id, |m| m.title = format!("title {n}"))
                    .unwrap();
            }
            n
        })
    };
    service.separate(&song.id).unwrap();
    events.wait(&song.id, STATE_DONE);
    stop.store(true, Ordering::Relaxed);
    let updates = writer.join().unwrap();
    let saved = lib.library.get(&song.id).unwrap();
    assert!(saved.stems_ready(), "{:?}", saved.stems);
    assert_eq!(saved.title, format!("title {updates}"));
    assert!(updates > 10, "the writer ran alongside: {updates}");
    service.shutdown();
}

// ---- the real model -----------------------------------------------------------

fn installed_model() -> Option<PathBuf> {
    let candidates = model_candidates(
        Some(Path::new(env!("CARGO_MANIFEST_DIR"))),
        default_cache_dir().as_deref(),
    );
    locate_model(&candidates)
}

/// A short synthetic band (drums, bass, chords, a voice-like lead): no
/// copyright, and enough like music for the model.
fn band(i: usize, c: usize) -> f32 {
    let tau = 2.0 * std::f64::consts::PI;
    let t = i as f64 / SR;
    let beat = 0.6;
    let k = (t / beat) as usize;
    let since = t - k as f64 * beat;
    let kick = (tau * (50.0 + 100.0 * (-30.0 * since).exp()) * since).sin() * (-12.0 * since).exp();
    let snare = if k % 2 == 1 {
        8.0 * noise(i, 7) * (-since / 0.08).exp()
    } else {
        0.0
    };
    let hat_since = t % (beat / 2.0);
    let hat = 2.0 * noise(i, 9) * (-hat_since / 0.02).exp();
    let root = [41.2, 55.0, 49.0, 36.7][(k / 8) % 4];
    let bass = (-since / 0.5).exp()
        * ((tau * root * t).sin()
            + 0.4 * (2.0 * tau * root * t).sin()
            + 0.2 * (3.0 * tau * root * t).sin());
    let chord_since = t % (4.0 * beat);
    let mut piano = 0.0;
    for ratio in [4.0, 5.0, 6.0] {
        for h in 1..=5 {
            let h = f64::from(h);
            piano += (tau * root * ratio * h * t).sin() / h.powf(1.5);
        }
    }
    piano *= (-chord_since / 1.2).exp();
    let step = ((t / (2.0 * beat)) as usize % 5) as f64;
    let pitch = 220.0 * 2.0_f64.powf(step * 2.0 / 12.0);
    let phase = tau * pitch * t + 0.02 * (tau * 5.0 * t).sin() * pitch / 5.0;
    let mut voice = 0.0;
    for h in 1..=8 {
        let h = f64::from(h);
        voice += (h * phase).sin() / h;
    }
    voice *= if (t / (4.0 * beat)).fract() < 0.7 {
        1.0
    } else {
        0.0
    };
    let pan = if c == 0 { 0.4 } else { 0.6 };
    let drums = 0.3 * kick + 0.15 * snare + 0.05 * hat;
    let mix = drums + 0.15 * bass + 0.08 * voice + 2.0 * pan * 0.02 * piano;
    mix as f32
}

#[test]
fn coreml_model_separates_a_short_song_into_six_stems() {
    let Some(path) = installed_model() else {
        report(&format!(
            "SKIPPED coreml_model_separates_a_short_song_into_six_stems: no compiled stem model \
             (run `mise run stem-model` or set {MODEL_ENV})"
        ));
        return;
    };
    let _heavy = heavy();
    let lib = TempLibrary::new("coreml");
    // 12 s at 44.1 kHz stereo: two chunks.
    let song = lib.import("band", 44_100, 2, 529_200, band);
    let loaded = Instant::now();
    let mut model = load_model(std::slice::from_ref(&path)).unwrap();
    let load_secs = loaded.elapsed().as_secs_f64();
    let stft = Stft::new();
    let mut work = Workspace::new();
    let warmed = Instant::now();
    work.warm_up(&stft, model.as_mut()).unwrap();
    let warm_secs = warmed.elapsed().as_secs_f64();
    let started = Instant::now();
    let mut progress = Vec::new();
    job::run(
        &lib.library,
        &song.id,
        &stft,
        model.as_mut(),
        &mut work,
        &AtomicBool::new(false),
        &mut |p| progress.push(p),
    )
    .unwrap();
    let separate_secs = started.elapsed().as_secs_f64();
    assert_eq!(progress.last(), Some(&1.0));

    let saved = lib.library.get(&song.id).unwrap();
    assert!(saved.stems_ready());
    assert_eq!(saved.tracks().len(), 6);
    let dir = lib.song_dir(&song.id);
    let (_, audio) = read_wav(&dir.join(AUDIO_FILE));
    let mut sum = vec![0.0_f32; audio.len()];
    let mut levels = Vec::new();
    let mut rms_dbfs = Vec::new();
    for name in MODEL_STEMS {
        let (spec, samples) = read_wav(&dir.join(name.relative_path()));
        assert_eq!(
            (spec.sample_rate, spec.channels, spec.bits_per_sample),
            (44_100, 2, 16)
        );
        assert_eq!(samples.len(), audio.len(), "{name:?} length");
        let rms = (energy(&samples) / samples.len() as f64).sqrt();
        let dbfs = 20.0 * rms.max(1e-9).log10();
        rms_dbfs.push(dbfs);
        levels.push(format!("{} {dbfs:.2} dBFS", name.as_str()));
        for (s, &x) in sum.iter_mut().zip(&samples) {
            *s += x;
        }
    }
    let snr = snr_db(&audio, &sum);
    report(&format!(
        "coreml separation ({}): load {load_secs:.2}s, warm-up {warm_secs:.2}s, 12 s song in \
         {separate_secs:.2}s; six stems sum to the song at {snr:.2} dB; {}",
        path.display(),
        levels.join(", ")
    ));
    if let Some(out) = std::env::var_os("WOODSHED_STEM_TEST_OUT") {
        // For comparing with PyTorch (tools/stem-model/ref_torch.py, compare.py).
        let out = PathBuf::from(out);
        fs::create_dir_all(&out).unwrap();
        fs::copy(dir.join(AUDIO_FILE), out.join("mix.wav")).unwrap();
        for name in MODEL_STEMS {
            let file = format!("{}.wav", name.as_str());
            fs::copy(dir.join(STEMS_DIR).join(&file), out.join(file)).unwrap();
        }
    }
    // PyTorch (`apply_model`, ref_torch.py) on the same audio.wav: its six
    // stems sum to the mix at 16.92 dB, and each stem has this level.
    assert!(
        (snr - PYTORCH_SUM_SNR_DB).abs() < 0.1,
        "six stems sum to the song at {snr:.2} dB, PyTorch {PYTORCH_SUM_SNR_DB} dB"
    );
    for (name, (level, expected)) in MODEL_STEMS
        .iter()
        .zip(rms_dbfs.iter().zip(PYTORCH_RMS_DBFS))
    {
        assert!(
            (level - expected).abs() < 0.1,
            "{name:?} at {level:.2} dBFS, PyTorch {expected} dBFS"
        );
    }
}

/// PyTorch reference for [`band`] after import (16-bit `audio.wav`): six
/// stems sum to the mix at this SNR, and stem levels in model order.
const PYTORCH_SUM_SNR_DB: f64 = 16.92;
const PYTORCH_RMS_DBFS: [f64; SOURCES] = [-24.72, -23.42, -61.91, -33.29, -26.14, -44.74];

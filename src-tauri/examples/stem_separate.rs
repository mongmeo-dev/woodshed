//! The app's separation path without a window: imports an audio file into a
//! song library, separates it through `SeparationService` with the real Core
//! ML model (cancelling once first), checks the manifest and stems, then
//! starts a new service as the app does after a restart and opens the stems
//! with the stem player, without separating again.
//!
//! ```sh
//! cargo run --release --example stem_separate -- --library <dir> <audio file>
//! # Kill the process at 30 % of a separation, as if the app quit mid-way:
//! cargo run --release --example stem_separate -- --library <dir> --crash-at 0.3 <audio file>
//! # Then start again and check what the next launch finds:
//! cargo run --release --example stem_separate -- --library <dir> --check <song id>
//! # Re-separate a song that is already separated, crashing at 30 %:
//! cargo run --release --example stem_separate -- --library <dir> --crash-at 0.3 --song <song id>
//! ```
//!
//! Use a scratch directory for `--library`, not the app's data.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use woodshed_lib::separation::job::{OLD_STEMS_DIR, WORK_DIR};
use woodshed_lib::separation::{
    STATE_CANCELLED, STATE_DONE, STATE_FAILED, STATE_RUNNING, SeparationService, ServiceConfig,
    app_model_loader, default_cache_dir,
};
use woodshed_lib::stems::{
    Library, Manifest, ProcessControl, STEMS_DIR, SeparationEvent, StemPlayer, TrackReader,
    WavTrack, import_file,
};

type Log = Arc<Mutex<Vec<(Instant, SeparationEvent)>>>;

fn service(library: &Library, log: &Log, crash_at: Option<f32>) -> SeparationService {
    let log = Arc::clone(log);
    let started = Instant::now();
    SeparationService::start(
        library.clone(),
        app_model_loader(
            Some(PathBuf::from(env!("CARGO_MANIFEST_DIR"))),
            default_cache_dir(),
        ),
        Arc::new(move |event: SeparationEvent| {
            println!(
                "  {:6.2}s event {} {:?}",
                started.elapsed().as_secs_f64(),
                event.state,
                event.progress
            );
            if let (Some(at), Some(progress)) = (crash_at, event.progress)
                && event.state == STATE_RUNNING
                && progress >= at
            {
                println!("  crash: aborting the process at {:.0} %", progress * 100.0);
                std::process::abort();
            }
            log.lock().unwrap().push((Instant::now(), event));
        }),
        ServiceConfig {
            warm_up_after: None,
            unload_after: Duration::from_secs(60),
        },
    )
}

fn wait_for(log: &Log, id: &str, states: &[&str]) -> SeparationEvent {
    let deadline = Instant::now() + Duration::from_secs(600);
    loop {
        if let Some((_, event)) = log
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(_, e)| e.song_id == id && states.contains(&e.state.as_str()))
        {
            return event.clone();
        }
        assert!(Instant::now() < deadline, "no {states:?} event for {id}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn describe(manifest: &Manifest) -> String {
    let stems: Vec<String> = manifest
        .stems
        .iter()
        .map(|s| format!("{}={:?}", s.name.as_str(), s.status))
        .collect();
    format!(
        "manifest: {} Hz, {} ch, {} frames, stems [{}]",
        manifest.sample_rate,
        manifest.channels,
        manifest.frames,
        stems.join(", ")
    )
}

/// Checks the stems against the contract and opens them with the player.
fn check_ready(library: &Library, id: &str) -> Vec<std::time::SystemTime> {
    let manifest = library.get(id).expect("song");
    println!("{}", describe(&manifest));
    assert!(manifest.stems_ready(), "all six stems ready");
    let dir = library.song_dir(id).unwrap();
    assert!(!dir.join(WORK_DIR).exists() && !dir.join(OLD_STEMS_DIR).exists());
    let tracks = manifest.tracks();
    let names: Vec<String> = tracks.iter().map(|(t, _)| format!("{t:?}")).collect();
    println!("mixer tracks: {}", names.join(", "));
    assert_eq!(tracks.len(), 6);
    let mut readers: Vec<Box<dyn TrackReader>> = Vec::new();
    let mut times = Vec::new();
    for (_, path) in &tracks {
        let path = dir.join(path);
        let reader = hound::WavReader::open(&path).expect("stem wav");
        let spec = reader.spec();
        assert_eq!(spec.sample_rate, manifest.sample_rate);
        assert_eq!(spec.channels, manifest.channels);
        assert_eq!(
            (spec.bits_per_sample, spec.sample_format),
            (16, hound::SampleFormat::Int)
        );
        assert_eq!(u64::from(reader.duration()), manifest.frames);
        times.push(std::fs::metadata(&path).unwrap().modified().unwrap());
        readers.push(Box::new(
            WavTrack::open(&path).expect("player opens the stem"),
        ));
    }
    let mut hash = 0xCBF2_9CE4_8422_2325_u64;
    for (_, path) in &tracks {
        for byte in std::fs::read(dir.join(path)).unwrap() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3);
        }
    }
    println!("stem files fingerprint {hash:016x}");
    let player = StemPlayer::new(readers, 0).expect("player");
    println!(
        "stem player opened {} tracks at {} Hz, {} frames",
        tracks.len(),
        player.song_rate,
        player.frames
    );
    times
}

/// What a new launch finds: recovery runs, nothing separates again.
fn check_restart(library: &Library, id: &str) {
    let log: Log = Arc::default();
    let service = service(library, &log, None);
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        service.status().is_empty(),
        "no separation runs after a restart"
    );
    assert!(log.lock().unwrap().is_empty(), "no events after a restart");
    let manifest = library.get(id).expect("song");
    println!("after restart: {}", describe(&manifest));
    let dir = library.song_dir(id).unwrap();
    println!(
        "after restart: .separating {}, .stems-old {}, stems/ {}",
        dir.join(WORK_DIR).exists(),
        dir.join(OLD_STEMS_DIR).exists(),
        dir.join(STEMS_DIR).exists()
    );
    service.shutdown();
}

fn main() {
    let mut library_dir = None;
    let mut crash_at = None;
    let mut check = None;
    let mut existing = None;
    let mut input = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--library" => library_dir = args.next().map(PathBuf::from),
            "--crash-at" => crash_at = args.next().and_then(|v| v.parse::<f32>().ok()),
            "--check" => check = args.next(),
            "--song" => existing = args.next(),
            _ => input = Some(PathBuf::from(arg)),
        }
    }
    let library = Library::new(&library_dir.expect("--library <dir> is required"));

    if let Some(id) = check {
        check_restart(&library, &id);
        if library.get(&id).is_ok_and(|m| m.stems_ready()) {
            check_ready(&library, &id);
        }
        return;
    }

    let song = if let Some(id) = existing {
        let song = library.get(&id).expect("song");
        println!("existing song {id}");
        if song.stems_ready() {
            check_ready(&library, &id);
        }
        song
    } else {
        let input = input.expect("an audio file");
        let started = Instant::now();
        let control = ProcessControl::default();
        let song = import_file(&library, Path::new(&input), &control, &mut |_| {}).expect("import");
        println!(
            "imported {} as {} in {:.2}s",
            input.display(),
            song.id,
            started.elapsed().as_secs_f64()
        );
        song
    };
    println!("{}", describe(&song));

    let log: Log = Arc::default();
    let service = service(&library, &log, crash_at);
    if crash_at.is_some() {
        service.separate(&song.id).expect("separate");
        wait_for(&log, &song.id, &[STATE_DONE, STATE_FAILED, STATE_CANCELLED]);
        println!("finished before the crash point");
        return;
    }

    // Cancel once the first chunk is done.
    println!("separate, then cancel after the first chunk:");
    service.separate(&song.id).expect("separate");
    let deadline = Instant::now() + Duration::from_secs(600);
    while !log
        .lock()
        .unwrap()
        .iter()
        .any(|(_, e)| e.song_id == song.id && e.progress.is_some_and(|p| p > 0.0))
    {
        assert!(Instant::now() < deadline, "no progress");
        std::thread::sleep(Duration::from_millis(10));
    }
    let asked = Instant::now();
    service.cancel(&song.id).expect("cancel");
    let event = wait_for(&log, &song.id, &[STATE_CANCELLED, STATE_DONE, STATE_FAILED]);
    let stopped = log
        .lock()
        .unwrap()
        .iter()
        .find(|(_, e)| e.state == event.state)
        .map(|(at, _)| at.duration_since(asked).as_secs_f64())
        .unwrap_or_default();
    println!("cancel → {} after {stopped:.2}s", event.state);
    let manifest = library.get(&song.id).unwrap();
    println!("after cancel: {}", describe(&manifest));
    let dir = library.song_dir(&song.id).unwrap();
    println!(
        "after cancel: .separating {}, stems/ {}",
        dir.join(WORK_DIR).exists(),
        dir.join(STEMS_DIR).exists()
    );
    assert_eq!(event.state, STATE_CANCELLED);
    assert!(manifest.stems.is_empty() && !dir.join(WORK_DIR).exists());

    println!("separate to the end:");
    log.lock().unwrap().clear();
    let started = Instant::now();
    service.separate(&song.id).expect("separate");
    let event = wait_for(&log, &song.id, &[STATE_DONE, STATE_FAILED, STATE_CANCELLED]);
    let secs = started.elapsed().as_secs_f64();
    assert_eq!(event.state, STATE_DONE, "separation finished");
    let progress: Vec<String> = log
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(_, e)| e.progress.map(|p| format!("{:.0}%", p * 100.0)))
        .collect();
    println!(
        "separated {:.1}s of audio in {secs:.2}s; progress {}",
        song.duration_secs(),
        progress.join(" ")
    );
    let times = check_ready(&library, &song.id);
    service.shutdown();

    println!("restart:");
    check_restart(&library, &song.id);
    let again = check_ready(&library, &song.id);
    assert_eq!(
        times, again,
        "stems untouched: opened without separating again"
    );
    println!("ok: reopened {} without separating again", song.id);
}

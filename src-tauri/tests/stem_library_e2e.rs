//! The stem splitter's real path without a device or a window: import each
//! supported file into a library, reopen the library as after an app
//! restart, and play every song through the engine with an A-B loop and a
//! slower speed.

use std::path::{Path, PathBuf};

use woodshed_lib::audio::{EventKind, OfflineEngine, StreamFormat};
use woodshed_lib::stems::{
    Library, LoopRange, ProcessControl, StemPlayer, TrackReader, WavTrack, import_file,
    loop_events, param,
};

const SONG_RATE: u32 = 44_100;
const OUT_RATE: u32 = 48_000;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("woodshed-e2e-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Same material as the fixtures: 1 s, 440 Hz, half scale, 44.1 kHz.
fn write_tone_wav(path: &Path) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SONG_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..SONG_RATE {
        let value = (std::f32::consts::TAU * 440.0 * i as f32 / SONG_RATE as f32).sin() * 0.5;
        writer.write_sample((value * 32_767.0) as i16).unwrap();
    }
    writer.finalize().unwrap();
}

fn frequency(samples: &[f32], rate: u32) -> f32 {
    let crossings: Vec<f32> = samples
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0)
        .map(|(i, w)| i as f32 + w[0] / (w[0] - w[1]))
        .collect();
    let periods = (crossings.len() - 1) as f32;
    periods * rate as f32 / (crossings[crossings.len() - 1] - crossings[0])
}

#[test]
fn imported_songs_survive_a_restart_and_play_with_loop_and_speed() {
    let dir = temp_dir("library");
    let wav = dir.join("tone-440.wav");
    write_tone_wav(&wav);
    let inputs = [
        fixtures().join("tone-440.mp3"),
        fixtures().join("tone-440.flac"),
        fixtures().join("tone-440.m4a"),
        fixtures().join("tone-440.ogg"),
        wav,
    ];

    let data = dir.join("app-data");
    let library = Library::new(&data);
    let mut imported = Vec::new();
    for input in &inputs {
        let song = import_file(&library, input, &ProcessControl::default(), &mut |_| {})
            .unwrap_or_else(|error| panic!("{}: {error}", input.display()));
        assert_eq!(song.sample_rate, SONG_RATE, "{}", input.display());
        assert!(
            (song.duration_secs() - 1.0).abs() < 0.06,
            "{}: {} s",
            input.display(),
            song.duration_secs()
        );
        assert_eq!(song.title, "tone-440");
        imported.push(song);
    }
    drop(library);

    // App restart: a new library over the same directory lists every song.
    let library = Library::new(&data);
    let listed = library.list();
    assert_eq!(listed.len(), inputs.len());
    for song in &imported {
        assert!(
            listed.contains(song),
            "{} listed after restart",
            song.original
        );
    }

    for song in &listed {
        let song_dir = library.song_dir(&song.id).unwrap();
        let tracks: Vec<Box<dyn TrackReader>> = song
            .tracks()
            .iter()
            .map(|(_, path)| {
                Box::new(WavTrack::open(&song_dir.join(path)).unwrap()) as Box<dyn TrackReader>
            })
            .collect();
        let player = StemPlayer::new(tracks, 0).unwrap();
        let mut feeder = player.feeder;
        let shared = player.shared;
        let mut engine = OfflineEngine::new(StreamFormat::new(OUT_RATE, 2));
        let id = engine
            .handle
            .add_source(Box::new(player.source), 1.0)
            .unwrap();

        // Loop 0.2–0.6 s at 75 % speed, the way the screen sends them.
        let range = LoopRange::new(8_820, 26_460).unwrap();
        for event in loop_events(Some(range)) {
            engine.handle.schedule(0, id, event).unwrap();
        }
        let speed = EventKind::Param {
            id: param::SPEED,
            value: 0.75,
        };
        engine.handle.schedule(0, id, speed).unwrap();
        engine.handle.schedule(0, id, EventKind::Start).unwrap();

        let mut out = vec![0.0_f32; OUT_RATE as usize * 2 * 2];
        for chunk in out.chunks_mut(128 * 2) {
            feeder.fill();
            engine.render_callback(chunk);
        }
        let left: Vec<f32> = out.as_chunks::<2>().0.iter().map(|f| f[0]).collect();

        // Two seconds is past the song's end without the loop; it keeps looping.
        assert!(!shared.ended(), "{}: still looping", song.original);
        assert!(shared.playing());
        let position = shared.position();
        assert!(
            (range.start as f64..range.end as f64).contains(&position),
            "{}: position {position}",
            song.original
        );
        let pitch = frequency(&left[OUT_RATE as usize / 2..], OUT_RATE);
        assert!((pitch - 440.0).abs() < 4.0, "{}: {pitch} Hz", song.original);
        let peak = left.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(
            (0.3..0.75).contains(&peak),
            "{}: peak {peak}",
            song.original
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

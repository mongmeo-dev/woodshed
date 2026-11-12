//! Imports files or YouTube links into a stem library and plays the newest
//! song offline, without a window or an audio device:
//!
//! ```sh
//! cargo run --example stem_import -- --library /tmp/songs \
//!     [--tools src-tauri/target/release/bundle/macos/Woodshed.app/Contents/MacOS] \
//!     <file or YouTube link>...
//! ```
//!
//! `--tools` is the folder holding `yt-dlp` and `ffmpeg` (default: the
//! target folder, where the build copies the sidecars). Prints each import,
//! the library after reopening it, and a short loop/speed render.

use std::path::PathBuf;

use woodshed_lib::audio::{EventKind, OfflineEngine, StreamFormat};
use woodshed_lib::stems::{
    Library, LoopRange, ProcessControl, StemPlayer, Tools, TrackReader, WavTrack, import_file,
    import_link, loop_events, param, parse_youtube_link,
};

fn main() {
    let mut library_dir = None;
    let mut tools_dir = None;
    let mut inputs = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--library" => library_dir = args.next().map(PathBuf::from),
            "--tools" => tools_dir = args.next().map(PathBuf::from),
            _ => inputs.push(arg),
        }
    }
    let library_dir = library_dir.expect("--library <dir> is required");
    let tools_dir = tools_dir.unwrap_or_else(|| {
        let exe = std::env::current_exe().expect("current exe");
        exe.parent()
            .and_then(|p| p.parent())
            .expect("target dir")
            .to_path_buf()
    });
    let tools = Tools::in_dir(&tools_dir);
    println!("tools: {tools:?}");

    let library = Library::new(&library_dir);
    let cache = library_dir.join("yt-dlp-cache");
    for input in &inputs {
        let control = ProcessControl::default();
        let mut last = None;
        let mut report = |progress: woodshed_lib::stems::ImportProgress| {
            let step = (progress.stage, progress.progress.map(|p| (p * 4.0) as u8));
            if last != Some(step) {
                last = Some(step);
                println!("  {:?} {:?}", progress.stage, progress.progress);
            }
        };
        let result = if parse_youtube_link(input).is_some() {
            import_link(
                &library,
                input,
                tools.clone(),
                &cache,
                &control,
                &mut report,
            )
        } else {
            import_file(&library, std::path::Path::new(input), &control, &mut report)
        };
        match result {
            Ok(song) => println!(
                "imported {input}: \"{}\" {} Hz {} ch {:.2} s ({})",
                song.title,
                song.sample_rate,
                song.channels,
                song.duration_secs(),
                song.original
            ),
            Err(error) => println!("failed {input}: {error} ({error:?})"),
        }
    }

    let reopened = Library::new(&library_dir);
    let songs = reopened.list();
    println!("library after reopening: {} song(s)", songs.len());
    for song in &songs {
        println!(
            "  {} \"{}\" {:.2} s",
            song.id,
            song.title,
            song.duration_secs()
        );
    }
    let Some(song) = songs.first() else { return };

    let dir = reopened.song_dir(&song.id).expect("song dir");
    let tracks: Vec<Box<dyn TrackReader>> = song
        .tracks()
        .iter()
        .map(|(_, path)| Box::new(WavTrack::open(&dir.join(path)).expect("track")) as _)
        .collect();
    let player = StemPlayer::new(tracks, 0).expect("player");
    let (mut feeder, shared, rate) = (player.feeder, player.shared, player.song_rate);
    let mut engine = OfflineEngine::new(StreamFormat::new(48_000, 2));
    let id = engine
        .handle
        .add_source(Box::new(player.source), 1.0)
        .expect("source");
    let start = u64::from(rate);
    let end = (u64::from(rate) * 3).min(player.frames);
    let range = LoopRange::new(start, end);
    for event in loop_events(range) {
        engine.handle.schedule(0, id, event).expect("loop");
    }
    let speed = EventKind::Param {
        id: param::SPEED,
        value: 0.75,
    };
    engine.handle.schedule(0, id, speed).expect("speed");
    engine
        .handle
        .schedule(0, id, EventKind::Start)
        .expect("start");
    let mut out = vec![0.0_f32; 48_000 * 2 * 6];
    for chunk in out.chunks_mut(256) {
        feeder.fill();
        engine.render_callback(chunk);
    }
    let rms = (out.iter().map(|s| s * s).sum::<f32>() / out.len() as f32).sqrt();
    println!(
        "rendered 6 s of \"{}\" at 75 % with loop {:?}: rms {rms:.3}, position {:.2} s, playing {}, ended {}, underruns {}",
        song.title,
        range.map(|r| (
            r.start as f64 / f64::from(rate),
            r.end as f64 / f64::from(rate)
        )),
        shared.position() / f64::from(rate),
        shared.playing(),
        shared.ended(),
        shared.underruns()
    );
}

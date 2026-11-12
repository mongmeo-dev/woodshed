//! Separates a WAV file with the app's stem pipeline (Rust DSP + Core ML) and
//! writes the six stems as 32-bit float WAVs at 44.1 kHz, to compare with
//! PyTorch (`tools/stem-model/ref_torch.py` and `compare.py`):
//!
//! ```sh
//! cargo run --release --example separate_wav -- <in.wav> <out_dir>
//! # Measure without writing stems (long files):
//! cargo run --release --example separate_wav -- --discard <in.wav>
//! ```
//!
//! The model is found as in the app's dev builds (`WOODSHED_STEM_MODEL`,
//! `src-tauri/models`, then `~/Library/Caches/dev.mongmeo.woodshed/models`).
//! Prints load, warm-up and separation times, and how often the GPU runs
//! needed a retry or the CPU fallback (`VerifiedModel`).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use woodshed_lib::separation::dsp::{MODEL_RATE, Stft};
use woodshed_lib::separation::{
    MODEL_STEMS, SeparationError, SeparationOutput, Workspace, default_cache_dir, load_verified,
    locate_model, model_candidates, read_song,
};

/// Float stems at the model rate, stereo (none with `--discard`).
struct FloatStems {
    writers: Vec<hound::WavWriter<std::io::BufWriter<std::fs::File>>>,
    chunks: usize,
}

impl SeparationOutput for FloatStems {
    fn write(&mut self, planes: &[f32], len: usize) -> Result<(), SeparationError> {
        for (source, writer) in self.writers.iter_mut().enumerate() {
            for i in 0..len {
                writer.write_sample(planes[source * 2 * len + i])?;
                writer.write_sample(planes[(source * 2 + 1) * len + i])?;
            }
        }
        Ok(())
    }

    fn chunk_done(&mut self, done: usize, total: usize) {
        self.chunks = total;
        println!("  chunk {done}/{total}");
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input, out_dir) = match args.as_slice() {
        [flag, input] if flag == "--discard" => (input, None),
        [input, out_dir] => (input, Some(PathBuf::from(out_dir))),
        _ => return Err("usage: separate_wav (<in.wav> <out_dir> | --discard <in.wav>)".into()),
    };
    if let Some(dir) = &out_dir {
        std::fs::create_dir_all(dir)?;
    }
    let candidates = model_candidates(
        Some(Path::new(env!("CARGO_MANIFEST_DIR"))),
        default_cache_dir().as_deref(),
    );
    let model_path =
        locate_model(&candidates).ok_or("no compiled stem model; run `mise run stem-model`")?;
    println!("model: {}", model_path.display());

    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let audio = read_song(Path::new(input), &cancel)?;
    let read_secs = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let mut model = load_verified(&model_path)?;
    let load_secs = started.elapsed().as_secs_f64();
    let stft = Stft::new();
    let mut work = Workspace::new();
    let started = Instant::now();
    work.warm_up(&stft, &mut model)?;
    let warm_secs = started.elapsed().as_secs_f64();

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: MODEL_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let writers = match &out_dir {
        Some(dir) => MODEL_STEMS
            .iter()
            .map(|name| hound::WavWriter::create(dir.join(format!("{}.wav", name.as_str())), spec))
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };
    let mut output = FloatStems { writers, chunks: 0 };
    let started = Instant::now();
    work.separate(
        &audio.left,
        &audio.right,
        &stft,
        &mut model,
        &cancel,
        &mut output,
    )?;
    let separate_secs = started.elapsed().as_secs_f64();
    for writer in output.writers {
        writer.finalize()?;
    }
    println!(
        "song {:.1}s ({} Hz, {} ch) | read {read_secs:.2}s | model load {load_secs:.2}s, warm-up {warm_secs:.2}s | separation {separate_secs:.2}s for {} chunks ({:.2}s per chunk)",
        audio.frames as f64 / f64::from(audio.rate),
        audio.rate,
        audio.channels,
        output.chunks,
        separate_secs / output.chunks.max(1) as f64,
    );
    let stats = model.stats();
    println!(
        "model runs: {} chunks (warm-up included), {} extra GPU runs, {} CPU fallbacks",
        stats.chunks, stats.extra_runs, stats.fallbacks
    );
    Ok(())
}

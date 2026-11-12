use std::path::Path;

/// Sidecars listed in `tauri.conf.json` → `bundle.externalBin`.
const SIDECARS: &[&str] = &["yt-dlp", "ffmpeg"];

/// The compiled stem model listed in `tauri.conf.json` → `bundle.resources`.
const STEM_MODEL: &str = "models/htdemucs_6s_fp32.mlmodelc";

fn main() {
    ensure_sidecar_placeholders();
    ensure_stem_model_placeholder();
    tauri_build::build()
}

/// tauri-build refuses a missing resource path but skips an empty directory.
/// `tools/stem-model/build.sh` (`mise run stem-model`) puts the real model in
/// place, and release builds run it first (`beforeBuildCommand`). Checks,
/// tests and dev builds get an empty directory instead; the app then looks
/// for the model in its cache directory and otherwise reports it missing.
fn ensure_stem_model_placeholder() {
    std::fs::create_dir_all(STEM_MODEL).expect("create the stem model directory");
}

/// tauri-build refuses to build while a sidecar file is missing.
/// `scripts/fetch-binaries.sh` (`mise run binaries`) puts the real ones in
/// place, and release builds run it first (`beforeBuildCommand`). Checks,
/// tests and dev builds get an empty placeholder instead, which the app
/// reports as a missing tool.
fn ensure_sidecar_placeholders() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET");
    let dir = Path::new("binaries");
    for name in SIDECARS {
        let path = dir.join(format!("{name}-{target}"));
        if path.exists() {
            continue;
        }
        std::fs::create_dir_all(dir).expect("create src-tauri/binaries");
        std::fs::write(&path, b"").expect("write sidecar placeholder");
    }
}

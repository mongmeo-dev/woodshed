use std::path::Path;

/// Sidecars listed in `tauri.conf.json` → `bundle.externalBin`.
const SIDECARS: &[&str] = &["yt-dlp", "ffmpeg"];

fn main() {
    ensure_sidecar_placeholders();
    tauri_build::build()
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

//! Third-party notices the app must ship (bundled via `bundle.resources`).

use std::path::Path;

fn read(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../LICENSES")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn audio_crate_notices_and_texts_are_present() {
    let list = read("rust-audio-crates.md");
    let crates: Vec<&str> = list
        .lines()
        .filter_map(|line| line.strip_prefix('|'))
        .filter_map(|row| row.split('|').next())
        .map(str::trim)
        .collect();
    for name in [
        "symphonia",
        "symphonia-core",
        "signalsmith-stretch",
        "rubato",
        "realfft",
        "rustfft",
        "hound",
        "audioadapter",
    ] {
        assert!(crates.contains(&name), "{name} listed");
    }
    let texts = read("rust-audio-crates.txt");
    for needle in [
        "Mozilla Public License Version 2.0",
        "Apache License",
        "Signalsmith Stretch 1.3.1 (C++)",
        "Signalsmith Linear (C++)",
        "Permission is hereby granted, free of charge",
        "rubato 5.0.1",
    ] {
        assert!(texts.contains(needle), "{needle} present");
    }
    assert!(
        !texts.contains("No license file"),
        "every crate has its text"
    );
}

#[test]
fn bundled_program_notices_are_present() {
    assert!(read("ffmpeg-LGPL-2.1.txt").contains("GNU LESSER GENERAL PUBLIC LICENSE"));
    assert!(read("yt-dlp-LICENSE.txt").contains("public domain"));
    let readme = read("README.md");
    assert!(readme.contains("ffmpeg-9.0.2.tar.xz"));
    assert!(readme.contains("GPLv3+"));
}

#[test]
fn the_bundle_carries_the_licenses_directory() {
    let config: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(config["bundle"]["resources"]["../LICENSES/"], "LICENSES/");
}

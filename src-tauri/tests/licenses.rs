//! Third-party notices the app must ship (bundled via `bundle.resources`).

use std::path::Path;

fn read(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../LICENSES")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn listed(file: &str) -> Vec<String> {
    read(file)
        .lines()
        .filter_map(|line| line.strip_prefix('|'))
        .filter_map(|row| row.split('|').next())
        .map(|name| name.trim().to_owned())
        .collect()
}

#[test]
fn rust_crate_notices_and_texts_are_present() {
    let crates = listed("rust-crates.md");
    for name in [
        "tauri",
        "wry",
        "cpal",
        "symphonia",
        "symphonia-core",
        "signalsmith-stretch",
        "rubato",
        "realfft",
        "rustfft",
        "hound",
        "audioadapter",
        "objc2-core-ml",
    ] {
        assert!(crates.iter().any(|c| c == name), "{name} listed");
    }
    let texts = read("rust-crates.txt");
    for needle in [
        "Mozilla Public License Version 2.0",
        "Apache License",
        "Signalsmith Stretch 1.3.1 (C++)",
        "Signalsmith Linear (C++)",
        "Permission is hereby granted, free of charge",
        "rubato 5.0.1",
        "UNICODE LICENSE V3",
    ] {
        assert!(texts.contains(needle), "{needle} present");
    }
}

#[test]
fn npm_package_notices_and_texts_are_present() {
    let packages = listed("npm-packages.md");
    for name in ["react", "react-dom", "i18next", "@tauri-apps/api"] {
        assert!(packages.iter().any(|p| p == name), "{name} listed");
    }
    assert!(read("npm-packages.txt").contains("Meta Platforms"));
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

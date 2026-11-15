# Third-party notices

Woodshed itself is released under the MIT License ([`../LICENSE`](../LICENSE)). This directory holds the notices of the third-party parts the app is built from or ships with, and it ships inside the app (`Contents/Resources/LICENSES/`).

| Part                                | How it reaches the app     | License                                           | Notice                                                                       |
| ----------------------------------- | -------------------------- | ------------------------------------------------- | ---------------------------------------------------------------------------- |
| Rust crates (Tauri, audio, Core ML) | linked into the app binary | MIT, Apache-2.0, MPL-2.0, BSD, Zlib, Unicode …    | [`rust-crates.md`](rust-crates.md), [`rust-crates.txt`](rust-crates.txt)     |
| npm packages (React, i18next, …)    | bundled into the web view  | MIT, Apache-2.0 OR MIT                            | [`npm-packages.md`](npm-packages.md), [`npm-packages.txt`](npm-packages.txt) |
| FFmpeg 9.0.2                        | separate program (sidecar) | LGPL-2.1-or-later                                 | [FFmpeg](#ffmpeg-902), [`ffmpeg-LGPL-2.1.txt`](ffmpeg-LGPL-2.1.txt)          |
| yt-dlp 2026.10.26 (`yt-dlp_macos`)  | separate program (sidecar) | Unlicense; the bundle as a whole GPL-3.0-or-later | [yt-dlp](#yt-dlp-20261026-yt-dlp_macos)                                      |
| Demucs `htdemucs_6s` (Core ML)      | bundled resource           | code MIT; **weights: presumed CC BY-NC**          | [Stem separation model](#stem-separation-model-bundled-resource)             |
| Drum samples (VCSL)                 | bundled resource           | CC0-1.0                                           | [`drum-samples.md`](drum-samples.md), [`CC0-1.0.txt`](CC0-1.0.txt)           |

No dependency of the app is under the GPL. The only GPL code shipped is inside the separate `yt-dlp_macos` program, which the app runs as a child process.

## Rust crates and npm packages

`scripts/collect-licenses.py` writes both lists from the lock files: every Rust crate linked into the app on macOS (proc-macros left out, since they only run at build time) and every npm production dependency. Each list gives the version, the license expression, the license the app uses where there is a choice, and the source location; the `.txt` files hold the full texts, each identical text written once. Rerun the script when dependencies change; it fails if a package has no license text.

Some crates publish no license file. Their texts come from their repositories and are kept in [`upstream/`](upstream):

- The objc2 crates (`objc2*`, `block2`, `dispatch2`): the repository's MIT text [`upstream/objc2-LICENSE-MIT.txt`](upstream/objc2-LICENSE-MIT.txt), with its licensing note [`upstream/objc2-LICENSE.md.txt`](upstream/objc2-LICENSE.md.txt) (<https://github.com/madsmtm/objc2>, commit `ee9a7ada2131`).
- `dasp*`: [`upstream/dasp-LICENSE-MIT.txt`](upstream/dasp-LICENSE-MIT.txt) (<https://github.com/rustaudio/dasp>).
- `alloc-stdlib`: [`upstream/alloc-stdlib-LICENSE.txt`](upstream/alloc-stdlib-LICENSE.txt) (<https://github.com/dropbox/rust-alloc-no-stdlib>, commit `0a81fd6928ea`).
- `selectors`: the repository has no license file and every source file carries the MPL-2.0 header, so the text is the MPL-2.0 from <https://mozilla.org/MPL/2.0/> ([`upstream/MPL-2.0.txt`](upstream/MPL-2.0.txt)).

The MPL-2.0 crates (Symphonia, `cssparser`, `selectors` and others) are used unmodified; their source is at the repositories listed in [`rust-crates.md`](rust-crates.md) and on crates.io. Signalsmith Stretch and Signalsmith Linear (C++, MIT), bundled inside the `signalsmith-stretch` crate, are listed in [`rust-crates.txt`](rust-crates.txt) too.

## Stem separation model (bundled resource)

`Contents/Resources/models/htdemucs_6s_fp32.mlmodelc` is a Core ML conversion of the Demucs v4 `htdemucs_6s` model (single model `5c90dfd2`), made by `tools/stem-model/build.sh` from the weights at <https://huggingface.co/adefossez/HTDemucs-6s> (revision `3c5ee475be622df764938de97e4281a7b07ffa58`, `5c90dfd2.safetensors` SHA-256 `d2a1745f0744721f6b8ca5bf469b67c651ea5ed1b52998cab033b2158609d411`).

- Demucs code: MIT License, Copyright (c) Meta Platforms, Inc. and affiliates ([`demucs-MIT.txt`](demucs-MIT.txt)). Source: <https://github.com/adefossez/demucs> (`demucs` 4.1.0 on PyPI).
- Weights: **the license is unclear; they are presumed to be under CC BY-NC (non-commercial).** The Hugging Face model card has no license field, and the Demucs README ("License" section, <https://github.com/adefossez/demucs/blob/829d21890023800713e5dd231c67e43ecc06069b/README.md#license>) says:

  > Demucs code is released under the MIT license as found in the LICENSE file. The license of the weights is not clear: MusDB contains some data that were only usable for research purpose. I sadly cannot help granting a license, both coming from Meta custom data, or from the original MusDB data. The 6 source variant of htdemucs was trained only on Meta internal dataset, which likely means CC-NC-BY license is applicable, as it was the case for all models released by FAIR.

- Use: Woodshed release builds that bundle this model are distributed free of charge, for non-commercial use only; the model must not be used commercially. Woodshed's own source code stays under the MIT License.
- Attribution: Meta Demucs, `htdemucs_6s` (<https://github.com/adefossez/demucs>, weights at <https://huggingface.co/adefossez/HTDemucs-6s>). The converted model's metadata records the same terms.

## Bundled programs

The app bundles two separate programs as sidecars (`Woodshed.app/Contents/MacOS/`). The app only runs them as child processes; it does not link them. `scripts/fetch-binaries.sh` fetches or builds both with pinned versions and SHA-256 checksums.

### FFmpeg 9.0.2

- License: GNU Lesser General Public License v2.1 or later ([`ffmpeg-LGPL-2.1.txt`](ffmpeg-LGPL-2.1.txt)). The build contains no GPL or nonfree parts (`--disable-gpl --disable-nonfree --disable-version3`) and no external libraries.
- Source: <https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz> (SHA-256 `8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e`, signed by the FFmpeg release key `FCF986EA15E6E293A5644F10B4322F04D67658D8`). The source is unmodified.
- Build: the exact `configure` flags are in [`scripts/fetch-binaries.sh`](../scripts/fetch-binaries.sh) and are also printed by `ffmpeg -buildconf`. It is an audio-only build: file/pipe protocols; mov, matroska, ogg, mp3, aac, wav and flac demuxers; aac, opus, vorbis, mp3, flac, alac and PCM decoders; PCM WAV output; swresample.
- FFmpeg is a trademark of Fabrice Bellard, originator of the FFmpeg project.

### yt-dlp 2026.10.26 (`yt-dlp_macos`)

- yt-dlp itself is released under the Unlicense ([`yt-dlp-LICENSE.txt`](yt-dlp-LICENSE.txt)).
- The `yt-dlp_macos` release file is a PyInstaller bundle of Python, yt-dlp and its dependencies. It includes GPL-licensed code such as mutagen (GPL-2.0-or-later), and per the yt-dlp README the bundle as a combined work is licensed under the GPLv3+. Its components and their licenses, including the full GPL text, are in [`yt-dlp-THIRD_PARTY_LICENSES.txt`](yt-dlp-THIRD_PARTY_LICENSES.txt) (from the same tag, trailing whitespace removed).
- Binary: <https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.26/yt-dlp_macos> (SHA-256 `0f922c15ccc0b83eefa1c8bc9104e434318c4bf16e408e1ee93b7745a6efe0aa`), unmodified.
- Source of this exact version: the tag <https://github.com/yt-dlp/yt-dlp/tree/2026.10.26> and the source archive `yt-dlp.tar.gz` on the release page <https://github.com/yt-dlp/yt-dlp/releases/tag/2026.10.26>. The build recipe of the macOS bundle is `bundle/pyinstaller.py` and `.github/workflows/build.yml` at that tag. Sources of the bundled third-party components are at the projects listed in [`yt-dlp-THIRD_PARTY_LICENSES.txt`](yt-dlp-THIRD_PARTY_LICENSES.txt).

## Repository-only material

Not shipped in the app, noticed here because it is copied into the repository:

- [`../DESIGN.md`](../DESIGN.md) is `design-md/apple/DESIGN.md` from <https://github.com/VoltAgent/awesome-design-md> (commit `fcc702849322`), MIT License, Copyright (c) 2026 VoltAgent ([`awesome-design-md-MIT.txt`](awesome-design-md-MIT.txt)).

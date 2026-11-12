#!/bin/sh
# Fetches the sidecars the stem splitter bundles (Tauri `bundle.externalBin`):
#   - yt-dlp: the official macOS universal build, pinned and checksum-verified.
#   - ffmpeg: built from the pinned, checksum-verified ffmpeg.org source as an
#     audio-only LGPL build (no GPL parts, no external libraries), linked
#     only against the system library.
# Results land in src-tauri/binaries/<name>-<target triple> and are cached in
# ~/Library/Caches/dev.mongmeo.woodshed/binaries, so later runs only copy.
#
# Usage: scripts/fetch-binaries.sh [--target aarch64-apple-darwin|x86_64-apple-darwin|universal-apple-darwin]
set -eu

YTDLP_VERSION=2026.10.26
YTDLP_SHA256=0f922c15ccc0b83eefa1c8bc9104e434318c4bf16e408e1ee93b7745a6efe0aa
FFMPEG_VERSION=9.0.2
FFMPEG_SHA256=8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e
MACOS_MIN=13.0
# Bump when the ffmpeg configuration below changes, so caches rebuild.
FFMPEG_BUILD_ID=1

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT="$ROOT/src-tauri/binaries"
CACHE="${WOODSHED_BINARY_CACHE:-$HOME/Library/Caches/dev.mongmeo.woodshed/binaries}"
JOBS="${CARGO_BUILD_JOBS:-3}"

case "$(uname -m)" in
  arm64) TARGET=aarch64-apple-darwin ;;
  x86_64) TARGET=x86_64-apple-darwin ;;
  *) echo "unsupported host: $(uname -m)" >&2; exit 1 ;;
esac
if [ "${1:-}" = "--target" ]; then
  TARGET="${2:?--target needs a value}"
fi
case "$TARGET" in
  aarch64-apple-darwin | x86_64-apple-darwin | universal-apple-darwin) ;;
  *) echo "unsupported target: $TARGET" >&2; exit 1 ;;
esac

mkdir -p "$OUT" "$CACHE"

sha256() {
  shasum -a 256 "$1" | cut -d ' ' -f 1
}

# download <url> <file> <sha256>: fetches into the cache unless a verified copy exists.
download() {
  if [ -f "$2" ] && [ "$(sha256 "$2")" = "$3" ]; then
    return
  fi
  echo "downloading $1"
  curl -fL --retry 3 -o "$2.part" "$1"
  actual=$(sha256 "$2.part")
  if [ "$actual" != "$3" ]; then
    rm -f "$2.part"
    echo "checksum mismatch for $1: expected $3, got $actual" >&2
    exit 1
  fi
  mv "$2.part" "$2"
}

# ---- yt-dlp (universal binary) ----------------------------------------------
YTDLP="$CACHE/yt-dlp_macos-$YTDLP_VERSION"
download "https://github.com/yt-dlp/yt-dlp/releases/download/$YTDLP_VERSION/yt-dlp_macos" \
  "$YTDLP" "$YTDLP_SHA256"
install -m 755 "$YTDLP" "$OUT/yt-dlp-$TARGET"

# ---- ffmpeg (audio-only LGPL build from source) -----------------------------
SOURCE="$CACHE/ffmpeg-$FFMPEG_VERSION.tar.xz"
download "https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.xz" "$SOURCE" "$FFMPEG_SHA256"

# build_ffmpeg <arch: arm64|x86_64> → prints the path of the built binary.
build_ffmpeg() {
  arch="$1"
  built="$CACHE/ffmpeg-$FFMPEG_VERSION-b$FFMPEG_BUILD_ID-$arch"
  if [ -x "$built" ]; then
    echo "$built"
    return
  fi
  work="$CACHE/build-$FFMPEG_VERSION-$arch"
  rm -rf "$work"
  mkdir -p "$work"
  tar -xJf "$SOURCE" -C "$work" --strip-components 1
  cross=""
  if [ "$arch" != "$(uname -m)" ]; then
    cross="--enable-cross-compile --arch=$arch --target-os=darwin --disable-x86asm"
  fi
  echo "building ffmpeg $FFMPEG_VERSION ($arch); this takes a few minutes" >&2
  (
    cd "$work"
    # shellcheck disable=SC2086 # $cross is a list of flags.
    ./configure \
      --cc="clang -arch $arch" \
      --extra-cflags="-mmacosx-version-min=$MACOS_MIN" \
      --extra-ldflags="-mmacosx-version-min=$MACOS_MIN" \
      $cross \
      --disable-gpl --disable-nonfree --disable-version3 \
      --disable-autodetect --disable-everything \
      --disable-doc --disable-debug --disable-network \
      --disable-ffplay --disable-ffprobe --disable-avdevice --disable-swscale \
      --enable-static --disable-shared \
      --enable-protocol=file,pipe \
      --enable-demuxer=mov,matroska,ogg,mp3,aac,wav,flac \
      --enable-decoder=aac,aac_latm,opus,vorbis,mp3,mp3float,flac,alac,pcm_s16le,pcm_s24le,pcm_f32le \
      --enable-parser=aac,aac_latm,opus,vorbis,mpegaudio,flac \
      --enable-encoder=pcm_s16le \
      --enable-muxer=wav \
      --enable-filter=aresample,aformat,anull \
      --enable-swresample >&2
    make -j"$JOBS" ffmpeg >&2
  )
  install -m 755 "$work/ffmpeg" "$built"
  rm -rf "$work"
  echo "$built"
}

case "$TARGET" in
  aarch64-apple-darwin) FFMPEG=$(build_ffmpeg arm64) ;;
  x86_64-apple-darwin) FFMPEG=$(build_ffmpeg x86_64) ;;
  universal-apple-darwin)
    arm=$(build_ffmpeg arm64)
    intel=$(build_ffmpeg x86_64)
    FFMPEG="$CACHE/ffmpeg-$FFMPEG_VERSION-b$FFMPEG_BUILD_ID-universal"
    lipo -create "$arm" "$intel" -output "$FFMPEG"
    ;;
esac
install -m 755 "$FFMPEG" "$OUT/ffmpeg-$TARGET"

echo "sidecars ready in $OUT:"
ls -l "$OUT"

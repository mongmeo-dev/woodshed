#!/bin/sh
# Builds the stem separation model the app bundles (Tauri `bundle.resources`):
#   htdemucs_6s (Demucs v4, 6 sources) → core graph without STFT/iSTFT
#   (core.py) → Core ML mlprogram, fp32 (convert.py) → compiled .mlmodelc.
# The weights come from the pinned Hugging Face revision and are checked
# against a pinned SHA-256. Python runs in a venv built by a pinned uv from
# requirements.lock.txt. Nothing here is committed: the result lands in
# src-tauri/models/htdemucs_6s_fp32.mlmodelc and is cached (with the weights)
# in ~/Library/Caches/dev.mongmeo.woodshed/models, so later runs only copy.
# The app also finds the cached copy in dev builds (`pnpm tauri dev`).
#
# Usage: tools/stem-model/build.sh
set -eu

HF_REPO=adefossez/HTDemucs-6s
HF_REVISION=3c5ee475be622df764938de97e4281a7b07ffa58
WEIGHTS_FILE=5c90dfd2.safetensors
WEIGHTS_SHA256=d2a1745f0744721f6b8ca5bf469b67c651ea5ed1b52998cab033b2158609d411
UV_VERSION=0.10.7
PYTHON_VERSION=3.12.8
MODEL=htdemucs_6s_fp32

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TOOLS="$ROOT/tools/stem-model"
OUT="$ROOT/src-tauri/models"
CACHE="${WOODSHED_MODEL_CACHE:-$HOME/Library/Caches/dev.mongmeo.woodshed/models}"

if [ "$(uname -s)" != Darwin ]; then
  echo "the stem model needs macOS (Core ML)" >&2
  exit 1
fi
mkdir -p "$OUT" "$CACHE"

sha256() {
  shasum -a 256 "$1" | cut -d ' ' -f 1
}

# A complete compiled model has its Core ML metadata (the build placeholder does not).
valid_model() {
  [ -f "$1/coremldata.bin" ] && [ -f "$1/model.mil" ]
}

# Everything that changes the converted model is in the cache key.
KEY=$(cat "$TOOLS/core.py" "$TOOLS/convert.py" "$TOOLS/requirements.lock.txt" | shasum -a 256 | cut -c 1-16)
KEY="$KEY-$(printf '%s' "$WEIGHTS_SHA256" | cut -c 1-16)-py$PYTHON_VERSION"
CACHED="$CACHE/$MODEL.mlmodelc"
STAMP="$CACHE/$MODEL.key"

if ! valid_model "$CACHED" || [ "$(cat "$STAMP" 2>/dev/null)" != "$KEY" ]; then
  # ---- weights (pinned revision, checksum-verified) -------------------------
  WEIGHTS="$CACHE/weights/$WEIGHTS_FILE"
  if [ ! -f "$WEIGHTS" ] || [ "$(sha256 "$WEIGHTS")" != "$WEIGHTS_SHA256" ]; then
    mkdir -p "$CACHE/weights"
    url="https://huggingface.co/$HF_REPO/resolve/$HF_REVISION/$WEIGHTS_FILE"
    echo "downloading $url"
    curl -fL --retry 3 -o "$WEIGHTS.part" "$url"
    actual=$(sha256 "$WEIGHTS.part")
    if [ "$actual" != "$WEIGHTS_SHA256" ]; then
      rm -f "$WEIGHTS.part"
      echo "checksum mismatch for $url: expected $WEIGHTS_SHA256, got $actual" >&2
      exit 1
    fi
    mv "$WEIGHTS.part" "$WEIGHTS"
  fi

  # ---- Python (pinned uv, Python and lock file) -----------------------------
  LOCK_KEY=$(sha256 "$TOOLS/requirements.lock.txt" | cut -c 1-12)
  VENV="$CACHE/venv-py$PYTHON_VERSION-$LOCK_KEY"
  if [ ! -x "$VENV/bin/python" ]; then
    rm -rf "$VENV"
    mise exec "uv@$UV_VERSION" -- uv venv --python "$PYTHON_VERSION" "$VENV"
    mise exec "uv@$UV_VERSION" -- uv pip sync --python "$VENV/bin/python" "$TOOLS/requirements.lock.txt"
  fi

  # ---- convert and compile ---------------------------------------------------
  WORK="$CACHE/build-$$"
  rm -rf "$WORK"
  mkdir -p "$WORK"
  trap 'rm -rf "$WORK"' EXIT INT TERM
  echo "converting $WEIGHTS_FILE to Core ML (fp32)"
  (cd "$WORK" && OMP_NUM_THREADS=4 PYTHONDONTWRITEBYTECODE=1 "$VENV/bin/python" "$TOOLS/convert.py" "$WEIGHTS" "$WORK/$MODEL.mlpackage")
  xcrun coremlcompiler compile "$WORK/$MODEL.mlpackage" "$WORK"
  valid_model "$WORK/$MODEL.mlmodelc" || { echo "coremlcompiler produced no model" >&2; exit 1; }
  rm -rf "$CACHED" "$STAMP"
  mv "$WORK/$MODEL.mlmodelc" "$CACHED"
  printf '%s\n' "$KEY" > "$STAMP"
  rm -rf "$WORK"
  trap - EXIT INT TERM
  # The venv is ~570MB; uv's own package cache rebuilds it in seconds.
  rm -rf "$VENV"
fi

# ---- into the bundle resources -----------------------------------------------
rm -rf "$OUT/$MODEL.mlmodelc"
cp -R "$CACHED" "$OUT/$MODEL.mlmodelc"
echo "stem model ready: $OUT/$MODEL.mlmodelc ($(du -sh "$OUT/$MODEL.mlmodelc" | cut -f 1))"

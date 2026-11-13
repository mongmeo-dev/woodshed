#!/bin/sh
# Builds Woodshed.app (src-tauri/target/release/bundle/macos/Woodshed.app).
#
# With the local signing identity (scripts/create-local-signing-cert.sh) the
# app is signed with it, so every rebuild keeps one code identity and macOS
# keeps the microphone permission. Without it the build is ad-hoc signed as
# before. Extra arguments go to `tauri build`.
set -eu
cd "$(dirname "$0")/.."

. scripts/local-signing.sh

if woodshed_signing_ready; then
  woodshed_signing_unlock
  woodshed_signing_add_to_search_list
  # Before signing, the bundler clears extended attributes with `xattr -cr`;
  # a Python `xattr` earlier on PATH (pip, mise) has no -r, so put Apple's first.
  tools=$(mktemp -d)
  trap 'rm -rf "$tools"' EXIT
  ln -s /usr/bin/xattr "$tools/xattr"
  echo "signing with \"$WOODSHED_SIGNING_NAME\""
  PATH="$tools:$PATH" pnpm tauri build --config src-tauri/tauri.local-signing.conf.json "$@"
  exit
fi
echo "no local signing identity: ad-hoc signed build (see scripts/create-local-signing-cert.sh)"
exec pnpm tauri build "$@"

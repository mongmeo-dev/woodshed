#!/bin/sh
# Builds a distributable Woodshed.app: signed with the Developer ID identity
# from the release keychain (scripts/create-release-keychain.sh), hardened
# runtime, notarized with the App Store Connect API key in notary.env and
# stapled. Extra arguments go to `tauri build`, except
# those that change its config (scripts/release-guard.sh).
set -eu
cd "$(dirname "$0")/.."
. scripts/release-guard.sh
woodshed_release_guard "$@"
. scripts/release-signing.sh
identity=$(woodshed_release_identity)
if [ -z "$identity" ] || [ ! -f "$WOODSHED_RELEASE_PASSWORD_FILE" ]; then
  echo "error: no release keychain; run scripts/create-release-keychain.sh (README)" >&2
  exit 1
fi
if [ ! -f "$WOODSHED_RELEASE_NOTARY_ENV" ]; then
  echo "error: $WOODSHED_RELEASE_NOTARY_ENV is missing (README)" >&2
  exit 1
fi
woodshed_release_unlock
woodshed_release_add_to_search_list
set -a
. "$WOODSHED_RELEASE_NOTARY_ENV"
set +a
export APPLE_SIGNING_IDENTITY="$identity"
# Before signing, the bundler clears extended attributes with `xattr -cr`;
# a Python `xattr` earlier on PATH (pip, mise) has no -r, so put Apple's first.
tools=$(mktemp -d)
trap 'rm -rf "$tools"' EXIT
ln -s /usr/bin/xattr "$tools/xattr"
echo "signing with Developer ID $identity, then notarizing"
PATH="$tools:$PATH" pnpm tauri build --config src-tauri/tauri.release.conf.json "$@"
app=src-tauri/target/release/bundle/macos/Woodshed.app
# Never ship the update test frontend (installs updates without a click).
if ! node scripts/check-release-dist.mjs; then
  rm -rf "$app"
  echo "error: refused an update test frontend; removed the app" >&2
  exit 1
fi
codesign --verify --deep --strict "$app"
xcrun stapler validate "$app"
spctl --assess --type execute "$app"
echo "release ready: $app"

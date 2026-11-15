#!/bin/sh
# Creates the updater signing key (`tauri signer generate`) once:
#   ~/Library/Application Support/woodshed-release/updater.key      (private key)
#   ~/Library/Application Support/woodshed-release/updater.key.pub  (public key)
#   ~/Library/Application Support/woodshed-release/updater-key-password
# all mode 600. An existing key is never replaced: installed apps only accept
# updates signed with the key whose public half is in tauri.conf.json
# (plugins.updater.pubkey), so losing or replacing it strands them (README).
# Prints the public key to put in tauri.conf.json.
set -eu
cd "$(dirname "$0")/.."
. scripts/release-signing.sh
if [ -e "$WOODSHED_UPDATER_KEY" ]; then
  echo "exists, kept: $WOODSHED_UPDATER_KEY" >&2
  cat "$WOODSHED_UPDATER_KEY.pub"
  echo
  exit 0
fi
umask 077
mkdir -p "$WOODSHED_RELEASE_DIR"
if [ ! -s "$WOODSHED_UPDATER_KEY_PASSWORD_FILE" ]; then
  /usr/bin/openssl rand -hex 24 >"$WOODSHED_UPDATER_KEY_PASSWORD_FILE"
fi
pnpm --silent tauri signer generate --ci -w "$WOODSHED_UPDATER_KEY" \
  -p "$(cat "$WOODSHED_UPDATER_KEY_PASSWORD_FILE")" >/dev/null
chmod 600 "$WOODSHED_UPDATER_KEY" "$WOODSHED_UPDATER_KEY.pub" "$WOODSHED_UPDATER_KEY_PASSWORD_FILE"
echo "created: $WOODSHED_UPDATER_KEY (back it up, README)" >&2
cat "$WOODSHED_UPDATER_KEY.pub"
echo

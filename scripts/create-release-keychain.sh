#!/bin/sh
# Puts a Developer ID Application identity (a .p12 exported from the Apple
# developer account) into a keychain of its own for release builds
# (`mise run app:release`), and adds that keychain to the search list.
#
#   sh scripts/create-release-keychain.sh <identity.p12> <file with the p12 password>
#   sh scripts/create-release-keychain.sh --remove
set -eu
. "$(dirname "$0")/release-signing.sh"
if [ "${1:-}" = "--remove" ]; then
  [ -e "$WOODSHED_RELEASE_KEYCHAIN" ] && security delete-keychain "$WOODSHED_RELEASE_KEYCHAIN"
  rm -f "$WOODSHED_RELEASE_PASSWORD_FILE"
  echo "removed: $WOODSHED_RELEASE_KEYCHAIN (notary.env kept)"
  exit 0
fi
if [ $# -ne 2 ]; then
  echo "usage: $0 <identity.p12> <p12 password file> | --remove" >&2
  exit 2
fi
p12=$1
p12_password=$(cat "$2")
if [ -e "$WOODSHED_RELEASE_KEYCHAIN" ]; then
  echo "error: $WOODSHED_RELEASE_KEYCHAIN exists; run with --remove first" >&2
  exit 1
fi
umask 077
mkdir -p "$WOODSHED_RELEASE_DIR"
/usr/bin/openssl rand -hex 24 >"$WOODSHED_RELEASE_PASSWORD_FILE"
password=$(cat "$WOODSHED_RELEASE_PASSWORD_FILE")
security create-keychain -p "$password" "$WOODSHED_RELEASE_KEYCHAIN"
# No auto-lock; builds unlock it after a restart (build-release.sh).
security set-keychain-settings "$WOODSHED_RELEASE_KEYCHAIN"
security unlock-keychain -p "$password" "$WOODSHED_RELEASE_KEYCHAIN"
security import "$p12" -k "$WOODSHED_RELEASE_KEYCHAIN" -P "$p12_password" -T /usr/bin/codesign >/dev/null
# Lets codesign use the key without a keychain prompt.
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" \
  "$WOODSHED_RELEASE_KEYCHAIN" >/dev/null
woodshed_release_add_to_search_list
identity=$(woodshed_release_identity)
if [ -z "$identity" ]; then
  echo "error: no Developer ID Application identity in $p12" >&2
  exit 1
fi
# Sign a scratch file to prove the identity works without prompts.
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp /usr/bin/true "$work/probe"
codesign --force --timestamp --options runtime --keychain "$WOODSHED_RELEASE_KEYCHAIN" \
  -s "$identity" "$work/probe"
echo "ready: Developer ID identity $identity in $WOODSHED_RELEASE_KEYCHAIN"

#!/bin/sh
# Creates the local code-signing identity "Woodshed Local Signing" in a keychain
# of its own (~/Library/Keychains/woodshed-local-signing.keychain-db) and adds
# that keychain to the user's keychain search list.
#
# Builds signed with it (`mise run app`) keep the same code identity across
# rebuilds, so macOS privacy permissions (the tuner's microphone access) carry
# over to the next build instead of being asked again. The certificate is
# self-signed and trusted by nothing; distribution builds use a Developer ID.
#
#   sh scripts/create-local-signing-cert.sh           # create (nothing if it exists)
#   sh scripts/create-local-signing-cert.sh --remove  # delete the keychain and password
set -eu

. "$(dirname "$0")/local-signing.sh"

if [ "${1:-}" = "--remove" ]; then
  if [ -e "$WOODSHED_SIGNING_KEYCHAIN" ]; then
    security delete-keychain "$WOODSHED_SIGNING_KEYCHAIN"
  fi
  rm -f "$WOODSHED_SIGNING_PASSWORD_FILE"
  rmdir "$(dirname "$WOODSHED_SIGNING_PASSWORD_FILE")" 2>/dev/null || true
  echo "removed: $WOODSHED_SIGNING_NAME"
  exit 0
fi

if woodshed_signing_ready; then
  woodshed_signing_add_to_search_list
  echo "already there: $WOODSHED_SIGNING_NAME ($WOODSHED_SIGNING_KEYCHAIN)"
  exit 0
fi
if [ -e "$WOODSHED_SIGNING_KEYCHAIN" ]; then
  echo "error: $WOODSHED_SIGNING_KEYCHAIN exists without the identity or its password;" >&2
  echo "       run with --remove first" >&2
  exit 1
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
umask 077

# Self-signed code-signing certificate. /usr/bin/openssl (LibreSSL) writes a
# PKCS#12 that `security import` reads.
cat >"$work/openssl.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $WOODSHED_SIGNING_NAME
[ext]
basicConstraints = critical, CA:FALSE
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
EOF
/usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -config "$work/openssl.cnf" \
  -keyout "$work/key.pem" -out "$work/cert.pem" 2>/dev/null
p12_password=$(/usr/bin/openssl rand -hex 16)
/usr/bin/openssl pkcs12 -export -name "$WOODSHED_SIGNING_NAME" -inkey "$work/key.pem" \
  -in "$work/cert.pem" -out "$work/identity.p12" -passout "pass:$p12_password"

mkdir -p "$(dirname "$WOODSHED_SIGNING_PASSWORD_FILE")"
/usr/bin/openssl rand -hex 24 >"$WOODSHED_SIGNING_PASSWORD_FILE"
password=$(cat "$WOODSHED_SIGNING_PASSWORD_FILE")

security create-keychain -p "$password" "$WOODSHED_SIGNING_KEYCHAIN"
# No auto-lock; builds unlock it after a restart (build-app.sh).
security set-keychain-settings "$WOODSHED_SIGNING_KEYCHAIN"
security unlock-keychain -p "$password" "$WOODSHED_SIGNING_KEYCHAIN"
security import "$work/identity.p12" -k "$WOODSHED_SIGNING_KEYCHAIN" -P "$p12_password" \
  -T /usr/bin/codesign >/dev/null
# Lets codesign use the key without a keychain prompt.
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" \
  "$WOODSHED_SIGNING_KEYCHAIN" >/dev/null
woodshed_signing_add_to_search_list

# Sign a scratch file to prove the identity works without prompts.
printf 'probe\n' >"$work/probe"
codesign --force --sign "$WOODSHED_SIGNING_NAME" --keychain "$WOODSHED_SIGNING_KEYCHAIN" "$work/probe"
codesign --display --verbose=2 "$work/probe" 2>&1 | grep '^Authority='
echo "created: $WOODSHED_SIGNING_NAME ($WOODSHED_SIGNING_KEYCHAIN)"

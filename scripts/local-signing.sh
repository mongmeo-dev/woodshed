# Shared by create-local-signing-cert.sh and build-app.sh (sourced, not run).
#
# The local signing identity lives in a keychain of its own, unlocked with a
# random password kept next to it, so builds never ask for the login keychain.

WOODSHED_SIGNING_NAME="Woodshed Local Signing"
WOODSHED_SIGNING_KEYCHAIN="$HOME/Library/Keychains/woodshed-local-signing.keychain-db"
WOODSHED_SIGNING_PASSWORD_FILE="$HOME/Library/Application Support/woodshed-local-signing/keychain-password"

# True when the identity and its keychain password are both present.
woodshed_signing_ready() {
  [ -f "$WOODSHED_SIGNING_PASSWORD_FILE" ] &&
    security find-certificate -c "$WOODSHED_SIGNING_NAME" "$WOODSHED_SIGNING_KEYCHAIN" >/dev/null 2>&1
}

# Unlocks the keychain (it locks again after a restart).
woodshed_signing_unlock() {
  security unlock-keychain -p "$(cat "$WOODSHED_SIGNING_PASSWORD_FILE")" "$WOODSHED_SIGNING_KEYCHAIN"
}

# Adds the keychain to the user's search list (codesign looks there), keeping
# the keychains already listed.
woodshed_signing_add_to_search_list() {
  listed=$(security list-keychains -d user | sed -e 's/^[[:space:]]*"//' -e 's/"$//')
  case "$listed" in
  *"$WOODSHED_SIGNING_KEYCHAIN"*) return 0 ;;
  esac
  set --
  while IFS= read -r keychain; do
    [ -n "$keychain" ] && set -- "$@" "$keychain"
  done <<EOF
$listed
EOF
  security list-keychains -d user -s "$@" "$WOODSHED_SIGNING_KEYCHAIN"
}

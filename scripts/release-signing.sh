# Shared by create-release-keychain.sh and build-release.sh (sourced, not run).
#
# The Developer ID identity lives in a keychain of its own, unlocked with a
# random password kept next to it, so release builds never ask for the login
# keychain (headless sessions cannot answer its prompt). Notarization
# credentials (an App Store Connect API key) are read from notary.env there.

WOODSHED_RELEASE_KEYCHAIN="$HOME/Library/Keychains/woodshed-release.keychain-db"
WOODSHED_RELEASE_DIR="$HOME/Library/Application Support/woodshed-release"
WOODSHED_RELEASE_PASSWORD_FILE="$WOODSHED_RELEASE_DIR/keychain-password"
WOODSHED_RELEASE_NOTARY_ENV="$WOODSHED_RELEASE_DIR/notary.env"

# SHA-1 of the Developer ID Application identity in the release keychain, or nothing.
woodshed_release_identity() {
  security find-identity -v -p codesigning "$WOODSHED_RELEASE_KEYCHAIN" 2>/dev/null |
    awk '/"Developer ID Application: / { print $2; exit }'
}

woodshed_release_unlock() {
  security unlock-keychain -p "$(cat "$WOODSHED_RELEASE_PASSWORD_FILE")" "$WOODSHED_RELEASE_KEYCHAIN"
}

# Adds the keychain to the user's search list (codesign looks there), keeping
# the keychains already listed.
woodshed_release_add_to_search_list() {
  listed=$(security list-keychains -d user | sed -e 's/^[[:space:]]*"//' -e 's/"$//')
  case "$listed" in
  *"$WOODSHED_RELEASE_KEYCHAIN"*) return 0 ;;
  esac
  set --
  while IFS= read -r keychain; do
    [ -n "$keychain" ] && set -- "$@" "$keychain"
  done <<LIST
$listed
LIST
  security list-keychains -d user -s "$@" "$WOODSHED_RELEASE_KEYCHAIN"
}

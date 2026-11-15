#!/bin/sh
# Test-only Woodshed.app for checking updates against a local server (README,
# "로컬 서버로 업데이트 확인"). Never a release: it differs from `mise run app` in
#   - the updater endpoint http://127.0.0.1:<port>/latest.json (plain http allowed),
#   - the frontend built in Vite mode "update-test", which installs a found
#     update without a click (src/lib/updater.ts AUTO_INSTALL),
#   - a window that does not take focus when it opens,
#   - the version: <version> instead of tauri.conf.json's.
# --updater-artifacts also makes the signed update archive (Woodshed.app.tar.gz
# and .sig) with the updater key (scripts/create-updater-key.sh).
#
#   sh scripts/build-update-test.sh <version> <port> <out dir> [--updater-artifacts]
#
# The app (and archive) are moved to <out dir>; target/ and dist/ keep no test
# build, so a later build cannot pick it up.
set -eu
cd "$(dirname "$0")/.."
if [ $# -lt 3 ]; then
  echo "usage: $0 <version> <port> <out dir> [--updater-artifacts]" >&2
  exit 2
fi
version=$1
port=$2
out=$3
artifacts=false
if [ "${4:-}" = "--updater-artifacts" ]; then
  artifacts=true
fi
. scripts/local-signing.sh
. scripts/release-signing.sh

work=$(mktemp -d)
trap 'rm -rf "$work"; rm -rf dist' EXIT
node --input-type=module - "$work/update-test.conf.json" "$version" "$port" "$artifacts" <<'EOF'
import fs from "node:fs";
const [file, version, port, artifacts] = process.argv.slice(2);
const base = JSON.parse(fs.readFileSync("src-tauri/tauri.conf.json", "utf8"));
const windows = base.app.windows.map((window) => ({ ...window, focus: false }));
fs.writeFileSync(
  file,
  JSON.stringify({
    version,
    build: {
      beforeBuildCommand:
        "pnpm binaries && pnpm stem-model && pnpm exec tsc --noEmit && pnpm exec vite build --mode update-test",
    },
    app: { windows },
    bundle: { createUpdaterArtifacts: artifacts === "true" },
    plugins: {
      updater: {
        endpoints: [`http://127.0.0.1:${port}/latest.json`],
        dangerousInsecureTransportProtocol: true,
      },
    },
  }),
);
EOF

if [ "$artifacts" = true ]; then
  TAURI_SIGNING_PRIVATE_KEY=$(cat "$WOODSHED_UPDATER_KEY")
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD=$(cat "$WOODSHED_UPDATER_KEY_PASSWORD_FILE")
  export TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD
fi
set -- --config "$work/update-test.conf.json"
if woodshed_signing_ready; then
  woodshed_signing_unlock
  woodshed_signing_add_to_search_list
  set -- --config src-tauri/tauri.local-signing.conf.json "$@"
fi
# Before signing, the bundler clears extended attributes with `xattr -cr`;
# a Python `xattr` earlier on PATH (pip, mise) has no -r, so put Apple's first.
ln -s /usr/bin/xattr "$work/xattr"
PATH="$work:$PATH" pnpm tauri build "$@"

bundle=src-tauri/target/release/bundle/macos
mkdir -p "$out"
rm -rf "$out/Woodshed.app" "$out/Woodshed.app.tar.gz" "$out/Woodshed.app.tar.gz.sig"
mv "$bundle/Woodshed.app" "$out/"
if [ "$artifacts" = true ]; then
  mv "$bundle/Woodshed.app.tar.gz" "$bundle/Woodshed.app.tar.gz.sig" "$out/"
fi
echo "update test build $version (endpoint http://127.0.0.1:$port/latest.json): $out"

#!/bin/sh
# Releases Woodshed to GitHub by hand (there is no CI release). README, "릴리스".
#
#   sh scripts/release.sh <X.Y.Z> [--notes-file <file>]                 (mise run release -- X.Y.Z)
#   sh scripts/release.sh <X.Y.Z> --dry-run [--allow-branch] [--notes-file <file>]
#   sh scripts/release.sh <X.Y.Z> --upload-only [--notes-file <file>]
#
# 1. Checks: on main, clean, equal to origin/main; no tag vX.Y.Z here or on
#    origin and no release for it; X.Y.Z above the current version and every
#    tag; gh signed in; release keychain, notary.env and updater key present.
# 2. Writes X.Y.Z into package.json, src-tauri/Cargo.toml, Cargo.lock and
#    tauri.conf.json, commits "버전 X.Y.Z" and makes the annotated tag vX.Y.Z.
# 3. Builds with scripts/build-release.sh (Developer ID, notarized, stapled,
#    signed update archive) and gathers in releases/vX.Y.Z/ (git-ignored):
#      Woodshed_X.Y.Z_aarch64.dmg             signed, notarized, stapled
#      Woodshed_X.Y.Z_aarch64.app.tar.gz(.sig) update archive and signature
#      latest.json                            what installed apps read
#      ffmpeg-<v>.tar.xz, yt-dlp-<v>.tar.gz   sources of the bundled sidecars
#      SHA256SUMS
# 4. Verifies: the latest.json signature against the updater public key in
#    tauri.conf.json, Gatekeeper (spctl) on the DMG and on the app in the DMG
#    and in the update archive, stapled tickets, and the version of each app.
# 5. Pushes the commit and tag together, then makes the GitHub release: a
#    draft, all assets, then published as latest. If the upload fails, rerun
#    with --upload-only: it verifies releases/vX.Y.Z/ again and uploads it.
#
# --dry-run makes and verifies the same files but pushes nothing and makes no
# release; at the end (and on any failure before the push) the version commit
# and tag are removed, leaving the branch and working tree as they were.
# --allow-branch (dry run only) skips "on main and equal to origin/main".
set -eu

usage() {
  echo "usage: $0 <X.Y.Z> [--dry-run [--allow-branch] | --upload-only] [--notes-file <file>]" >&2
  exit 2
}

version=""
dry_run=false
allow_branch=false
upload_only=false
notes_file=""
while [ $# -gt 0 ]; do
  case "$1" in
  --dry-run) dry_run=true ;;
  --allow-branch) allow_branch=true ;;
  --upload-only) upload_only=true ;;
  --notes-file)
    [ $# -ge 2 ] || usage
    notes_file=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
    shift
    ;;
  -h | --help) usage ;;
  -*) echo "error: unknown option $1" >&2 && usage ;;
  *)
    [ -z "$version" ] || usage
    version=$1
    ;;
  esac
  shift
done
[ -n "$version" ] || usage
if [ "$allow_branch" = true ] && [ "$dry_run" = false ]; then
  echo "error: --allow-branch is only for --dry-run; releases are made from main" >&2
  exit 2
fi
if [ "$dry_run" = true ] && [ "$upload_only" = true ]; then
  echo "error: --dry-run and --upload-only do not go together" >&2
  exit 2
fi
if [ -n "$notes_file" ] && [ ! -s "$notes_file" ]; then
  echo "error: notes file $notes_file is empty or missing" >&2
  exit 2
fi

cd "$(dirname "$0")/.."
. scripts/release-guard.sh
woodshed_release_guard
. scripts/release-signing.sh

step() {
  printf '\n==> %s\n' "$*"
}

fail() {
  echo "error: $*" >&2
  exit 1
}

if [ "$upload_only" = true ]; then
  node scripts/release-files.mjs valid "$version"
else
  node scripts/release-files.mjs check-version "$version"
fi
[ "$(uname -m)" = arm64 ] || fail "releases are built on Apple Silicon (darwin-aarch64 only)"

repo=$(node -e '
  const conf = require("./src-tauri/tauri.conf.json");
  const url = conf.plugins.updater.endpoints[0];
  const m = /^https:\/\/github\.com\/([^/]+\/[^/]+)\/releases\/latest\/download\/latest\.json$/.exec(url);
  if (!m) { console.error(`updater endpoint is not a GitHub latest release: ${url}`); process.exit(1); }
  console.log(m[1]);
')
tag=v$version
out=releases/$tag
dmg_name=Woodshed_${version}_aarch64.dmg
archive_name=Woodshed_${version}_aarch64.app.tar.gz
download_base=https://github.com/$repo/releases/download/$tag
bundle=src-tauri/target/release/bundle/macos
binary_cache=${WOODSHED_BINARY_CACHE:-$HOME/Library/Caches/dev.mongmeo.woodshed/binaries}

pinned() {
  value=$(sed -n "s/^$1=//p" scripts/fetch-binaries.sh)
  [ -n "$value" ] || fail "scripts/fetch-binaries.sh has no $1"
  echo "$value"
}
ytdlp_version=$(pinned YTDLP_VERSION)
ytdlp_source_sha=$(pinned YTDLP_SOURCE_SHA256)
ffmpeg_version=$(pinned FFMPEG_VERSION)
ffmpeg_sha=$(pinned FFMPEG_SHA256)
ytdlp_source=yt-dlp-$ytdlp_version.tar.gz
ffmpeg_source=ffmpeg-$ffmpeg_version.tar.xz
assets="$dmg_name $archive_name $archive_name.sig latest.json SHA256SUMS $ffmpeg_source $ytdlp_source"

work=$(mktemp -d)
bumped=false
pushed=false
mounted=false
orig_head=""
release_commit=""

# Puts the branch back where it was: removes the version commit and tag (only
# while they are ours and not pushed). Runs at the end of a dry run and on
# any failure before the push.
restore_git() {
  [ "$bumped" = true ] && [ "$pushed" = false ] || return 0
  bumped=false
  if [ -n "$release_commit" ] &&
    [ "$(git rev-parse -q --verify "refs/tags/$tag^{commit}" || true)" = "$release_commit" ]; then
    git tag -d "$tag" >/dev/null
  fi
  head=$(git rev-parse HEAD)
  if [ "$head" = "$release_commit" ] || [ "$head" = "$orig_head" ]; then
    git reset -q --hard "$orig_head"
  else
    echo "warning: HEAD moved to $head during the release; left as is (was $orig_head)" >&2
    return 1
  fi
  if [ -n "$(git status --porcelain)" ]; then
    echo "warning: working tree is not clean after restoring $orig_head" >&2
    return 1
  fi
  echo "restored: $(git rev-parse --abbrev-ref HEAD) at $(git rev-parse --short HEAD), no tag $tag, clean working tree"
}

on_exit() {
  status=$?
  if [ "$mounted" = true ]; then
    hdiutil detach -quiet -force "$work/mnt" || true
  fi
  rm -rf "$work"
  if [ "$bumped" = true ] && [ "$pushed" = false ]; then
    echo "release stopped before anything was pushed; removing the version commit and tag" >&2
    restore_git || true
  fi
  exit "$status"
}
trap on_exit EXIT
trap 'exit 130' INT TERM HUP

# ---- checks -----------------------------------------------------------------

check_gh() {
  command -v gh >/dev/null || fail "gh is not installed"
  gh auth status --hostname github.com >/dev/null 2>&1 || fail "gh is not signed in (gh auth login)"
  gh repo view "$repo" --json name >/dev/null || fail "gh cannot open $repo"
}

check_keys() {
  identity=$(woodshed_release_identity)
  { [ -n "$identity" ] && [ -f "$WOODSHED_RELEASE_PASSWORD_FILE" ]; } ||
    fail "no release keychain; run scripts/create-release-keychain.sh (README)"
  [ -f "$WOODSHED_RELEASE_NOTARY_ENV" ] || fail "$WOODSHED_RELEASE_NOTARY_ENV is missing (README)"
  (
    set -a
    . "$WOODSHED_RELEASE_NOTARY_ENV"
    [ -n "${APPLE_API_KEY:-}" ] && [ -n "${APPLE_API_ISSUER:-}" ] && [ -f "${APPLE_API_KEY_PATH:-}" ]
  ) || fail "notary.env needs APPLE_API_KEY, APPLE_API_ISSUER and an existing APPLE_API_KEY_PATH"
  { [ -s "$WOODSHED_UPDATER_KEY" ] && [ -s "$WOODSHED_UPDATER_KEY_PASSWORD_FILE" ]; } ||
    fail "no updater key; run scripts/create-updater-key.sh (README)"
  node -e '
    const fs = require("node:fs");
    const conf = JSON.parse(fs.readFileSync("src-tauri/tauri.conf.json", "utf8"));
    if (fs.readFileSync(process.argv[1], "utf8").trim() !== conf.plugins.updater.pubkey) process.exit(1);
  ' "$WOODSHED_UPDATER_KEY.pub" ||
    fail "$WOODSHED_UPDATER_KEY.pub is not plugins.updater.pubkey in tauri.conf.json"
}

# exits 0 when origin has the tag, 1 when it does not; fails on a network error.
remote_has_tag() {
  code=0
  git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null || code=$?
  [ "$code" = 0 ] && return 0
  [ "$code" = 2 ] || fail "cannot list the tags of origin (git ls-remote exit $code)"
  return 1
}

check_git() {
  [ -z "$(git status --porcelain)" ] || fail "working tree is not clean (git status)"
  git fetch -q origin main
  if [ "$allow_branch" = false ]; then
    branch=$(git symbolic-ref -q --short HEAD || true)
    [ "$branch" = main ] || fail "releases are made from main (on ${branch:-a detached HEAD})"
    [ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] ||
      fail "main is not origin/main (pull or push first)"
  else
    echo "dry run on $(git rev-parse --abbrev-ref HEAD) (--allow-branch): branch checks skipped"
  fi
  ! git rev-parse -q --verify "refs/tags/$tag" >/dev/null || fail "tag $tag already exists here"
  ! remote_has_tag || fail "tag $tag already exists on origin"
  ! gh release view "$tag" -R "$repo" >/dev/null 2>&1 || fail "release $tag already exists on $repo"
  tags=$({
    git tag -l 'v*'
    git ls-remote --tags --refs origin 'v*' | sed 's|.*refs/tags/||'
  } | sort -u)
  # shellcheck disable=SC2086
  node scripts/release-files.mjs check-version "$version" $tags
  echo "version $(node scripts/release-files.mjs current) → $version, tag $tag, repo $repo"
}

# ---- sources of the bundled sidecars -------------------------------------------

# fetch_source <url> <file in $out> <sha256> [cached copy]
fetch_source() {
  target=$out/$2
  if [ -n "${4:-}" ] && [ -f "$4" ] && [ "$(shasum -a 256 "$4" | cut -d ' ' -f 1)" = "$3" ]; then
    cp "$4" "$target"
  else
    curl -fsSL --retry 3 -o "$target" "$1"
  fi
  actual=$(shasum -a 256 "$target" | cut -d ' ' -f 1)
  [ "$actual" = "$3" ] || fail "$2: sha256 $actual, scripts/fetch-binaries.sh pins $3"
  echo "$2: sha256 matches scripts/fetch-binaries.sh"
}

fetch_sources() {
  fetch_source "https://ffmpeg.org/releases/$ffmpeg_source" "$ffmpeg_source" "$ffmpeg_sha" \
    "$binary_cache/$ffmpeg_source"
  fetch_source "https://github.com/yt-dlp/yt-dlp/releases/download/$ytdlp_version/yt-dlp.tar.gz" \
    "$ytdlp_source" "$ytdlp_source_sha"
}

# ---- version commit and tag ----------------------------------------------------

bump_version() {
  orig_head=$(git rev-parse HEAD)
  bumped=true
  node scripts/release-files.mjs bump "$version"
  (cd src-tauri && cargo metadata --locked --format-version 1 >/dev/null) ||
    fail "Cargo.lock does not match after the version change"
  git add package.json src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/tauri.conf.json
  git commit -q -m "버전 $version"
  release_commit=$(git rev-parse HEAD)
  git tag -a "$tag" -m "Woodshed $version"
  echo "committed $(git rev-parse --short HEAD) \"버전 $version\", tag $tag"
}

# ---- build and files -------------------------------------------------------------

notarize() {
  xcrun notarytool submit "$1" --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" \
    --issuer "$APPLE_API_ISSUER" --wait --timeout 1h --output-format json >"$work/notary.json" ||
    true
  result=$(node -e '
    const r = JSON.parse(require("node:fs").readFileSync(process.argv[1], "utf8") || "{}");
    console.log(`${r.status ?? "unknown"} ${r.id ?? ""}`);
  ' "$work/notary.json" 2>/dev/null || echo "unknown")
  echo "notarization of $(basename "$1"): $result"
  case "$result" in
  "Accepted "*) ;;
  *)
    id=${result#* }
    if [ -n "$id" ] && [ "$id" != "$result" ]; then
      xcrun notarytool log "$id" --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" \
        --issuer "$APPLE_API_ISSUER" >&2 || true
    fi
    fail "notarization of $(basename "$1") was not accepted"
    ;;
  esac
  xcrun stapler staple -q "$1"
}

make_dmg() {
  stage=$work/dmg
  mkdir -p "$stage"
  ditto "$bundle/Woodshed.app" "$stage/Woodshed.app"
  ln -s /Applications "$stage/Applications"
  hdiutil create -quiet -volname Woodshed -srcfolder "$stage" -format UDZO -ov "$out/$dmg_name"
  rm -rf "$stage"
  woodshed_release_unlock
  codesign --sign "$identity" --keychain "$WOODSHED_RELEASE_KEYCHAIN" --timestamp "$out/$dmg_name"
  set -a
  . "$WOODSHED_RELEASE_NOTARY_ENV"
  set +a
  notarize "$out/$dmg_name"
}

build_files() {
  step "build (Developer ID, notarization, update archive)"
  sh scripts/build-release.sh
  [ -z "$(git status --porcelain)" ] || {
    git status --short >&2
    fail "the build changed tracked files; the tag would not match the build"
  }
  cp "$bundle/Woodshed.app.tar.gz" "$out/$archive_name"
  cp "$bundle/Woodshed.app.tar.gz.sig" "$out/$archive_name.sig"
  step "DMG (signed, notarized, stapled)"
  make_dmg
  step "latest.json"
  notes=$out/release-notes.md
  if [ -n "$notes_file" ]; then
    cp "$notes_file" "$notes"
  else
    echo "Woodshed $version" >"$notes"
  fi
  node scripts/release-files.mjs latest-json "$out/latest.json" "$version" "$notes" \
    "$out/$archive_name.sig" "$download_base/$archive_name"
  (cd "$out" && shasum -a 256 "$dmg_name" "$archive_name" "$archive_name.sig" latest.json \
    "$ffmpeg_source" "$ytdlp_source" >SHA256SUMS)
}

# ---- verification --------------------------------------------------------------

# check_app <Woodshed.app> <where>: signature, notarization ticket, Gatekeeper, version.
check_app() {
  codesign --verify --deep --strict "$1" || fail "$2: codesign verification failed"
  xcrun stapler validate -q "$1" || fail "$2: app is not stapled"
  spctl --assess --type execute "$1" || fail "$2: Gatekeeper refused the app"
  app_version=$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$1/Contents/Info.plist")
  [ "$app_version" = "$version" ] || fail "$2: app version $app_version, expected $version"
  echo "$2: Woodshed.app $app_version signed, stapled, accepted by Gatekeeper"
}

verify_files() {
  step "verify $out"
  for asset in $assets; do
    [ -s "$out/$asset" ] || fail "missing $out/$asset"
  done
  (cd "$out" && shasum -a 256 -c -s SHA256SUMS) || fail "SHA256SUMS does not match"
  node scripts/release-files.mjs verify-latest "$out/latest.json" "$out/$archive_name" "$version" \
    "$download_base/$archive_name"
  codesign --verify --strict "$out/$dmg_name" || fail "DMG signature does not verify"
  xcrun stapler validate -q "$out/$dmg_name" || fail "DMG is not stapled"
  spctl --assess --type open --context context:primary-signature -v "$out/$dmg_name" ||
    fail "Gatekeeper refused the DMG"
  mnt=$work/mnt
  mkdir -p "$mnt"
  hdiutil attach -quiet -nobrowse -readonly -noautoopen -mountpoint "$mnt" "$out/$dmg_name"
  mounted=true
  check_app "$mnt/Woodshed.app" "DMG"
  hdiutil detach -quiet "$mnt"
  mounted=false
  extracted=$work/archive
  mkdir -p "$extracted"
  tar -xzf "$out/$archive_name" -C "$extracted"
  check_app "$extracted/Woodshed.app" "update archive"
  rm -rf "$extracted"
}

# ---- push and upload ------------------------------------------------------------

# Each step returns instead of exiting, so a failure can print how to retry.
upload_release() {
  if ! gh release view "$tag" -R "$repo" >/dev/null 2>&1; then
    gh release create "$tag" -R "$repo" --verify-tag --draft --title "Woodshed $version" \
      --notes-file "$out/release-notes.md" || return 1
  fi
  set --
  for asset in $assets; do
    set -- "$@" "$out/$asset"
  done
  gh release upload "$tag" -R "$repo" --clobber "$@" || return 1
  gh release edit "$tag" -R "$repo" --draft=false --latest || return 1
}

upload_or_explain() {
  step "GitHub release $tag ($repo)"
  if ! upload_release; then
    echo "error: the upload of release $tag failed. The commit and tag are on origin." >&2
    echo "Retry the upload only, with the same files in $out:" >&2
    echo "  mise run release -- $version --upload-only" >&2
    exit 1
  fi
  echo "released: https://github.com/$repo/releases/tag/$tag"
}

list_files() {
  step "files in $out"
  (cd "$out" && ls -l $assets)
}

# ---- main ------------------------------------------------------------------------

if [ "$upload_only" = true ]; then
  step "upload only: $tag"
  check_gh
  remote_has_tag || fail "tag $tag is not on origin; run a full release instead"
  if [ -n "$notes_file" ]; then
    cp "$notes_file" "$out/release-notes.md"
  fi
  [ -s "$out/release-notes.md" ] || fail "no $out/release-notes.md (pass --notes-file)"
  verify_files
  upload_or_explain
  exit 0
fi

step "checks"
check_gh
check_keys
check_git
rm -rf "$out"
mkdir -p "$out"
step "sources of yt-dlp $ytdlp_version and FFmpeg $ffmpeg_version"
fetch_sources
step "version $version"
bump_version
build_files
verify_files
list_files

if [ "$dry_run" = true ]; then
  step "dry run: nothing pushed, no release made"
  restore_git
  exit 0
fi

step "push $tag and main"
git push --atomic origin "$release_commit:refs/heads/main" "refs/tags/$tag"
pushed=true
upload_or_explain

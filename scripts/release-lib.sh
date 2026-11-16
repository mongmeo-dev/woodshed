# shellcheck shell=sh disable=SC2154  # variables are the caller's (below)
# The git and GitHub steps of scripts/release.sh (sourced, not run): undoing
# the version commit, the push, and the GitHub release. They live apart so
# scripts/release-lib.test.mjs can run them against stand-in git and gh.
#
# Uses the caller's variables: version, tag, repo, out, assets (file names in
# $out), work (temporary directory), orig_head, release_commit, bumped
# (true once the version files changed), pushing (true while a push may be
# under way), pushed (false, true, or unknown when origin could not tell),
# push_check_seconds (how long to wait for origin when judging a push) and
# mounted.

step() {
  printf '\n==> %s\n' "$*"
}

fail() {
  echo "error: $*" >&2
  exit 1
}

# ---- undoing the version commit ------------------------------------------------

# Puts the branch back where it was: removes the version commit and tag (only
# while they are ours and surely not pushed). Runs at the end of a dry run and
# on any failure before the push or after a push origin did not take.
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
  # Stopped (a signal, an error) while a push may have reached origin: undo
  # nothing until origin says it took neither ref.
  if [ "${pushing:-false}" = true ]; then
    echo "release stopped during the push; reading origin to see what it took" >&2
    judge_push
    settle_push
  fi
  if [ "$mounted" = true ]; then
    hdiutil detach -quiet -force "$work/mnt" || true
  fi
  rm -rf "$work"
  if [ "$bumped" = true ] && [ "$pushed" = false ]; then
    echo "release stopped before origin took anything; removing the version commit and tag" >&2
    restore_git || true
  fi
  exit "$status"
}

# ---- origin ----------------------------------------------------------------------

# Prints "<main> <tag>": the commits origin's main and tag $tag (peeled) point
# to, "-" for a missing ref. Fails when origin cannot be read.
remote_refs() {
  listing=$(git ls-remote origin refs/heads/main "refs/tags/$tag" "refs/tags/$tag^{}") || return 1
  printf '%s\n' "$listing" | read_remote_refs
}

# The "<main> <tag>" of remote_refs from a `git ls-remote` listing on stdin.
read_remote_refs() {
  awk -v tag="refs/tags/$tag" '
    $2 == "refs/heads/main" { main = $1 }
    $2 == tag { direct = $1 }
    $2 == tag "^{}" { peeled = $1 }
    END {
      if (peeled == "") peeled = direct
      printf "%s %s\n", (main == "" ? "-" : main), (peeled == "" ? "-" : peeled)
    }'
}

# remote_refs, given up after push_check_seconds (default 15): a hung origin
# must not hold the exit trap. git runs as a direct background child so that
# it is the process stopped when time is up.
remote_refs_within() {
  limit=$((${push_check_seconds:-15} * 10))
  answer=$(mktemp)
  git ls-remote origin refs/heads/main "refs/tags/$tag" "refs/tags/$tag^{}" >"$answer" &
  reader=$!
  ticks=0
  while kill -0 "$reader" 2>/dev/null; do
    if [ "$ticks" -ge "$limit" ]; then
      kill "$reader" 2>/dev/null || true
      wait "$reader" 2>/dev/null || true
      rm -f "$answer"
      return 1
    fi
    sleep 0.1
    ticks=$((ticks + 1))
  done
  if ! wait "$reader"; then
    rm -f "$answer"
    return 1
  fi
  read_remote_refs <"$answer"
  rm -f "$answer"
}

# Sets push_state (pushed, not_pushed or unknown) and push_detail from what
# origin has after a push that did not finish cleanly:
#   - tag at the version commit: pushed;
#   - neither main nor the tag at it: not pushed;
#   - origin unreadable in time, or main at it without the tag: unknown.
judge_push() {
  if ! refs=$(remote_refs_within); then
    push_state=unknown
    push_detail="origin could not be read within ${push_check_seconds:-15}s"
    return 0
  fi
  remote_main=${refs% *}
  remote_tag=${refs#* }
  if [ "$remote_tag" = "$release_commit" ]; then
    push_state=pushed
    push_detail="origin has tag $tag at $release_commit (main at $remote_main)"
  elif [ "$remote_main" != "$release_commit" ]; then
    push_state=not_pushed
    push_detail="origin took neither main nor tag $tag (main at $remote_main, tag at $remote_tag)"
  else
    push_state=unknown
    push_detail="origin has main at $release_commit but tag $tag at $remote_tag"
  fi
}

# Acts on judge_push: keeps everything unless origin took neither ref (then
# pushed stays false and on_exit undoes the version commit and tag).
settle_push() {
  case "$push_state" in
  pushed)
    pushed=true
    echo "error: the push did not finish cleanly, but $push_detail." >&2
    echo "The version commit and tag are kept. Upload the release with:" >&2
    echo "  mise run release -- $version --upload-only" >&2
    ;;
  not_pushed)
    echo "error: $push_detail" >&2
    ;;
  *)
    pushed=unknown
    explain_unknown_push "$push_detail"
    ;;
  esac
  pushing=false
}

explain_unknown_push() {
  echo "error: cannot tell whether origin took the push ($1). Nothing was removed here." >&2
  echo "Check origin:" >&2
  echo "  git ls-remote origin refs/heads/main refs/tags/$tag 'refs/tags/$tag^{}'" >&2
  echo "- tag $tag at $release_commit: upload the release with" >&2
  echo "    mise run release -- $version --upload-only" >&2
  echo "- neither main nor tag $tag at $release_commit: undo the version commit and tag with" >&2
  echo "    git tag -d $tag && git reset --hard $orig_head" >&2
  echo "  and run the release again." >&2
}

# Pushes the version commit to main and the tag, together. A push can fail
# on this side after origin took it (a dropped connection), and the release
# can be stopped (INT, TERM) while it runs, so from just before the push until
# its outcome is known `pushing` is true: a failure here, or on_exit, then
# judges the push by what origin has (judge_push) and undoes the version
# commit and tag only when origin took neither ref.
push_release() {
  step "push $tag and main"
  pushing=true
  if git push --atomic origin "$release_commit:refs/heads/main" "refs/tags/$tag"; then
    pushed=true
    pushing=false
    return 0
  fi
  echo "git push reported a failure; reading origin to see what it took" >&2
  judge_push
  settle_push
  exit 1
}

# ---- GitHub release --------------------------------------------------------------

# The files in $out must be a release build (not a dry run) of the commit
# that tag $tag points to on origin (provenance.json, scripts/release-files.mjs).
check_upload_source() {
  refs=$(remote_refs) || fail "cannot read origin"
  tag_commit=${refs#* }
  [ "$tag_commit" != - ] || fail "tag $tag is not on origin; run a full release instead"
  # shellcheck disable=SC2086
  node scripts/release-files.mjs provenance-check "$out" "$version" "$tag_commit" $assets ||
    fail "not uploading $out to $tag"
}

# Prints none, draft or published, leaving gh's JSON in $work/release.json.
release_state() {
  if gh release view "$tag" -R "$repo" --json isDraft,assets >"$work/release.json" 2>"$work/release.err"; then
    node scripts/release-files.mjs release-state "$work/release.json"
    return
  fi
  if grep -q 'release not found' "$work/release.err"; then
    echo none
    return 0
  fi
  cat "$work/release.err" >&2
  return 1
}

# Makes the release public only after every file is on it: a draft first
# (new, or one an earlier attempt left), the files uploaded to the draft and
# compared by SHA-256, then published as latest. A published release is
# never changed: with the same files it counts as done, otherwise refused.
# Returns 0 when released, 1 on a failure worth retrying, 2 when refused.
upload_release() {
  state=$(release_state) || return 1
  if [ "$state" = published ]; then
    # shellcheck disable=SC2086
    if node scripts/release-files.mjs assets-match "$work/release.json" "$out" $assets; then
      echo "release $tag is already published with these files"
      return 0
    fi
    echo "error: release $tag is already published with other files; a public release is not changed" >&2
    return 2
  fi
  gh release list -R "$repo" --exclude-drafts --limit 1000 --json tagName,isDraft,isPrerelease \
    >"$work/releases.json" || return 1
  node scripts/release-files.mjs check-newest "$version" "$work/releases.json" || return 2
  if [ "$state" = none ]; then
    gh release create "$tag" -R "$repo" --verify-tag --draft --title "Woodshed $version" \
      --notes-file "$out/release-notes.md" || return 1
  else
    echo "continuing the draft release $tag"
  fi
  set --
  for asset in $assets; do
    set -- "$@" "$out/$asset"
  done
  # Only ever a draft: --clobber replaces files no one can download yet.
  gh release upload "$tag" -R "$repo" --clobber "$@" || return 1
  state=$(release_state) || return 1
  if [ "$state" != draft ]; then
    echo "error: release $tag was published by someone else during the upload" >&2
    return 2
  fi
  # shellcheck disable=SC2086
  node scripts/release-files.mjs assets-match "$work/release.json" "$out" $assets || return 1
  gh release edit "$tag" -R "$repo" --draft=false --latest || return 1
}

upload_or_explain() {
  step "GitHub release $tag ($repo)"
  check_upload_source
  code=0
  upload_release || code=$?
  case "$code" in
  0)
    echo "released: https://github.com/$repo/releases/tag/$tag"
    ;;
  2)
    echo "error: release $tag left as it is on GitHub (see above)" >&2
    exit 1
    ;;
  *)
    echo "error: the upload of release $tag failed; the commit and tag are on origin." >&2
    echo "Retry the upload only, with the same files in $out:" >&2
    echo "  mise run release -- $version --upload-only" >&2
    exit 1
    ;;
  esac
}

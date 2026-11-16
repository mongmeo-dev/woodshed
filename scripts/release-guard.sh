# Shared by build-app.sh and build-release.sh (sourced, not run), before
# anything else. The frontend in their app must be tauri.conf.json's ../dist,
# built by its beforeBuildCommand (`pnpm build`) and checked by
# scripts/check-release-dist.mjs, merged only with the entry point's own
# --config file. So everything that lets `tauri build` read another config is
# refused: a --config/-c argument, the TAURI_CONFIG, TAURI_FRONTEND_PATH,
# TAURI_APP_PATH and TAURI_PATH variables, and a macOS platform config file the
# CLI would merge by itself. Update test builds use scripts/build-update-test.sh.

woodshed_release_guard() {
  for arg in "$@"; do
    case "$arg" in
    --config | --config=* | -c* | -[!-]*c*)
      echo "error: $0 takes no --config/-c ($arg); the app's frontend stays tauri.conf.json's dist" >&2
      exit 2
      ;;
    esac
  done
  for name in TAURI_CONFIG TAURI_FRONTEND_PATH TAURI_APP_PATH TAURI_PATH; do
    if eval "[ -n \"\${$name+set}\" ]"; then
      echo "error: unset $name; $0 builds only with tauri.conf.json and its own --config" >&2
      exit 2
    fi
  done
  for file in src-tauri/tauri.macos.conf.json src-tauri/tauri.macos.conf.json5 src-tauri/Tauri.macos.toml src-tauri/Tauri.toml; do
    if [ -e "$file" ]; then
      echo "error: $file would be merged into the build config; remove it" >&2
      exit 2
    fi
  done
}

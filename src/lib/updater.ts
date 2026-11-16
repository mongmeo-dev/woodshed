import { getVersion } from "@tauri-apps/api/app";
import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";
import { inTauri } from "./ipc";

/**
 * A newer release found in latest.json (plugins.updater in tauri.conf.json).
 * The plugin verifies the downloaded archive against plugins.updater.pubkey
 * before installing; a bad signature rejects `install`.
 */
export interface AvailableUpdate {
  version: string;
  /** Downloads, verifies and installs, then restarts the app. `fraction` is 0–1, or null while the size is unknown. */
  install: (onProgress: (fraction: number | null) => void) => Promise<void>;
}

/**
 * Vite mode of the local update test build only (`scripts/build-update-test.sh`
 * runs `vite build --mode update-test`). No environment variable or `.env`
 * file turns it on: every other build runs in `production` (or `development`,
 * `test`), where this is a constant false and the bundler drops the branch.
 */
export const UPDATE_TEST_MODE = "update-test";

/** Installs a found update without a click. Update test builds only. */
export const AUTO_INSTALL = import.meta.env.MODE === UPDATE_TEST_MODE;

/**
 * Written to `<html data-update-test>` by the auto install branch. A DOM write
 * survives minification (unlike a console call), so its presence in a built
 * frontend means the branch is there: scripts/check-release-dist.mjs refuses it.
 */
export const UPDATE_TEST_MARKER = "woodshed-update-test";

/** Updates work only inside the Tauri app. */
export function updatesSupported(): boolean {
  return inTauri();
}

export async function currentVersion(): Promise<string | null> {
  if (!updatesSupported()) return null;
  return getVersion();
}

/** The newer release, or null when this version is the latest. Rejects when the check fails. */
export async function checkForUpdate(): Promise<AvailableUpdate | null> {
  if (!updatesSupported()) return null;
  const update = await check();
  if (!update) return null;
  return {
    version: update.version,
    install: async (onProgress) => {
      let total = 0;
      let received = 0;
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            total = event.data.contentLength ?? 0;
            received = 0;
            onProgress(total > 0 ? 0 : null);
            break;
          case "Progress":
            received += event.data.chunkLength;
            onProgress(total > 0 ? Math.min(received / total, 1) : null);
            break;
          case "Finished":
            onProgress(1);
            break;
        }
      });
      await relaunch();
    },
  };
}

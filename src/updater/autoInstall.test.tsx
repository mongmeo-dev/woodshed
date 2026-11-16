import { render, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { UPDATE_TEST_MARKER } from "../lib/updater";
import { UpdaterProvider } from "./UpdaterProvider";

// The update test build (Vite mode "update-test"), where AUTO_INSTALL is on.
const downloadAndInstall = vi.fn(async () => {});
const relaunch = vi.fn();
vi.mock("@tauri-apps/plugin-updater", () => ({
  check: async () => ({ version: "0.2.0", downloadAndInstall }),
}));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: () => relaunch() }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: async () => "0.1.0" }));
vi.mock("../lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/ipc")>()),
  inTauri: () => true,
}));
vi.mock("../lib/updater", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/updater")>()),
  AUTO_INSTALL: true,
}));

afterEach(() => {
  delete document.documentElement.dataset.updateTest;
});

test("the update test build installs without a click and leaves the marker the release check looks for", async () => {
  render(
    <UpdaterProvider startupDelayMs={0}>
      <div />
    </UpdaterProvider>,
  );
  await waitFor(() => expect(relaunch).toHaveBeenCalledTimes(1));
  expect(downloadAndInstall).toHaveBeenCalledTimes(1);
  expect(document.documentElement.dataset.updateTest).toBe(UPDATE_TEST_MARKER);
});

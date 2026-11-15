import { readFileSync } from "node:fs";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { i18n } from "../i18n";
import { UpdateButton } from "./UpdateButton";
import { UpdaterProvider } from "./UpdaterProvider";
import { UpdateSection } from "./UpdateSection";

// The plugins, as the Tauri app answers them.
type DownloadEvent =
  | { event: "Started"; data: { contentLength?: number } }
  | { event: "Progress"; data: { chunkLength: number } }
  | { event: "Finished" };
const check = vi.fn();
const downloadAndInstall = vi.fn();
const relaunch = vi.fn();

vi.mock("@tauri-apps/plugin-updater", () => ({ check: () => check() }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: () => relaunch() }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: async () => "0.1.0" }));
vi.mock("../lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/ipc")>()),
  inTauri: () => true,
}));

function newVersion(version = "0.2.0") {
  return { version, downloadAndInstall };
}

/** A download whose events the test plays one by one. */
function controlledDownload() {
  let emit: (event: DownloadEvent) => void = () => {};
  let finish: (error?: Error) => void = () => {};
  downloadAndInstall.mockImplementation(
    (onEvent: (event: DownloadEvent) => void) =>
      new Promise<void>((resolve, reject) => {
        emit = onEvent;
        finish = (error) => (error ? reject(error) : resolve());
      }),
  );
  return {
    emit: (event: DownloadEvent) => act(() => emit(event)),
    finish: (error?: Error) => act(async () => finish(error)),
  };
}

function renderUpdater(startupDelayMs = 0) {
  render(
    <UpdaterProvider startupDelayMs={startupDelayMs}>
      <aside aria-label="sidebar">
        <UpdateButton />
      </aside>
      <UpdateSection />
    </UpdaterProvider>,
  );
  return {
    sidebar: within(screen.getByRole("complementary", { name: "sidebar" })),
    status: () => (screen.queryByRole("alert") ?? screen.queryByRole("status"))?.textContent ?? "–",
  };
}

beforeEach(async () => {
  check.mockReset();
  downloadAndInstall.mockReset();
  relaunch.mockReset();
  await i18n.changeLanguage("en");
});

test("launch check finds a newer version: sidebar shows Install Update, settings shows the version", async () => {
  check.mockResolvedValue(newVersion("0.2.0"));
  const { sidebar, status } = renderUpdater();
  expect(await sidebar.findByRole("button", { name: "Install Update" })).toBeTruthy();
  expect(status()).toBe("Version 0.2.0 Available");
  expect(await screen.findByText("0.1.0")).toBeTruthy();
  expect(check).toHaveBeenCalledTimes(1);
  // Never installs without a click.
  expect(downloadAndInstall).not.toHaveBeenCalled();
});

test("launch check waits a few seconds after start", async () => {
  vi.useFakeTimers();
  try {
    check.mockResolvedValue(null);
    renderUpdater(3000);
    await act(async () => vi.advanceTimersByTime(2900));
    expect(check).not.toHaveBeenCalled();
    await act(async () => vi.advanceTimersByTime(100));
    expect(check).toHaveBeenCalledTimes(1);
  } finally {
    vi.useRealTimers();
  }
});

test("a failed launch check stays quiet", async () => {
  check.mockRejectedValue(new Error("offline"));
  const { sidebar, status } = renderUpdater();
  await waitFor(() => expect(check).toHaveBeenCalled());
  expect(status()).toBe("–");
  expect(screen.queryByRole("alert")).toBeNull();
  expect(sidebar.queryByRole("button")).toBeNull();
});

test("Check for Updates shows checking, then up to date", async () => {
  check.mockResolvedValueOnce(null);
  const { sidebar, status } = renderUpdater();
  await waitFor(() => expect(status()).toBe("Up to Date"));

  let answer: (value: null) => void = () => {};
  check.mockImplementationOnce(() => new Promise((resolve) => (answer = resolve)));
  const button = screen.getByRole("button", { name: "Check for Updates" });
  fireEvent.click(button);
  await waitFor(() => expect(status()).toBe("Checking"));
  expect(button).toHaveProperty("disabled", true);

  await act(async () => answer(null));
  expect(status()).toBe("Up to Date");
  expect(button).toHaveProperty("disabled", false);
  expect(sidebar.queryByRole("button")).toBeNull();
});

test("Check for Updates shows an error when the check fails", async () => {
  check.mockResolvedValueOnce(null).mockRejectedValueOnce(new Error("offline"));
  const { status } = renderUpdater();
  await waitFor(() => expect(status()).toBe("Up to Date"));
  fireEvent.click(screen.getByRole("button", { name: "Check for Updates" }));
  expect((await screen.findByRole("alert")).textContent).toBe("Couldn't check for updates");
});

test("Install Update downloads with progress, then restarts", async () => {
  check.mockResolvedValue(newVersion("0.2.0"));
  const download = controlledDownload();
  const { sidebar, status } = renderUpdater();
  fireEvent.click(await sidebar.findByRole("button", { name: "Install Update" }));

  expect(status()).toBe("Downloading");
  expect(sidebar.getByText("Downloading")).toBeTruthy();
  download.emit({ event: "Started", data: { contentLength: 1000 } });
  download.emit({ event: "Progress", data: { chunkLength: 450 } });
  expect(status()).toBe("Downloading 45%");
  expect(sidebar.getByText("Downloading 45%")).toBeTruthy();
  expect(sidebar.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("45");
  expect(sidebar.queryByRole("button")).toBeNull();
  expect(screen.queryByRole("button", { name: "Check for Updates" })).toBeNull();

  download.emit({ event: "Progress", data: { chunkLength: 550 } });
  download.emit({ event: "Finished" });
  expect(relaunch).not.toHaveBeenCalled();
  await download.finish();
  expect(relaunch).toHaveBeenCalledTimes(1);
});

test("a failed install (download or signature) shows the error and offers Install Update again", async () => {
  check.mockResolvedValue(newVersion("0.2.0"));
  const download = controlledDownload();
  renderUpdater();
  const settingsInstall = await screen.findAllByRole("button", { name: "Install Update" });
  fireEvent.click(settingsInstall[1] as HTMLElement);
  await download.finish(new Error("signature verification failed"));

  expect((await screen.findByRole("alert")).textContent).toBe("Couldn't install the update");
  expect(relaunch).not.toHaveBeenCalled();
  expect(screen.getAllByRole("button", { name: "Install Update" })).toHaveLength(2);

  controlledDownload();
  fireEvent.click(screen.getAllByRole("button", { name: "Install Update" })[0] as HTMLElement);
  expect(downloadAndInstall).toHaveBeenCalledTimes(2);
});

test("Korean labels", async () => {
  await i18n.changeLanguage("ko");
  check.mockResolvedValue(newVersion("0.2.0"));
  const { sidebar, status } = renderUpdater();
  expect(await sidebar.findByRole("button", { name: "업데이트 설치" })).toBeTruthy();
  expect(status()).toBe("새 버전 0.2.0");
  expect(screen.getByText("현재 버전")).toBeTruthy();
});

test("updates come from the GitHub release and are verified with the release public key", () => {
  const config = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
  const updater = config.plugins.updater;
  expect(updater.endpoints).toEqual([
    "https://github.com/mongmeo-dev/woodshed/releases/latest/download/latest.json",
  ]);
  const publicKey = Buffer.from(updater.pubkey, "base64").toString("utf8");
  expect(publicKey).toMatch(
    /^untrusted comment: minisign public key: [0-9A-F]{16}\nRW[A-Za-z0-9+/]{54}\n?$/,
  );
  // Plain http stays off, and update archives need the private key, so only release builds make them.
  expect(updater.dangerousInsecureTransportProtocol).toBeUndefined();
  expect(config.bundle.createUpdaterArtifacts).toBeUndefined();
});

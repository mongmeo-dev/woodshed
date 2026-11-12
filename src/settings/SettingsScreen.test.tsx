import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { i18n } from "../i18n";
import { SettingsProvider } from "./SettingsProvider";
import { SettingsScreen } from "./SettingsScreen";

const setOutputDevice = vi.fn();

vi.mock("../lib/audio", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/audio")>();
  return {
    ...actual,
    listDevices: async () => [
      { id: "coreaudio:C", name: "Interface", isDefault: false, channels: 2, sampleRate: 48_000 },
    ],
    getAudioStatus: async () => ({
      output: {
        deviceId: "coreaudio:A",
        deviceName: "Speakers",
        sampleRate: 48_000,
        channels: 2,
        requestedBufferFrames: 64,
        bufferFrames: 64,
      },
      outputLatencyMs: 3.5,
      stats: {
        generation: 1,
        framesRendered: 0,
        callbacks: 0,
        maxCallbackFrames: 64,
        lateEvents: 0,
        droppedEvents: 0,
      },
    }),
    setOutputDevice: (deviceId: string | null) => setOutputDevice(deviceId),
  };
});

beforeEach(async () => {
  setOutputDevice.mockReset();
  await i18n.changeLanguage("en");
});

async function chooseInterface() {
  render(
    <SettingsProvider>
      <SettingsScreen />
    </SettingsProvider>,
  );
  const select = await screen.findByLabelText("Output Device");
  await screen.findByRole("option", { name: "Interface" });
  fireEvent.change(select, { target: { value: "coreaudio:C" } });
  return select as HTMLSelectElement;
}

test("a save failure shows the save error and keeps the new choice", async () => {
  setOutputDevice.mockRejectedValue({ code: "saveFailed", message: "disk full" });
  const select = await chooseInterface();
  expect(await screen.findByRole("alert")).toHaveProperty("textContent", "Couldn't save settings");
  expect(select.value).toBe("coreaudio:C");
  expect(setOutputDevice).toHaveBeenCalledWith("coreaudio:C");
});

test("an open failure shows the device error and keeps the choice", async () => {
  setOutputDevice.mockRejectedValue({ code: "openFailed", message: "busy" });
  const select = await chooseInterface();
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "Can't open the output device",
  );
  expect(select.value).toBe("coreaudio:C");
});

test("a successful switch shows no error", async () => {
  setOutputDevice.mockResolvedValue({
    output: null,
    outputLatencyMs: null,
    stats: {
      generation: 2,
      framesRendered: 0,
      callbacks: 0,
      maxCallbackFrames: 0,
      lateEvents: 0,
      droppedEvents: 0,
    },
  });
  const select = await chooseInterface();
  await waitFor(() => expect(select.value).toBe("coreaudio:C"));
  expect(screen.queryByText("Couldn't save settings")).toBeNull();
});

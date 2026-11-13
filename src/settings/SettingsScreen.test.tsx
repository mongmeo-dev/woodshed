import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { i18n } from "../i18n";
import { SettingsProvider } from "./SettingsProvider";
import { SettingsScreen } from "./SettingsScreen";

/** The app's running preference, as audio_status reports it. */
let runningPreferred: string | null = null;
/** audio_status: no output yet, held for the microphone answer. */
let waitingForMicrophone = false;
const setOutputDevice = vi.fn();

vi.mock("../lib/audio", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/audio")>();
  return {
    ...actual,
    listDevices: async () => [
      { id: "coreaudio:C", name: "Interface", isDefault: false, channels: 2, sampleRate: 48_000 },
    ],
    getAudioStatus: async () => ({
      output: waitingForMicrophone
        ? null
        : {
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
      preferredDeviceId: runningPreferred,
      waitingForMicrophone,
    }),
    setOutputDevice: (deviceId: string | null) => setOutputDevice(deviceId),
  };
});

beforeEach(async () => {
  runningPreferred = null;
  waitingForMicrophone = false;
  setOutputDevice.mockReset();
  await i18n.changeLanguage("en");
});

async function chooseInterface() {
  render(
    <SettingsProvider>
      <SettingsScreen />
    </SettingsProvider>,
  );
  const select = (await screen.findByLabelText("Output Device")) as HTMLSelectElement;
  await screen.findByRole("option", { name: "Interface" });
  fireEvent.change(select, { target: { value: "coreaudio:C" } });
  return select;
}

/** The backend applies the choice, then answers with `answer`. */
function backendApplies(answer: () => Promise<unknown>) {
  setOutputDevice.mockImplementation((deviceId: string | null) => {
    runningPreferred = deviceId;
    return answer();
  });
}

test("a save failure shows the save error and keeps the new choice", async () => {
  backendApplies(() => Promise.reject({ code: "saveFailed", message: "disk full" }));
  const select = await chooseInterface();
  expect(await screen.findByRole("alert")).toHaveProperty("textContent", "Couldn't save settings");
  await waitFor(() => expect(select.value).toBe("coreaudio:C"));
  expect(setOutputDevice).toHaveBeenCalledWith("coreaudio:C");
});

test("an open failure shows the device error and keeps the choice", async () => {
  backendApplies(() => Promise.reject({ code: "openFailed", message: "busy" }));
  const select = await chooseInterface();
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "Can't open the output device",
  );
  await waitFor(() => expect(select.value).toBe("coreaudio:C"));
});

test("a successful switch shows no error", async () => {
  backendApplies(() => Promise.resolve({ output: null, preferredDeviceId: "coreaudio:C" }));
  const select = await chooseInterface();
  await waitFor(() => expect(select.value).toBe("coreaudio:C"));
  expect(screen.queryByRole("alert")).toBeNull();
});

test("after a timeout the screen follows the state the app ends up in", async () => {
  // No answer in time; the request had not been applied yet.
  setOutputDevice.mockRejectedValue({ code: "unavailable" });
  const select = await chooseInterface();
  await waitFor(() => expect(select.value).toBe(""), { timeout: 1500 });

  // The manager finishes the switch later; the next status poll shows it.
  runningPreferred = "coreaudio:C";
  await waitFor(() => expect(select.value).toBe("coreaudio:C"), { timeout: 2500 });
});

test("while macOS holds the output for the microphone answer, the screen asks for access", async () => {
  waitingForMicrophone = true;
  backendApplies(() => Promise.reject({ code: "openFailed", message: "waiting" }));
  await chooseInterface();
  await waitFor(() => expect(setOutputDevice).toHaveBeenCalledWith("coreaudio:C"));
  expect(await screen.findByRole("status")).toHaveProperty(
    "textContent",
    "Allow microphone access",
  );
  expect(screen.queryByText("Can't open the output device")).toBeNull();
});

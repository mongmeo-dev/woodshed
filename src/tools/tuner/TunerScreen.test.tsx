import { act, fireEvent, render, screen } from "@testing-library/react";
import { vi } from "vitest";
import { i18n } from "../../i18n";
import {
  channelFromKey,
  channelKey,
  clampReference,
  DEFAULT_STATUS,
  levelFraction,
  noteName,
  type TunerEvent,
} from "./tuner";

const calls = vi.hoisted(() => ({
  onEvent: null as ((event: TunerEvent) => void) | null,
  selected: [] as unknown[],
  references: [] as number[],
  stopped: 0,
}));

vi.mock("./tuner", async (original) => {
  const actual = await original<typeof import("./tuner")>();
  return {
    ...actual,
    startTuner: (onEvent: (event: TunerEvent) => void) => {
      calls.onEvent = onEvent;
      return Promise.resolve(actual.DEFAULT_STATUS);
    },
    stopTuner: () => {
      calls.stopped += 1;
      return Promise.resolve();
    },
    selectInput: (deviceId: string | null, channel: unknown) => {
      calls.selected.push([deviceId, channel]);
      return Promise.resolve();
    },
    setReference: (hz: number) => {
      calls.references.push(hz);
      return Promise.resolve(actual.clampReference(hz));
    },
  };
});

const { TunerScreen } = await import("./TunerScreen");

beforeEach(async () => {
  await i18n.changeLanguage("en");
  calls.onEvent = null;
  calls.selected = [];
  calls.references = [];
});

test("note names and octaves follow MIDI numbers", () => {
  expect(noteName(69)).toEqual({ name: "A", octave: 4 });
  expect(noteName(40)).toEqual({ name: "E", octave: 2 });
  expect(noteName(23)).toEqual({ name: "B", octave: 0 });
  expect(noteName(61)).toEqual({ name: "C♯", octave: 4 });
});

test("helpers keep values in range", () => {
  expect(clampReference(425)).toBe(430);
  expect(clampReference(451)).toBe(450);
  expect(clampReference(441.4)).toBe(441);
  expect(levelFraction(-90)).toBe(0);
  expect(levelFraction(-30)).toBe(0.5);
  expect(levelFraction(3)).toBe(1);
  expect(channelFromKey(channelKey({ type: "mixDown" }))).toEqual({ type: "mixDown" });
  expect(channelFromKey(channelKey({ type: "single", channel: 3 }))).toEqual({
    type: "single",
    channel: 3,
  });
});

test("a frame shows the note, cents and frequency", async () => {
  render(<TunerScreen />);
  expect(await screen.findByRole("heading", { name: "Tuner" })).toBeTruthy();
  expect(calls.onEvent).not.toBeNull();
  act(() => {
    calls.onEvent?.({ type: "status", ...DEFAULT_STATUS, state: "listening" });
    calls.onEvent?.({
      type: "frame",
      levelDb: -20,
      pitch: { frequency: 82.5, midi: 40, cents: 2.2, clarity: 0.99 },
    });
  });
  await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
  const note = screen.getByRole("status", { name: "Note" });
  expect(note.textContent).toBe("E2");
  expect(screen.getByText("+2 cents")).toBeTruthy();
  expect(screen.getByText("82.5 Hz")).toBeTruthy();
  const meter = screen.getByRole("meter", { name: "Input Level" }) as HTMLMeterElement;
  expect(meter.value).toBe(67);
});

for (const state of ["noInput", "permissionDenied"] as const) {
  test(`losing the input (${state}) clears the last reading at once`, async () => {
    render(<TunerScreen />);
    await screen.findByRole("heading", { name: "Tuner" });
    act(() => {
      calls.onEvent?.({
        type: "status",
        ...DEFAULT_STATUS,
        state: "listening",
        inputGeneration: 1,
      });
      calls.onEvent?.({
        type: "frame",
        levelDb: -20,
        pitch: { frequency: 82.4, midi: 40, cents: 0.4, clarity: 0.99 },
      });
    });
    await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
    const readout = document.querySelector(".tuner-readout");
    expect(readout?.getAttribute("data-in-tune")).toBe("true");

    // Only a status arrives; no frame follows.
    act(() => {
      calls.onEvent?.({ type: "status", ...DEFAULT_STATUS, state, inputGeneration: 1 });
    });
    await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
    expect(readout?.getAttribute("data-active")).toBe("false");
    expect(readout?.getAttribute("data-in-tune")).toBe("false");
    expect(screen.getByRole("status", { name: "Note" }).textContent).toBe("–");
    const meter = screen.getByRole("meter", { name: "Input Level" }) as HTMLMeterElement;
    expect(meter.value).toBe(0);
  });
}

test("a new input stream clears readings from the old one", async () => {
  render(<TunerScreen />);
  await screen.findByRole("heading", { name: "Tuner" });
  act(() => {
    calls.onEvent?.({ type: "status", ...DEFAULT_STATUS, state: "listening", inputGeneration: 1 });
    calls.onEvent?.({
      type: "frame",
      levelDb: -20,
      pitch: { frequency: 110, midi: 45, cents: 0, clarity: 0.99 },
    });
  });
  await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
  expect(screen.getByRole("status", { name: "Note" }).textContent).toBe("A2");
  act(() => {
    calls.onEvent?.({ type: "status", ...DEFAULT_STATUS, state: "listening", inputGeneration: 2 });
  });
  await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
  expect(screen.getByRole("status", { name: "Note" }).textContent).toBe("–");
});

test("the reference pitch steps by 1 Hz and stops at the range ends", async () => {
  render(<TunerScreen />);
  const raise = await screen.findByRole("button", { name: "Raise Reference Pitch" });
  fireEvent.click(raise);
  expect(await screen.findByText("A4 = 441 Hz")).toBeTruthy();
  expect(calls.references).toEqual([441]);
  act(() => {
    calls.onEvent?.({ type: "status", ...DEFAULT_STATUS, referenceHz: 430 });
  });
  expect(
    (screen.getByRole("button", { name: "Lower Reference Pitch" }) as HTMLButtonElement).disabled,
  ).toBe(true);
});

test("a denied microphone shows what to do", async () => {
  render(<TunerScreen />);
  await screen.findByRole("heading", { name: "Tuner" });
  act(() => {
    calls.onEvent?.({ type: "status", ...DEFAULT_STATUS, state: "permissionDenied" });
  });
  expect(screen.getByRole("alert").textContent).toContain(
    "Allow microphone access in System Settings",
  );
  expect(screen.getByRole("button", { name: "Open System Settings" })).toBeTruthy();
});

test("leaving the screen stops the tuner", async () => {
  const { unmount } = render(<TunerScreen />);
  await screen.findByRole("heading", { name: "Tuner" });
  const before = calls.stopped;
  unmount();
  await act(() => new Promise((resolve) => setTimeout(resolve, 0)));
  expect(calls.stopped).toBe(before + 1);
});

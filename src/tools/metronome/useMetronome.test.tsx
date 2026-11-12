import { act, renderHook, waitFor } from "@testing-library/react";
import type { MetronomeMessage } from "./api";
import { DEFAULT_SETTINGS } from "./model";
import { retractHeard, useMetronome } from "./useMetronome";

const mocks = vi.hoisted(() => ({ send: null as null | ((message: MetronomeMessage) => void) }));

vi.mock("./api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./api")>();
  return {
    ...actual,
    subscribeMetronome: async (onMessage: (message: MetronomeMessage) => void) => {
      mocks.send = onMessage;
    },
    loadMetronome: async () => ({ settings: DEFAULT_SETTINGS, presets: [], playing: true }),
  };
});

/** Heard a moment ago, so the next animation frame shows it. */
const heardNow = () => performance.timeOrigin + performance.now() - 5;

const beat = (frame: number, index: number, bpm: number): MetronomeMessage => ({
  type: "beat",
  frame,
  heardAtMs: heardNow(),
  bar: 0,
  beat: index,
  beats: 4,
  accent: "normal",
  bpm,
});

const send = (message: MetronomeMessage) => act(() => mocks.send?.(message));

test("an output reset withdraws the beat, pulse and trainer tempo already shown", async () => {
  const { result } = renderHook(() => useMetronome());
  await waitFor(() => expect(result.current.playing).toBe(true));

  send(beat(48_000, 1, 150));
  send({ type: "pulse", frame: 50_000, heardAtMs: heardNow(), pulse: 1, pulses: 3 });
  await waitFor(() => expect(result.current.heard.beat?.bpm).toBe(150));
  await waitFor(() => expect(result.current.heard.pulse?.pulse).toBe(1));

  // A partial reset keeps what lies before its boundary.
  send({ type: "reset", fromFrame: 49_000 });
  expect(result.current.heard.beat?.frame).toBe(48_000);
  expect(result.current.heard.pulse).toBeNull();

  // An output switch resets from frame 0: nothing old stays lit.
  send({ type: "reset", fromFrame: 0 });
  expect(result.current.heard).toEqual({ beat: null, pulse: null });

  // Beats of the new output light again.
  send(beat(96_000, 2, 120));
  await waitFor(() => expect(result.current.heard.beat?.beat).toBe(2));
});

test("retracting keeps the same object when nothing is withdrawn", () => {
  const heard = { beat: null, pulse: null };
  expect(retractHeard(heard, 0)).toBe(heard);
});

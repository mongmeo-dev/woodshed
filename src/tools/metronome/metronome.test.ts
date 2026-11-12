import { BeatQueue } from "./beatQueue";
import {
  DEFAULT_SETTINGS,
  defaultAccents,
  formatBpm,
  normalizeSettings,
  parseBpm,
  roundBpm,
  withMeter,
} from "./model";
import { TAP_RESET_MS, TapTempo } from "./tapTempo";

describe("model", () => {
  test("tempo rounds to tenths within 20–400", () => {
    expect(roundBpm(120.04)).toBe(120);
    expect(roundBpm(120.06)).toBe(120.1);
    expect(roundBpm(3)).toBe(20);
    expect(roundBpm(999)).toBe(400);
    expect(roundBpm(Number.NaN)).toBe(120);
  });

  test("typed tempo accepts a decimal point or comma and rejects out-of-range values", () => {
    expect(parseBpm("96")).toBe(96);
    expect(parseBpm(" 96.55 ")).toBe(96.6);
    expect(parseBpm("96,5")).toBe(96.5);
    expect(parseBpm("19.9")).toBeNull();
    expect(parseBpm("400.1")).toBeNull();
    expect(parseBpm("")).toBeNull();
    expect(parseBpm("fast")).toBeNull();
  });

  test("default accents match the engine's (src-tauri/src/metronome/pattern.rs)", () => {
    expect(defaultAccents(4, 4)).toEqual(["strong", "normal", "normal", "normal"]);
    expect(defaultAccents(6, 8)).toEqual(["strong", "weak", "weak", "normal", "weak", "weak"]);
    expect(defaultAccents(7, 8)).toEqual([
      "strong",
      "weak",
      "normal",
      "weak",
      "normal",
      "weak",
      "weak",
    ]);
    expect(defaultAccents(12, 8).filter((accent) => accent !== "weak")).toHaveLength(4);
  });

  test("a new meter resets the accents", () => {
    const custom = {
      ...DEFAULT_SETTINGS,
      accents: DEFAULT_SETTINGS.accents.map(() => "mute" as const),
    };
    const next = withMeter(custom, 5, 4);
    expect(next.beats).toBe(5);
    expect(next.accents).toEqual(defaultAccents(5, 4));
    expect(withMeter(custom, 40, 3)).toMatchObject({ beats: 16, noteValue: 4 });
  });

  test("normalizing repairs partial and invalid data", () => {
    const settings = normalizeSettings({
      bpm: 512,
      beats: 3,
      accents: ["strong"],
      sound: "gong",
      polyrhythm: { enabled: true, pulses: 99 },
      trainer: { stepBpm: 0.04, atMax: "never" },
    });
    expect(settings.bpm).toBe(400);
    expect(settings.accents).toEqual(defaultAccents(3, 4));
    expect(settings.sound).toBe("click");
    expect(settings.polyrhythm).toEqual({
      enabled: true,
      pulses: 16,
      beats: 2,
      sound: "woodblock",
    });
    expect(settings.trainer.stepBpm).toBe(0.1);
    expect(settings.trainer.atMax).toBe("hold");
    expect(normalizeSettings(null)).toEqual(DEFAULT_SETTINGS);
  });

  test("tempo shows one decimal in the UI language", () => {
    expect(formatBpm(120, "en")).toBe("120.0");
    expect(formatBpm(96.5, "ko")).toBe("96.5");
  });
});

describe("tap tempo", () => {
  test("averages the intervals once there are two taps", () => {
    const tap = new TapTempo();
    expect(tap.tap(1_000)).toBeNull();
    expect(tap.tap(1_500)).toBe(120);
    expect(tap.tap(2_010)).toBe(118.8);
    expect(tap.tap(2_500)).toBe(120);
  });

  test("a long pause starts over", () => {
    const tap = new TapTempo();
    tap.tap(0);
    tap.tap(500);
    expect(tap.tap(500 + TAP_RESET_MS + 1)).toBeNull();
    expect(tap.tap(500 + TAP_RESET_MS + 1 + 1_000)).toBe(60);
  });

  test("a sudden change of pace follows the new pace", () => {
    const tap = new TapTempo();
    tap.tap(0);
    tap.tap(500);
    tap.tap(1_000);
    expect(tap.tap(1_250)).toBe(240);
  });

  test("slow tempos down to 20 BPM can be tapped, with uneven taps", () => {
    for (const [interval, bpm] of [
      [3_000, 20],
      [2_400, 25],
      [2_000, 30],
    ] as const) {
      const tap = new TapTempo();
      expect(tap.tap(0)).toBeNull();
      expect(tap.tap(interval)).toBe(bpm);
      expect(tap.tap(interval * 2)).toBe(bpm);
    }
    const uneven = new TapTempo();
    uneven.tap(0);
    uneven.tap(3_100);
    expect(uneven.tap(6_000)).toBe(20);
  });

  test("a pause just past the reset starts over", () => {
    const tap = new TapTempo();
    tap.tap(0);
    tap.tap(1_000);
    expect(tap.tap(1_000 + TAP_RESET_MS + 1)).toBeNull();
  });

  test("results stay within the tempo range", () => {
    const tap = new TapTempo();
    tap.tap(0);
    expect(tap.tap(50)).toBe(400);
  });
});

describe("beat queue", () => {
  const beat = (frame: number, heardAtMs: number, index: number) => ({
    type: "beat" as const,
    frame,
    heardAtMs,
    bar: 0,
    beat: index,
    beats: 4,
    accent: "normal" as const,
    bpm: 120,
  });

  test("returns the latest beat heard by now and keeps later ones", () => {
    const queue = new BeatQueue(1_000);
    queue.push(beat(48_000, 2_500, 2));
    queue.push(beat(0, 1_500, 0));
    queue.push(beat(24_000, 2_000, 1));
    queue.push({ type: "pulse", frame: 16_000, heardAtMs: 1_833, pulse: 1, pulses: 3 });

    expect(queue.take(400)).toEqual({});
    const due = queue.take(1_000);
    expect(due.beat?.beat).toBe(1);
    expect(due.pulse?.pulse).toBe(1);
    expect(queue.size).toBe(1);
  });

  test("a reset drops what will no longer play", () => {
    const queue = new BeatQueue(0);
    queue.push(beat(0, 10, 0));
    queue.push(beat(24_000, 20, 1));
    queue.push(beat(48_000, 30, 2));
    queue.reset(24_000);
    expect(queue.size).toBe(1);
    expect(queue.take(100).beat?.beat).toBe(0);
  });
});

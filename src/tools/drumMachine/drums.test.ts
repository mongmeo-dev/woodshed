import {
  deletePattern,
  emptyPattern,
  MAX_BPM,
  MAX_STEPS,
  MIN_SWING,
  normalizePattern,
  paintStep,
  resetMemoryStore,
  savePattern,
  setBpm,
  setLength,
  setSwing,
  setTrackVolume,
  storeErrorCode,
  toggleMute,
} from "./drums";

test("painting a step sets the chosen level, and the same level again clears it", () => {
  let pattern = emptyPattern();
  pattern = paintStep(pattern, "snare", 4, 2);
  expect(pattern.tracks.snare.steps[4]).toBe(2);
  pattern = paintStep(pattern, "snare", 4, 3);
  expect(pattern.tracks.snare.steps[4]).toBe(3);
  pattern = paintStep(pattern, "snare", 4, 3);
  expect(pattern.tracks.snare.steps[4]).toBe(0);
  expect(emptyPattern().tracks.snare.steps[4]).toBe(0);
});

test("growing to two bars copies an empty second bar from the first", () => {
  let pattern = paintStep(emptyPattern(), "kick", 0, 2);
  pattern = setLength(pattern, 32);
  expect(pattern.length).toBe(32);
  expect(pattern.tracks.kick.steps[16]).toBe(2);

  // Back to one bar keeps bar two; a filled bar two is not overwritten.
  pattern = paintStep(pattern, "kick", 20, 3);
  pattern = setLength(setLength(pattern, 16), 32);
  expect(pattern.tracks.kick.steps[20]).toBe(3);
  expect(pattern.tracks.kick.steps.length).toBe(MAX_STEPS);
});

test("tempo, swing and mix stay in range", () => {
  const pattern = emptyPattern();
  expect(setBpm(pattern, 999).bpm).toBe(MAX_BPM);
  expect(setBpm(pattern, 92.6).bpm).toBe(93);
  expect(setSwing(pattern, 10).swing).toBe(MIN_SWING);
  expect(setTrackVolume(pattern, "ride", 2).tracks.ride.volume).toBe(1);
  expect(toggleMute(pattern, "ride").tracks.ride.muted).toBe(true);
});

test("normalizing fills missing tracks and clamps levels", () => {
  const pattern = normalizePattern({
    length: 32,
    bpm: 10,
    tracks: { kick: { steps: [9, 2], volume: Number.NaN, muted: true } } as never,
  });
  expect(pattern.length).toBe(32);
  expect(pattern.bpm).toBe(40);
  expect(pattern.tracks.kick.steps.slice(0, 3)).toEqual([3, 2, 0]);
  expect(pattern.tracks.kick.muted).toBe(true);
  expect(pattern.tracks.snare.steps).toHaveLength(MAX_STEPS);
});

test("saved patterns are replaced by name and need a name", async () => {
  resetMemoryStore();
  await savePattern("Groove", emptyPattern());
  const saved = await savePattern(" Groove ", setBpm(emptyPattern(), 120));
  expect(saved).toHaveLength(1);
  expect(saved[0]?.pattern.bpm).toBe(120);
  await expect(savePattern("  ", emptyPattern())).rejects.toEqual({ code: "emptyName" });
  expect(storeErrorCode({ code: "emptyName" })).toBe("emptyName");
  expect(storeErrorCode(new Error("disk"))).toBe("saveFailed");
  expect(await deletePattern("Groove")).toEqual([]);
});

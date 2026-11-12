import en from "../../locales/en.json";
import ko from "../../locales/ko.json";
import { MAX_BPM, MAX_SWING, MIN_BPM, MIN_SWING, normalizePattern, TRACKS } from "./drums";
import { PRESET_IDS, PRESET_SPECS, parseRow, presetPattern } from "./presets";

test("there are at least eight genre presets, each named in both languages", () => {
  expect(PRESET_IDS.length).toBeGreaterThanOrEqual(8);
  for (const id of PRESET_IDS) {
    expect(ko.drumMachine.presets[id]).toBeTruthy();
    expect(en.drumMachine.presets[id]).toBeTruthy();
  }
});

test.each(PRESET_IDS)("%s is a valid, playable pattern", (id) => {
  const pattern = presetPattern(id);
  expect(pattern.bpm).toBeGreaterThanOrEqual(MIN_BPM);
  expect(pattern.bpm).toBeLessThanOrEqual(MAX_BPM);
  expect(pattern.swing).toBeGreaterThanOrEqual(MIN_SWING);
  expect(pattern.swing).toBeLessThanOrEqual(MAX_SWING);
  // Unchanged by normalizing: what the engine plays is what the grid shows.
  expect(normalizePattern(pattern)).toEqual(pattern);
  const hits = TRACKS.flatMap((track) => pattern.tracks[track].steps.slice(0, pattern.length));
  expect(hits.some((level) => level > 0)).toBe(true);
  for (const row of Object.values(PRESET_SPECS[id].rows)) {
    expect(() => parseRow(row, pattern.length)).not.toThrow();
  }
});

test("rows parse levels and repeat a one-bar row over two bars", () => {
  expect(parseRow("Xxg.............", 16).slice(0, 4)).toEqual([3, 2, 1, 0]);
  const twoBars = parseRow("x...............", 32);
  expect(twoBars[0]).toBe(2);
  expect(twoBars[16]).toBe(2);
  expect(parseRow("x...............", 16)[16]).toBe(0);
  expect(() => parseRow("x..", 16)).toThrow();
  expect(() => parseRow("x..............o", 16)).toThrow();
});

test("genre feels: shuffles swing, metal doubles the kick", () => {
  expect(presetPattern("shuffle").swing).toBeGreaterThanOrEqual(66);
  expect(presetPattern("jazzSwing").swing).toBeGreaterThanOrEqual(66);
  expect(presetPattern("halfTimeShuffle").swingUnit).toBe("sixteenth");
  expect(
    presetPattern("metal")
      .tracks.kick.steps.slice(0, 16)
      .every((level) => level > 0),
  ).toBe(true);
  expect(presetPattern("bossaNova").length).toBe(32);
});

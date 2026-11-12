import { delayUntilHeard, heardAtToPerformanceTime } from "./audio";
import { DEFAULT_SETTINGS, normalizeSettings } from "./settings";

test("normalizeSettings keeps valid values and drops the rest", () => {
  expect(normalizeSettings(null)).toEqual(DEFAULT_SETTINGS);
  expect(
    normalizeSettings({ language: "ko", theme: "dark", outputDeviceId: "coreaudio:x" }),
  ).toEqual({
    language: "ko",
    theme: "dark",
    outputDeviceId: "coreaudio:x",
  });
  expect(normalizeSettings({ language: "fr", theme: "blue", outputDeviceId: 3 })).toEqual(
    DEFAULT_SETTINGS,
  );
});

test("heard time converts from Unix epoch to the performance timeline", () => {
  const timeOrigin = 1_700_000_000_000;
  expect(heardAtToPerformanceTime(timeOrigin + 2_500, timeOrigin)).toBe(2_500);
  expect(delayUntilHeard(timeOrigin + 2_500, 2_480, timeOrigin)).toBe(20);
  expect(delayUntilHeard(timeOrigin + 2_500, 2_600, timeOrigin)).toBe(0);
});

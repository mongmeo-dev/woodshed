import { coveredTicks, labelHalfWidth, rulerTicks } from "./timeline";

test("ruler marks are at least 56 px apart and stay off the right edge", () => {
  expect(rulerTicks(180, 1000)).toEqual([0, 15, 30, 45, 60, 75, 90, 105, 120, 135, 150, 165]);
  expect(rulerTicks(180, 400)).toEqual([0, 30, 60, 90, 120, 150]);
  expect(rulerTicks(3600, 700)).toEqual([
    0, 300, 600, 900, 1200, 1500, 1800, 2100, 2400, 2700, 3000, 3300,
  ]);
  expect(rulerTicks(0, 500)).toEqual([]);
  expect(rulerTicks(120, 0)).toEqual([]);
});

test("ruler marks under a label are found from the label's width", () => {
  const ticks = [0, 30, 60, 90];
  const labels = ["0:00", "0:30", "1:00", "1:30"];
  // A 400 px ruler for 120 s: marks at 0, 100, 200 and 300 px.
  expect([...coveredTicks(ticks, labels, 250, 40, 120, 400)]).toEqual([60, 90]);
  expect([...coveredTicks(ticks, labels, 250, 26, 120, 400)]).toEqual([]);
  expect([...coveredTicks(ticks, labels, 150, 20, 120, 400)]).toEqual([]);
  // The first label starts at its mark, so a label near the left edge covers it.
  expect([...coveredTicks(ticks, labels, 40, 26, 120, 400)]).toEqual([0]);
  expect(labelHalfWidth("1:25.6", 6)).toBeGreaterThan(22);
});

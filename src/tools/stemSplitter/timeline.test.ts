import {
  coveredTicks,
  dragRange,
  labelHalfWidth,
  loopable,
  MIN_LOOP_SECS,
  moveEdge,
  rulerTicks,
  stepEdge,
} from "./timeline";

test("ruler marks are at least 56 px apart and stay off the right edge", () => {
  expect(rulerTicks(180, 1000)).toEqual([0, 15, 30, 45, 60, 75, 90, 105, 120, 135, 150, 165]);
  expect(rulerTicks(180, 400)).toEqual([0, 30, 60, 90, 120, 150]);
  expect(rulerTicks(3600, 700)).toEqual([
    0, 300, 600, 900, 1200, 1500, 1800, 2100, 2400, 2700, 3000, 3300,
  ]);
  expect(rulerTicks(0, 500)).toEqual([]);
  expect(rulerTicks(120, 0)).toEqual([]);
});

test("a loop edge stays inside the song and short of the other edge", () => {
  const loop = { start: 10, end: 20 };
  expect(moveEdge(loop, "start", -5, 60)).toEqual({ start: 0, end: 20 });
  expect(moveEdge(loop, "start", 25, 60)).toEqual({ start: 20 - MIN_LOOP_SECS, end: 20 });
  expect(moveEdge(loop, "end", 5, 60)).toEqual({ start: 10, end: 10 + MIN_LOOP_SECS });
  expect(moveEdge(loop, "end", 99, 60)).toEqual({ start: 10, end: 60 });
});

test("an edge pushed against the other one still makes a loop", () => {
  expect(loopable(moveEdge({ start: 10, end: 72 }, "start", 99, 180))).toBe(true);
  expect(loopable(moveEdge({ start: 0.3, end: 72 }, "end", 0, 180))).toBe(true);
  expect(loopable({ start: 10, end: 10.05 })).toBe(false);
});

test("a stepped edge moves by the step from the time shown for it", () => {
  const loop = { start: 18.123, end: 54 };
  expect(stepEdge(loop, "start", 0.1, 180).start).toBe(18.2);
  expect(stepEdge(loop, "start", -0.1, 180).start).toBe(18);
  // 1:36.29 shows as 1:36.3, so one press shows 1:36.4.
  expect(stepEdge({ start: 96.29, end: 120 }, "start", 0.1, 180).start).toBe(96.4);
  expect(stepEdge({ start: 18.2, end: 54 }, "start", 0.1, 180).start).toBe(18.3);
  expect(stepEdge(loop, "end", 1, 180).end).toBe(55);
  expect(stepEdge(loop, "end", -1, 180).end).toBe(53);
  expect(stepEdge({ start: 0, end: 54 }, "start", -0.1, 180).start).toBe(0);
  expect(stepEdge({ start: 10, end: 179.95 }, "end", 0.1, 180).end).toBe(180);
  expect(stepEdge({ start: 53.95, end: 54 }, "start", 0.1, 180).start).toBeCloseTo(53.9);
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

test("a drag selects, moves or resizes a loop inside the song", () => {
  expect(dragRange({ kind: "select", from: 30 }, 12, 60)).toEqual({ start: 12, end: 30 });
  const origin = { start: 10, end: 20 };
  expect(dragRange({ kind: "move", from: 15, origin }, 20, 60)).toEqual({ start: 15, end: 25 });
  expect(dragRange({ kind: "move", from: 15, origin }, 0, 60)).toEqual({ start: 0, end: 10 });
  expect(dragRange({ kind: "move", from: 15, origin }, 60, 60)).toEqual({ start: 50, end: 60 });
  expect(dragRange({ kind: "resize", edge: "end", from: 20, origin }, 31, 60)).toEqual({
    start: 10,
    end: 31,
  });
  // Grabbing an edge 1 s off its line moves it by the pointer's travel, without a jump.
  expect(dragRange({ kind: "resize", edge: "start", from: 11, origin }, 12, 60)).toEqual({
    start: 11,
    end: 20,
  });
});

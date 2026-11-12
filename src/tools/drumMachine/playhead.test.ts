import { applyStepEvent, type QueuedStep, takeHeard } from "./playhead";

const ORIGIN = 1_000_000;

function steps(...entries: [index: number, frame: number, heardAtMs: number][]) {
  return {
    type: "steps" as const,
    generation: 1,
    steps: entries.map(([index, frame, heardAtMs]) => ({ index, frame, heardAtMs })),
  };
}

test("steps are queued on the webview clock in frame order", () => {
  let queue: QueuedStep[] = [];
  queue = applyStepEvent(queue, steps([1, 6_000, ORIGIN + 125], [2, 12_000, ORIGIN + 250]), ORIGIN);
  queue = applyStepEvent(queue, steps([0, 0, ORIGIN + 0.5]), ORIGIN);
  expect(queue.map((step) => [step.index, step.at])).toEqual([
    [0, 0.5],
    [1, 125],
    [2, 250],
  ]);
});

test("the latest heard step is shown and later ones wait", () => {
  const queue = applyStepEvent(
    [],
    steps([0, 0, ORIGIN + 10], [1, 6_000, ORIGIN + 135], [2, 12_000, ORIGIN + 260]),
    ORIGIN,
  );
  expect(takeHeard(queue, 5)).toEqual({ index: undefined, rest: queue });
  const heard = takeHeard(queue, 140);
  expect(heard.index).toBe(1);
  expect(heard.rest.map((step) => step.index)).toEqual([2]);
});

test("a clear withdraws rescheduled steps and their new times replace them", () => {
  let queue = applyStepEvent(
    [],
    steps([4, 24_000, ORIGIN + 500], [5, 30_000, ORIGIN + 625]),
    ORIGIN,
  );
  queue = applyStepEvent(queue, { type: "clear", fromFrame: 30_000 });
  queue = applyStepEvent(queue, steps([5, 29_000, ORIGIN + 604]), ORIGIN);
  expect(queue.map((step) => [step.frame, step.at])).toEqual([
    [24_000, 500],
    [29_000, 604],
  ]);
  expect(applyStepEvent(queue, { type: "clear", fromFrame: 0 })).toEqual([]);
});

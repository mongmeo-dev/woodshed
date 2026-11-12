import { useEffect, useRef, useState } from "react";
import { heardAtToPerformanceTime } from "../../lib/audio";
import { type StepEvent, subscribeSteps } from "./drums";

/** A step waiting to be shown, timed on the webview's `performance.now()` clock. */
export interface QueuedStep {
  index: number;
  frame: number;
  at: number;
}

/** Applies a step channel message to the queue (kept sorted by frame). */
export function applyStepEvent(
  queue: readonly QueuedStep[],
  event: StepEvent,
  timeOrigin = performance.timeOrigin,
): QueuedStep[] {
  if (event.type === "clear") return queue.filter((step) => step.frame < event.fromFrame);
  const added = event.steps.map((step) => ({
    index: step.index,
    frame: step.frame,
    at: heardAtToPerformanceTime(step.heardAtMs, timeOrigin),
  }));
  return [...queue, ...added].sort((a, b) => a.frame - b.frame);
}

/**
 * Takes every step already heard at `now`: returns the latest one's index
 * (`undefined` if none is due) and the steps still to come.
 */
export function takeHeard(
  queue: readonly QueuedStep[],
  now: number,
): { index: number | undefined; rest: QueuedStep[] } {
  let due = 0;
  while (due < queue.length && (queue[due]?.at ?? Number.POSITIVE_INFINITY) <= now) due += 1;
  return { index: due > 0 ? queue[due - 1]?.index : undefined, rest: queue.slice(due) };
}

/**
 * The pattern step being heard now, or null when stopped. Steps arrive as
 * they are scheduled with their heard time; each animation frame shows the
 * latest one whose time has come.
 */
export function useStepPlayhead(playing: boolean): number | null {
  const [step, setStep] = useState<number | null>(null);
  const queue = useRef<QueuedStep[]>([]);

  useEffect(
    () =>
      subscribeSteps((event) => {
        queue.current = applyStepEvent(queue.current, event);
      }),
    [],
  );

  useEffect(() => {
    if (!playing) {
      setStep(null);
      return;
    }
    let handle = 0;
    const tick = () => {
      const { index, rest } = takeHeard(queue.current, performance.now());
      queue.current = rest;
      if (index !== undefined) setStep(index);
      handle = requestAnimationFrame(tick);
    };
    handle = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(handle);
  }, [playing]);

  return playing ? step : null;
}

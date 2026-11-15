import type { LoopRange } from "../../lib/stems";

/** Shortest loop the screen makes (the player ignores loops under 256 frames). */
export const MIN_LOOP_SECS = 0.1;

export type LoopEdge = "start" | "end";

/** A pointer drag on the waveform, from where it started (`from`, seconds). */
export type LoopGesture =
  | { kind: "select"; from: number }
  | { kind: "move"; from: number; origin: LoopRange }
  | { kind: "resize"; edge: LoopEdge; from: number; origin: LoopRange };

const TICK_STEPS_SECS = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800] as const;
const LONGEST_TICK_STEP_SECS = 3600;
/** Narrowest gap between two ruler labels. */
const MIN_TICK_GAP_PX = 56;
/** A label closer than this to the right edge would be cut off. */
const TICK_EDGE_PX = 24;
/** Float noise allowed when comparing a loop length with MIN_LOOP_SECS. */
const LENGTH_EPSILON = 1e-6;

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max);
}

/** Whether `range` is long enough to loop. An edge clamped to the shortest
 * loop comes out a hair under MIN_LOOP_SECS (72 − (72 − 0.1) is 0.0999… in floats). */
export function loopable(range: LoopRange): boolean {
  return range.end - range.start >= MIN_LOOP_SECS - LENGTH_EPSILON;
}

/** Ruler marks (seconds) for a song `duration` long drawn `width` px wide. */
export function rulerTicks(duration: number, width: number): number[] {
  if (!(duration > 0) || !(width > 0)) return [];
  const step =
    TICK_STEPS_SECS.find((seconds) => (seconds / duration) * width >= MIN_TICK_GAP_PX) ??
    LONGEST_TICK_STEP_SECS;
  const last = duration - (TICK_EDGE_PX / width) * duration;
  const ticks: number[] = [];
  for (let index = 0; index * step <= last; index++) ticks.push(index * step);
  return ticks;
}

/** `loop` with `edge` at `seconds`, kept inside the song and at least MIN_LOOP_SECS long. */
export function moveEdge(
  loop: LoopRange,
  edge: LoopEdge,
  seconds: number,
  duration: number,
): LoopRange {
  return edge === "start"
    ? { start: clamp(seconds, 0, loop.end - MIN_LOOP_SECS), end: loop.end }
    : { start: loop.start, end: clamp(seconds, loop.start + MIN_LOOP_SECS, duration) };
}

/** Moves `edge` by `delta` seconds from the time shown for it (a tenth of a second). */
export function stepEdge(
  loop: LoopRange,
  edge: LoopEdge,
  delta: number,
  duration: number,
): LoopRange {
  const shown = Math.round(loop[edge] * 10) / 10;
  return moveEdge(loop, edge, Math.round((shown + delta) * 10) / 10, duration);
}

/** Rough half width (px) of a ruler label, from its text at the fine-print size. */
export function labelHalfWidth(text: string, padding: number): number {
  return text.length * 3.6 + padding;
}

/**
 * Ruler marks whose labels would sit under a label `halfWidth` px either side
 * of `center` px (a ruler `width` px wide for a song `duration` long).
 */
export function coveredTicks(
  ticks: number[],
  labels: string[],
  center: number,
  halfWidth: number,
  duration: number,
  width: number,
): Set<number> {
  const covered = new Set<number>();
  if (!(duration > 0)) return covered;
  ticks.forEach((seconds, index) => {
    const half = labelHalfWidth(labels[index] ?? "", 2);
    const x = (seconds / duration) * width;
    // The first label starts at its mark; the others are centred on theirs.
    const [from, to] = seconds === 0 ? [0, 2 * half] : [x - half, x + half];
    if (from < center + halfWidth && to > center - halfWidth) covered.add(seconds);
  });
  return covered;
}

/** The loop a waveform drag makes with the pointer at `at` seconds. */
export function dragRange(gesture: LoopGesture, at: number, duration: number): LoopRange {
  switch (gesture.kind) {
    case "select":
      return { start: Math.min(gesture.from, at), end: Math.max(gesture.from, at) };
    case "move": {
      const length = gesture.origin.end - gesture.origin.start;
      const start = clamp(
        gesture.origin.start + at - gesture.from,
        0,
        Math.max(0, duration - length),
      );
      return { start, end: start + length };
    }
    case "resize":
      return moveEdge(
        gesture.origin,
        gesture.edge,
        gesture.origin[gesture.edge] + at - gesture.from,
        duration,
      );
  }
}

const TICK_STEPS_SECS = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800] as const;
const LONGEST_TICK_STEP_SECS = 3600;
/** Narrowest gap between two ruler labels. */
const MIN_TICK_GAP_PX = 56;
/** A label closer than this to the right edge would be cut off. */
const TICK_EDGE_PX = 24;

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

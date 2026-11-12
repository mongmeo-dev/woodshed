import { roundBpm } from "./model";

/** A pause longer than this starts a new tap sequence. */
export const TAP_RESET_MS = 2000;
/** Taps averaged at most. */
export const TAP_WINDOW = 6;
/** An interval this far (as a ratio) from the running average starts over. */
const TAP_JUMP = 0.4;

/**
 * Tap tempo: the average interval of the recent taps. A long pause or a
 * sudden change of pace starts over from the last two taps.
 */
export class TapTempo {
  private taps: number[] = [];

  /** Records a tap at `now` (ms) and returns the tempo, once there are two taps. */
  tap(now: number): number | null {
    const last = this.taps[this.taps.length - 1];
    if (last !== undefined && (now - last > TAP_RESET_MS || now <= last)) {
      this.taps = [];
    }
    const first = this.taps[0];
    const previous = this.taps[this.taps.length - 1];
    if (first !== undefined && previous !== undefined && this.taps.length >= 2) {
      const average = (previous - first) / (this.taps.length - 1);
      const interval = now - previous;
      if (Math.abs(interval - average) > average * TAP_JUMP) this.taps = [previous];
    }
    this.taps.push(now);
    if (this.taps.length > TAP_WINDOW) this.taps.shift();

    const start = this.taps[0];
    if (start === undefined || this.taps.length < 2) return null;
    const average = (now - start) / (this.taps.length - 1);
    return roundBpm(60_000 / average);
  }

  reset(): void {
    this.taps = [];
  }
}

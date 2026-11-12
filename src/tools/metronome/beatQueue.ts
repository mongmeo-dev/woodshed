import { heardAtToPerformanceTime } from "../../lib/audio";
import type { MetronomeMessage } from "./api";

export type BeatMessage = Extract<MetronomeMessage, { type: "beat" }>;
export type PulseMessage = Extract<MetronomeMessage, { type: "pulse" }>;

interface Item {
  /** `performance.now()` time at which it is heard. */
  at: number;
  message: BeatMessage | PulseMessage;
}

export interface Due {
  beat?: BeatMessage;
  pulse?: PulseMessage;
}

/**
 * Beats and pulses announced ahead of time, ordered by when they are heard.
 * The screen calls {@link BeatQueue.take} every animation frame and lights
 * whatever has become audible.
 */
export class BeatQueue {
  private items: Item[] = [];

  constructor(private readonly timeOrigin = performance.timeOrigin) {}

  get size(): number {
    return this.items.length;
  }

  push(message: BeatMessage | PulseMessage): void {
    const at = heardAtToPerformanceTime(message.heardAtMs, this.timeOrigin);
    let index = this.items.length;
    while (index > 0 && (this.items[index - 1]?.at ?? 0) > at) index -= 1;
    this.items.splice(index, 0, { at, message });
  }

  /** Drops what was announced at or after `fromFrame` (it will not play). */
  reset(fromFrame: number): void {
    this.items = this.items.filter((item) => item.message.frame < fromFrame);
  }

  clear(): void {
    this.items = [];
  }

  /** Removes everything heard by `now` and returns the latest beat and pulse. */
  take(now: number): Due {
    const due: Due = {};
    let count = 0;
    for (const item of this.items) {
      if (item.at > now) break;
      if (item.message.type === "beat") due.beat = item.message;
      else due.pulse = item.message;
      count += 1;
    }
    if (count > 0) this.items.splice(0, count);
    return due;
  }
}

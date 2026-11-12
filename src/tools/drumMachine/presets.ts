import {
  DEFAULT_VOLUME,
  emptyTrack,
  MAX_STEPS,
  type Pattern,
  type PatternLength,
  type StepLevel,
  type SwingUnit,
  TRACKS,
  type TrackId,
  type TrackPattern,
} from "./drums";

export const PRESET_IDS = [
  "rock",
  "funk",
  "shuffle",
  "halfTimeShuffle",
  "blues",
  "jazzSwing",
  "bossaNova",
  "reggae",
  "hipHop",
  "metal",
  "ballad",
  "disco",
] as const;
export type PresetId = (typeof PRESET_IDS)[number];

/**
 * One row per track, one character per sixteenth: `.` rest, `g` soft (ghost),
 * `x` normal, `X` accent. A 16-character row repeats in a 32-step pattern.
 */
interface PresetSpec {
  bpm: number;
  swing: number;
  swingUnit: SwingUnit;
  length: PatternLength;
  rows: Partial<Record<TrackId, string>>;
}

export const PRESET_SPECS: Record<PresetId, PresetSpec> = {
  // Straight eighths on the hi-hat, backbeat on 2 and 4.
  rock: {
    bpm: 110,
    swing: 50,
    swingUnit: "eighth",
    length: 16,
    rows: {
      closedHat: "X.x.X.x.X.x.X.x.",
      snare: "....X.......X...",
      kick: "x.......x.x.....",
    },
  },
  // Sixteenth hats, ghost notes around the backbeat, a lightly swung feel.
  funk: {
    bpm: 98,
    swing: 54,
    swingUnit: "sixteenth",
    length: 16,
    rows: {
      closedHat: "XgxgXgxgXgxgXg.g",
      openHat: "..............x.",
      snare: "....X..g.g..X..g",
      kick: "x..x......x..x..",
    },
  },
  // Triplet-swung eighths.
  shuffle: {
    bpm: 120,
    swing: 67,
    swingUnit: "eighth",
    length: 16,
    rows: {
      closedHat: "X.x.X.x.X.x.X.x.",
      snare: "....X.......X...",
      kick: "x.....x.x.....x.",
    },
  },
  // Swung sixteenths at half time: backbeat on 3, ghost notes on the
  // triplet "lets".
  halfTimeShuffle: {
    bpm: 86,
    swing: 67,
    swingUnit: "sixteenth",
    length: 16,
    rows: {
      closedHat: "XxXxXxXxXxXxXxXx",
      snare: ".g.g.g.gX..g.g.g",
      kick: "x.....x.......x.",
    },
  },
  // Slow 12/8: with 67% eighth swing, the first three sixteenths of each
  // beat fall on its triplets.
  blues: {
    bpm: 66,
    swing: 67,
    swingUnit: "eighth",
    length: 16,
    rows: {
      closedHat: "Xxx.Xxx.Xxx.Xxx.",
      snare: "....X.......X...",
      kick: "x.......x.....x.",
    },
  },
  // Spang-a-lang ride, hi-hat foot on 2 and 4, feathered kick.
  jazzSwing: {
    bpm: 150,
    swing: 67,
    swingUnit: "eighth",
    length: 16,
    rows: {
      ride: "x...X.x.x...X.x.",
      closedHat: "....x.......x...",
      kick: "g...g...g...g...",
      snare: "..........g.....",
    },
  },
  // Two bars: bossa clave on the side stick over a steady bass drum.
  bossaNova: {
    bpm: 132,
    swing: 50,
    swingUnit: "eighth",
    length: 32,
    rows: {
      closedHat: "x.x.x.x.x.x.x.x.",
      sidestick: "x.....x.....x.......x.....x.....",
      kick: "x..x....x..x....",
    },
  },
  // One drop: kick and rim shot together on 3, offbeat hi-hat.
  reggae: {
    bpm: 74,
    swing: 58,
    swingUnit: "eighth",
    length: 16,
    rows: {
      closedHat: "x.X.x.X.x.X.x.X.",
      sidestick: "........X.......",
      kick: "........X.......",
    },
  },
  // Boom bap with lazy sixteenths.
  hipHop: {
    bpm: 90,
    swing: 56,
    swingUnit: "sixteenth",
    length: 16,
    rows: {
      closedHat: "xgxgxgxgxgxgxgxg",
      snare: "....X.......X...",
      clap: "....x.......x...",
      kick: "x......x..x.....",
    },
  },
  // Double bass sixteenths under a quarter-note ride and a crash on 1.
  metal: {
    bpm: 160,
    swing: 50,
    swingUnit: "eighth",
    length: 16,
    rows: {
      crash: "X...............",
      ride: "....x...x...x...",
      snare: "....X.......X...",
      kick: "xxxxxxxxxxxxxxxx",
    },
  },
  // Cross-stick backbeat, soft eighths.
  ballad: {
    bpm: 68,
    swing: 50,
    swingUnit: "eighth",
    length: 16,
    rows: {
      closedHat: "x.g.x.g.x.g.x.g.",
      sidestick: "....x.......x...",
      kick: "x.........x.....",
    },
  },
  // Four on the floor, open hats on the offbeats choked by the closed hat.
  disco: {
    bpm: 120,
    swing: 50,
    swingUnit: "eighth",
    length: 16,
    rows: {
      openHat: "..x...x...x...x.",
      closedHat: "x...x...x...x...",
      clap: "....x.......x...",
      snare: "....X.......X...",
      kick: "x...x...x...x...",
    },
  },
};

const LEVELS: Record<string, StepLevel> = { ".": 0, g: 1, x: 2, X: 3 };

/** Parses a row; throws on characters outside `.gxX` or a bad length. */
export function parseRow(row: string, length: PatternLength): StepLevel[] {
  if (row.length !== length && row.length !== 16) {
    throw new Error(`row "${row}" must have 16 or ${length} steps`);
  }
  const steps = Array<StepLevel>(MAX_STEPS).fill(0);
  for (let step = 0; step < length; step += 1) {
    const char = row[step % row.length] ?? ".";
    const level = LEVELS[char];
    if (level === undefined) throw new Error(`unknown step "${char}" in "${row}"`);
    steps[step] = level;
  }
  return steps;
}

/** The preset as a pattern at default mix (every track at default volume, unmuted). */
export function presetPattern(id: PresetId): Pattern {
  const spec = PRESET_SPECS[id];
  const tracks = {} as Record<TrackId, TrackPattern>;
  for (const track of TRACKS) {
    const row = spec.rows[track];
    tracks[track] = row
      ? { steps: parseRow(row, spec.length), volume: DEFAULT_VOLUME, muted: false }
      : emptyTrack();
  }
  return {
    length: spec.length,
    bpm: spec.bpm,
    swing: spec.swing,
    swingUnit: spec.swingUnit,
    tracks,
  };
}

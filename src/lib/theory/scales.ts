import { type Interval, parseInterval } from "./intervals";
import {
  chromaticName,
  formatNote,
  mod12,
  type PitchClass,
  pitchClassOf,
  type SpelledNote,
  spellAt,
} from "./notes";

/** Scale definitions as degree names from the root. */
export const SCALE_DEFINITIONS = {
  major: ["1", "2", "3", "4", "5", "6", "7"],
  naturalMinor: ["1", "2", "b3", "4", "5", "b6", "b7"],
  harmonicMinor: ["1", "2", "b3", "4", "5", "b6", "7"],
  melodicMinor: ["1", "2", "b3", "4", "5", "6", "7"],
  majorPentatonic: ["1", "2", "3", "5", "6"],
  minorPentatonic: ["1", "b3", "4", "5", "b7"],
  blues: ["1", "b3", "4", "b5", "5", "b7"],
  ionian: ["1", "2", "3", "4", "5", "6", "7"],
  dorian: ["1", "2", "b3", "4", "5", "6", "b7"],
  phrygian: ["1", "b2", "b3", "4", "5", "b6", "b7"],
  lydian: ["1", "2", "3", "#4", "5", "6", "7"],
  mixolydian: ["1", "2", "3", "4", "5", "6", "b7"],
  aeolian: ["1", "2", "b3", "4", "5", "b6", "b7"],
  locrian: ["1", "b2", "b3", "4", "b5", "b6", "b7"],
  majorBlues: ["1", "2", "b3", "3", "5", "6"],
  phrygianDominant: ["1", "b2", "3", "4", "5", "b6", "b7"],
  lydianDominant: ["1", "2", "3", "#4", "5", "6", "b7"],
  wholeTone: ["1", "2", "3", "#4", "#5", "b7"],
} as const satisfies Record<string, readonly string[]>;

export type ScaleId = keyof typeof SCALE_DEFINITIONS;

export const SCALE_GROUPS = [
  { id: "basic", scales: ["major", "naturalMinor", "harmonicMinor", "melodicMinor"] },
  { id: "pentatonic", scales: ["majorPentatonic", "minorPentatonic", "blues", "majorBlues"] },
  {
    id: "modes",
    scales: ["ionian", "dorian", "phrygian", "lydian", "mixolydian", "aeolian", "locrian"],
  },
  { id: "other", scales: ["phrygianDominant", "lydianDominant", "wholeTone"] },
] as const satisfies readonly { id: string; scales: readonly ScaleId[] }[];

export type ScaleGroupId = (typeof SCALE_GROUPS)[number]["id"];

export const SCALE_IDS = Object.keys(SCALE_DEFINITIONS) as ScaleId[];

export function isScaleId(value: unknown): value is ScaleId {
  return typeof value === "string" && Object.hasOwn(SCALE_DEFINITIONS, value);
}

export function scaleIntervals(scale: ScaleId): Interval[] {
  return SCALE_DEFINITIONS[scale].map(parseInterval);
}

export interface ScaleNote {
  /** Note name with ASCII accidentals ("Db", "F#"). */
  name: string;
  /** Degree name ("1", "b3"). */
  interval: string;
  pitchClass: PitchClass;
}

export interface SpelledScale {
  scale: ScaleId;
  root: PitchClass;
  notes: ScaleNote[];
}

/** Spells every scale tone from `root`, one letter per degree. */
export function spellFrom(root: SpelledNote, scale: ScaleId): ScaleNote[] {
  const rootPitchClass = pitchClassOf(root);
  return scaleIntervals(scale).map((interval) => {
    const note = spellAt(root, interval.degree, mod12(rootPitchClass + interval.semitones));
    return { name: formatNote(note), interval: interval.name, pitchClass: pitchClassOf(note) };
  });
}

/** Used only to break ties between equally cheap spellings of a root. */
const DEFAULT_ROOT_NAMES = ["C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];

/** A single accidental costs 1; a double accidental is avoided whenever possible. */
function spellingCost(notes: readonly ScaleNote[]): number {
  return notes.reduce((cost, note) => {
    const marks = note.name.length - 1;
    return cost + (marks === 0 ? 0 : marks === 1 ? 1 : 10);
  }, 0);
}

function rootCandidates(root: PitchClass): SpelledNote[] {
  const sharp = chromaticName(root, "sharp");
  const flat = chromaticName(root, "flat");
  const names = sharp === flat ? [sharp] : [sharp, flat];
  return names.map((name) => ({
    letter: name.charAt(0) as SpelledNote["letter"],
    accidental: name.length === 1 ? 0 : name.endsWith("#") ? 1 : -1,
  }));
}

/**
 * Spells the scale from a pitch-class root, choosing the sharp or flat name of
 * the root that gives the fewest accidentals (C# minor, Db major, Bb minor).
 * Equal cost falls back to F#, Eb, Ab, Bb, Db.
 */
export function spellScale(root: PitchClass, scale: ScaleId): SpelledScale {
  const pitchClass = mod12(root);
  const preferred = DEFAULT_ROOT_NAMES[pitchClass];
  let best: { notes: ScaleNote[]; cost: number } | null = null;
  for (const candidate of rootCandidates(pitchClass)) {
    const notes = spellFrom(candidate, scale);
    const cost = spellingCost(notes);
    const wins =
      best === null ||
      cost < best.cost ||
      (cost === best.cost && formatNote(candidate) === preferred);
    if (wins) best = { notes, cost };
  }
  return { scale, root: pitchClass, notes: (best as { notes: ScaleNote[] }).notes };
}

import { chromaticName, mod12, type PitchClass } from "./notes";

export const MAX_FRET = 24;

/** Open-string pitches are MIDI note numbers (E2 = 40, middle C = 60). */
export const INSTRUMENTS = {
  guitar6: { strings: [40, 45, 50, 55, 59, 64] }, // E A D G B E
  bass4: { strings: [28, 33, 38, 43] }, // E A D G
  bass5: { strings: [23, 28, 33, 38, 43] }, // B E A D G
} as const satisfies Record<string, { strings: readonly number[] }>;

export type InstrumentId = keyof typeof INSTRUMENTS;

export const INSTRUMENT_IDS = Object.keys(INSTRUMENTS) as InstrumentId[];

export function isInstrumentId(value: unknown): value is InstrumentId {
  return typeof value === "string" && Object.hasOwn(INSTRUMENTS, value);
}

/** Open-string MIDI notes, lowest string first. */
export function openStrings(instrument: InstrumentId): readonly number[] {
  return INSTRUMENTS[instrument].strings;
}

/** MIDI note sounding at `fret` of string `string` (0 = lowest string). */
export function midiAt(instrument: InstrumentId, string: number, fret: number): number {
  const open = openStrings(instrument)[string];
  if (open === undefined) throw new RangeError(`No string ${string} on ${instrument}`);
  if (!Number.isInteger(fret) || fret < 0 || fret > MAX_FRET) {
    throw new RangeError(`Fret out of range: ${fret}`);
  }
  return open + fret;
}

export function pitchClassAt(instrument: InstrumentId, string: number, fret: number): PitchClass {
  return mod12(midiAt(instrument, string, fret));
}

/** Natural name of an open string ("E", "B"), for the string label. */
export function openStringName(instrument: InstrumentId, string: number): string {
  return chromaticName(pitchClassAt(instrument, string, 0), "sharp");
}

/** Frets that carry a position marker, and those with the double marker. */
export const SINGLE_MARKER_FRETS: readonly number[] = [3, 5, 7, 9, 15, 17, 19, 21];
export const DOUBLE_MARKER_FRETS: readonly number[] = [12, 24];

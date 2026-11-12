import {
  type InstrumentId,
  openStringName,
  openStrings,
  pitchClassAt,
  type ScaleId,
  type SpelledScale,
  spellScale,
} from "../../lib/theory";
import type { DisplayMode } from "./preferences";

export interface BoardMarker {
  /** Text on the marker: the note name or the interval degree, ASCII accidentals. */
  label: string;
  note: string;
  interval: string;
  root: boolean;
}

export interface BoardString {
  /** Index from the lowest string (0). */
  index: number;
  /** String number as players count it: 1 is the highest string. */
  number: number;
  /** Natural name of the open string, for the string label. */
  name: string;
  /** One entry per fret, 0 (open) to `frets`; null where the note is outside the scale. */
  markers: (BoardMarker | null)[];
}

export interface Board {
  scale: SpelledScale;
  /** Highest string first, as in tablature. */
  strings: BoardString[];
}

/** Scale tones on every string of the instrument, frets 0 through `frets`. */
export function buildBoard(
  instrument: InstrumentId,
  key: number,
  scaleId: ScaleId,
  frets: number,
  display: DisplayMode,
): Board {
  const scale = spellScale(key, scaleId);
  const byPitchClass = new Map(scale.notes.map((note) => [note.pitchClass, note]));
  const open = openStrings(instrument);
  const strings = open.map((_, index): BoardString => {
    const markers = Array.from({ length: frets + 1 }, (_fret, fret): BoardMarker | null => {
      const note = byPitchClass.get(pitchClassAt(instrument, index, fret));
      if (!note) return null;
      return {
        label: display === "notes" ? note.name : note.interval,
        note: note.name,
        interval: note.interval,
        root: note.pitchClass === scale.root,
      };
    });
    return { index, number: open.length - index, name: openStringName(instrument, index), markers };
  });
  return { scale, strings: strings.reverse() };
}

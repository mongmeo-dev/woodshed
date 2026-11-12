/** Semitones above the root for degrees 1–7 of the major scale. */
const MAJOR_SEMITONES = [0, 2, 4, 5, 7, 9, 11] as const;

export interface Interval {
  /** Degree name such as "1", "b3", "#4"; the label shown in interval view. */
  name: string;
  /** Scale degree, 1–7: decides the letter of the spelled note. */
  degree: number;
  /** Distance from the root in semitones, 0–11. */
  semitones: number;
}

/** Parses a degree name: an optional `bb`, `b`, `#` or `##` followed by 1–7. */
export function parseInterval(name: string): Interval {
  const match = /^(bb|b|#|##)?([1-7])$/.exec(name);
  if (!match) throw new Error(`Not an interval name: ${name}`);
  const degree = Number(match[2]);
  const mark = match[1] ?? "";
  const offset = mark === "" ? 0 : mark.startsWith("#") ? mark.length : -mark.length;
  const semitones = ((MAJOR_SEMITONES[degree - 1] as number) + offset + 12) % 12;
  return { name, degree, semitones };
}

/** Pitch class: 0 = C … 11 = B. */
export type PitchClass = number;

export const LETTERS = ["C", "D", "E", "F", "G", "A", "B"] as const;
export type Letter = (typeof LETTERS)[number];

/** Pitch class of each natural note. */
export const NATURAL_PITCH_CLASS: Record<Letter, PitchClass> = {
  C: 0,
  D: 2,
  E: 4,
  F: 5,
  G: 7,
  A: 9,
  B: 11,
};

/** A note spelled with a letter and an accidental: -2 (bb) … +2 (##). */
export interface SpelledNote {
  letter: Letter;
  accidental: number;
}

export type AccidentalPreference = "sharp" | "flat";

const SHARP_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"] as const;
const FLAT_NAMES = ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"] as const;

export function mod12(value: number): PitchClass {
  return ((value % 12) + 12) % 12;
}

export function pitchClassOf(note: SpelledNote): PitchClass {
  return mod12(NATURAL_PITCH_CLASS[note.letter] + note.accidental);
}

export function formatNote(note: SpelledNote): string {
  const accidental =
    note.accidental > 0 ? "#".repeat(note.accidental) : "b".repeat(-note.accidental);
  return `${note.letter}${accidental}`;
}

/** Parses "C", "F#", "Bb", "G##", "Ebb". */
export function parseNote(text: string): SpelledNote {
  const match = /^([A-G])(#{0,2}|b{0,2})$/.exec(text);
  if (!match) throw new Error(`Not a note name: ${text}`);
  const letter = match[1] as Letter;
  const marks = match[2] ?? "";
  return { letter, accidental: marks.startsWith("#") ? marks.length : -marks.length };
}

/** Chromatic spelling of a pitch class when no key is known. */
export function chromaticName(pitchClass: PitchClass, preference: AccidentalPreference): string {
  const names = preference === "sharp" ? SHARP_NAMES : FLAT_NAMES;
  return names[mod12(pitchClass)] as string;
}

/** Spells `target` with the letter `degree` steps above `root`'s letter (1 = same letter). */
export function spellAt(root: SpelledNote, degree: number, target: PitchClass): SpelledNote {
  const letter = LETTERS[(LETTERS.indexOf(root.letter) + degree - 1) % 7] as Letter;
  const accidental = mod12(target - NATURAL_PITCH_CLASS[letter] + 6) - 6;
  return { letter, accidental };
}

/** Typographic accidentals for display: "Bb" → "B♭", "F#" → "F♯", "b3" → "♭3". */
export function displayName(name: string): string {
  return name.replace(/#/g, "♯").replace(/b/g, "♭");
}

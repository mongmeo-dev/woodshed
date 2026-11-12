import {
  chromaticName,
  formatNote,
  isScaleId,
  type PitchClass,
  parseNote,
  pitchClassOf,
  SCALE_GROUPS,
  SCALE_IDS,
  type ScaleId,
  scaleIntervals,
  spellFrom,
  spellScale,
} from ".";

/** Semitone pattern of every scale, written out by hand. */
const SEMITONES: Record<ScaleId, number[]> = {
  major: [0, 2, 4, 5, 7, 9, 11],
  naturalMinor: [0, 2, 3, 5, 7, 8, 10],
  harmonicMinor: [0, 2, 3, 5, 7, 8, 11],
  melodicMinor: [0, 2, 3, 5, 7, 9, 11],
  majorPentatonic: [0, 2, 4, 7, 9],
  minorPentatonic: [0, 3, 5, 7, 10],
  blues: [0, 3, 5, 6, 7, 10],
  ionian: [0, 2, 4, 5, 7, 9, 11],
  dorian: [0, 2, 3, 5, 7, 9, 10],
  phrygian: [0, 1, 3, 5, 7, 8, 10],
  lydian: [0, 2, 4, 6, 7, 9, 11],
  mixolydian: [0, 2, 4, 5, 7, 9, 10],
  aeolian: [0, 2, 3, 5, 7, 8, 10],
  locrian: [0, 1, 3, 5, 6, 8, 10],
  majorBlues: [0, 2, 3, 4, 7, 9],
  phrygianDominant: [0, 1, 4, 5, 7, 8, 10],
  lydianDominant: [0, 2, 4, 6, 7, 9, 10],
  wholeTone: [0, 2, 4, 6, 8, 10],
};

/** Interval names of every scale, written out by hand. */
const DEGREES: Record<ScaleId, string[]> = {
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
};

const KEYS = Array.from({ length: 12 }, (_, pitchClass) => pitchClass);

function names(root: PitchClass, scale: ScaleId): string[] {
  return spellScale(root, scale).notes.map((note) => note.name);
}

describe("scale catalog", () => {
  test("lists the requested scales and nothing without a definition", () => {
    expect([...SCALE_IDS].sort()).toEqual(Object.keys(SEMITONES).sort());
    for (const id of [
      "major",
      "naturalMinor",
      "harmonicMinor",
      "melodicMinor",
      "majorPentatonic",
      "minorPentatonic",
      "blues",
      "ionian",
      "dorian",
      "phrygian",
      "lydian",
      "mixolydian",
      "aeolian",
      "locrian",
    ]) {
      expect(isScaleId(id)).toBe(true);
    }
    expect(isScaleId("nope")).toBe(false);
    expect(isScaleId("toString")).toBe(false);
  });

  test("every scale belongs to exactly one group", () => {
    const grouped = SCALE_GROUPS.flatMap((group) => [...group.scales]);
    expect(grouped.sort()).toEqual([...SCALE_IDS].sort());
  });

  test("interval names and semitones match the written-out tables", () => {
    for (const id of SCALE_IDS) {
      const intervals = scaleIntervals(id);
      expect(intervals.map((interval) => interval.name)).toEqual(DEGREES[id]);
      expect(intervals.map((interval) => interval.semitones)).toEqual(SEMITONES[id]);
    }
  });
});

describe("scale tones in all 12 keys", () => {
  test.each(SCALE_IDS)("%s: pitch classes and interval names in every key", (scale) => {
    for (const root of KEYS) {
      const spelled = spellScale(root, scale);
      expect(spelled.root).toBe(root);
      expect(spelled.notes.map((note) => note.interval)).toEqual(DEGREES[scale]);
      expect(spelled.notes.map((note) => note.pitchClass)).toEqual(
        SEMITONES[scale].map((semitones) => (root + semitones) % 12),
      );
      // The printed name always agrees with the pitch class it stands for.
      for (const note of spelled.notes) {
        expect(pitchClassOf(parseNote(note.name))).toBe(note.pitchClass);
      }
    }
  });

  test.each(SCALE_IDS)("%s: root of the scale is the key's own note", (scale) => {
    for (const root of KEYS) {
      const first = spellScale(root, scale).notes[0];
      expect(first?.pitchClass).toBe(root);
      expect(first?.interval).toBe("1");
    }
  });

  test.each(SCALE_IDS)("%s: each degree takes its own letter", (scale) => {
    for (const root of KEYS) {
      const letters = names(root, scale).map((name) => name.charAt(0));
      const expected = scaleIntervals(scale).map(
        (interval) =>
          "CDEFGAB"[("CDEFGAB".indexOf(letters[0] as string) + interval.degree - 1) % 7],
      );
      expect(letters).toEqual(expected);
    }
  });

  test("the seven-note diatonic scales never need a double accidental", () => {
    const diatonic: ScaleId[] = [
      "major",
      "naturalMinor",
      "ionian",
      "dorian",
      "phrygian",
      "lydian",
      "mixolydian",
      "aeolian",
      "locrian",
    ];
    for (const scale of diatonic) {
      for (const root of KEYS) {
        for (const name of names(root, scale)) expect(name).not.toMatch(/##|bb/);
      }
    }
  });

  test("no scale tone ever gets a triple accidental", () => {
    for (const scale of SCALE_IDS) {
      for (const root of KEYS) {
        for (const name of names(root, scale)) expect(name).toMatch(/^[A-G](#{0,2}|b{0,2})$/);
      }
    }
  });
});

describe("enharmonic spelling follows the key", () => {
  const MAJOR: Record<number, string[]> = {
    0: ["C", "D", "E", "F", "G", "A", "B"],
    1: ["Db", "Eb", "F", "Gb", "Ab", "Bb", "C"],
    2: ["D", "E", "F#", "G", "A", "B", "C#"],
    3: ["Eb", "F", "G", "Ab", "Bb", "C", "D"],
    4: ["E", "F#", "G#", "A", "B", "C#", "D#"],
    5: ["F", "G", "A", "Bb", "C", "D", "E"],
    6: ["F#", "G#", "A#", "B", "C#", "D#", "E#"],
    7: ["G", "A", "B", "C", "D", "E", "F#"],
    8: ["Ab", "Bb", "C", "Db", "Eb", "F", "G"],
    9: ["A", "B", "C#", "D", "E", "F#", "G#"],
    10: ["Bb", "C", "D", "Eb", "F", "G", "A"],
    11: ["B", "C#", "D#", "E", "F#", "G#", "A#"],
  };

  const NATURAL_MINOR: Record<number, string[]> = {
    0: ["C", "D", "Eb", "F", "G", "Ab", "Bb"],
    1: ["C#", "D#", "E", "F#", "G#", "A", "B"],
    2: ["D", "E", "F", "G", "A", "Bb", "C"],
    3: ["Eb", "F", "Gb", "Ab", "Bb", "Cb", "Db"],
    4: ["E", "F#", "G", "A", "B", "C", "D"],
    5: ["F", "G", "Ab", "Bb", "C", "Db", "Eb"],
    6: ["F#", "G#", "A", "B", "C#", "D", "E"],
    7: ["G", "A", "Bb", "C", "D", "Eb", "F"],
    8: ["G#", "A#", "B", "C#", "D#", "E", "F#"],
    9: ["A", "B", "C", "D", "E", "F", "G"],
    10: ["Bb", "C", "Db", "Eb", "F", "Gb", "Ab"],
    11: ["B", "C#", "D", "E", "F#", "G", "A"],
  };

  test.each(KEYS)("major in key %i uses the standard key signature", (root) => {
    expect(names(root, "major")).toEqual(MAJOR[root]);
    expect(names(root, "ionian")).toEqual(MAJOR[root]);
  });

  test.each(KEYS)("natural minor in key %i uses the standard key signature", (root) => {
    expect(names(root, "naturalMinor")).toEqual(NATURAL_MINOR[root]);
    expect(names(root, "aeolian")).toEqual(NATURAL_MINOR[root]);
  });

  test("the same pitch class is a flat key for major and a sharp key for minor", () => {
    expect(names(1, "major")[0]).toBe("Db");
    expect(names(1, "naturalMinor")[0]).toBe("C#");
    expect(names(8, "major")[0]).toBe("Ab");
    expect(names(8, "naturalMinor")[0]).toBe("G#");
    expect(names(10, "major")[0]).toBe("Bb");
    expect(names(10, "naturalMinor")[0]).toBe("Bb");
  });

  test("equally cheap spellings settle on F# and Eb", () => {
    expect(names(6, "major")[0]).toBe("F#");
    expect(names(3, "naturalMinor")[0]).toBe("Eb");
  });

  test("black-key pentatonic and blues scales pick the cheaper spelling", () => {
    expect(names(10, "minorPentatonic")).toEqual(["Bb", "Db", "Eb", "F", "Ab"]);
    expect(names(1, "minorPentatonic")).toEqual(["C#", "E", "F#", "G#", "B"]);
    expect(names(6, "blues")).toEqual(["F#", "A", "B", "C", "C#", "E"]);
  });

  test("chromatic names pick # or b by preference", () => {
    expect(chromaticName(1, "sharp")).toBe("C#");
    expect(chromaticName(1, "flat")).toBe("Db");
    expect(chromaticName(13, "flat")).toBe("Db");
    expect(chromaticName(-2, "sharp")).toBe("A#");
    expect(chromaticName(7, "sharp")).toBe("G");
  });

  test("formatNote prints accidentals and parseNote reads them back", () => {
    for (const text of ["C", "F#", "Bb", "G##", "Ebb"]) {
      expect(formatNote(parseNote(text))).toBe(text);
    }
    expect(() => parseNote("H")).toThrow();
    expect(() => parseNote("C###")).toThrow();
  });
});

describe("well-known scales", () => {
  test.each([
    [9, "harmonicMinor", ["A", "B", "C", "D", "E", "F", "G#"]],
    [9, "melodicMinor", ["A", "B", "C", "D", "E", "F#", "G#"]],
    [0, "majorPentatonic", ["C", "D", "E", "G", "A"]],
    [9, "minorPentatonic", ["A", "C", "D", "E", "G"]],
    [0, "blues", ["C", "Eb", "F", "Gb", "G", "Bb"]],
    [9, "blues", ["A", "C", "D", "Eb", "E", "G"]],
    [0, "majorBlues", ["C", "D", "Eb", "E", "G", "A"]],
    [2, "dorian", ["D", "E", "F", "G", "A", "B", "C"]],
    [4, "phrygian", ["E", "F", "G", "A", "B", "C", "D"]],
    [5, "lydian", ["F", "G", "A", "B", "C", "D", "E"]],
    [7, "mixolydian", ["G", "A", "B", "C", "D", "E", "F"]],
    [11, "locrian", ["B", "C", "D", "E", "F", "G", "A"]],
    [4, "phrygianDominant", ["E", "F", "G#", "A", "B", "C", "D"]],
    [0, "lydianDominant", ["C", "D", "E", "F#", "G", "A", "Bb"]],
    [0, "wholeTone", ["C", "D", "E", "F#", "G#", "Bb"]],
    [2, "locrian", ["D", "Eb", "F", "G", "Ab", "Bb", "C"]],
  ] as [PitchClass, ScaleId, string[]][])("key %i %s", (root, scale, expected) => {
    expect(names(root, scale)).toEqual(expected);
  });
});

describe("mode notation", () => {
  // Each mode is the major scale of its parent key, started on another degree:
  // C# Dorian = B major from C#, G# Phrygian = E major from G#, and so on.
  test.each([
    [1, "dorian", ["C#", "D#", "E", "F#", "G#", "A#", "B"]],
    [8, "phrygian", ["G#", "A", "B", "C#", "D#", "E", "F#"]],
    [6, "mixolydian", ["F#", "G#", "A#", "B", "C#", "D#", "E"]],
    [3, "mixolydian", ["Eb", "F", "G", "Ab", "Bb", "C", "Db"]],
    [6, "locrian", ["F#", "G", "A", "B", "C", "D", "E"]],
    [1, "lydian", ["Db", "Eb", "F", "G", "Ab", "Bb", "C"]],
    [10, "lydian", ["Bb", "C", "D", "E", "F", "G", "A"]],
  ] as [PitchClass, ScaleId, string[]][])("key %i %s", (root, scale, expected) => {
    expect(names(root, scale)).toEqual(expected);
  });

  test("the key picker spells key 6 Lydian from Gb, the root with fewer accidentals", () => {
    // Db major from Gb: five accidentals. F# Lydian would need seven (B# and E#).
    expect(names(6, "lydian")).toEqual(["Gb", "Ab", "Bb", "C", "Db", "Eb", "F"]);
  });

  test("an explicitly spelled sharp root keeps its sharps, including E# and B#", () => {
    const from = (root: string, scale: ScaleId) =>
      spellFrom(parseNote(root), scale).map((note) => note.name);
    // C# major from F#.
    expect(from("F#", "lydian")).toEqual(["F#", "G#", "A#", "B#", "C#", "D#", "E#"]);
    // C# major from D#.
    expect(from("D#", "dorian")).toEqual(["D#", "E#", "F#", "G#", "A#", "B#", "C#"]);
    // The same pitch class spelled from the flat root is a different, cheaper notation.
    expect(from("Gb", "lydian")).toEqual(["Gb", "Ab", "Bb", "C", "Db", "Eb", "F"]);
  });
});

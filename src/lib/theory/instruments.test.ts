import {
  chromaticName,
  DOUBLE_MARKER_FRETS,
  INSTRUMENT_IDS,
  type InstrumentId,
  isInstrumentId,
  MAX_FRET,
  midiAt,
  openStringName,
  openStrings,
  pitchClassAt,
  SINGLE_MARKER_FRETS,
} from ".";

/** Note names of frets 0–12 on a string, written out per open note (sharp spelling). */
const CHROMATIC_FROM: Record<string, string[]> = {
  B: ["B", "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"],
  E: ["E", "F", "F#", "G", "G#", "A", "A#", "B", "C", "C#", "D", "D#", "E"],
  A: ["A", "A#", "B", "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A"],
  D: ["D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B", "C", "C#", "D"],
  G: ["G", "G#", "A", "A#", "B", "C", "C#", "D", "D#", "E", "F", "F#", "G"],
};

/** Open strings from the lowest, as the instrument is tuned. */
const TUNING: Record<InstrumentId, string[]> = {
  guitar6: ["E", "A", "D", "G", "B", "E"],
  bass4: ["E", "A", "D", "G"],
  bass5: ["B", "E", "A", "D", "G"],
};

function sharpNameAt(instrument: InstrumentId, string: number, fret: number): string {
  return chromaticName(pitchClassAt(instrument, string, fret), "sharp");
}

describe("instrument tunings", () => {
  test("lists guitar 6-string, bass 4-string and bass 5-string", () => {
    expect([...INSTRUMENT_IDS].sort()).toEqual(["bass4", "bass5", "guitar6"]);
    expect(isInstrumentId("bass5")).toBe(true);
    expect(isInstrumentId("ukulele")).toBe(false);
    expect(isInstrumentId("toString")).toBe(false);
  });

  test("guitar is E A D G B E, bass is E A D G, 5-string bass is B E A D G", () => {
    for (const id of INSTRUMENT_IDS) {
      const names = openStrings(id).map((_, string) => openStringName(id, string));
      expect(names).toEqual(TUNING[id]);
    }
  });

  test("string counts", () => {
    expect(openStrings("guitar6")).toHaveLength(6);
    expect(openStrings("bass4")).toHaveLength(4);
    expect(openStrings("bass5")).toHaveLength(5);
  });

  test("open strings sit at their standard octaves", () => {
    expect(openStrings("guitar6")).toEqual([40, 45, 50, 55, 59, 64]); // E2 A2 D3 G3 B3 E4
    expect(openStrings("bass4")).toEqual([28, 33, 38, 43]); // E1 A1 D2 G2
    expect(openStrings("bass5")).toEqual([23, 28, 33, 38, 43]); // B0 E1 A1 D2 G2
  });

  test("a bass sounds an octave below the matching guitar strings", () => {
    expect(openStrings("bass4")).toEqual(
      openStrings("guitar6")
        .slice(0, 4)
        .map((n) => n - 12),
    );
    expect(openStrings("bass5").slice(1)).toEqual(openStrings("bass4"));
  });
});

describe("note names on every string and fret", () => {
  for (const id of INSTRUMENT_IDS) {
    test(`${id}: frets 0–12 of every string follow the chromatic scale from the open note`, () => {
      TUNING[id].forEach((open, string) => {
        const expected = CHROMATIC_FROM[open] as string[];
        for (let fret = 0; fret <= 12; fret++) {
          expect(sharpNameAt(id, string, fret)).toBe(expected[fret]);
        }
      });
    });

    test(`${id}: the same name repeats an octave up, to fret ${MAX_FRET}`, () => {
      TUNING[id].forEach((_open, string) => {
        for (let fret = 0; fret <= 12; fret++) {
          expect(sharpNameAt(id, string, fret + 12)).toBe(sharpNameAt(id, string, fret));
          expect(midiAt(id, string, fret + 12)).toBe(midiAt(id, string, fret) + 12);
        }
      });
    });
  }

  test("guitar low E string, frets 0 to 12", () => {
    const names = Array.from({ length: 13 }, (_, fret) => sharpNameAt("guitar6", 0, fret));
    expect(names).toEqual(["E", "F", "F#", "G", "G#", "A", "A#", "B", "C", "C#", "D", "D#", "E"]);
  });

  test("guitar high E string, 5th fret is A and 12th fret is E", () => {
    expect(sharpNameAt("guitar6", 5, 5)).toBe("A");
    expect(sharpNameAt("guitar6", 5, 12)).toBe("E");
  });

  test("guitar: fret 5 matches the next open string, except G to B at fret 4", () => {
    for (let string = 0; string < 5; string++) {
      const fret = string === 3 ? 4 : 5;
      expect(pitchClassAt("guitar6", string, fret)).toBe(pitchClassAt("guitar6", string + 1, 0));
    }
  });

  test("bass 4-string: fret 5 matches the next open string", () => {
    for (let string = 0; string < 3; string++) {
      expect(pitchClassAt("bass4", string, 5)).toBe(pitchClassAt("bass4", string + 1, 0));
    }
  });

  test("bass 5-string: fret 5 matches the next open string, B string reaches E at fret 5", () => {
    for (let string = 0; string < 4; string++) {
      expect(pitchClassAt("bass5", string, 5)).toBe(pitchClassAt("bass5", string + 1, 0));
    }
    expect(sharpNameAt("bass5", 0, 5)).toBe("E");
    expect(sharpNameAt("bass5", 0, 3)).toBe("D");
  });

  test("sample notes from the 24th fret", () => {
    expect(sharpNameAt("guitar6", 0, 24)).toBe("E");
    expect(sharpNameAt("guitar6", 5, 24)).toBe("E");
    expect(sharpNameAt("bass4", 3, 24)).toBe("G");
    expect(sharpNameAt("bass5", 0, 24)).toBe("B");
    expect(midiAt("guitar6", 5, 24)).toBe(88); // E6
  });

  test("rejects strings and frets the instrument does not have", () => {
    expect(() => midiAt("bass4", 4, 0)).toThrow(RangeError);
    expect(() => midiAt("guitar6", -1, 0)).toThrow(RangeError);
    expect(() => midiAt("guitar6", 0, -1)).toThrow(RangeError);
    expect(() => midiAt("guitar6", 0, MAX_FRET + 1)).toThrow(RangeError);
    expect(() => midiAt("guitar6", 0, 1.5)).toThrow(RangeError);
  });
});

describe("position markers", () => {
  test("single dots at 3 5 7 9 15 17 19 21, double dots at 12 and 24", () => {
    expect(SINGLE_MARKER_FRETS).toEqual([3, 5, 7, 9, 15, 17, 19, 21]);
    expect(DOUBLE_MARKER_FRETS).toEqual([12, 24]);
  });
});

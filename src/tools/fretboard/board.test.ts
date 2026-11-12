import { buildBoard } from "./board";

function labels(board: ReturnType<typeof buildBoard>): (string | null)[][] {
  return board.strings.map((string) => string.markers.map((marker) => marker?.label ?? null));
}

describe("buildBoard", () => {
  test("C major on guitar, frets 0–12, highest string first", () => {
    const board = buildBoard("guitar6", 0, "major", 12, "notes");
    expect(board.strings.map((string) => string.name)).toEqual(["E", "B", "G", "D", "A", "E"]);
    expect(labels(board)).toEqual([
      ["E", "F", null, "G", null, "A", null, "B", "C", null, "D", null, "E"],
      ["B", "C", null, "D", null, "E", "F", null, "G", null, "A", null, "B"],
      ["G", null, "A", null, "B", "C", null, "D", null, "E", "F", null, "G"],
      ["D", null, "E", "F", null, "G", null, "A", null, "B", "C", null, "D"],
      ["A", null, "B", "C", null, "D", null, "E", "F", null, "G", null, "A"],
      ["E", "F", null, "G", null, "A", null, "B", "C", null, "D", null, "E"],
    ]);
  });

  test("interval view labels every tone with its degree and marks only the root", () => {
    const board = buildBoard("guitar6", 9, "minorPentatonic", 12, "intervals");
    const lowE = board.strings[5];
    expect(lowE?.markers.map((marker) => marker?.label ?? null)).toEqual([
      "5", // E
      null,
      null,
      "b7", // G
      null,
      "1", // A
      null,
      null,
      "b3", // C
      null,
      "4", // D
      null,
      "5",
    ]);
    const roots = board.strings.flatMap((string) =>
      string.markers.flatMap((marker, fret) => (marker?.root ? [[string.name, fret]] : [])),
    );
    // A on each string, highest first: E string fret 5, B 10, G 2, D 7, A 0 and 12, E 5.
    expect(roots).toEqual([
      ["E", 5],
      ["B", 10],
      ["G", 2],
      ["D", 7],
      ["A", 0],
      ["A", 12],
      ["E", 5],
    ]);
  });

  test("a flat key spells the board with flats, a sharp key with sharps", () => {
    const flat = buildBoard("guitar6", 1, "major", 12, "notes");
    const names = new Set(flat.strings.flatMap((s) => s.markers.map((m) => m?.note)));
    expect(names.has("Db")).toBe(true);
    expect(names.has("Gb")).toBe(true);
    expect(names.has("C#")).toBe(false);

    const sharp = buildBoard("guitar6", 1, "naturalMinor", 12, "notes");
    const sharpNames = new Set(sharp.strings.flatMap((s) => s.markers.map((m) => m?.note)));
    expect(sharpNames.has("C#")).toBe(true);
    expect(sharpNames.has("D#")).toBe(true);
    expect(sharpNames.has("Db")).toBe(false);
  });

  test("bass 4-string and 5-string rows and fret counts", () => {
    const bass4 = buildBoard("bass4", 4, "major", 15, "notes");
    expect(bass4.strings.map((string) => string.name)).toEqual(["G", "D", "A", "E"]);
    expect(bass4.strings.every((string) => string.markers.length === 16)).toBe(true);

    const bass5 = buildBoard("bass5", 4, "major", 24, "notes");
    expect(bass5.strings.map((string) => string.name)).toEqual(["G", "D", "A", "E", "B"]);
    expect(bass5.strings.every((string) => string.markers.length === 25)).toBe(true);
    // E major on the B string: open B is the 5th, fret 2 is C#.
    expect(bass5.strings[4]?.markers[0]?.label).toBe("B");
    expect(bass5.strings[4]?.markers[2]?.label).toBe("C#");
  });

  test("every tone of the scale appears on every string within an octave", () => {
    for (const instrument of ["guitar6", "bass4", "bass5"] as const) {
      const board = buildBoard(instrument, 2, "dorian", 12, "notes");
      for (const string of board.strings) {
        const seen = new Set(string.markers.flatMap((marker) => (marker ? [marker.note] : [])));
        expect([...seen].sort()).toEqual(["A", "B", "C", "D", "E", "F", "G"]);
      }
    }
  });
});

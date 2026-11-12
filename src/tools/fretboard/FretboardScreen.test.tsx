import { fireEvent, render, screen } from "@testing-library/react";
import { i18n } from "../../i18n";
import { FretboardScreen } from "./FretboardScreen";

const STORAGE_KEY = "woodshed.fretboard";

function stringNames(): string[] {
  return Array.from(document.querySelectorAll(".string-name")).map((el) => el.textContent ?? "");
}

function rootNotes(): string[] {
  return Array.from(document.querySelectorAll('.marker[data-root="true"]')).map(
    (el) => el.getAttribute("data-note") ?? "",
  );
}

function scaleTexts(selector: string): string[] {
  return Array.from(document.querySelectorAll(`.scale-note ${selector}`)).map(
    (el) => el.textContent ?? "",
  );
}

function markerLabels(): string[] {
  return Array.from(document.querySelectorAll(".marker")).map((el) => el.textContent ?? "");
}

/** Accessible name of the marker at `row` (0 = top string) and `fret`; empty when there is none. */
function markerAt(row: number, fret: number): string {
  const cell = document.querySelectorAll(".string-row")[row]?.querySelectorAll(".fret-cell")[fret];
  return cell?.querySelector(".marker")?.getAttribute("aria-label") ?? "";
}

function markerNames(): string[] {
  return Array.from(document.querySelectorAll(".marker")).map(
    (el) => el.getAttribute("aria-label") ?? "",
  );
}

/** Line width of every string row from the top, with the open-string name of that row. */
function stringLines(): { name: string; width: string }[] {
  return Array.from(document.querySelectorAll<HTMLElement>(".string-row")).map((row) => ({
    name: row.querySelector(".string-name")?.textContent ?? "",
    width: row.style.getPropertyValue("--string-width"),
  }));
}

beforeEach(async () => {
  localStorage.clear();
  await i18n.changeLanguage("en");
});

test("starts on a 6-string guitar in C major with the root marked", () => {
  render(<FretboardScreen />);
  expect(screen.getByRole("heading", { name: "Scale Fretboard" })).toBeTruthy();
  expect(stringNames()).toEqual(["E", "B", "G", "D", "A", "E"]);
  expect(rootNotes().every((note) => note === "C")).toBe(true);
  expect(rootNotes().length).toBeGreaterThan(0);
  expect(screen.getByRole("heading", { name: "C Major" })).toBeTruthy();
  // frets 0–15 by default
  expect(document.querySelectorAll(".string-row")[0]?.querySelectorAll(".fret-cell")).toHaveLength(
    16,
  );
});

test("changing the instrument switches the strings right away", () => {
  render(<FretboardScreen />);
  fireEvent.click(screen.getByRole("radio", { name: "4-String Bass" }));
  expect(stringNames()).toEqual(["G", "D", "A", "E"]);
  fireEvent.click(screen.getByRole("radio", { name: "5-String Bass" }));
  expect(stringNames()).toEqual(["G", "D", "A", "E", "B"]);
  fireEvent.click(screen.getByRole("radio", { name: "6-String Guitar" }));
  expect(stringNames()).toEqual(["E", "B", "G", "D", "A", "E"]);
});

test("changing the key moves the root, and the scale changes the tones", () => {
  render(<FretboardScreen />);
  fireEvent.change(screen.getByLabelText("Key"), { target: { value: "2" } });
  expect(rootNotes().every((note) => note === "D")).toBe(true);
  expect(screen.getByRole("heading", { name: "D Major" })).toBeTruthy();

  fireEvent.change(screen.getByLabelText("Scale"), { target: { value: "naturalMinor" } });
  expect(screen.getByRole("heading", { name: "D Natural Minor" })).toBeTruthy();
  expect(scaleTexts(".scale-note-name")).toEqual(["D", "E", "F", "G", "A", "B♭", "C"]);
  expect(scaleTexts(".scale-note-interval")).toEqual(["1", "2", "♭3", "4", "5", "♭6", "♭7"]);
});

test("a black-key root takes the spelling of the chosen scale", () => {
  render(<FretboardScreen />);
  fireEvent.change(screen.getByLabelText("Key"), { target: { value: "1" } });
  expect(screen.getByRole("heading", { name: "D♭ Major" })).toBeTruthy();
  fireEvent.change(screen.getByLabelText("Scale"), { target: { value: "naturalMinor" } });
  expect(screen.getByRole("heading", { name: "C♯ Natural Minor" })).toBeTruthy();
});

test("modes are spelled the way the key's notation spells them", () => {
  render(<FretboardScreen />);
  fireEvent.change(screen.getByLabelText("Scale"), { target: { value: "dorian" } });
  fireEvent.change(screen.getByLabelText("Key"), { target: { value: "1" } });
  expect(screen.getByRole("heading", { name: "C♯ Dorian" })).toBeTruthy();
  expect(scaleTexts(".scale-note-name")).toEqual(["C♯", "D♯", "E", "F♯", "G♯", "A♯", "B"]);
  expect(scaleTexts(".scale-note-interval")).toEqual(["1", "2", "♭3", "4", "5", "6", "♭7"]);

  fireEvent.change(screen.getByLabelText("Scale"), { target: { value: "lydian" } });
  fireEvent.change(screen.getByLabelText("Key"), { target: { value: "6" } });
  expect(screen.getByRole("heading", { name: "G♭ Lydian" })).toBeTruthy();
  expect(scaleTexts(".scale-note-name")).toEqual(["G♭", "A♭", "B♭", "C", "D♭", "E♭", "F"]);
  expect(scaleTexts(".scale-note-interval")).toEqual(["1", "2", "3", "♯4", "5", "6", "7"]);
});

test("the display switch shows intervals instead of note names", () => {
  render(<FretboardScreen />);
  expect(markerLabels()).toContain("C");
  fireEvent.click(screen.getByRole("radio", { name: "Intervals" }));
  expect(markerLabels()).not.toContain("C");
  expect(markerLabels()).toContain("1");
  expect(markerLabels()).toContain("5");
  expect(
    Array.from(document.querySelectorAll('.marker[data-root="true"]')).every(
      (el) => el.textContent === "1",
    ),
  ).toBe(true);
  fireEvent.click(screen.getByRole("radio", { name: "Note Names" }));
  expect(markerLabels()).toContain("C");
});

test("widening the board adds frets, and left-handed mirrors it", () => {
  render(<FretboardScreen />);
  fireEvent.click(screen.getByRole("radio", { name: "24" }));
  expect(document.querySelectorAll(".string-row")[0]?.querySelectorAll(".fret-cell")).toHaveLength(
    25,
  );
  const board = screen.getByRole("figure", { name: "Fretboard" });
  expect(board.getAttribute("dir")).toBe("ltr");
  fireEvent.click(screen.getByRole("checkbox", { name: "Left-Handed" }));
  expect(board.getAttribute("dir")).toBe("rtl");
});

test("choices are remembered across a remount", () => {
  const first = render(<FretboardScreen />);
  fireEvent.click(screen.getByRole("radio", { name: "5-String Bass" }));
  fireEvent.change(screen.getByLabelText("Key"), { target: { value: "9" } });
  fireEvent.change(screen.getByLabelText("Scale"), { target: { value: "minorPentatonic" } });
  fireEvent.click(screen.getByRole("radio", { name: "Intervals" }));
  fireEvent.click(screen.getByRole("radio", { name: "12" }));
  first.unmount();

  render(<FretboardScreen />);
  expect(stringNames()).toEqual(["G", "D", "A", "E", "B"]);
  expect(screen.getByRole("heading", { name: "A Minor Pentatonic" })).toBeTruthy();
  expect(markerLabels()).toContain("♭3");
  expect(document.querySelectorAll(".string-row")[0]?.querySelectorAll(".fret-cell")).toHaveLength(
    13,
  );
});

test("unreadable stored choices fall back to the defaults", () => {
  localStorage.setItem(STORAGE_KEY, JSON.stringify({ instrument: "banjo", key: 99, scale: "x" }));
  const first = render(<FretboardScreen />);
  expect(stringNames()).toEqual(["E", "B", "G", "D", "A", "E"]);
  expect(screen.getByRole("heading", { name: "C Major" })).toBeTruthy();
  first.unmount();

  localStorage.setItem(STORAGE_KEY, "not json");
  render(<FretboardScreen />);
  expect(stringNames()).toEqual(["E", "B", "G", "D", "A", "E"]);
  expect(screen.getByRole("heading", { name: "C Major" })).toBeTruthy();
});

describe("accessible names", () => {
  test("the same note in different places has different names", () => {
    render(<FretboardScreen />);
    // Guitar in C major: the high and low E strings, open and at the 12th fret.
    expect(markerAt(0, 0)).toBe("String 1, Fret 0, E");
    expect(markerAt(0, 12)).toBe("String 1, Fret 12, E");
    expect(markerAt(5, 0)).toBe("String 6, Fret 0, E");
    expect(markerAt(5, 12)).toBe("String 6, Fret 12, E");
    // The root closes the name.
    expect(markerAt(1, 1)).toBe("String 2, Fret 1, C, Root");
    expect(markerAt(4, 3)).toBe("String 5, Fret 3, C, Root");
  });

  test("no two markers share a name, on every instrument and at 24 frets", () => {
    render(<FretboardScreen />);
    fireEvent.click(screen.getByRole("radio", { name: "24" }));
    for (const instrument of ["6-String Guitar", "4-String Bass", "5-String Bass"]) {
      fireEvent.click(screen.getByRole("radio", { name: instrument }));
      for (const mode of ["Note Names", "Intervals"]) {
        fireEvent.click(screen.getByRole("radio", { name: mode }));
        const names = markerNames();
        expect(names.length).toBeGreaterThan(0);
        expect(names.every((name) => name !== "")).toBe(true);
        expect(new Set(names).size).toBe(names.length);
      }
    }
  });

  test("intervals mode adds the degree to the name", () => {
    render(<FretboardScreen />);
    fireEvent.click(screen.getByRole("radio", { name: "Intervals" }));
    expect(markerAt(0, 0)).toBe("String 1, Fret 0, E, Degree 3");
    expect(markerAt(1, 1)).toBe("String 2, Fret 1, C, Degree 1, Root");

    fireEvent.change(screen.getByLabelText("Key"), { target: { value: "9" } });
    fireEvent.change(screen.getByLabelText("Scale"), { target: { value: "minorPentatonic" } });
    // A minor pentatonic on the low E string: fret 8 is C, the ♭3.
    expect(markerAt(5, 8)).toBe("String 6, Fret 8, C, Degree ♭3");
    expect(markerAt(5, 5)).toBe("String 6, Fret 5, A, Degree 1, Root");
  });

  test("string numbers count from the highest string on every instrument", () => {
    render(<FretboardScreen />);
    fireEvent.click(screen.getByRole("radio", { name: "4-String Bass" }));
    expect(markerAt(0, 0)).toBe("String 1, Fret 0, G");
    expect(markerAt(3, 0)).toBe("String 4, Fret 0, E");
    fireEvent.click(screen.getByRole("radio", { name: "5-String Bass" }));
    expect(markerAt(0, 0)).toBe("String 1, Fret 0, G");
    expect(markerAt(4, 0)).toBe("String 5, Fret 0, B");
  });

  test("the names are translated", async () => {
    await i18n.changeLanguage("ko");
    render(<FretboardScreen />);
    expect(markerAt(0, 0)).toBe("1번 줄 0프렛, E");
    expect(markerAt(5, 12)).toBe("6번 줄 12프렛, E");
    expect(markerAt(1, 1)).toBe("2번 줄 1프렛, C, 루트");
    fireEvent.click(screen.getByRole("radio", { name: "음정" }));
    expect(markerAt(0, 0)).toBe("1번 줄 0프렛, E, 도수 3");
  });
});

describe("keyboard scrolling", () => {
  test("the scrolling board takes focus, also at 24 frets", () => {
    render(<FretboardScreen />);
    const board = screen.getByRole("figure", { name: "Fretboard" });
    for (const frets of ["15", "24"]) {
      fireEvent.click(screen.getByRole("radio", { name: frets }));
      const region = screen.getByRole("region", { name: "Fretboard Scroll" });
      expect(region.tabIndex).toBe(0);
      expect(region.contains(board)).toBe(true);
      region.focus();
      expect(document.activeElement).toBe(region);
    }
  });

  test("the focusable region is named in Korean too", async () => {
    await i18n.changeLanguage("ko");
    render(<FretboardScreen />);
    expect(screen.getByRole("region", { name: "지판 스크롤" }).tabIndex).toBe(0);
  });
});

describe("string thickness", () => {
  test("guitar: the high E at the top is thin and the low E at the bottom is thick", () => {
    render(<FretboardScreen />);
    const lines = stringLines();
    expect(lines).toHaveLength(6);
    expect(lines[0]).toEqual({ name: "E", width: "1px" });
    expect(lines[5]).toEqual({ name: "E", width: "3px" });
  });

  test("bass: G at the top is thin and the lowest string at the bottom is thick", () => {
    render(<FretboardScreen />);
    fireEvent.click(screen.getByRole("radio", { name: "4-String Bass" }));
    const bass4 = stringLines();
    expect(bass4[0]).toEqual({ name: "G", width: "1px" });
    expect(bass4[3]).toEqual({ name: "E", width: "3px" });

    fireEvent.click(screen.getByRole("radio", { name: "5-String Bass" }));
    const bass5 = stringLines();
    expect(bass5[0]).toEqual({ name: "G", width: "1px" });
    expect(bass5[4]).toEqual({ name: "B", width: "3px" });
  });

  test("a lower string is never thinner than the string above it", () => {
    render(<FretboardScreen />);
    for (const instrument of ["6-String Guitar", "4-String Bass", "5-String Bass"]) {
      fireEvent.click(screen.getByRole("radio", { name: instrument }));
      const widths = stringLines().map((line) => Number.parseFloat(line.width));
      expect(widths.every((width) => Number.isFinite(width))).toBe(true);
      for (let row = 1; row < widths.length; row += 1) {
        expect(widths[row]).toBeGreaterThanOrEqual(widths[row - 1] as number);
      }
      expect(widths.at(-1)).toBeGreaterThan(widths[0] as number);
    }
  });
});

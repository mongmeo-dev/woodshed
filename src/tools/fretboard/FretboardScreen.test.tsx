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

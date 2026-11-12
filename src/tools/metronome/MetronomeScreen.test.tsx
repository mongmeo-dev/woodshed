import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { i18n } from "../../i18n";
import { resetMemoryStore } from "./api";
import { MetronomeScreen } from "./MetronomeScreen";

beforeEach(async () => {
  resetMemoryStore();
  await i18n.changeLanguage("en");
});

async function renderScreen() {
  render(<MetronomeScreen />);
  // Let the initial load settle.
  await act(async () => {});
  return screen.getByRole("spinbutton", { name: "Tempo" });
}

test("tempo changes with arrow keys, Shift, Page keys and Home/End", async () => {
  const tempo = await renderScreen();
  expect(tempo.textContent).toBe("120.0");
  fireEvent.keyDown(tempo, { key: "ArrowUp" });
  expect(tempo.textContent).toBe("121.0");
  fireEvent.keyDown(tempo, { key: "ArrowDown", shiftKey: true });
  expect(tempo.textContent).toBe("111.0");
  fireEvent.keyDown(tempo, { key: "ArrowUp", altKey: true });
  expect(tempo.textContent).toBe("111.1");
  fireEvent.keyDown(tempo, { key: "PageUp" });
  expect(tempo.getAttribute("aria-valuenow")).toBe("121.1");
  fireEvent.keyDown(tempo, { key: "End" });
  expect(tempo.textContent).toBe("400.0");
  fireEvent.keyDown(tempo, { key: "ArrowUp" });
  expect(tempo.textContent).toBe("400.0");
  fireEvent.keyDown(tempo, { key: "Home" });
  expect(tempo.textContent).toBe("20.0");
});

test("tempo can be typed, and an out-of-range value shows an error", async () => {
  const tempo = await renderScreen();
  fireEvent.keyDown(tempo, { key: "Enter" });
  const input = screen.getByRole("textbox", { name: "Tempo" });
  fireEvent.change(input, { target: { value: "500" } });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(screen.getByRole("alert").textContent).toBe("Enter a number from 20 to 400");

  fireEvent.change(input, { target: { value: "96.5" } });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(screen.getByRole("spinbutton", { name: "Tempo" }).textContent).toBe("96.5");

  fireEvent.keyDown(screen.getByRole("spinbutton", { name: "Tempo" }), { key: "Enter" });
  const again = screen.getByRole("textbox", { name: "Tempo" });
  fireEvent.change(again, { target: { value: "200" } });
  fireEvent.keyDown(again, { key: "Escape" });
  expect(screen.getByRole("spinbutton", { name: "Tempo" }).textContent).toBe("96.5");
});

test("tap tempo sets the tempo from the taps", async () => {
  const tempo = await renderScreen();
  const now = vi.spyOn(performance, "now");
  const tap = screen.getByRole("button", { name: "Tap" });
  for (const at of [1_000, 1_750, 2_500]) {
    now.mockReturnValue(at);
    fireEvent.click(tap);
  }
  now.mockRestore();
  expect(tempo.textContent).toBe("80.0");
});

test("clicking a beat steps through the accents and a new meter resets them", async () => {
  await renderScreen();
  const accents = within(screen.getByRole("group", { name: "Beat Accents" }));
  expect(accents.getAllByRole("button")).toHaveLength(4);
  fireEvent.click(accents.getByRole("button", { name: "Beat 2, Normal" }));
  fireEvent.click(accents.getByRole("button", { name: "Beat 2, Weak" }));
  expect(accents.getByRole("button", { name: "Beat 2, Muted" })).toBeTruthy();

  fireEvent.change(screen.getByRole("combobox", { name: "Beats per Bar" }), {
    target: { value: "6" },
  });
  fireEvent.change(screen.getByRole("combobox", { name: "Beat Unit" }), {
    target: { value: "8" },
  });
  const labels = accents.getAllByRole("button").map((button) => button.getAttribute("aria-label"));
  expect(labels).toEqual([
    "Beat 1, Strong",
    "Beat 2, Weak",
    "Beat 3, Weak",
    "Beat 4, Normal",
    "Beat 5, Weak",
    "Beat 6, Weak",
  ]);
});

test("space starts and stops, except while typing", async () => {
  await renderScreen();
  const play = screen.getByRole("button", { name: "Start" });
  fireEvent.keyDown(window, { key: " " });
  expect(play.textContent).toBe("Stop");
  expect(play.getAttribute("aria-pressed")).toBe("true");

  const name = screen.getByRole("textbox", { name: "Preset Name" });
  fireEvent.keyDown(name, { key: " " });
  expect(play.textContent).toBe("Stop");

  fireEvent.keyDown(window, { key: " " });
  expect(play.textContent).toBe("Start");
});

test("trainer and polyrhythm switches reveal their settings", async () => {
  await renderScreen();
  expect(screen.queryByLabelText("Raise Every")).toBeNull();
  fireEvent.click(screen.getByRole("switch", { name: "Tempo Trainer On" }));
  expect(
    screen.getByRole("switch", { name: "Tempo Trainer On" }).getAttribute("aria-checked"),
  ).toBe("true");
  const every = screen.getByLabelText("Raise Every");
  fireEvent.change(every, { target: { value: "8" } });
  expect((every as HTMLInputElement).value).toBe("8");
  const max = screen.getByLabelText("Maximum Tempo") as HTMLInputElement;
  // Partial input is kept while typing and applied once it is in range.
  fireEvent.change(max, { target: { value: "1" } });
  expect(max.value).toBe("1");
  fireEvent.change(max, { target: { value: "150" } });
  fireEvent.blur(max);
  expect(max.value).toBe("150");

  fireEvent.click(screen.getByRole("switch", { name: "Polyrhythm On" }));
  expect(screen.getByRole("combobox", { name: "Polyrhythm Clicks" })).toBeTruthy();
  expect(screen.getByRole("group", { name: "Polyrhythm Sound" })).toBeTruthy();
});

test("presets save, replace, load and delete after confirming", async () => {
  const tempo = await renderScreen();
  expect(screen.getByText("No Presets")).toBeTruthy();
  const name = screen.getByRole("textbox", { name: "Preset Name" });

  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect((await screen.findByRole("alert")).textContent).toBe("Enter a name");

  fireEvent.keyDown(tempo, { key: "End" });
  fireEvent.change(name, { target: { value: "Fast" } });
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
  });
  expect(screen.getByText("Fast")).toBeTruthy();

  fireEvent.change(name, { target: { value: "Fast" } });
  expect(screen.getByRole("button", { name: "Replace" })).toBeTruthy();
  fireEvent.change(name, { target: { value: "" } });

  fireEvent.keyDown(tempo, { key: "Home" });
  expect(tempo.textContent).toBe("20.0");
  fireEvent.click(screen.getByRole("button", { name: "Load Fast" }));
  expect(tempo.textContent).toBe("400.0");

  fireEvent.click(screen.getByRole("button", { name: "Delete Fast" }));
  expect(screen.getByText("Delete “Fast”?")).toBeTruthy();
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  });
  expect(screen.getByRole("status").textContent).toBe("Deleted “Fast”");
  expect(screen.getByText("No Presets")).toBeTruthy();
});

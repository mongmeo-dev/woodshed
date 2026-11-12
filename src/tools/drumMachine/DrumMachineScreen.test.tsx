import { fireEvent, render, screen, within } from "@testing-library/react";
import { i18n } from "../../i18n";
import { DrumMachineScreen } from "./DrumMachineScreen";
import { resetMemoryStore } from "./drums";

beforeEach(async () => {
  resetMemoryStore();
  await i18n.changeLanguage("en");
});

function row(name: string) {
  return within(screen.getByRole("group", { name }));
}

function stepButtons(name: string) {
  return row(name).getAllByRole("button", { name: new RegExp(`^${name}, step`) });
}

test("shows a row of 16 steps for every track", async () => {
  render(<DrumMachineScreen />);
  expect(await screen.findByRole("group", { name: "Kick" })).toBeTruthy();
  for (const name of ["Kick", "Snare", "Closed Hi-Hat", "Open Hi-Hat", "Ride", "Crash"]) {
    expect(stepButtons(name)).toHaveLength(16);
  }
});

test("clicking a step paints the chosen velocity and clicking again clears it", async () => {
  render(<DrumMachineScreen />);
  const step = await screen.findByRole("button", { name: "Snare, step 5" });
  fireEvent.click(step);
  expect(step.getAttribute("aria-pressed")).toBe("true");
  expect(step.dataset.level).toBe("2");

  fireEvent.click(screen.getByRole("radio", { name: "Accent" }));
  fireEvent.click(step);
  expect(step.dataset.level).toBe("3");
  fireEvent.click(step);
  expect(step.getAttribute("aria-pressed")).toBe("false");
});

test("a genre preset sets the grid and the tempo", async () => {
  render(<DrumMachineScreen />);
  fireEvent.change(await screen.findByLabelText("Preset"), { target: { value: "disco" } });
  expect(
    (screen.getByLabelText("Tempo", { selector: "input[type=number]" }) as HTMLInputElement).value,
  ).toBe("120");
  const kicks = stepButtons("Kick").map((button) => button.getAttribute("aria-pressed"));
  expect(kicks.slice(0, 5)).toEqual(["true", "false", "false", "false", "true"]);
});

test("two-bar length shows 32 steps", async () => {
  render(<DrumMachineScreen />);
  await screen.findByRole("group", { name: "Kick" });
  fireEvent.click(screen.getByRole("radio", { name: "32 Steps" }));
  expect(stepButtons("Kick")).toHaveLength(32);
});

test("space plays and stops, except while typing", async () => {
  render(<DrumMachineScreen />);
  const play = await screen.findByRole("button", { name: "Play" });
  fireEvent.keyDown(window, { code: "Space", key: " " });
  expect(play.textContent).toBe("Stop");
  fireEvent.keyDown(screen.getByLabelText("Tempo", { selector: "input[type=number]" }), {
    code: "Space",
    key: " ",
  });
  expect(play.textContent).toBe("Stop");
  fireEvent.keyDown(window, { code: "Space", key: " " });
  expect(play.textContent).toBe("Play");
});

test("an out-of-range tempo is flagged and not applied", async () => {
  render(<DrumMachineScreen />);
  const bpm = (await screen.findByLabelText("Tempo", {
    selector: "input[type=number]",
  })) as HTMLInputElement;
  fireEvent.change(bpm, { target: { value: "500" } });
  expect(screen.getByRole("alert").textContent).toBe("Enter 40 to 300");
  fireEvent.blur(bpm);
  expect(bpm.value).toBe("100");
  expect(screen.queryByRole("alert")).toBeNull();
});

test("patterns are saved, loaded and deleted by name", async () => {
  render(<DrumMachineScreen />);
  fireEvent.click(await screen.findByRole("button", { name: "Save" }));
  fireEvent.click(screen.getAllByRole("button", { name: "Save" })[1] as HTMLElement);
  expect(screen.getByRole("alert").textContent).toBe("Enter a name");

  fireEvent.change(screen.getByLabelText("Pattern Name"), { target: { value: "Verse" } });
  fireEvent.click(screen.getAllByRole("button", { name: "Save" })[1] as HTMLElement);
  const saved = screen.getByLabelText("My Patterns") as HTMLSelectElement;
  expect(await within(saved).findByRole("option", { name: "Verse" })).toBeTruthy();
  expect(saved.value).toBe("Verse");

  // Change the groove, then load the saved one back.
  fireEvent.click(screen.getByRole("button", { name: "Kick, step 1" }));
  expect(screen.getByRole("button", { name: "Kick, step 1" }).getAttribute("aria-pressed")).toBe(
    "true",
  );
  fireEvent.change(saved, { target: { value: "Verse" } });
  fireEvent.change(screen.getByLabelText("Preset"), { target: { value: "rock" } });
  fireEvent.change(saved, { target: { value: "Verse" } });
  expect(screen.getByRole("button", { name: "Kick, step 1" }).getAttribute("aria-pressed")).toBe(
    "false",
  );

  // Saving under the same name offers to replace it.
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(screen.getByRole("button", { name: "Replace" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

  fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  const confirm = screen.getByRole("alertdialog", { name: "Delete “Verse”?" });
  fireEvent.click(within(confirm).getByRole("button", { name: "Delete" }));
  expect(await screen.findByRole("option", { name: "No Saved Patterns" })).toBeTruthy();
});

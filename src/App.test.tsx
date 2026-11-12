import { fireEvent, render, screen, within } from "@testing-library/react";
import { App } from "./App";
import { i18n } from "./i18n";

const KO_TOOLS = ["튜너", "메트로놈", "드럼 머신", "스템 분리", "스케일 지판"];
const EN_TOOLS = ["Tuner", "Metronome", "Drum Machine", "Stem Splitter", "Scale Fretboard"];

function toolNames(language: "ko" | "en") {
  const nav = screen.getByRole("navigation", { name: language === "ko" ? "도구" : "Tools" });
  return within(nav)
    .getAllByRole("button")
    .map((button) => button.textContent);
}

beforeEach(async () => {
  await i18n.changeLanguage("en");
});

test("sidebar lists the five tools and settings", async () => {
  render(<App />);
  expect(await screen.findByRole("heading", { name: "Tuner" })).toBeTruthy();
  expect(toolNames("en")).toEqual([...EN_TOOLS, "Settings"]);
});

test("selecting a tool shows its screen and marks it current", async () => {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "Metronome" }));
  expect(screen.getByRole("heading", { name: "Metronome" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "Metronome" }).getAttribute("aria-current")).toBe(
    "page",
  );
  expect(screen.getByRole("button", { name: "Tuner" }).getAttribute("aria-current")).toBeNull();
});

test("language setting switches the navigation between Korean and English", async () => {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
  fireEvent.click(screen.getByRole("radio", { name: "한국어" }));
  expect(await screen.findByRole("heading", { name: "설정" })).toBeTruthy();
  expect(toolNames("ko")).toEqual([...KO_TOOLS, "설정"]);
  expect(document.documentElement.lang).toBe("ko");

  fireEvent.click(screen.getByRole("radio", { name: "English" }));
  expect(await screen.findByRole("heading", { name: "Settings" })).toBeTruthy();
  expect(toolNames("en")).toEqual([...EN_TOOLS, "Settings"]);
});

test("theme setting sets data-theme on the document", async () => {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
  const appearance = within(screen.getByRole("group", { name: "Appearance" }));
  fireEvent.click(appearance.getByRole("radio", { name: "Dark" }));
  expect(document.documentElement.dataset.theme).toBe("dark");
  fireEvent.click(appearance.getByRole("radio", { name: "System" }));
  expect(document.documentElement.dataset.theme).toBeUndefined();
});

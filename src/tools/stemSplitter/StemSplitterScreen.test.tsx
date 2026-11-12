import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { i18n } from "../../i18n";
import type { OpenedSong, SongSummary } from "../../lib/stems";
import { StemSplitterScreen } from "./StemSplitterScreen";

const backend = vi.hoisted(() => ({
  calls: [] as Array<[string, Record<string, unknown>]>,
  playing: false,
  tracks: ["original"] as string[],
}));

vi.mock("../../lib/ipc", () => ({
  inTauri: () => false,
  call: vi.fn(async (command: string, args: Record<string, unknown>, fallback: unknown) => {
    backend.calls.push([command, args]);
    const song: SongSummary = {
      id: "song1",
      title: "Practice Song",
      source: { kind: "file", fileName: "song.mp3" },
      durationSecs: 180,
      sampleRate: 44_100,
      addedAtMs: 1,
      stems: [],
    };
    switch (command) {
      case "stems_list":
        return [song];
      case "stems_open":
        return { song, tracks: backend.tracks, waveform: [10, 200, 30] } as OpenedSong;
      case "stems_transport":
        return {
          songId: song.id,
          playing: backend.playing,
          ended: false,
          positionSecs: 12,
          heardAtMs: null,
        };
      case "stems_play":
        backend.playing = true;
        return undefined;
      case "stems_pause":
        backend.playing = false;
        return undefined;
      default:
        return fallback;
    }
  }),
}));

function called(command: string) {
  return backend.calls.filter(([name]) => name === command).map(([, args]) => args);
}

beforeEach(async () => {
  // jsdom has no canvas; the waveform skips drawing without a context.
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  backend.calls = [];
  backend.playing = false;
  backend.tracks = ["original"];
  await i18n.changeLanguage("en");
});

async function renderOpened() {
  render(<StemSplitterScreen />);
  expect(await screen.findByRole("heading", { name: "Practice Song" })).toBeTruthy();
}

test("the newest song opens with one original track and no separate button", async () => {
  await renderOpened();
  expect(called("stems_open")).toEqual([{ id: "song1" }]);
  const mixer = screen.getByRole("heading", { name: "Mixer" }).closest("section");
  if (!mixer) throw new Error("mixer section");
  expect(within(mixer).getByText("Original")).toBeTruthy();
  expect(within(mixer).getByRole("slider", { name: "Original Volume" })).toBeTruthy();
  expect(within(mixer).getByRole("slider", { name: "Master Volume" })).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Separate Stems" })).toBeNull();
});

test("six ready stems become six mixer tracks", async () => {
  backend.tracks = ["vocals", "drums", "bass", "guitar", "piano", "other"];
  await renderOpened();
  for (const name of ["Vocals", "Drums", "Bass", "Guitar", "Piano", "Other"]) {
    expect(screen.getByRole("slider", { name: `${name} Volume` })).toBeTruthy();
  }
});

test("space plays and pauses, but not while typing a link", async () => {
  await renderOpened();
  fireEvent.keyDown(window, { code: "Space", key: " " });
  await waitFor(() => expect(called("stems_play")).toHaveLength(1));
  expect(await screen.findByRole("button", { name: "Pause" })).toBeTruthy();

  fireEvent.keyDown(window, { code: "Space", key: " " });
  await waitFor(() => expect(called("stems_pause")).toHaveLength(1));

  const link = screen.getByRole("textbox", { name: "YouTube Link" });
  fireEvent.keyDown(link, { code: "Space", key: " " });
  expect(called("stems_play")).toHaveLength(1);
});

test("mute, solo and volume go to the player as one track mix", async () => {
  await renderOpened();
  fireEvent.click(screen.getByRole("button", { name: "Mute Original" }));
  expect(called("stems_set_track").at(-1)).toEqual({
    index: 0,
    volume: 1,
    muted: true,
    solo: false,
  });
  expect(screen.getByRole("button", { name: "Mute Original" }).getAttribute("aria-pressed")).toBe(
    "true",
  );
  fireEvent.click(screen.getByRole("button", { name: "Solo Original" }));
  fireEvent.change(screen.getByRole("slider", { name: "Original Volume" }), {
    target: { value: "40" },
  });
  expect(called("stems_set_track").at(-1)).toEqual({
    index: 0,
    volume: 0.4,
    muted: true,
    solo: true,
  });
});

test("speed slider sets 50–100 % and shows the value", async () => {
  await renderOpened();
  const speed = screen.getByRole("slider", { name: "Speed" });
  expect(speed.getAttribute("min")).toBe("50");
  expect(speed.getAttribute("max")).toBe("100");
  fireEvent.change(speed, { target: { value: "75" } });
  expect(called("stems_set_speed").at(-1)).toEqual({ speed: 0.75 });
  expect(screen.getByText("75%")).toBeTruthy();
});

test("dragging on the waveform sets the A-B loop, a click seeks", async () => {
  vi.spyOn(HTMLCanvasElement.prototype, "getBoundingClientRect").mockReturnValue({
    left: 0,
    top: 0,
    right: 1000,
    bottom: 88,
    width: 1000,
    height: 88,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  });
  await renderOpened();
  const wave = screen.getByRole("slider", { name: "Playback Position" });
  fireEvent.pointerDown(wave, { button: 0, clientX: 100, pointerId: 1 });
  fireEvent.pointerMove(wave, { clientX: 300, pointerId: 1 });
  fireEvent.pointerUp(wave, { clientX: 300, pointerId: 1 });
  expect(called("stems_set_loop").at(-1)).toEqual({ range: { start: 18, end: 54 } });
  expect(screen.getByText("Loop 0:18–0:54")).toBeTruthy();

  fireEvent.click(screen.getByRole("button", { name: "Clear Loop" }));
  expect(called("stems_set_loop").at(-1)).toEqual({ range: null });
  expect(screen.queryByText("Loop 0:18–0:54")).toBeNull();

  fireEvent.pointerDown(wave, { button: 0, clientX: 500, pointerId: 1 });
  fireEvent.pointerUp(wave, { clientX: 500, pointerId: 1 });
  await waitFor(() => expect(called("stems_seek").at(-1)).toEqual({ seconds: 90 }));
});

test("waveform keys seek and a loop can be cleared", async () => {
  await renderOpened();
  const position = screen.getByRole("slider", { name: "Playback Position" });
  fireEvent.keyDown(position, { key: "End" });
  await waitFor(() => expect(called("stems_seek").at(-1)).toEqual({ seconds: 180 }));
  expect(screen.queryByRole("button", { name: "Clear Loop" })).toBeNull();
});

test("a link that is not a YouTube video shows a short error", async () => {
  await renderOpened();
  const link = screen.getByRole("textbox", { name: "YouTube Link" });
  fireEvent.change(link, { target: { value: "https://example.com/video" } });
  fireEvent.click(screen.getByRole("button", { name: "Download" }));
  expect((await screen.findByRole("alert")).textContent).toBe("Not a YouTube video link");
  expect(called("stems_import_link")).toHaveLength(0);
});

test("deleting asks first", async () => {
  await renderOpened();
  fireEvent.click(screen.getByRole("button", { name: "Delete Practice Song" }));
  expect(screen.getByText("Delete this song?")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(called("stems_delete")).toHaveLength(0);
  fireEvent.click(screen.getByRole("button", { name: "Delete Practice Song" }));
  fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  await waitFor(() => expect(called("stems_delete")).toEqual([{ id: "song1" }]));
});

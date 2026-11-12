import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { i18n } from "../../i18n";
import type { OpenedSong, SongSummary } from "../../lib/stems";
import { StemSplitterScreen } from "./StemSplitterScreen";

/**
 * A player that answers commands at once but applies them APPLY_MS later,
 * on an engine frame clock, like the real one (events are scheduled ahead).
 */
const backend = vi.hoisted(() => {
  interface Mix {
    volume: number;
    muted: boolean;
    solo: boolean;
  }
  const state = {
    calls: [] as Array<[string, Record<string, unknown>]>,
    tracks: ["original"] as string[],
    applyMs: 150,
    /** Bumped per test so timers left by an earlier test do nothing. */
    run: 0,
    frame: 0,
    pending: [] as Array<{ frame: number; apply: () => void }>,
    applied: {
      playing: false,
      position: 0,
      speed: 1,
      loop: null as { start: number; end: number } | null,
      mix: [] as Mix[],
    },
    outputChanged: null as null | ((change: unknown) => void),
    reset() {
      state.run += 1;
      state.calls = [];
      state.tracks = ["original"];
      state.frame = 0;
      state.pending = [];
      state.applied = { playing: false, position: 0, speed: 1, loop: null, mix: [] };
    },
    /** Queues `apply` for a later frame and returns that frame. */
    schedule(apply: () => void) {
      const frame = state.frame + 480;
      state.pending.push({ frame, apply });
      const run = state.run;
      setTimeout(() => {
        if (run === state.run) state.renderTo(frame);
      }, state.applyMs);
      return { frame };
    },
    renderTo(frame: number) {
      const due = state.pending.filter((event) => event.frame <= frame);
      state.pending = state.pending.filter((event) => event.frame > frame);
      for (const event of due) event.apply();
      state.frame = Math.max(state.frame, frame);
    },
  };
  return state;
});

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
    const applied = backend.applied;
    switch (command) {
      case "stems_list":
        return [song];
      case "stems_open":
        applied.mix = backend.tracks.map(() => ({ volume: 1, muted: false, solo: false }));
        return { song, tracks: backend.tracks, waveform: [10, 200, 30] } as OpenedSong;
      case "stems_transport":
        return {
          songId: song.id,
          playing: applied.playing,
          ended: false,
          positionSecs: applied.position,
          heardAtMs: null,
          renderedFrame: backend.frame,
        };
      case "stems_play":
        return backend.schedule(() => {
          applied.playing = true;
        });
      case "stems_pause":
        return backend.schedule(() => {
          applied.playing = false;
        });
      case "stems_seek":
        return backend.schedule(() => {
          applied.position = args.seconds as number;
        });
      case "stems_set_speed":
        return backend.schedule(() => {
          applied.speed = args.speed as number;
        });
      case "stems_set_loop":
        return backend.schedule(() => {
          applied.loop = args.range as typeof applied.loop;
        });
      case "stems_set_track": {
        const { index, ...mix } = args as {
          index: number;
          volume: number;
          muted: boolean;
          solo: boolean;
        };
        return backend.schedule(() => {
          applied.mix[index] = mix;
        });
      }
      case "stems_restore": {
        // The engine dropped everything pending for the old output.
        backend.pending = [];
        const intent = args.intent as {
          playing: boolean;
          seek: number | null;
          speed: number;
          loop: typeof applied.loop;
          tracks: typeof applied.mix;
        };
        return backend.schedule(() => {
          if (intent.seek !== null) applied.position = intent.seek;
          applied.speed = intent.speed;
          applied.loop = intent.loop;
          applied.mix = intent.tracks.map((track) => ({ ...track }));
          applied.playing = intent.playing;
        });
      }
      default:
        return fallback;
    }
  }),
}));

vi.mock("../../lib/audio", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/audio")>()),
  onOutputChanged: async (listener: (change: unknown) => void) => {
    backend.outputChanged = listener;
    return () => {
      backend.outputChanged = null;
    };
  },
}));

function called(command: string) {
  return backend.calls.filter(([name]) => name === command).map(([, args]) => args);
}

/** Lets scheduled events apply and the screen poll once more. */
async function settle() {
  await new Promise((resolve) => setTimeout(resolve, backend.applyMs + 120));
}

beforeEach(async () => {
  // jsdom has no canvas; the waveform skips drawing without a context.
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  backend.reset();
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
  await waitFor(() =>
    expect(called("stems_set_track").at(-1)).toEqual({
      index: 0,
      volume: 1,
      muted: true,
      solo: false,
    }),
  );
  expect(screen.getByRole("button", { name: "Mute Original" }).getAttribute("aria-pressed")).toBe(
    "true",
  );
  fireEvent.click(screen.getByRole("button", { name: "Solo Original" }));
  fireEvent.change(screen.getByRole("slider", { name: "Original Volume" }), {
    target: { value: "40" },
  });
  await waitFor(() =>
    expect(called("stems_set_track").at(-1)).toEqual({
      index: 0,
      volume: 0.4,
      muted: true,
      solo: true,
    }),
  );
});

test("speed slider sets 50–100 % and shows the value", async () => {
  await renderOpened();
  const speed = screen.getByRole("slider", { name: "Speed" });
  expect(speed.getAttribute("min")).toBe("50");
  expect(speed.getAttribute("max")).toBe("100");
  fireEvent.change(speed, { target: { value: "75" } });
  await waitFor(() => expect(called("stems_set_speed").at(-1)).toEqual({ speed: 0.75 }));
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
  await waitFor(() =>
    expect(called("stems_set_loop").at(-1)).toEqual({ range: { start: 18, end: 54 } }),
  );
  expect(screen.getByText("Loop 0:18–0:54")).toBeTruthy();

  fireEvent.click(screen.getByRole("button", { name: "Clear Loop" }));
  await waitFor(() => expect(called("stems_set_loop").at(-1)).toEqual({ range: null }));
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

test("play then pause sends both, even though the engine applies them later", async () => {
  await renderOpened();
  fireEvent.click(screen.getByRole("button", { name: "Play" }));
  expect(screen.getByRole("button", { name: "Pause" })).toBeTruthy();
  // A poll answers before Start is applied: the screen keeps the user's choice.
  await new Promise((resolve) => setTimeout(resolve, backend.applyMs / 2 + 10));
  expect(called("stems_transport").length).toBeGreaterThan(0);
  expect(backend.applied.playing).toBe(false);
  expect(screen.getByRole("button", { name: "Pause" })).toBeTruthy();
  await settle();
  expect(backend.applied.playing).toBe(true);

  fireEvent.click(screen.getByRole("button", { name: "Pause" }));
  expect(screen.getByRole("button", { name: "Play" })).toBeTruthy();
  await settle();
  expect(called("stems_play")).toHaveLength(1);
  expect(called("stems_pause")).toHaveLength(1);
  expect(backend.applied.playing).toBe(false);
  expect(screen.getByRole("button", { name: "Play" })).toBeTruthy();
});

test("a seek while paused shows the new position before and after it applies", async () => {
  await renderOpened();
  const position = screen.getByRole("slider", { name: "Playback Position" });
  const time = () => screen.getByText(/ \/ 3:00$/).textContent;
  expect(time()).toBe("0:00 / 3:00");

  fireEvent.keyDown(position, { key: "End" });
  await waitFor(() => expect(position.getAttribute("aria-valuenow")).toBe("180"));
  expect(time()).toBe("3:00 / 3:00");
  // A poll from before the seek applied does not move it back.
  await new Promise((resolve) => setTimeout(resolve, backend.applyMs / 2 + 10));
  expect(backend.applied.position).toBe(0);
  expect(position.getAttribute("aria-valuenow")).toBe("180");
  await settle();
  expect(backend.applied.position).toBe(180);
  expect(position.getAttribute("aria-valuenow")).toBe("180");

  fireEvent.keyDown(position, { key: "Home" });
  await waitFor(() => expect(position.getAttribute("aria-valuenow")).toBe("0"));
  expect(time()).toBe("0:00 / 3:00");
  await settle();
  expect(position.getAttribute("aria-valuenow")).toBe("0");
});

/** Makes a command, changes the output before it applies, and settles. */
async function changeOutputAfter(action: () => void) {
  action();
  await waitFor(() => expect(backend.pending.length).toBeGreaterThan(0), {
    onTimeout: (error) => {
      error.message += ` calls: ${JSON.stringify(backend.calls.map(([name]) => name))}`;
      return error;
    },
  });
  backend.outputChanged?.({ generation: 2, sampleRate: 44_100 });
  await settle();
  expect(called("stems_restore").length).toBeGreaterThan(0);
}

test("an output change re-applies a pending start or stop", async () => {
  await renderOpened();
  await waitFor(() => expect(backend.outputChanged).not.toBeNull());
  await changeOutputAfter(() => fireEvent.click(screen.getByRole("button", { name: "Play" })));
  expect(backend.applied.playing).toBe(true);
  expect(screen.getByRole("button", { name: "Pause" })).toBeTruthy();

  await changeOutputAfter(() => fireEvent.click(screen.getByRole("button", { name: "Pause" })));
  expect(backend.applied.playing).toBe(false);
  expect(screen.getByRole("button", { name: "Play" })).toBeTruthy();
});

test("an output change re-applies a pending seek, speed, loop and mute", async () => {
  await renderOpened();
  await waitFor(() => expect(backend.outputChanged).not.toBeNull());
  const position = screen.getByRole("slider", { name: "Playback Position" });

  await changeOutputAfter(() => fireEvent.keyDown(position, { key: "End" }));
  expect(backend.applied.position).toBe(180);

  await changeOutputAfter(() =>
    fireEvent.change(screen.getByRole("slider", { name: "Speed" }), { target: { value: "60" } }),
  );
  expect(backend.applied.speed).toBe(0.6);

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
  await changeOutputAfter(() => {
    fireEvent.pointerDown(position, { button: 0, clientX: 100, pointerId: 1 });
    fireEvent.pointerMove(position, { clientX: 300, pointerId: 1 });
    fireEvent.pointerUp(position, { clientX: 300, pointerId: 1 });
  });
  expect(backend.applied.loop).toEqual({ start: 18, end: 54 });

  await changeOutputAfter(() =>
    fireEvent.click(screen.getByRole("button", { name: "Mute Original" })),
  );
  expect(backend.applied.mix[0]).toEqual({ volume: 1, muted: true, solo: false });
  // The last restore carried the whole intent, not just the latest change.
  expect(called("stems_restore").at(-1)).toEqual({
    intent: {
      playing: false,
      seek: null,
      speed: 0.6,
      loop: { start: 18, end: 54 },
      tracks: [{ volume: 1, muted: true, solo: false }],
      master: 1,
    },
  });
});

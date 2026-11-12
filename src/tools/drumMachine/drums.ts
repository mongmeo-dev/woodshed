import { Channel } from "@tauri-apps/api/core";
import { call, inTauri } from "../../lib/ipc";

// Mirrors the serde types in src-tauri/src/drums (pattern.rs, store.rs,
// announce.rs, service.rs). A step is a sixteenth note.

/** Grid rows, top to bottom (`Track` in kit.rs). */
export const TRACKS = [
  "kick",
  "snare",
  "sidestick",
  "clap",
  "closedHat",
  "openHat",
  "highTom",
  "lowTom",
  "ride",
  "crash",
] as const;
export type TrackId = (typeof TRACKS)[number];

export const MAX_STEPS = 32;
export const LENGTHS = [16, 32] as const;
export type PatternLength = (typeof LENGTHS)[number];
export const MIN_BPM = 40;
export const MAX_BPM = 300;
export const MIN_SWING = 50;
export const MAX_SWING = 75;
/** 0 off, 1 soft (ghost), 2 normal, 3 accent. */
export type StepLevel = 0 | 1 | 2 | 3;
export const MAX_LEVEL = 3;
export const DEFAULT_VOLUME = 0.8;

export type SwingUnit = "eighth" | "sixteenth";

export interface TrackPattern {
  /** Level per step, {@link MAX_STEPS} long. */
  steps: StepLevel[];
  /** Linear 0–1. */
  volume: number;
  muted: boolean;
}

export interface Pattern {
  length: PatternLength;
  bpm: number;
  /** Percent, {@link MIN_SWING}–{@link MAX_SWING}. */
  swing: number;
  swingUnit: SwingUnit;
  tracks: Record<TrackId, TrackPattern>;
}

export interface SavedPattern {
  name: string;
  pattern: Pattern;
}

export interface DrumState {
  playing: boolean;
  pattern: Pattern;
  saved: SavedPattern[];
}

/** A step and the time it is heard (see `StepEvent` in announce.rs). */
export interface HeardStep {
  index: number;
  frame: number;
  /** Unix-epoch ms; see `heardAtToPerformanceTime` in lib/audio.ts. */
  heardAtMs: number;
}

export type StepEvent =
  | { type: "steps"; generation: number; steps: HeardStep[] }
  | { type: "clear"; fromFrame: number };

/** Why saving or deleting a pattern failed (`PatternStoreError`). */
export type PatternStoreErrorCode = "emptyName" | "saveFailed";

export function emptyTrack(): TrackPattern {
  return { steps: Array<StepLevel>(MAX_STEPS).fill(0), volume: DEFAULT_VOLUME, muted: false };
}

export function emptyPattern(): Pattern {
  return {
    length: 16,
    bpm: 100,
    swing: MIN_SWING,
    swingUnit: "eighth",
    tracks: Object.fromEntries(TRACKS.map((track) => [track, emptyTrack()])) as Record<
      TrackId,
      TrackPattern
    >,
  };
}

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function toLevel(value: unknown): StepLevel {
  return typeof value === "number" && value >= 0
    ? (Math.min(MAX_LEVEL, Math.round(value)) as StepLevel)
    : 0;
}

/** Fills gaps and clamps values, like `Pattern::normalized` in Rust. */
export function normalizePattern(raw: Partial<Pattern> | null | undefined): Pattern {
  const base = emptyPattern();
  const length = raw?.length === 32 ? 32 : 16;
  const bpm = Number.isFinite(raw?.bpm)
    ? clamp(Math.round(raw?.bpm ?? 0), MIN_BPM, MAX_BPM)
    : base.bpm;
  const swing = Number.isFinite(raw?.swing)
    ? clamp(Math.round(raw?.swing ?? 0), MIN_SWING, MAX_SWING)
    : base.swing;
  const swingUnit: SwingUnit = raw?.swingUnit === "sixteenth" ? "sixteenth" : "eighth";
  const tracks = { ...base.tracks };
  for (const track of TRACKS) {
    const given = raw?.tracks?.[track];
    if (!given) continue;
    const steps = Array.from({ length: MAX_STEPS }, (_, step) => toLevel(given.steps?.[step]));
    const volume = Number.isFinite(given.volume) ? clamp(given.volume, 0, 1) : DEFAULT_VOLUME;
    tracks[track] = { steps, volume, muted: given.muted === true };
  }
  return { length, bpm, swing, swingUnit, tracks };
}

function withTrack(pattern: Pattern, track: TrackId, change: Partial<TrackPattern>): Pattern {
  return {
    ...pattern,
    tracks: { ...pattern.tracks, [track]: { ...pattern.tracks[track], ...change } },
  };
}

/**
 * Clicking a step with the chosen velocity: an empty step or one at another
 * level takes `level`; a step already at `level` turns off.
 */
export function paintStep(
  pattern: Pattern,
  track: TrackId,
  step: number,
  level: Exclude<StepLevel, 0>,
): Pattern {
  const steps = [...pattern.tracks[track].steps];
  steps[step] = steps[step] === level ? 0 : level;
  return withTrack(pattern, track, { steps });
}

export function setTrackVolume(pattern: Pattern, track: TrackId, volume: number): Pattern {
  return withTrack(pattern, track, { volume: clamp(volume, 0, 1) });
}

export function toggleMute(pattern: Pattern, track: TrackId): Pattern {
  return withTrack(pattern, track, { muted: !pattern.tracks[track].muted });
}

/**
 * Switches between one and two bars. Growing to 32 steps copies the first
 * bar into an empty second bar; shrinking keeps the second bar for later.
 */
export function setLength(pattern: Pattern, length: PatternLength): Pattern {
  if (length === pattern.length) return pattern;
  if (length === 16) return { ...pattern, length };
  const secondBarEmpty = TRACKS.every((track) =>
    pattern.tracks[track].steps.slice(16).every((level) => level === 0),
  );
  if (!secondBarEmpty) return { ...pattern, length };
  const tracks = { ...pattern.tracks };
  for (const track of TRACKS) {
    const steps = pattern.tracks[track].steps;
    tracks[track] = {
      ...pattern.tracks[track],
      steps: [...steps.slice(0, 16), ...steps.slice(0, 16)],
    };
  }
  return { ...pattern, length, tracks };
}

export function setBpm(pattern: Pattern, bpm: number): Pattern {
  return { ...pattern, bpm: clamp(Math.round(bpm), MIN_BPM, MAX_BPM) };
}

export function setSwing(pattern: Pattern, swing: number): Pattern {
  return { ...pattern, swing: clamp(Math.round(swing), MIN_SWING, MAX_SWING) };
}

export function setSwingUnit(pattern: Pattern, swingUnit: SwingUnit): Pattern {
  return { ...pattern, swingUnit };
}

/** Reads the error code of a rejected save or delete. */
export function storeErrorCode(error: unknown): PatternStoreErrorCode {
  if (typeof error === "object" && error !== null && "code" in error) {
    if ((error as { code: unknown }).code === "emptyName") return "emptyName";
  }
  return "saveFailed";
}

// ---- commands ----------------------------------------------------------------

/** Saved patterns outside Tauri (browser dev server and tests). */
let memorySaved: SavedPattern[] = [];

export function resetMemoryStore(): void {
  memorySaved = [];
}

export function loadDrumState(): Promise<DrumState> {
  return call<DrumState>(
    "drums_state",
    {},
    { playing: false, pattern: emptyPattern(), saved: memorySaved },
  ).then((state) => ({ ...state, pattern: normalizePattern(state.pattern) }));
}

export function setDrumsPlaying(playing: boolean): Promise<boolean> {
  return call<boolean>("drums_set_playing", { playing }, playing);
}

/** Sends the edited pattern; while playing it is heard without stopping. */
export function sendPattern(pattern: Pattern): Promise<Pattern> {
  return call<Pattern>("drums_set_pattern", { pattern }, pattern);
}

export async function savePattern(name: string, pattern: Pattern): Promise<SavedPattern[]> {
  if (!inTauri()) {
    const trimmed = name.trim().slice(0, 40);
    if (!trimmed) throw { code: "emptyName" };
    const existing = memorySaved.findIndex((saved) => saved.name === trimmed);
    const entry = { name: trimmed, pattern };
    memorySaved =
      existing >= 0
        ? memorySaved.map((saved, index) => (index === existing ? entry : saved))
        : [...memorySaved, entry];
    return memorySaved;
  }
  return call<SavedPattern[]>("drums_save_pattern", { name, pattern }, []);
}

export async function deletePattern(name: string): Promise<SavedPattern[]> {
  if (!inTauri()) {
    memorySaved = memorySaved.filter((saved) => saved.name !== name);
    return memorySaved;
  }
  return call<SavedPattern[]>("drums_delete_pattern", { name }, []);
}

/** Increases with every subscription, also across webview reloads. */
let subscriptionToken = Date.now();

/**
 * Receives the steps as they are scheduled, each with the time it is heard.
 * The app keeps one step channel: the subscription with the highest token.
 */
export function subscribeSteps(listener: (event: StepEvent) => void): () => void {
  if (!inTauri()) return () => {};
  let active = true;
  const channel = new Channel<StepEvent>();
  channel.onmessage = (event) => {
    if (active) listener(event);
  };
  subscriptionToken += 1;
  call("drums_subscribe", { onEvent: channel, token: subscriptionToken }, undefined).catch(
    () => {},
  );
  return () => {
    active = false;
  };
}

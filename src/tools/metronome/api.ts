import { Channel, invoke } from "@tauri-apps/api/core";
import { inTauri } from "../../lib/ipc";
import { type Accent, DEFAULT_SETTINGS, type MetronomeSettings, normalizeSettings } from "./model";

// Mirrors src-tauri/src/metronome/service.rs and store.rs.

export interface Preset {
  name: string;
  settings: MetronomeSettings;
}

export interface MetronomeState {
  settings: MetronomeSettings;
  presets: Preset[];
  playing: boolean;
}

/** Sent by the metronome worker as beats are scheduled. */
export type MetronomeMessage =
  | {
      type: "beat";
      frame: number;
      /** Unix-epoch ms at which the beat is heard. */
      heardAtMs: number;
      bar: number;
      beat: number;
      beats: number;
      accent: Accent;
      bpm: number;
    }
  | { type: "pulse"; frame: number; heardAtMs: number; pulse: number; pulses: number }
  | { type: "reset"; fromFrame: number }
  | { type: "stopped" };

export type PresetErrorCode = "emptyName" | "nameTooLong" | "tooMany" | "saveFailed";

const PRESET_ERROR_CODES = new Set<PresetErrorCode>([
  "emptyName",
  "nameTooLong",
  "tooMany",
  "saveFailed",
]);

/** Reads the code of a rejected {@link savePreset} / {@link deletePreset}. */
export function presetErrorCode(error: unknown): PresetErrorCode {
  if (typeof error === "object" && error !== null && "code" in error) {
    const { code } = error as { code: unknown };
    if (PRESET_ERROR_CODES.has(code as PresetErrorCode)) return code as PresetErrorCode;
  }
  return "saveFailed";
}

function normalizePresets(raw: unknown): Preset[] {
  if (!Array.isArray(raw)) return [];
  return raw.flatMap((item) => {
    const preset = item as Partial<Preset> | null;
    return typeof preset?.name === "string"
      ? [{ name: preset.name, settings: normalizeSettings(preset.settings) }]
      : [];
  });
}

// Outside Tauri (browser dev server, tests) the screen works against memory.
const memory: { settings: MetronomeSettings; presets: Preset[] } = {
  settings: DEFAULT_SETTINGS,
  presets: [],
};

/** Resets the in-memory stand-in (tests only). */
export function resetMemoryStore(): void {
  memory.settings = DEFAULT_SETTINGS;
  memory.presets = [];
}

export async function loadMetronome(): Promise<MetronomeState> {
  if (!inTauri()) {
    return { settings: memory.settings, presets: memory.presets, playing: false };
  }
  const state = await invoke<Partial<MetronomeState>>("metronome_load");
  return {
    settings: normalizeSettings(state.settings),
    presets: normalizePresets(state.presets),
    playing: state.playing === true,
  };
}

/** Receives the worker's messages; replaces any earlier subscriber. */
export async function subscribeMetronome(
  onMessage: (message: MetronomeMessage) => void,
): Promise<void> {
  if (!inTauri()) return;
  const channel = new Channel<MetronomeMessage>();
  channel.onmessage = onMessage;
  await invoke("metronome_subscribe", { onEvent: channel });
}

export async function startMetronome(settings: MetronomeSettings): Promise<void> {
  memory.settings = settings;
  if (inTauri()) await invoke("metronome_start", { settings });
}

/** Applies from the next beat while playing; also remembers the settings. */
export async function updateMetronome(settings: MetronomeSettings): Promise<void> {
  memory.settings = settings;
  if (inTauri()) await invoke("metronome_update", { settings });
}

export async function stopMetronome(): Promise<void> {
  if (inTauri()) await invoke("metronome_stop");
}

export async function savePreset(name: string, settings: MetronomeSettings): Promise<Preset[]> {
  if (inTauri()) {
    return normalizePresets(await invoke("metronome_save_preset", { name, settings }));
  }
  const trimmed = name.trim();
  if (!trimmed) throw { code: "emptyName" };
  const preset = { name: trimmed, settings };
  const exists = memory.presets.some((kept) => kept.name === trimmed);
  memory.presets = exists
    ? memory.presets.map((kept) => (kept.name === trimmed ? preset : kept))
    : [...memory.presets, preset];
  return memory.presets;
}

export async function deletePreset(name: string): Promise<Preset[]> {
  if (inTauri()) return normalizePresets(await invoke("metronome_delete_preset", { name }));
  memory.presets = memory.presets.filter((preset) => preset.name !== name);
  return memory.presets;
}

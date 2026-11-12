import { Channel } from "@tauri-apps/api/core";
import { call, inTauri } from "../../lib/ipc";

// Mirrors the serde types in src-tauri/src/tuner/ and src-tauri/src/audio/input.rs.

export type InputChannel = { type: "single"; channel: number } | { type: "mixDown" };

export interface OpenedInput {
  deviceId: string;
  deviceName: string;
  sampleRate: number;
  channels: number;
  channel: InputChannel;
}

export type TunerState =
  | "stopped"
  | "permissionPending"
  | "permissionDenied"
  | "noInput"
  | "listening";

export interface TunerStatus {
  state: TunerState;
  input: OpenedInput | null;
  /** The chosen device is gone or failed; the system default plays. */
  fallback: boolean;
  preferredDeviceId: string | null;
  channel: InputChannel;
  referenceHz: number;
  /** Changes when the input is (re)opened or restarted after a gap. */
  inputGeneration: number;
}

export interface Reading {
  frequency: number;
  /** Nearest MIDI note; 69 = A4. */
  midi: number;
  /** Offset from `midi` in cents (about −50..50). */
  cents: number;
  clarity: number;
}

export type TunerEvent =
  | ({ type: "status" } & TunerStatus)
  | { type: "frame"; levelDb: number; pitch: Reading | null };

export type TunerError = { code: "saveFailed"; message: string };

export const MIN_REFERENCE_HZ = 430;
export const MAX_REFERENCE_HZ = 450;
export const DEFAULT_REFERENCE_HZ = 440;

export const DEFAULT_STATUS: TunerStatus = {
  state: "stopped",
  input: null,
  fallback: false,
  preferredDeviceId: null,
  channel: { type: "single", channel: 0 },
  referenceHz: DEFAULT_REFERENCE_HZ,
  inputGeneration: 0,
};

/** Starts listening; events arrive on `onEvent` until {@link stopTuner}. */
export function startTuner(onEvent: (event: TunerEvent) => void): Promise<TunerStatus> {
  if (!inTauri()) return Promise.resolve(DEFAULT_STATUS);
  const channel = new Channel<TunerEvent>();
  channel.onmessage = onEvent;
  return call<TunerStatus>("tuner_start", { onEvent: channel }, DEFAULT_STATUS);
}

export function stopTuner(): Promise<void> {
  return call<void>("tuner_stop", {}, undefined);
}

/** Chooses the input (`null` = system default). Rejects with {@link TunerError}; the choice still applies. */
export function selectInput(deviceId: string | null, channel: InputChannel): Promise<void> {
  return call<void>("tuner_select_input", { deviceId, channel }, undefined);
}

/** Sets A4; resolves to the value in effect (clamped to 430–450 Hz). */
export function setReference(hz: number): Promise<number> {
  return call<number>("tuner_set_reference", { hz }, clampReference(hz));
}

export function openMicrophoneSettings(): Promise<void> {
  return call<void>("tuner_open_microphone_settings", {}, undefined);
}

export function clampReference(hz: number): number {
  return Math.min(MAX_REFERENCE_HZ, Math.max(MIN_REFERENCE_HZ, Math.round(hz)));
}

const NOTE_NAMES = ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"] as const;

/** Note letter and octave of a MIDI number (60 = C4). */
export function noteName(midi: number): { name: string; octave: number } {
  const pitchClass = ((midi % 12) + 12) % 12;
  return { name: NOTE_NAMES[pitchClass] ?? "", octave: Math.floor(midi / 12) - 1 };
}

/** Standard open strings, low to high, as MIDI numbers. */
export const OPEN_STRINGS = {
  guitar6: [40, 45, 50, 55, 59, 64],
  bass4: [28, 33, 38, 43],
  bass5: [23, 28, 33, 38, 43],
} as const;

export type StringSet = keyof typeof OPEN_STRINGS;

/** Value used by the channel picker. */
export function channelKey(channel: InputChannel): string {
  return channel.type === "mixDown" ? "mix" : String(channel.channel);
}

export function channelFromKey(key: string): InputChannel {
  return key === "mix" ? { type: "mixDown" } : { type: "single", channel: Number(key) };
}

/** Maps a level in dBFS to 0..1 for the meter (−60 dB … 0 dB). */
export function levelFraction(levelDb: number): number {
  return Math.min(1, Math.max(0, (levelDb + 60) / 60));
}

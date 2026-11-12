import { Channel } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { call, inTauri } from "./ipc";

// Mirrors the serde types in src-tauri/src/app_audio.rs and src-tauri/src/audio.

export interface DeviceInfo {
  id: string;
  name: string;
  isDefault: boolean;
  channels: number;
  sampleRate: number;
}

export interface OutputStreamInfo {
  deviceId: string;
  deviceName: string;
  sampleRate: number;
  channels: number;
  requestedBufferFrames: number | null;
  bufferFrames: number | null;
}

export interface EngineStats {
  /** Output generation; changes whenever the output stream is (re)opened. */
  generation: number;
  framesRendered: number;
  callbacks: number;
  maxCallbackFrames: number;
  lateEvents: number;
  droppedEvents: number;
}

export interface AudioStatus {
  output: OutputStreamInfo | null;
  outputLatencyMs: number | null;
  stats: EngineStats;
  /**
   * Output device chosen and in effect (`null` = system default). May differ
   * from the device playing (fallback) and from the settings file.
   */
  preferredDeviceId: string | null;
  /**
   * No output yet: macOS holds it until the user answers the microphone
   * prompt (the output device, or the system default, has inputs).
   */
  waitingForMicrophone: boolean;
}

export type EventKind =
  | { type: "trigger"; note: number; velocity: number }
  | { type: "start" }
  | { type: "stop" }
  | { type: "seek"; frame: number }
  | { type: "param"; id: number; value: number };

/** A rendered `notify` event and the wall-clock time it is heard. */
export interface PlaybackEvent {
  sourceId: number;
  scheduledFrame: number;
  renderedFrame: number;
  kind: EventKind;
  /** Output generation it was rendered in. Reports from older generations are not sent. */
  generation: number;
  /** Unix-epoch milliseconds; see {@link heardAtToPerformanceTime}. */
  heardAtMs: number;
}

export interface ScheduledSound {
  frame: number;
  heardAtMs: number | null;
}

export interface DeviceSnapshot {
  outputs: string[];
  inputs: string[];
  defaultOutput: string | null;
  defaultInput: string | null;
}

export const DEVICES_CHANGED_EVENT = "audio://devices-changed";
export const OUTPUT_CHANGED_EVENT = "audio://output-changed";

/** Payload of {@link OUTPUT_CHANGED_EVENT}. */
export interface OutputChanged {
  generation: number;
  deviceId: string;
  sampleRate: number;
  channels: number;
  bufferFrames: number | null;
}

const EMPTY_STATUS: AudioStatus = {
  output: null,
  outputLatencyMs: null,
  preferredDeviceId: null,
  waitingForMicrophone: false,
  stats: {
    generation: 0,
    framesRendered: 0,
    callbacks: 0,
    maxCallbackFrames: 0,
    lateEvents: 0,
    droppedEvents: 0,
  },
};

export function getAudioStatus(): Promise<AudioStatus> {
  return call("audio_status", {}, EMPTY_STATUS);
}

export function listDevices(direction: "input" | "output"): Promise<DeviceInfo[]> {
  return call("audio_list_devices", { direction }, []);
}

/** Why choosing an output failed (mirrors `OutputSelectError` in app_audio.rs). */
export type OutputSelectError =
  | { code: "openFailed"; message: string }
  | { code: "saveFailed"; message: string }
  | { code: "unavailable" };

const SELECT_ERROR_CODES = new Set(["openFailed", "saveFailed", "unavailable"]);

/** Reads the error code of a rejected {@link setOutputDevice}. */
export function outputSelectErrorCode(error: unknown): OutputSelectError["code"] {
  if (typeof error === "object" && error !== null && "code" in error) {
    const { code } = error as { code: unknown };
    if (typeof code === "string" && SELECT_ERROR_CODES.has(code)) {
      return code as OutputSelectError["code"];
    }
  }
  return "unavailable";
}

/** Output and preference right after the output manager handled a choice. */
export interface OutputSelection {
  output: OutputChanged | null;
  preferredDeviceId: string | null;
}

/**
 * Asks the output manager to switch the output (`null` follows the system
 * default) and save the choice. Rejects with an {@link OutputSelectError}:
 * on `openFailed` and `saveFailed` the choice is still in effect; on
 * `unavailable` it may still take effect later, so read the final state from
 * {@link getAudioStatus} (`preferredDeviceId`) and {@link onOutputChanged}.
 */
export function setOutputDevice(deviceId: string | null): Promise<OutputSelection> {
  return call<OutputSelection>(
    "audio_set_output_device",
    { deviceId },
    { output: null, preferredDeviceId: deviceId },
  );
}

export function playTestSound(): Promise<ScheduledSound | null> {
  return call<ScheduledSound | null>("audio_play_test_sound", {}, null);
}

/** Unix-epoch ms at which an engine frame is heard on the current output. */
export function heardAt(frame: number): Promise<number | null> {
  return call<number | null>("audio_heard_at", { frame }, null);
}

/**
 * Converts the engine's Unix-epoch "heard at" time to this webview's
 * `performance.now()` timeline.
 */
export function heardAtToPerformanceTime(
  heardAtMs: number,
  timeOrigin = performance.timeOrigin,
): number {
  return heardAtMs - timeOrigin;
}

/** Milliseconds from `now` until the sound is heard (0 if already heard). */
export function delayUntilHeard(
  heardAtMs: number,
  now = performance.now(),
  timeOrigin = performance.timeOrigin,
): number {
  return Math.max(0, heardAtToPerformanceTime(heardAtMs, timeOrigin) - now);
}

type PlaybackListener = (event: PlaybackEvent) => void;
const playbackListeners = new Set<PlaybackListener>();
type SubscriptionState = "idle" | "pending" | "ready";
let subscription: SubscriptionState = "idle";

/** Delay before retrying a failed `audio_subscribe`. */
export const SUBSCRIBE_RETRY_MS = 1000;

/** Opens the shared channel; on failure retries while listeners remain. */
function ensureSubscribed(): void {
  if (subscription !== "idle" || playbackListeners.size === 0 || !inTauri()) return;
  subscription = "pending";
  const channel = new Channel<PlaybackEvent>();
  channel.onmessage = (event) => {
    for (const notify of playbackListeners) notify(event);
  };
  call("audio_subscribe", { onEvent: channel }, undefined).then(
    () => {
      subscription = "ready";
    },
    () => {
      subscription = "idle";
      setTimeout(ensureSubscribed, SUBSCRIBE_RETRY_MS);
    },
  );
}

/**
 * Receives every rendered `notify` event. One Tauri channel is shared by all
 * listeners; it is opened on first use and retried if opening fails.
 */
export function onPlaybackEvent(listener: PlaybackListener): () => void {
  playbackListeners.add(listener);
  ensureSubscribed();
  return () => {
    playbackListeners.delete(listener);
  };
}

export async function onDevicesChanged(
  listener: (snapshot: DeviceSnapshot) => void,
): Promise<UnlistenFn> {
  if (!inTauri()) return () => {};
  return listen<DeviceSnapshot>(DEVICES_CHANGED_EVENT, (event) => listener(event.payload));
}

/**
 * Fires after every successful output (re)open — manual switch, following the
 * system default, or recovery. Frames scheduled for an older `generation`
 * were computed at the old sample rate: reschedule them.
 */
export async function onOutputChanged(
  listener: (change: OutputChanged) => void,
): Promise<UnlistenFn> {
  if (!inTauri()) return () => {};
  return listen<OutputChanged>(OUTPUT_CHANGED_EVENT, (event) => listener(event.payload));
}

/**
 * Starts an async subscription and returns a synchronous stop function that
 * also covers the case where the subscription finishes after stop was called.
 * Errors are reported to `onError` instead of becoming unhandled rejections.
 */
export function subscribeAsync(
  subscribe: () => Promise<UnlistenFn>,
  onError: (error: unknown) => void = () => {},
): () => void {
  let stopped = false;
  let unlisten: UnlistenFn | undefined;
  subscribe().then(
    (stop) => {
      if (stopped) stop();
      else unlisten = stop;
    },
    (error: unknown) => {
      if (!stopped) onError(error);
    },
  );
  return () => {
    stopped = true;
    unlisten?.();
  };
}

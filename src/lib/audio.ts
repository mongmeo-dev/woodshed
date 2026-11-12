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
  framesRendered: number;
  callbacks: number;
  maxCallbackFrames: number;
  lateEvents: number;
  droppedEvents: number;
  droppedCommands: number;
}

export interface AudioStatus {
  output: OutputStreamInfo | null;
  outputLatencyMs: number | null;
  stats: EngineStats;
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
  /** Unix-epoch milliseconds; see {@link heardAtToPerformanceTime}. */
  heardAtMs: number | null;
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

const EMPTY_STATUS: AudioStatus = {
  output: null,
  outputLatencyMs: null,
  stats: {
    framesRendered: 0,
    callbacks: 0,
    maxCallbackFrames: 0,
    lateEvents: 0,
    droppedEvents: 0,
    droppedCommands: 0,
  },
};

export function getAudioStatus(): Promise<AudioStatus> {
  return call("audio_status", {}, EMPTY_STATUS);
}

export function listDevices(direction: "input" | "output"): Promise<DeviceInfo[]> {
  return call("audio_list_devices", { direction }, []);
}

/** Switches the output device; `null` follows the system default. */
export function setOutputDevice(deviceId: string | null): Promise<AudioStatus> {
  return call("audio_set_output_device", { deviceId }, EMPTY_STATUS);
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
let subscribed = false;

/**
 * Receives every rendered `notify` event. One Tauri channel is shared by all
 * listeners; it is opened on first use.
 */
export function onPlaybackEvent(listener: PlaybackListener): () => void {
  playbackListeners.add(listener);
  if (!subscribed && inTauri()) {
    subscribed = true;
    const channel = new Channel<PlaybackEvent>();
    channel.onmessage = (event) => {
      for (const notify of playbackListeners) notify(event);
    };
    void call("audio_subscribe", { onEvent: channel }, undefined);
  }
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

import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { call, inTauri } from "./ipc";

// Mirrors the serde types in src-tauri/src/stems (commands.rs, library.rs, import.rs).

export const STEM_NAMES = ["vocals", "drums", "bass", "guitar", "piano", "other"] as const;
export type StemName = (typeof STEM_NAMES)[number];
export type StemStatus = "pending" | "running" | "ready" | "failed";
export type TrackName = "original" | StemName;

export interface StemEntry {
  name: StemName;
  status: StemStatus;
}

export type SongSource =
  | { kind: "file"; fileName: string }
  | { kind: "youtube"; url: string; videoId: string };

export interface SongSummary {
  id: string;
  title: string;
  source: SongSource;
  durationSecs: number;
  sampleRate: number;
  addedAtMs: number;
  stems: StemEntry[];
}

export interface OpenedSong {
  song: SongSummary;
  tracks: TrackName[];
  /** Peak per bin, 0–255. */
  waveform: number[];
}

/** `positionSecs` is heard at `heardAtMs` (Unix epoch ms; null before the first callback). */
export interface Transport {
  songId: string;
  playing: boolean;
  ended: boolean;
  positionSecs: number;
  heardAtMs: number | null;
}

export type ImportStage = "download" | "extract" | "decode";

export interface ImportProgress {
  stage: ImportStage;
  progress: number | null;
}

export type ImportErrorCode =
  | "invalidLink"
  | "toolsMissing"
  | "network"
  | "unavailable"
  | "unsupportedFile"
  | "busy"
  | "cancelled"
  | "failed";

const IMPORT_ERROR_CODES = new Set<string>([
  "invalidLink",
  "toolsMissing",
  "network",
  "unavailable",
  "unsupportedFile",
  "busy",
  "cancelled",
  "failed",
]);

/** Reads the `{ code }` of a rejected import. */
export function importErrorCode(error: unknown): ImportErrorCode {
  if (typeof error === "object" && error !== null && "code" in error) {
    const { code } = error as { code: unknown };
    if (typeof code === "string" && IMPORT_ERROR_CODES.has(code)) {
      return code as ImportErrorCode;
    }
  }
  return "failed";
}

export interface LoopRange {
  start: number;
  end: number;
}

/** Progress of a stem separation (emitted by the stem engine). */
export interface SeparationEvent {
  songId: string;
  state: "running" | "done" | "failed";
  progress: number | null;
}

export const SEPARATION_EVENT = "stems://separation";

/** Audio file extensions the importer accepts (mirrors `AUDIO_EXTENSIONS`). */
export const AUDIO_EXTENSIONS = ["mp3", "wav", "flac", "m4a", "aac", "ogg", "oga"] as const;

export function isAudioFile(path: string): boolean {
  const extension = path.split(".").pop()?.toLowerCase() ?? "";
  return (AUDIO_EXTENSIONS as readonly string[]).includes(extension);
}

const VIDEO_ID = /^[A-Za-z0-9_-]{11}$/;

/** Mirrors `parse_youtube_link` in youtube.rs; the backend checks again. */
export function youtubeVideoId(input: string): string | null {
  let rest = input.trim();
  const scheme = rest.match(/^([a-z][a-z0-9+.-]*):\/\//i);
  if (scheme) {
    const name = scheme[1]?.toLowerCase();
    if (name !== "http" && name !== "https") return null;
    rest = rest.slice(scheme[0].length);
  }
  const split = rest.search(/[/?#]/);
  let host = (split === -1 ? rest : rest.slice(0, split)).toLowerCase();
  for (const prefix of ["www.", "m.", "music."]) {
    if (host.startsWith(prefix)) {
      host = host.slice(prefix.length);
      break;
    }
  }
  const tail = split === -1 ? "" : rest.slice(split);
  const [pathPart = "", queryPart = ""] = (tail.split("#")[0] ?? "").split("?");
  const segments = pathPart.split("/").filter(Boolean);
  let id: string | undefined;
  if (host === "youtu.be") {
    id = segments[0];
  } else if (host === "youtube.com" || host === "youtube-nocookie.com") {
    if (segments.length === 1 && segments[0] === "watch") {
      id = queryPart
        .split("&")
        .find((pair) => pair.startsWith("v="))
        ?.slice(2);
    } else if (["shorts", "live", "embed", "v"].includes(segments[0] ?? "")) {
      id = segments[1];
    }
  }
  return id && VIDEO_ID.test(id) ? id : null;
}

export function listSongs(): Promise<SongSummary[]> {
  return call("stems_list", {}, []);
}

/** System open panel for audio files; null when dismissed. */
export function pickAudioFile(): Promise<string | null> {
  return call<string | null>("stems_pick_file", {}, null);
}

function progressChannel(onProgress: (progress: ImportProgress) => void) {
  const channel = new Channel<ImportProgress>();
  channel.onmessage = onProgress;
  return channel;
}

/** Rejects with `{ code: ImportErrorCode }`. */
export function importFile(
  path: string,
  onProgress: (progress: ImportProgress) => void,
): Promise<SongSummary> {
  if (!inTauri()) return Promise.reject({ code: "failed" });
  return invoke<SongSummary>("stems_import_file", {
    path,
    onProgress: progressChannel(onProgress),
  });
}

/** Rejects with `{ code: ImportErrorCode }`. */
export function importLink(
  url: string,
  onProgress: (progress: ImportProgress) => void,
): Promise<SongSummary> {
  if (!inTauri()) return Promise.reject({ code: "failed" });
  return invoke<SongSummary>("stems_import_link", {
    url,
    onProgress: progressChannel(onProgress),
  });
}

export function cancelImport(): Promise<void> {
  return call("stems_cancel_import", {}, undefined);
}

export function deleteSong(id: string): Promise<void> {
  return call("stems_delete", { id }, undefined);
}

export function openSong(id: string): Promise<OpenedSong | null> {
  return call<OpenedSong | null>("stems_open", { id }, null);
}

export function closeSong(): Promise<void> {
  return call("stems_close", {}, undefined);
}

export function play(): Promise<void> {
  return call("stems_play", {}, undefined);
}

export function pause(): Promise<void> {
  return call("stems_pause", {}, undefined);
}

export function seek(seconds: number): Promise<void> {
  return call("stems_seek", { seconds }, undefined);
}

/** 0.5–1.0; pitch is kept. */
export function setSpeed(speed: number): Promise<void> {
  return call("stems_set_speed", { speed }, undefined);
}

export function setLoop(range: LoopRange | null): Promise<void> {
  return call("stems_set_loop", { range }, undefined);
}

export function setTrack(
  index: number,
  mix: { volume: number; muted: boolean; solo: boolean },
): Promise<void> {
  return call("stems_set_track", { index, ...mix }, undefined);
}

export function setMaster(volume: number): Promise<void> {
  return call("stems_set_master", { volume }, undefined);
}

export function getTransport(): Promise<Transport | null> {
  return call<Transport | null>("stems_transport", {}, null);
}

/** After `audio://output-changed`: drops events pending for the old output. */
export function resync(): Promise<void> {
  return call("stems_resync", {}, undefined);
}

export async function onSeparation(
  listener: (event: SeparationEvent) => void,
): Promise<UnlistenFn> {
  if (!inTauri()) return () => {};
  return listen<SeparationEvent>(SEPARATION_EVENT, (event) => listener(event.payload));
}

/** Positive modulo. */
function wrap(value: number, length: number): number {
  return ((value % length) + length) % length;
}

/**
 * Song position heard at `now` (performance.now() ms), extrapolated from a
 * transport sample taken when `sample.positionSecs` is heard at `heardAt`
 * (performance time). Wraps inside the loop the way the player does.
 */
export function positionAt(
  sample: { positionSecs: number; playing: boolean },
  heardAt: number,
  now: number,
  speed: number,
  loop: LoopRange | null,
  duration: number,
): number {
  if (!sample.playing) return sample.positionSecs;
  let position = sample.positionSecs + ((now - heardAt) / 1000) * speed;
  if (loop && loop.end > loop.start) {
    const length = loop.end - loop.start;
    const inside = sample.positionSecs >= loop.start && sample.positionSecs < loop.end;
    const crossed = sample.positionSecs < loop.start && position >= loop.end;
    if (inside || crossed) position = loop.start + wrap(position - loop.start, length);
  }
  return Math.min(Math.max(position, 0), duration);
}

/** `m:ss` (or `h:mm:ss`) for a song position. */
export function formatTime(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const s = total % 60;
  const m = Math.floor(total / 60) % 60;
  const h = Math.floor(total / 3600);
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

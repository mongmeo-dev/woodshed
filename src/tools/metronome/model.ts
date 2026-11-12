// Mirrors src-tauri/src/metronome/pattern.rs (serde camelCase).

export type Accent = "strong" | "normal" | "weak" | "mute";
export type Sound = "click" | "woodblock" | "beep" | "cowbell";
export type TrainerEnd = "hold" | "stop";

export interface Polyrhythm {
  enabled: boolean;
  /** Clicks of the second layer per cycle. */
  pulses: number;
  /** Main beats per cycle (3:2 = 3 pulses over 2 beats). */
  beats: number;
  sound: Sound;
}

export interface Trainer {
  enabled: boolean;
  everyBars: number;
  stepBpm: number;
  maxBpm: number;
  atMax: TrainerEnd;
}

export interface MetronomeSettings {
  bpm: number;
  /** Meter numerator. */
  beats: number;
  /** Meter denominator; one beat is one of these notes. */
  noteValue: number;
  accents: Accent[];
  /** Clicks per beat, 1 = none. */
  subdivision: number;
  sound: Sound;
  polyrhythm: Polyrhythm;
  trainer: Trainer;
}

export const MIN_BPM = 20;
export const MAX_BPM = 400;
export const MAX_BEATS = 16;
export const NOTE_VALUES = [1, 2, 4, 8, 16, 32] as const;
export const SUBDIVISIONS = [1, 2, 3, 4, 5, 6] as const;
export const MAX_SUBDIVISION = 8;
export const MAX_POLY = 16;
export const MAX_TRAINER_BARS = 64;
export const MIN_TRAINER_STEP = 0.1;
export const MAX_TRAINER_STEP = 50;
export const SOUNDS: readonly Sound[] = ["click", "woodblock", "beep", "cowbell"];
export const ACCENTS: readonly Accent[] = ["strong", "normal", "weak", "mute"];
export const MAX_PRESET_NAME = 40;

/** Clicking a beat steps through the accents in this order. */
export const NEXT_ACCENT: Record<Accent, Accent> = {
  strong: "normal",
  normal: "weak",
  weak: "mute",
  mute: "strong",
};

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/** Rounds to tenths (as the engine does) and clamps to 20.0–400.0. */
export function roundBpm(bpm: number): number {
  if (!Number.isFinite(bpm)) return 120;
  return clamp(Math.round(bpm * 10), MIN_BPM * 10, MAX_BPM * 10) / 10;
}

function groupStarts(beats: number): number[] {
  if (beats % 3 === 0) {
    return Array.from({ length: beats / 3 }, (_, group) => group * 3);
  }
  const starts: number[] = [];
  let beat = 0;
  while (beat < beats) {
    starts.push(beat);
    beat += beats - beat === 3 ? 3 : 2;
  }
  return starts;
}

/**
 * Accents a meter starts with: a strong downbeat; in eighth-note (and
 * shorter) meters the other group starts are normal and the rest weak
 * (6/8 → 3+3, 7/8 → 2+2+3); otherwise every other beat is normal.
 */
export function defaultAccents(beats: number, noteValue: number): Accent[] {
  const count = clamp(Math.round(beats), 1, MAX_BEATS);
  const grouped = noteValue >= 8 && count > 3;
  const starts = grouped ? groupStarts(count) : [];
  return Array.from({ length: count }, (_, beat) => {
    if (beat === 0) return "strong";
    if (!grouped || starts.includes(beat)) return "normal";
    return "weak";
  });
}

export const DEFAULT_SETTINGS: MetronomeSettings = {
  bpm: 120,
  beats: 4,
  noteValue: 4,
  accents: defaultAccents(4, 4),
  subdivision: 1,
  sound: "click",
  polyrhythm: { enabled: false, pulses: 3, beats: 2, sound: "woodblock" },
  trainer: { enabled: false, everyBars: 4, stepBpm: 5, maxBpm: 160, atMax: "hold" },
};

/** New meter; accents start over from the meter's defaults. */
export function withMeter(
  settings: MetronomeSettings,
  beats: number,
  noteValue: number,
): MetronomeSettings {
  const count = clamp(Math.round(beats), 1, MAX_BEATS);
  const value = (NOTE_VALUES as readonly number[]).includes(noteValue) ? noteValue : 4;
  return { ...settings, beats: count, noteValue: value, accents: defaultAccents(count, value) };
}

function int(value: unknown, fallback: number, min: number, max: number): number {
  return typeof value === "number" && Number.isFinite(value)
    ? clamp(Math.round(value), min, max)
    : fallback;
}

function oneOf<T extends string>(value: unknown, options: readonly T[], fallback: T): T {
  return options.includes(value as T) ? (value as T) : fallback;
}

/** Fills gaps and repairs values, so anything loaded is playable. */
export function normalizeSettings(raw: unknown): MetronomeSettings {
  const r = (typeof raw === "object" && raw !== null ? raw : {}) as Record<string, unknown>;
  const d = DEFAULT_SETTINGS;
  const beats = int(r.beats, d.beats, 1, MAX_BEATS);
  const noteValue = (NOTE_VALUES as readonly number[]).includes(r.noteValue as number)
    ? (r.noteValue as number)
    : d.noteValue;
  const accents =
    Array.isArray(r.accents) && r.accents.length === beats
      ? r.accents.map((accent) => oneOf(accent, ACCENTS, "normal"))
      : defaultAccents(beats, noteValue);
  const p = (
    typeof r.polyrhythm === "object" && r.polyrhythm !== null ? r.polyrhythm : {}
  ) as Record<string, unknown>;
  const t = (typeof r.trainer === "object" && r.trainer !== null ? r.trainer : {}) as Record<
    string,
    unknown
  >;
  const stepBpm =
    typeof t.stepBpm === "number" && Number.isFinite(t.stepBpm)
      ? clamp(Math.round(t.stepBpm * 10) / 10, MIN_TRAINER_STEP, MAX_TRAINER_STEP)
      : d.trainer.stepBpm;
  return {
    bpm: typeof r.bpm === "number" ? roundBpm(r.bpm) : d.bpm,
    beats,
    noteValue,
    accents,
    subdivision: int(r.subdivision, d.subdivision, 1, MAX_SUBDIVISION),
    sound: oneOf(r.sound, SOUNDS, d.sound),
    polyrhythm: {
      enabled: p.enabled === true,
      pulses: int(p.pulses, d.polyrhythm.pulses, 1, MAX_POLY),
      beats: int(p.beats, d.polyrhythm.beats, 1, MAX_POLY),
      sound: oneOf(p.sound, SOUNDS, d.polyrhythm.sound),
    },
    trainer: {
      enabled: t.enabled === true,
      everyBars: int(t.everyBars, d.trainer.everyBars, 1, MAX_TRAINER_BARS),
      stepBpm,
      maxBpm: typeof t.maxBpm === "number" ? roundBpm(t.maxBpm) : d.trainer.maxBpm,
      atMax: oneOf(t.atMax, ["hold", "stop"] as const, d.trainer.atMax),
    },
  };
}

/** "120.0" in the UI language's digits and separator. */
export function formatBpm(bpm: number, language: string): string {
  return new Intl.NumberFormat(language, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  }).format(bpm);
}

/** Parses typed tempo ("96", "96.5", "96,5"); `null` if not a number in range. */
export function parseBpm(text: string): number | null {
  const value = Number(text.trim().replace(",", "."));
  if (text.trim() === "" || !Number.isFinite(value)) return null;
  if (value < MIN_BPM || value > MAX_BPM) return null;
  return roundBpm(value);
}

/**
 * Note value of a subdivision click when it is a plain note: halving or
 * quartering the beat note (in 6/8, 2 → 16th, 4 → 32nd). `null` for tuplets
 * and for no subdivision.
 */
export function subdivisionNote(subdivision: number, noteValue: number): number | null {
  return subdivision === 2 || subdivision === 4 ? noteValue * subdivision : null;
}

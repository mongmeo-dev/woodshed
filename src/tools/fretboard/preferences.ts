import { type InstrumentId, isInstrumentId, isScaleId, type ScaleId } from "../../lib/theory";

/**
 * The fretboard's last choices, kept in webview storage like the theme and
 * language cache (`src/lib/prefsCache.ts`), so they stay out of the shared
 * settings file.
 */
const KEY = "woodshed.fretboard";

export const FRET_COUNTS = [12, 15, 24] as const;
export type FretCount = (typeof FRET_COUNTS)[number];

export type DisplayMode = "notes" | "intervals";

export interface FretboardPrefs {
  instrument: InstrumentId;
  /** Pitch class of the key, 0 = C. */
  key: number;
  scale: ScaleId;
  display: DisplayMode;
  frets: FretCount;
  leftHanded: boolean;
}

export const DEFAULT_PREFS: FretboardPrefs = {
  instrument: "guitar6",
  key: 0,
  scale: "major",
  display: "notes",
  frets: 15,
  leftHanded: false,
};

/** Fills gaps and drops unknown values, so a stored object is always usable. */
export function normalizePrefs(raw: unknown): FretboardPrefs {
  const value = (typeof raw === "object" && raw !== null ? raw : {}) as Record<string, unknown>;
  const key = value.key;
  return {
    instrument: isInstrumentId(value.instrument) ? value.instrument : DEFAULT_PREFS.instrument,
    key: typeof key === "number" && Number.isInteger(key) && key >= 0 && key < 12 ? key : 0,
    scale: isScaleId(value.scale) ? value.scale : DEFAULT_PREFS.scale,
    display: value.display === "intervals" ? "intervals" : "notes",
    frets: FRET_COUNTS.find((count) => count === value.frets) ?? DEFAULT_PREFS.frets,
    leftHanded: value.leftHanded === true,
  };
}

export function readPrefs(): FretboardPrefs {
  try {
    return normalizePrefs(JSON.parse(localStorage.getItem(KEY) ?? "null"));
  } catch {
    return DEFAULT_PREFS;
  }
}

export function writePrefs(prefs: FretboardPrefs): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(prefs));
  } catch {
    // Storage can be unavailable; the choices simply are not remembered.
  }
}

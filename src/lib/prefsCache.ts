import type { Language } from "../i18n";

/**
 * Last applied theme and language, kept in webview storage so the first frame
 * already matches the saved settings (the settings file is read asynchronously).
 * Settings from the app data directory stay the source of truth.
 */
const KEY = "woodshed.prefs";

export interface CachedPrefs {
  theme: "system" | "light" | "dark";
  language: Language | null;
}

export function readCachedPrefs(): CachedPrefs | null {
  try {
    const raw = JSON.parse(localStorage.getItem(KEY) ?? "null") as Partial<CachedPrefs> | null;
    if (!raw) return null;
    const theme = raw.theme === "light" || raw.theme === "dark" ? raw.theme : "system";
    const language = raw.language === "ko" || raw.language === "en" ? raw.language : null;
    return { theme, language };
  } catch {
    return null;
  }
}

export function writeCachedPrefs(prefs: CachedPrefs): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(prefs));
  } catch {
    // Storage can be unavailable; the settings file still applies.
  }
}

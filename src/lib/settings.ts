import type { Language } from "../i18n";
import { call } from "./ipc";

export type Theme = "system" | "light" | "dark";

/** Mirrors `Settings` in src-tauri/src/settings.rs. */
export interface Settings {
  language: Language | null;
  outputDeviceId: string | null;
  theme: Theme;
}

export const DEFAULT_SETTINGS: Settings = {
  language: null,
  outputDeviceId: null,
  theme: "system",
};

/** Fills gaps and drops unknown values, so a partial object is always usable. */
export function normalizeSettings(raw: Partial<Record<keyof Settings, unknown>> | null): Settings {
  const language = raw?.language === "ko" || raw?.language === "en" ? raw.language : null;
  const theme =
    raw?.theme === "light" || raw?.theme === "dark" || raw?.theme === "system"
      ? raw.theme
      : "system";
  const outputDeviceId = typeof raw?.outputDeviceId === "string" ? raw.outputDeviceId : null;
  return { language, outputDeviceId, theme };
}

export async function loadSettings(): Promise<Settings> {
  return normalizeSettings(await call<Partial<Settings>>("settings_load", {}, DEFAULT_SETTINGS));
}

export async function saveSettings(settings: Settings): Promise<Settings> {
  return normalizeSettings(await call<Partial<Settings>>("settings_save", { settings }, settings));
}

/** Applies the theme to the document (`data-theme` overrides the system scheme). */
export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  if (theme === "system") delete root.dataset.theme;
  else root.dataset.theme = theme;
}

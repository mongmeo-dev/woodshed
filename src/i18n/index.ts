import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import { readCachedPrefs } from "../lib/prefsCache";
import en from "../locales/en.json";
import ko from "../locales/ko.json";

export const LANGUAGES = ["ko", "en"] as const;
export type Language = (typeof LANGUAGES)[number];

export const resources = {
  ko: { translation: ko },
  en: { translation: en },
} as const;

/** Native names, shown the same in every UI language. */
export const LANGUAGE_NAMES: Record<Language, string> = {
  ko: "한국어",
  en: "English",
};

/** Picks the UI language: the saved choice, else the system language. */
export function resolveLanguage(
  saved: Language | null,
  systemLanguages: readonly string[],
): Language {
  if (saved) return saved;
  for (const tag of systemLanguages) {
    const base = tag.toLowerCase().split("-")[0];
    if (base === "ko" || base === "en") return base;
  }
  return "en";
}

export function systemLanguages(): readonly string[] {
  if (typeof navigator === "undefined") return [];
  return navigator.languages?.length ? navigator.languages : [navigator.language];
}

export async function setLanguage(language: Language): Promise<void> {
  await i18next.changeLanguage(language);
  document.documentElement.lang = language;
}

export const i18n = i18next;

void i18next.use(initReactI18next).init({
  resources,
  lng: resolveLanguage(readCachedPrefs()?.language ?? null, systemLanguages()),
  fallbackLng: "en",
  interpolation: { escapeValue: false },
  returnNull: false,
});

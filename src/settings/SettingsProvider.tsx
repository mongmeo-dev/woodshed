import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { resolveLanguage, setLanguage, systemLanguages } from "../i18n";
import { writeCachedPrefs } from "../lib/prefsCache";
import {
  applyTheme,
  DEFAULT_SETTINGS,
  loadSettings,
  type Settings,
  saveSettings,
} from "../lib/settings";

interface SettingsContextValue {
  settings: Settings;
  loaded: boolean;
  saveFailed: boolean;
  update: (patch: Partial<Settings>) => Promise<void>;
}

const SettingsContext = createContext<SettingsContextValue | null>(null);

function apply(settings: Settings) {
  applyTheme(settings.theme);
  writeCachedPrefs({ theme: settings.theme, language: settings.language });
  void setLanguage(resolveLanguage(settings.language, systemLanguages()));
}

export function SettingsProvider({ children }: { children: ReactNode }) {
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [loaded, setLoaded] = useState(false);
  const [saveFailed, setSaveFailed] = useState(false);
  const latest = useRef(settings);

  useEffect(() => {
    let cancelled = false;
    void loadSettings()
      .catch(() => DEFAULT_SETTINGS)
      .then((loadedSettings) => {
        if (cancelled) return;
        latest.current = loadedSettings;
        setSettings(loadedSettings);
        apply(loadedSettings);
        setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const update = useCallback(async (patch: Partial<Settings>) => {
    const next = { ...latest.current, ...patch };
    latest.current = next;
    setSettings(next);
    apply(next);
    try {
      await saveSettings(next);
      setSaveFailed(false);
    } catch {
      setSaveFailed(true);
    }
  }, []);

  const value = useMemo(
    () => ({ settings, loaded, saveFailed, update }),
    [settings, loaded, saveFailed, update],
  );
  return <SettingsContext.Provider value={value}>{children}</SettingsContext.Provider>;
}

export function useSettings(): SettingsContextValue {
  const context = useContext(SettingsContext);
  if (!context) throw new Error("useSettings must be used inside SettingsProvider");
  return context;
}

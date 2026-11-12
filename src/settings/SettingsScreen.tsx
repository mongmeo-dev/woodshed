import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { LANGUAGE_NAMES, type Language } from "../i18n";
import {
  type AudioStatus,
  type DeviceInfo,
  delayUntilHeard,
  getAudioStatus,
  listDevices,
  onDevicesChanged,
  onPlaybackEvent,
  playTestSound,
  setOutputDevice,
} from "../lib/audio";
import type { Theme } from "../lib/settings";
import { SegmentedControl } from "../shell/SegmentedControl";
import { useSettings } from "./SettingsProvider";

const SYSTEM_DEFAULT = "";
const STATUS_POLL_MS = 1000;
const BEAT_FLASH_MS = 600;

function useAudioStatus() {
  const [status, setStatus] = useState<AudioStatus | null>(null);
  const [devices, setDevices] = useState<DeviceInfo[]>([]);

  const refresh = useCallback(async () => {
    const [nextStatus, nextDevices] = await Promise.all([getAudioStatus(), listDevices("output")]);
    setStatus(nextStatus);
    setDevices(nextDevices);
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => {
      void getAudioStatus().then(setStatus);
    }, STATUS_POLL_MS);
    let unlisten: (() => void) | undefined;
    void onDevicesChanged(() => void refresh()).then((stop) => {
      unlisten = stop;
    });
    return () => {
      window.clearInterval(timer);
      unlisten?.();
    };
  }, [refresh]);

  return { status, setStatus, devices };
}

/**
 * Example of the engine → screen sync contract: the test sound is scheduled
 * with `notify`, the engine reports when it is heard over a Tauri channel, and
 * the dot lights up at that moment.
 */
function useHeardBeat() {
  const [beat, setBeat] = useState(0);
  const pendingFrame = useRef<number | null>(null);
  const timers = useRef<number[]>([]);

  useEffect(() => {
    const stop = onPlaybackEvent((event) => {
      if (event.renderedFrame !== pendingFrame.current || event.heardAtMs === null) return;
      pendingFrame.current = null;
      const showAt = window.setTimeout(() => {
        setBeat((count) => count + 1);
        const hideAt = window.setTimeout(() => setBeat(0), BEAT_FLASH_MS);
        timers.current.push(hideAt);
      }, delayUntilHeard(event.heardAtMs));
      timers.current.push(showAt);
    });
    return () => {
      stop();
      for (const timer of timers.current) window.clearTimeout(timer);
    };
  }, []);

  const play = useCallback(async () => {
    const scheduled = await playTestSound();
    if (scheduled) pendingFrame.current = scheduled.frame;
  }, []);

  return { beat, play };
}

function formatNumber(value: number, language: string, digits = 0) {
  return new Intl.NumberFormat(language, { maximumFractionDigits: digits }).format(value);
}

export function SettingsScreen() {
  const { t, i18n } = useTranslation();
  const { settings, update, saveFailed } = useSettings();
  const { status, setStatus, devices } = useAudioStatus();
  const { beat, play } = useHeardBeat();
  const [outputError, setOutputError] = useState(false);
  const language = i18n.language;
  const output = status?.output ?? null;

  const changeDevice = async (value: string) => {
    const deviceId = value === SYSTEM_DEFAULT ? null : value;
    try {
      setStatus(await setOutputDevice(deviceId));
      setOutputError(false);
      await update({ outputDeviceId: deviceId });
    } catch {
      setOutputError(true);
    }
  };

  const languageOptions = [
    { value: "system", label: t("settings.language.system") },
    { value: "ko", label: LANGUAGE_NAMES.ko },
    { value: "en", label: LANGUAGE_NAMES.en },
  ] as const;
  const themeOptions = [
    { value: "system", label: t("settings.theme.system") },
    { value: "light", label: t("settings.theme.light") },
    { value: "dark", label: t("settings.theme.dark") },
  ] as const;

  return (
    <section className="screen" aria-labelledby="screen-title-settings">
      <header className="screen-header" data-tauri-drag-region>
        <h1 className="screen-title" id="screen-title-settings">
          {t("settings.title")}
        </h1>
      </header>
      <div className="screen-body">
        <div className="settings">
          <section aria-labelledby="settings-output">
            <h2 className="settings-section-title" id="settings-output">
              {t("settings.output.section")}
            </h2>
            <div className="card">
              <div className="row">
                <label className="row-label" htmlFor="output-device">
                  {t("settings.output.device")}
                </label>
                <select
                  id="output-device"
                  className="select"
                  value={settings.outputDeviceId ?? SYSTEM_DEFAULT}
                  onChange={(event) => void changeDevice(event.target.value)}
                >
                  <option value={SYSTEM_DEFAULT}>{t("settings.output.systemDefault")}</option>
                  {devices.map((device) => (
                    <option key={device.id} value={device.id}>
                      {device.name}
                    </option>
                  ))}
                </select>
              </div>
              {(outputError || (status && !output)) && (
                <div className="row">
                  <span className="row-error" role="alert">
                    {t("settings.output.notAvailable")}
                  </span>
                </div>
              )}
              <div className="row">
                <span className="row-label">{t("settings.output.sampleRate")}</span>
                <span className="row-value">
                  {output
                    ? t("units.hz", { value: formatNumber(output.sampleRate, language) })
                    : "–"}
                </span>
              </div>
              <div className="row">
                <span className="row-label">{t("settings.output.buffer")}</span>
                <span className="row-value">
                  {output?.bufferFrames ? t("units.frames", { value: output.bufferFrames }) : "–"}
                </span>
              </div>
              <div className="row">
                <span className="row-label">{t("settings.output.latency")}</span>
                <span className="row-value">
                  {status?.outputLatencyMs != null
                    ? t("units.ms", { value: formatNumber(status.outputLatencyMs, language, 1) })
                    : "–"}
                </span>
              </div>
              <div className="row">
                <div className="test-sound">
                  <button
                    type="button"
                    className="button-primary"
                    onClick={() => void play()}
                    disabled={!output}
                  >
                    {t("settings.output.test")}
                  </button>
                  <span
                    className="beat-dot"
                    data-beat={beat > 0 ? "on" : "off"}
                    key={beat}
                    aria-hidden="true"
                  />
                </div>
              </div>
            </div>
          </section>

          <section aria-labelledby="settings-language">
            <h2 className="settings-section-title" id="settings-language">
              {t("settings.language.section")}
            </h2>
            <div className="card">
              <div className="row">
                <SegmentedControl
                  label={t("settings.language.section")}
                  options={languageOptions}
                  value={settings.language ?? "system"}
                  onChange={(value) =>
                    void update({ language: value === "system" ? null : (value as Language) })
                  }
                />
              </div>
            </div>
          </section>

          <section aria-labelledby="settings-theme">
            <h2 className="settings-section-title" id="settings-theme">
              {t("settings.theme.section")}
            </h2>
            <div className="card">
              <div className="row">
                <SegmentedControl<Theme>
                  label={t("settings.theme.section")}
                  options={themeOptions}
                  value={settings.theme}
                  onChange={(theme) => void update({ theme })}
                />
              </div>
            </div>
          </section>

          {saveFailed && (
            <p className="row-error" role="alert">
              {t("settings.saveFailed")}
            </p>
          )}
        </div>
      </div>
    </section>
  );
}

import "./tuner.css";
import { type RefObject, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { DeviceInfo } from "../../lib/audio";
import { SegmentedControl } from "../../shell/SegmentedControl";
import { TunerReadout } from "./TunerReadout";
import {
  channelFromKey,
  channelKey,
  MAX_REFERENCE_HZ,
  MIN_REFERENCE_HZ,
  noteName,
  OPEN_STRINGS,
  openMicrophoneSettings,
  type StringSet,
} from "./tuner";
import { type LatestFrame, useTuner } from "./useTuner";

const SYSTEM_DEFAULT = "";
const STRING_SET_KEY = "woodshed.tuner.strings";

function readStringSet(): StringSet {
  try {
    const saved = localStorage.getItem(STRING_SET_KEY);
    if (saved && saved in OPEN_STRINGS) return saved as StringSet;
  } catch {
    // Storage can be unavailable; the default applies.
  }
  return "guitar6";
}

function writeStringSet(set: StringSet) {
  try {
    localStorage.setItem(STRING_SET_KEY, set);
  } catch {
    // Not kept; harmless.
  }
}

/** Which open string the current note is, polled from the latest frame. */
function useCurrentMidi(latest: RefObject<LatestFrame>): number | null {
  const [midi, setMidi] = useState<number | null>(null);
  useEffect(() => {
    let frame = 0;
    const poll = () => {
      setMidi(latest.current.pitch?.midi ?? null);
      frame = requestAnimationFrame(poll);
    };
    frame = requestAnimationFrame(poll);
    return () => cancelAnimationFrame(frame);
  }, [latest]);
  return midi;
}

function OpenStrings({ latest }: { latest: RefObject<LatestFrame> }) {
  const { t } = useTranslation();
  const [set, setSet] = useState<StringSet>(readStringSet);
  const current = useCurrentMidi(latest);
  const options = [
    { value: "guitar6", label: t("tuner.strings.guitar6") },
    { value: "bass4", label: t("tuner.strings.bass4") },
    { value: "bass5", label: t("tuner.strings.bass5") },
  ] as const;

  return (
    <div className="tuner-strings">
      <SegmentedControl<StringSet>
        label={t("tuner.strings.label")}
        options={options}
        value={set}
        onChange={(next) => {
          setSet(next);
          writeStringSet(next);
        }}
      />
      <ol className="tuner-string-list" aria-label={t("tuner.strings.label")}>
        {OPEN_STRINGS[set].map((midi) => {
          const { name, octave } = noteName(midi);
          return (
            <li key={midi} className="tuner-string" data-current={current === midi}>
              {name}
              <span className="tuner-string-octave">{octave}</span>
            </li>
          );
        })}
      </ol>
    </div>
  );
}

function StepIcon({ kind }: { kind: "minus" | "plus" }) {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
      <path d={kind === "minus" ? "M3 8h10" : "M3 8h10M8 3v10"} />
    </svg>
  );
}

function channelCount(devices: DeviceInfo[], deviceId: string | null): number {
  const device = deviceId
    ? devices.find((d) => d.id === deviceId)
    : devices.find((d) => d.isDefault);
  return Math.max(1, device?.channels ?? 1);
}

export function TunerScreen() {
  const { t } = useTranslation();
  const { status, devices, latest, saveFailed, chooseInput, changeReference } = useTuner();
  const preferred = status.preferredDeviceId;
  const channels = channelCount(devices, preferred);
  const selectedChannel = channelKey(status.channel);
  const channelOptions = Array.from({ length: channels }, (_, index) => String(index));
  if (channels > 1) channelOptions.push("mix");
  // Keep showing a choice the device list does not have (yet).
  const channelValue = channelOptions.includes(selectedChannel) ? selectedChannel : "0";
  const missingPreferred = preferred !== null && !devices.some((d) => d.id === preferred);

  return (
    <section className="screen" aria-labelledby="screen-title-tuner">
      <header className="screen-header" data-tauri-drag-region>
        <h1 className="screen-title" id="screen-title-tuner">
          {t("tools.tuner")}
        </h1>
      </header>
      <div className="screen-body">
        <div className="tuner">
          <TunerReadout latest={latest} levelLabel={t("tuner.input.level")} />

          {status.state === "permissionDenied" && (
            <div className="tuner-notice" role="alert">
              <span>{t("tuner.status.permissionDenied")}</span>
              <button
                type="button"
                className="button-primary"
                onClick={() => void openMicrophoneSettings()}
              >
                {t("tuner.status.openSettings")}
              </button>
            </div>
          )}
          {status.state === "permissionPending" && (
            <p className="tuner-notice" role="status">
              {t("tuner.status.permissionPending")}
            </p>
          )}
          {status.state === "noInput" && (
            <p className="tuner-notice" role="alert">
              {t("tuner.status.noInput")}
            </p>
          )}

          <OpenStrings latest={latest} />

          <div className="card tuner-card">
            <div className="row">
              <label className="row-label" htmlFor="tuner-device">
                {t("tuner.input.device")}
              </label>
              <select
                id="tuner-device"
                className="select"
                value={preferred ?? SYSTEM_DEFAULT}
                onChange={(event) => {
                  const deviceId =
                    event.target.value === SYSTEM_DEFAULT ? null : event.target.value;
                  const count = channelCount(devices, deviceId);
                  const channel =
                    status.channel.type === "single" && status.channel.channel >= count
                      ? { type: "single" as const, channel: 0 }
                      : status.channel;
                  void chooseInput(deviceId, channel);
                }}
              >
                <option value={SYSTEM_DEFAULT}>{t("tuner.input.systemDefault")}</option>
                {devices.map((device) => (
                  <option key={device.id} value={device.id}>
                    {device.name}
                  </option>
                ))}
                {missingPreferred && (
                  <option value={preferred}>{t("tuner.input.disconnected")}</option>
                )}
              </select>
            </div>
            {status.fallback && status.input && (
              <div className="row">
                <span className="row-label">{t("tuner.input.using")}</span>
                <span className="row-value">{status.input.deviceName}</span>
              </div>
            )}
            <div className="row">
              <label className="row-label" htmlFor="tuner-channel">
                {t("tuner.input.channel")}
              </label>
              <select
                id="tuner-channel"
                className="select"
                value={channelValue}
                disabled={channels < 2}
                onChange={(event) =>
                  void chooseInput(preferred, channelFromKey(event.target.value))
                }
              >
                {channelOptions.map((key) => (
                  <option key={key} value={key}>
                    {key === "mix"
                      ? t("tuner.input.mixDown")
                      : t("tuner.input.channelNumber", { value: Number(key) + 1 })}
                  </option>
                ))}
              </select>
            </div>
            <div className="row">
              <span className="row-label" aria-hidden="true">
                {t("tuner.reference.label")}
              </span>
              <fieldset className="tuner-stepper">
                <legend className="visually-hidden">{t("tuner.reference.label")}</legend>
                <button
                  type="button"
                  className="tuner-step"
                  aria-label={t("tuner.reference.lower")}
                  disabled={status.referenceHz <= MIN_REFERENCE_HZ}
                  onClick={() => void changeReference(status.referenceHz - 1)}
                >
                  <StepIcon kind="minus" />
                </button>
                <output className="row-value tuner-reference" aria-live="polite">
                  {t("tuner.reference.value", { value: status.referenceHz })}
                </output>
                <button
                  type="button"
                  className="tuner-step"
                  aria-label={t("tuner.reference.raise")}
                  disabled={status.referenceHz >= MAX_REFERENCE_HZ}
                  onClick={() => void changeReference(status.referenceHz + 1)}
                >
                  <StepIcon kind="plus" />
                </button>
              </fieldset>
            </div>
          </div>

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

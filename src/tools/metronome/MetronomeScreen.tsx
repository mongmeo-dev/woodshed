import "./metronome.css";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { SegmentedControl } from "../../shell/SegmentedControl";
import { BeatRow, PulseRow } from "./BeatRow";
import { BpmControl } from "./BpmControl";
import {
  MAX_BEATS,
  MAX_BPM,
  MAX_POLY,
  MAX_TRAINER_BARS,
  MAX_TRAINER_STEP,
  type MetronomeSettings,
  MIN_BPM,
  MIN_TRAINER_STEP,
  NEXT_ACCENT,
  NOTE_VALUES,
  normalizeSettings,
  roundBpm,
  SOUNDS,
  type Sound,
  SUBDIVISIONS,
  subdivisionNote,
  type TrainerEnd,
  withMeter,
} from "./model";
import { PresetsCard } from "./PresetsCard";
import { Switch } from "./Switch";
import { TapTempo } from "./tapTempo";
import { useMetronome } from "./useMetronome";

const range = (from: number, to: number) =>
  Array.from({ length: to - from + 1 }, (_, index) => from + index);

/** Text fields and selects keep the space bar; everywhere else it plays/stops. */
function keepsSpace(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true;
  if (target instanceof HTMLInputElement) {
    return !["radio", "checkbox", "button", "range"].includes(target.type);
  }
  return false;
}

function Icon({ name }: { name: "minus" | "plus" }) {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.8} aria-hidden="true">
      <path d={name === "minus" ? "M6 12h12" : "M6 12h12M12 6v12"} strokeLinecap="round" />
    </svg>
  );
}

interface NumberFieldProps {
  id: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  unit?: string;
  onChange: (value: number) => void;
}

/** Number input that applies only in-range values, so typing is never cut short. */
function NumberField({ id, value, min, max, step = 1, unit, onChange }: NumberFieldProps) {
  const [draft, setDraft] = useState<string | null>(null);
  return (
    <span className="number-field">
      <input
        id={id}
        className="number-input"
        type="number"
        min={min}
        max={max}
        step={step}
        value={draft ?? String(value)}
        onChange={(event) => {
          const text = event.target.value;
          setDraft(text);
          const next = Number(text);
          if (text !== "" && Number.isFinite(next) && next >= min && next <= max) onChange(next);
        }}
        onBlur={() => setDraft(null)}
      />
      {unit && <span className="number-unit">{unit}</span>}
    </span>
  );
}

export function MetronomeScreen() {
  const { t } = useTranslation();
  const { settings, presets, playing, heard, change, start, stop, savePreset, deletePreset } =
    useMetronome();
  const tap = useRef(new TapTempo());
  const toggle = useRef(() => {});
  toggle.current = playing ? stop : start;

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== " " || keepsSpace(event.target)) return;
      // Also on keyup, so a focused button does not click on release.
      event.preventDefault();
      if (event.type === "keydown" && !event.repeat) toggle.current();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onKey);
    };
  }, []);

  const update = (patch: Partial<MetronomeSettings>) => change({ ...settings, ...patch });
  const setBpm = (bpm: number) => update({ bpm: roundBpm(bpm) });
  const trainerPlaying = playing && settings.trainer.enabled && heard.beat !== null;
  const shownBpm = trainerPlaying && heard.beat ? heard.beat.bpm : settings.bpm;
  const activeBeat =
    playing && heard.beat && heard.beat.beats === settings.beats ? heard.beat.beat : null;
  const activePulse =
    playing && heard.pulse && heard.pulse.pulses === settings.polyrhythm.pulses
      ? heard.pulse.pulse
      : null;

  const soundOptions = SOUNDS.map((sound) => ({
    value: sound,
    label: t(`metronome.sound.${sound}`),
  }));
  // Keys built from numbers; both families exist in ko and en (i18n.test.ts).
  const label = t as (key: string) => string;
  const subdivisionOptions = SUBDIVISIONS.map((value) => {
    const note = subdivisionNote(value, settings.noteValue);
    return {
      value: String(value),
      label: note ? label(`metronome.noteName.${note}`) : label(`metronome.subdivision.${value}`),
    };
  });
  const atMaxOptions = [
    { value: "hold", label: t("metronome.trainer.hold") },
    { value: "stop", label: t("metronome.trainer.stop") },
  ] as const;

  return (
    <section className="screen" aria-labelledby="screen-title-metronome">
      <header className="screen-header" data-tauri-drag-region>
        <h1 className="screen-title" id="screen-title-metronome">
          {t("tools.metronome")}
        </h1>
      </header>
      <div className="screen-body">
        <div className="metronome">
          <div className="metronome-stage">
            <BeatRow
              accents={settings.accents}
              active={activeBeat}
              onCycle={(beat) =>
                update({
                  accents: settings.accents.map((accent, index) =>
                    index === beat ? NEXT_ACCENT[accent] : accent,
                  ),
                })
              }
            />
            {settings.polyrhythm.enabled && (
              <PulseRow pulses={settings.polyrhythm.pulses} active={activePulse} />
            )}
            <BpmControl display={shownBpm} value={settings.bpm} onChange={setBpm} />
            <div className="metronome-transport">
              <button
                type="button"
                className="icon-button"
                aria-label={t("metronome.slower")}
                onClick={() => setBpm(settings.bpm - 1)}
              >
                <Icon name="minus" />
              </button>
              <button
                type="button"
                className="button-secondary"
                onClick={() => {
                  const bpm = tap.current.tap(performance.now());
                  if (bpm !== null) setBpm(bpm);
                }}
              >
                {t("metronome.tap")}
              </button>
              <button
                type="button"
                className="button-primary transport-play"
                aria-pressed={playing}
                onClick={() => toggle.current()}
              >
                {playing ? t("metronome.stop") : t("metronome.start")}
              </button>
              <button
                type="button"
                className="icon-button"
                aria-label={t("metronome.faster")}
                onClick={() => setBpm(settings.bpm + 1)}
              >
                <Icon name="plus" />
              </button>
            </div>
          </div>

          <section aria-labelledby="metronome-rhythm">
            <h2 className="settings-section-title" id="metronome-rhythm">
              {t("metronome.rhythm")}
            </h2>
            <div className="card">
              <div className="row">
                <span className="row-label" aria-hidden="true">
                  {t("metronome.meter")}
                </span>
                <fieldset className="meter">
                  <legend className="visually-hidden">{t("metronome.meter")}</legend>
                  <select
                    className="select"
                    aria-label={t("metronome.beatsPerBar")}
                    value={settings.beats}
                    onChange={(event) =>
                      change(withMeter(settings, Number(event.target.value), settings.noteValue))
                    }
                  >
                    {range(1, MAX_BEATS).map((beats) => (
                      <option key={beats} value={beats}>
                        {beats}
                      </option>
                    ))}
                  </select>
                  <span className="meter-slash" aria-hidden="true" />
                  <select
                    className="select"
                    aria-label={t("metronome.beatUnit")}
                    value={settings.noteValue}
                    onChange={(event) =>
                      change(withMeter(settings, settings.beats, Number(event.target.value)))
                    }
                  >
                    {NOTE_VALUES.map((value) => (
                      <option key={value} value={value}>
                        {value}
                      </option>
                    ))}
                  </select>
                </fieldset>
              </div>
              <div className="row">
                <span className="row-label">{t("metronome.subdivisionLabel")}</span>
                <SegmentedControl
                  label={t("metronome.subdivisionLabel")}
                  options={subdivisionOptions}
                  value={String(settings.subdivision)}
                  onChange={(value) => update({ subdivision: Number(value) })}
                />
              </div>
              <div className="row">
                <span className="row-label">{t("metronome.soundLabel")}</span>
                <SegmentedControl<Sound>
                  label={t("metronome.soundLabel")}
                  options={soundOptions}
                  value={settings.sound}
                  onChange={(sound) => update({ sound })}
                />
              </div>
            </div>
          </section>

          <section aria-labelledby="metronome-trainer">
            <h2 className="settings-section-title" id="metronome-trainer">
              {t("metronome.trainer.section")}
            </h2>
            <div className="card">
              <div className="row">
                <span className="row-label" id="metronome-trainer-on">
                  {t("metronome.trainer.on")}
                </span>
                <Switch
                  checked={settings.trainer.enabled}
                  labelledBy="metronome-trainer metronome-trainer-on"
                  onChange={(enabled) => update({ trainer: { ...settings.trainer, enabled } })}
                />
              </div>
              {settings.trainer.enabled && (
                <>
                  <div className="row">
                    <label className="row-label" htmlFor="trainer-every">
                      {t("metronome.trainer.every")}
                    </label>
                    <NumberField
                      id="trainer-every"
                      value={settings.trainer.everyBars}
                      min={1}
                      max={MAX_TRAINER_BARS}
                      unit={t("metronome.trainer.bars")}
                      onChange={(everyBars) =>
                        update({
                          trainer: { ...settings.trainer, everyBars: Math.round(everyBars) },
                        })
                      }
                    />
                  </div>
                  <div className="row">
                    <label className="row-label" htmlFor="trainer-step">
                      {t("metronome.trainer.step")}
                    </label>
                    <NumberField
                      id="trainer-step"
                      value={settings.trainer.stepBpm}
                      min={MIN_TRAINER_STEP}
                      max={MAX_TRAINER_STEP}
                      step={0.1}
                      unit={t("metronome.bpm")}
                      onChange={(stepBpm) =>
                        update({
                          trainer: { ...settings.trainer, stepBpm: Math.round(stepBpm * 10) / 10 },
                        })
                      }
                    />
                  </div>
                  <div className="row">
                    <label className="row-label" htmlFor="trainer-max">
                      {t("metronome.trainer.max")}
                    </label>
                    <NumberField
                      id="trainer-max"
                      value={settings.trainer.maxBpm}
                      min={MIN_BPM}
                      max={MAX_BPM}
                      unit={t("metronome.bpm")}
                      onChange={(maxBpm) =>
                        update({ trainer: { ...settings.trainer, maxBpm: roundBpm(maxBpm) } })
                      }
                    />
                  </div>
                  <div className="row">
                    <span className="row-label">{t("metronome.trainer.atMax")}</span>
                    <SegmentedControl<TrainerEnd>
                      label={t("metronome.trainer.atMax")}
                      options={atMaxOptions}
                      value={settings.trainer.atMax}
                      onChange={(atMax) => update({ trainer: { ...settings.trainer, atMax } })}
                    />
                  </div>
                </>
              )}
            </div>
          </section>

          <section aria-labelledby="metronome-poly">
            <h2 className="settings-section-title" id="metronome-poly">
              {t("metronome.poly.section")}
            </h2>
            <div className="card">
              <div className="row">
                <span className="row-label" id="metronome-poly-on">
                  {t("metronome.poly.on")}
                </span>
                <Switch
                  checked={settings.polyrhythm.enabled}
                  labelledBy="metronome-poly metronome-poly-on"
                  onChange={(enabled) =>
                    update({ polyrhythm: { ...settings.polyrhythm, enabled } })
                  }
                />
              </div>
              {settings.polyrhythm.enabled && (
                <>
                  <div className="row">
                    <span className="row-label" aria-hidden="true">
                      {t("metronome.poly.ratio")}
                    </span>
                    <fieldset className="ratio">
                      <legend className="visually-hidden">{t("metronome.poly.ratio")}</legend>
                      <select
                        className="select"
                        aria-label={t("metronome.poly.pulses")}
                        value={settings.polyrhythm.pulses}
                        onChange={(event) =>
                          update({
                            polyrhythm: {
                              ...settings.polyrhythm,
                              pulses: Number(event.target.value),
                            },
                          })
                        }
                      >
                        {range(1, MAX_POLY).map((value) => (
                          <option key={value} value={value}>
                            {value}
                          </option>
                        ))}
                      </select>
                      <span className="ratio-colon" aria-hidden="true" />
                      <select
                        className="select"
                        aria-label={t("metronome.poly.beats")}
                        value={settings.polyrhythm.beats}
                        onChange={(event) =>
                          update({
                            polyrhythm: {
                              ...settings.polyrhythm,
                              beats: Number(event.target.value),
                            },
                          })
                        }
                      >
                        {range(1, MAX_POLY).map((value) => (
                          <option key={value} value={value}>
                            {value}
                          </option>
                        ))}
                      </select>
                    </fieldset>
                  </div>
                  <div className="row">
                    <span className="row-label">{t("metronome.soundLabel")}</span>
                    <SegmentedControl<Sound>
                      label={t("metronome.poly.sound")}
                      options={soundOptions}
                      value={settings.polyrhythm.sound}
                      onChange={(sound) =>
                        update({ polyrhythm: { ...settings.polyrhythm, sound } })
                      }
                    />
                  </div>
                </>
              )}
            </div>
          </section>

          <section aria-labelledby="metronome-presets">
            <h2 className="settings-section-title" id="metronome-presets">
              {t("metronome.presets.section")}
            </h2>
            <PresetsCard
              presets={presets}
              onLoad={(preset) => change(normalizeSettings(preset.settings))}
              onSave={savePreset}
              onDelete={deletePreset}
            />
          </section>
        </div>
      </div>
    </section>
  );
}

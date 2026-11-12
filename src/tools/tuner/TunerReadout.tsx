import { type RefObject, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { levelFraction, noteName } from "./tuner";
import type { LatestFrame } from "./useTuner";

/** Cents within which the note counts as in tune. */
export const IN_TUNE_CENTS = 3;
/** Needle follow time constant (Kinetics "Inertial Dial", without overshoot). */
const NEEDLE_TAU_MS = 40;
const TICKS = [-50, -40, -30, -20, -10, 0, 10, 20, 30, 40, 50];

interface Shown {
  midi: number | null;
  active: boolean;
  cents: number;
  /** Tenths of a hertz. */
  decihertz: number;
}

function reducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

function formatCents(cents: number): string {
  return cents > 0 ? `+${cents}` : String(cents);
}

/**
 * Note, cents needle, frequency and input level. Frames arrive over a Tauri
 * channel into `latest`; one requestAnimationFrame loop draws them.
 */
export function TunerReadout({
  latest,
  levelLabel,
}: {
  latest: RefObject<LatestFrame>;
  levelLabel: string;
}) {
  const { t, i18n } = useTranslation();
  const needle = useRef<HTMLDivElement>(null);
  const level = useRef<HTMLMeterElement>(null);
  const [shown, setShown] = useState<Shown>({ midi: null, active: false, cents: 0, decihertz: 0 });

  useEffect(() => {
    let frame = 0;
    let position = 0;
    let last = performance.now();
    let lastLevel = -1;
    const instant = reducedMotion();
    const draw = (now: number) => {
      const { pitch, levelDb } = latest.current;
      const elapsed = Math.min(100, now - last);
      last = now;
      if (pitch) {
        const target = Math.max(-50, Math.min(50, pitch.cents));
        position = instant
          ? target
          : position + (target - position) * (1 - Math.exp(-elapsed / NEEDLE_TAU_MS));
      }
      if (needle.current) needle.current.style.transform = `translateX(${position}%)`;

      const percent = Math.round(levelFraction(levelDb) * 100);
      if (level.current && percent !== lastLevel) {
        lastLevel = percent;
        level.current.value = percent;
      }

      setShown((current) => {
        const next: Shown = pitch
          ? {
              midi: pitch.midi,
              active: true,
              cents: Math.round(pitch.cents),
              decihertz: Math.round(pitch.frequency * 10),
            }
          : { ...current, active: false };
        return next.midi === current.midi &&
          next.active === current.active &&
          next.cents === current.cents &&
          next.decihertz === current.decihertz
          ? current
          : next;
      });
      frame = requestAnimationFrame(draw);
    };
    frame = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(frame);
  }, [latest]);

  const note = shown.midi === null ? null : noteName(shown.midi);
  const inTune = shown.active && Math.abs(shown.cents) <= IN_TUNE_CENTS;
  const hertz = new Intl.NumberFormat(i18n.language, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  }).format(shown.decihertz / 10);

  return (
    <div className="tuner-readout" data-active={shown.active} data-in-tune={inTune}>
      <output className="tuner-note" aria-label={t("tuner.note")}>
        {note ? (
          <>
            <span className="tuner-note-name">{note.name}</span>
            <span className="tuner-note-octave">{note.octave}</span>
          </>
        ) : (
          <span className="tuner-note-name">–</span>
        )}
      </output>

      <div className="tuner-scale" aria-hidden="true">
        {TICKS.map((tick) => (
          <span
            key={tick}
            className="tuner-tick"
            data-center={tick === 0}
            style={{ left: `${50 + tick}%` }}
          />
        ))}
        <div className="tuner-needle-track" ref={needle}>
          <span className="tuner-needle" />
        </div>
      </div>

      <div className="tuner-values">
        <span>
          <span className="visually-hidden">{t("tuner.centsLabel")} </span>
          {note ? t("tuner.cents", { value: formatCents(shown.cents) }) : "–"}
        </span>
        <span>
          <span className="visually-hidden">{t("tuner.frequency")} </span>
          {note ? t("units.hz", { value: hertz }) : "–"}
        </span>
      </div>

      <div className="tuner-level">
        <span className="tuner-level-label">{levelLabel}</span>
        <meter
          className="tuner-meter"
          aria-label={levelLabel}
          min={0}
          max={100}
          defaultValue={0}
          ref={level}
        />
      </div>
    </div>
  );
}

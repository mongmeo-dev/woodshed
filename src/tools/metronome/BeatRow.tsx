import { useTranslation } from "react-i18next";
import type { Accent } from "./model";

interface BeatRowProps {
  accents: readonly Accent[];
  /** Beat in the bar heard last, or `null` when stopped. */
  active: number | null;
  onCycle: (beat: number) => void;
}

/**
 * One bar per beat; its height shows the accent and a click steps to the
 * next accent. The beat heard last is lit until the next one.
 */
export function BeatRow({ accents, active, onCycle }: BeatRowProps) {
  const { t } = useTranslation();
  return (
    <fieldset className="beat-row">
      <legend className="visually-hidden">{t("metronome.accents")}</legend>
      {accents.map((accent, beat) => (
        <button
          // biome-ignore lint/suspicious/noArrayIndexKey: beats are positions in the bar.
          key={beat}
          type="button"
          className="beat-cell"
          aria-label={t("metronome.beatAccent", {
            beat: beat + 1,
            accent: t(`metronome.accent.${accent}`),
          })}
          onClick={() => onCycle(beat)}
        >
          <span className="beat-bar" data-accent={accent} data-lit={active === beat} />
        </button>
      ))}
    </fieldset>
  );
}

interface PulseRowProps {
  pulses: number;
  active: number | null;
}

/** Dots for the polyrhythm cycle, lit as each pulse is heard. */
export function PulseRow({ pulses, active }: PulseRowProps) {
  return (
    <div className="pulse-row" aria-hidden="true">
      {Array.from({ length: pulses }, (_, pulse) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: pulses are positions in the cycle.
        <span key={pulse} className="pulse-dot" data-lit={active === pulse} />
      ))}
    </div>
  );
}

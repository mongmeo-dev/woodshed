import { useTranslation } from "react-i18next";
import type { TrackName } from "../../lib/stems";
import type { TrackMix } from "./usePlayer";

interface FaderProps {
  label: string;
  value: number;
  onChange: (value: number) => void;
}

/** Volume 0–100 %, as a native range input. */
export function Fader({ label, value, onChange }: FaderProps) {
  const { t } = useTranslation();
  const percent = Math.round(value * 100);
  return (
    <div className="fader">
      <input
        type="range"
        className="range"
        min={0}
        max={100}
        step={1}
        value={percent}
        aria-label={label}
        aria-valuetext={t("stems.percent", { value: percent })}
        onChange={(event) => onChange(Number(event.target.value) / 100)}
      />
      <span className="fader-value">{t("stems.percent", { value: percent })}</span>
    </div>
  );
}

interface ToggleProps {
  label: string;
  short: string;
  pressed: boolean;
  onToggle: () => void;
}

function Toggle({ label, short, pressed, onToggle }: ToggleProps) {
  return (
    <button
      type="button"
      className="mix-toggle"
      aria-label={label}
      aria-pressed={pressed}
      title={label}
      onClick={onToggle}
    >
      {short}
    </button>
  );
}

interface MixerProps {
  tracks: TrackName[];
  mix: TrackMix[];
  master: number;
  onTrack: (index: number, patch: Partial<TrackMix>) => void;
  onMaster: (value: number) => void;
}

export function Mixer({ tracks, mix, master, onTrack, onMaster }: MixerProps) {
  const { t } = useTranslation();
  return (
    <div className="card mixer">
      {tracks.map((track, index) => {
        const state = mix[index];
        if (!state) return null;
        const name = t(`stems.tracks.${track}`);
        return (
          <div className="row mixer-row" key={track}>
            <span className="row-label mixer-name">{name}</span>
            <div className="mixer-toggles">
              <Toggle
                label={t("stems.mixer.mute", { track: name })}
                short={t("stems.mixer.muteShort")}
                pressed={state.muted}
                onToggle={() => onTrack(index, { muted: !state.muted })}
              />
              <Toggle
                label={t("stems.mixer.solo", { track: name })}
                short={t("stems.mixer.soloShort")}
                pressed={state.solo}
                onToggle={() => onTrack(index, { solo: !state.solo })}
              />
            </div>
            <Fader
              label={t("stems.mixer.volume", { track: name })}
              value={state.volume}
              onChange={(volume) => onTrack(index, { volume })}
            />
          </div>
        );
      })}
      <div className="row mixer-row">
        <span className="row-label mixer-name">{t("stems.mixer.master")}</span>
        <div className="mixer-toggles" />
        <Fader label={t("stems.mixer.masterVolume")} value={master} onChange={onMaster} />
      </div>
    </div>
  );
}

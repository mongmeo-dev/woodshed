import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import type { Preset, PresetErrorCode } from "./api";
import { MAX_PRESET_NAME } from "./model";

interface PresetsCardProps {
  presets: readonly Preset[];
  onLoad: (preset: Preset) => void;
  onSave: (name: string) => Promise<PresetErrorCode | null>;
  onDelete: (name: string) => Promise<PresetErrorCode | null>;
}

/** Saved presets: load, delete (after a confirmation), save the current settings. */
export function PresetsCard({ presets, onLoad, onSave, onDelete }: PresetsCardProps) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [error, setError] = useState<PresetErrorCode | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [deleted, setDeleted] = useState<string | null>(null);
  const exists = presets.some((preset) => preset.name === name.trim());

  const save = async (event: FormEvent) => {
    event.preventDefault();
    setDeleted(null);
    const code = await onSave(name);
    setError(code);
    if (!code) setName("");
  };

  const remove = async (target: string) => {
    const code = await onDelete(target);
    setConfirming(null);
    setError(code);
    setDeleted(code ? null : target);
  };

  return (
    <div className="card">
      {presets.length === 0 && (
        <div className="row">
          <span className="row-value">{t("metronome.presets.empty")}</span>
        </div>
      )}
      <ul className="preset-list">
        {presets.map((preset) => (
          <li key={preset.name} className="row">
            {confirming === preset.name ? (
              <>
                <span className="row-label">
                  {t("metronome.presets.confirmDelete", { name: preset.name })}
                </span>
                <span className="row-actions">
                  <button
                    type="button"
                    className="button-secondary"
                    onClick={() => setConfirming(null)}
                  >
                    {t("metronome.presets.cancel")}
                  </button>
                  <button
                    type="button"
                    className="button-primary"
                    onClick={() => void remove(preset.name)}
                  >
                    {t("metronome.presets.delete")}
                  </button>
                </span>
              </>
            ) : (
              <>
                <span className="row-label preset-name">{preset.name}</span>
                <span className="row-actions">
                  <button
                    type="button"
                    className="button-secondary"
                    aria-label={t("metronome.presets.loadNamed", { name: preset.name })}
                    onClick={() => {
                      setDeleted(null);
                      onLoad(preset);
                    }}
                  >
                    {t("metronome.presets.load")}
                  </button>
                  <button
                    type="button"
                    className="button-secondary"
                    aria-label={t("metronome.presets.deleteNamed", { name: preset.name })}
                    onClick={() => {
                      setDeleted(null);
                      setConfirming(preset.name);
                    }}
                  >
                    {t("metronome.presets.delete")}
                  </button>
                </span>
              </>
            )}
          </li>
        ))}
      </ul>
      <form className="row" onSubmit={(event) => void save(event)}>
        <input
          className="text-input"
          aria-label={t("metronome.presets.name")}
          placeholder={t("metronome.presets.name")}
          maxLength={MAX_PRESET_NAME}
          value={name}
          aria-invalid={error === "emptyName" || error === "nameTooLong"}
          onChange={(event) => {
            setName(event.target.value);
            setError(null);
          }}
        />
        <button type="submit" className="button-primary">
          {exists ? t("metronome.presets.replace") : t("metronome.presets.save")}
        </button>
      </form>
      {error && (
        <div className="row">
          <span className="row-error" role="alert">
            {t(`metronome.presets.error.${error}`, { max: MAX_PRESET_NAME })}
          </span>
        </div>
      )}
      {deleted && (
        <div className="row">
          <span className="row-value" role="status">
            {t("metronome.presets.deleted", { name: deleted })}
          </span>
        </div>
      )}
    </div>
  );
}

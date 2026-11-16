import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ImportErrorCode } from "../../lib/stems";
import { ProgressBar } from "../../shell/ProgressBar";
import type { ImportState } from "./useLibrary";

interface ImportBarProps {
  importing: ImportState | null;
  error: ImportErrorCode | null;
  dragging: boolean;
  onOpenFile: () => void;
  onLink: (url: string) => Promise<unknown>;
  onCancel: () => void;
  onEdit: () => void;
}

export function ImportBar({
  importing,
  error,
  dragging,
  onOpenFile,
  onLink,
  onCancel,
  onEdit,
}: ImportBarProps) {
  const { t } = useTranslation();
  const [url, setUrl] = useState("");
  const busy = importing !== null;

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (busy || !url.trim()) return;
    const song = await onLink(url);
    if (song) setUrl("");
  };

  const progress = importing?.progress;
  const percent =
    progress?.progress === null || progress?.progress === undefined
      ? null
      : Math.round(progress.progress * 100);
  const stageLabel = progress
    ? percent === null
      ? t(`stems.import.stage.${progress.stage}`)
      : t("stems.import.stageProgress", {
          stage: t(`stems.import.stage.${progress.stage}`),
          percent,
        })
    : "";

  return (
    <div className="card import" data-dragging={dragging}>
      <div className="row import-row">
        <button type="button" className="button-primary" onClick={onOpenFile} disabled={busy}>
          {t("stems.import.openFile")}
        </button>
        <form className="link-form" onSubmit={(event) => void submit(event)}>
          <input
            type="text"
            inputMode="url"
            className="text-input"
            placeholder={t("stems.import.link")}
            aria-label={t("stems.import.link")}
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="none"
            spellCheck={false}
            value={url}
            disabled={busy}
            onChange={(event) => {
              setUrl(event.target.value);
              onEdit();
            }}
          />
          <button type="submit" className="button-secondary" disabled={busy || !url.trim()}>
            {t("stems.import.download")}
          </button>
        </form>
      </div>
      {dragging && !busy && (
        <div className="row">
          <span className="import-drop">{t("stems.import.drop")}</span>
        </div>
      )}
      {progress && (
        <div className="row import-progress">
          <span className="import-stage">{stageLabel}</span>
          <ProgressBar label={stageLabel} value={progress.progress} />
          <button type="button" className="button-secondary" onClick={onCancel}>
            {t("stems.import.cancel")}
          </button>
        </div>
      )}
      {error && (
        <div className="row">
          <span className="row-error" role="alert">
            {t(`stems.errors.${error}`)}
          </span>
        </div>
      )}
    </div>
  );
}

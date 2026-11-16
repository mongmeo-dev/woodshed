import { useTranslation } from "react-i18next";
import type { SeparationEvent } from "../../lib/stems";
import { ProgressBar } from "../../shell/ProgressBar";

interface SeparationSlotProps {
  /** Latest separation event for the open song, if any. */
  separation: SeparationEvent | null;
  /** Starts separation; without it no button shows. */
  onSeparate?: () => void;
  /** Cancels a running separation; without it no cancel button shows. */
  onCancel?: () => void;
}

/** Where the separate button and the separation progress live. */
export function SeparationSlot({ separation, onSeparate, onCancel }: SeparationSlotProps) {
  const { t } = useTranslation();
  if (separation?.state === "running") {
    const label =
      separation.progress === null
        ? t("stems.separation.preparing")
        : t("stems.separation.running", { percent: Math.round(separation.progress * 100) });
    return (
      <div className="separation">
        <span className="separation-label">{label}</span>
        <ProgressBar label={label} value={separation.progress} />
        {onCancel && (
          <button type="button" className="button-secondary" onClick={onCancel}>
            {t("stems.separation.cancel")}
          </button>
        )}
      </div>
    );
  }
  return (
    <div className="separation">
      {separation?.state === "failed" && (
        <span className="row-error" role="alert">
          {t("stems.separation.failed")}
        </span>
      )}
      {onSeparate && (
        <button type="button" className="button-secondary" onClick={onSeparate}>
          {t("stems.separation.start")}
        </button>
      )}
    </div>
  );
}

import { useTranslation } from "react-i18next";
import type { SeparationEvent } from "../../lib/stems";
import { ProgressBar } from "./ProgressBar";

interface SeparationSlotProps {
  /** Latest separation event for the open song, if any. */
  separation: SeparationEvent | null;
  /** Starts separation. The stem engine wires this; without it no button shows. */
  onSeparate?: () => void;
}

/** Where the separate button and the separation progress live. */
export function SeparationSlot({ separation, onSeparate }: SeparationSlotProps) {
  const { t } = useTranslation();
  if (separation?.state === "running") {
    const percent = Math.round((separation.progress ?? 0) * 100);
    return (
      <div className="separation">
        <span className="separation-label">{t("stems.separation.running", { percent })}</span>
        <ProgressBar
          label={t("stems.separation.running", { percent })}
          value={separation.progress}
        />
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

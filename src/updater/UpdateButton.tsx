import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import { ProgressBar } from "../shell/ProgressBar";
import { canInstall, type UpdateStatus, useUpdater } from "./UpdaterProvider";

export function downloadLabel(t: TFunction, progress: number | null): string {
  return progress === null
    ? t("updates.downloadingUnknown")
    : t("updates.downloading", { percent: Math.round(progress * 100) });
}

function shown(status: UpdateStatus) {
  return canInstall(status) || status.kind === "downloading";
}

/** Sidebar slot: appears only while a newer version is waiting or downloading. */
export function UpdateButton() {
  const { t } = useTranslation();
  const { status, install } = useUpdater();
  if (!shown(status)) return null;
  return (
    <div className="sidebar-update">
      {status.kind === "downloading" ? (
        <>
          <span className="sidebar-update-label">{downloadLabel(t, status.progress)}</span>
          <ProgressBar label={t("updates.downloadingUnknown")} value={status.progress} />
        </>
      ) : (
        <button type="button" className="button-primary" onClick={() => void install()}>
          {t("updates.install")}
        </button>
      )}
    </div>
  );
}

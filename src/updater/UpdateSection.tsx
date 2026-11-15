import { useTranslation } from "react-i18next";
import { ProgressBar } from "../shell/ProgressBar";
import { downloadLabel } from "./UpdateButton";
import { canInstall, useUpdater } from "./UpdaterProvider";

/** Settings card: current version, check, install. */
export function UpdateSection() {
  const { t } = useTranslation();
  const { status, version, supported, check, install } = useUpdater();

  let statusText = "–";
  let failed = false;
  switch (status.kind) {
    case "checking":
      statusText = t("settings.updates.checking");
      break;
    case "upToDate":
      statusText = t("settings.updates.upToDate");
      break;
    case "available":
      statusText = t("settings.updates.available", { version: status.version });
      break;
    case "downloading":
      statusText = downloadLabel(t, status.progress);
      break;
    case "checkFailed":
      statusText = t("settings.updates.checkFailed");
      failed = true;
      break;
    case "installFailed":
      statusText = t("settings.updates.installFailed");
      failed = true;
      break;
  }

  return (
    <section aria-labelledby="settings-updates">
      <h2 className="settings-section-title" id="settings-updates">
        {t("settings.updates.section")}
      </h2>
      <div className="card">
        <div className="row">
          <span className="row-label">{t("settings.updates.version")}</span>
          <span className="row-value">{version ?? "–"}</span>
        </div>
        <div className="row">
          <span className="row-label">{t("settings.updates.status")}</span>
          <span
            className={failed ? "row-error" : "row-value"}
            role={failed ? "alert" : status.kind === "idle" ? undefined : "status"}
          >
            {statusText}
          </span>
        </div>
        <div className="row update-actions">
          {status.kind === "downloading" ? (
            <ProgressBar label={t("updates.downloadingUnknown")} value={status.progress} />
          ) : canInstall(status) ? (
            <button type="button" className="button-primary" onClick={() => void install()}>
              {t("updates.install")}
            </button>
          ) : (
            <button
              type="button"
              className="button-primary"
              onClick={() => void check()}
              disabled={!supported || status.kind === "checking"}
            >
              {t("settings.updates.check")}
            </button>
          )}
        </div>
      </div>
    </section>
  );
}

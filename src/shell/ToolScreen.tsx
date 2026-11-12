import { useTranslation } from "react-i18next";
import type { ToolId } from "../tools/tools";

/** Empty screen for a tool; the tool's own node fills the body. */
export function ToolScreen({ tool }: { tool: ToolId }) {
  const { t } = useTranslation();
  const titleId = `screen-title-${tool}`;
  return (
    <section className="screen" aria-labelledby={titleId}>
      <header className="screen-header" data-tauri-drag-region>
        <h1 className="screen-title" id={titleId}>
          {t(`tools.${tool}`)}
        </h1>
      </header>
      <div className="screen-body" />
    </section>
  );
}

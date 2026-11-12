import { useTranslation } from "react-i18next";
import { TOOL_IDS, type ViewId } from "../tools/tools";
import { ViewIcon } from "./icons";
import { useGlide } from "./useGlide";

interface SidebarProps {
  active: ViewId;
  onSelect: (view: ViewId) => void;
}

interface NavItemProps {
  view: ViewId;
  active: boolean;
  onSelect: (view: ViewId) => void;
}

function NavItem({ view, active, onSelect }: NavItemProps) {
  const { t } = useTranslation();
  const label = view === "settings" ? t("nav.settings") : t(`tools.${view}`);
  return (
    <li>
      <button
        type="button"
        className="nav-item"
        aria-current={active ? "page" : undefined}
        data-glide-active={active}
        onClick={() => onSelect(view)}
      >
        <ViewIcon view={view} />
        <span>{label}</span>
      </button>
    </li>
  );
}

export function Sidebar({ active, onSelect }: SidebarProps) {
  const { t, i18n } = useTranslation();
  const glide = useGlide<HTMLElement>(`${active}:${i18n.language}`);

  return (
    <aside className="sidebar">
      <div className="titlebar" data-tauri-drag-region />
      <nav className="sidebar-nav" aria-label={t("nav.label")} ref={glide.containerRef}>
        <div
          aria-hidden="true"
          className="nav-indicator"
          data-ready={glide.ready}
          style={glide.indicatorStyle}
        />
        <ul className="nav-list">
          {TOOL_IDS.map((tool) => (
            <NavItem key={tool} view={tool} active={active === tool} onSelect={onSelect} />
          ))}
        </ul>
        <ul className="nav-list nav-list--bottom">
          <NavItem view="settings" active={active === "settings"} onSelect={onSelect} />
        </ul>
      </nav>
    </aside>
  );
}

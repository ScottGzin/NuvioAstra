// Side navigation: Início, TV, Buscar, Biblioteca; Configurações at the bottom above the profile.
// Line icons (`SidebarIcons.tsx`) in the app icon's palette; the logo at the top is the app's own,
// centered on the same axis as the icons. The sync state is shown in Settings → Conta only.

import type { ReactNode } from "react";
import type { ProfileInfo } from "../types";
import { Avatar } from "./ProfileMenu";
import { HomeLine as HomeIcon, LibraryLine as LibraryIcon, SearchLine as SearchIcon, SettingsLine as SettingsIcon, TvLine as TvIcon } from "./SidebarIcons";
import { t } from "../i18n";

export type Tab = "home" | "tv" | "search" | "library" | "settings";

const ITEMS: { tab: Tab; label: string; icon: ReactNode }[] = [
  { tab: "home", label: "Início", icon: <HomeIcon /> },
  { tab: "tv", label: "TV", icon: <TvIcon /> },
  { tab: "search", label: "Buscar", icon: <SearchIcon /> },
  { tab: "library", label: "Biblioteca", icon: <LibraryIcon /> },
];

interface Props {
  current: Tab;
  onSelect(tab: Tab): void;
  profile: ProfileInfo | null;
  /** Opens the profile switcher. */
  onProfile(): void;
}

export function Sidebar({ current, onSelect, profile, onProfile }: Props) {
  return (
    <nav className="sidebar" aria-label={t("Navegação principal")}>
      <div className="brand" title="Nuvio Astra">
        <img className="brand-logo" src="./nuvio-astra-128.png" alt="" draggable={false} />
        <span className="nav-label">Nuvio Astra</span>
      </div>
      <ul>
        {ITEMS.map((item) => (
          <li key={item.tab}>
            <button
              className={`nav-item ${current === item.tab ? "active" : ""}`}
              aria-current={current === item.tab ? "page" : undefined}
              onClick={() => onSelect(item.tab)}
              data-tab={item.tab}
              title={t(item.label)}
            >
              <span className="nav-icon" aria-hidden>{item.icon}</span>
              <span className="nav-label">{t(item.label)}</span>
            </button>
          </li>
        ))}
      </ul>
      <span className="spacer" />
      <div className="sidebar-divider" aria-hidden />
      <button
        className={`nav-item ${current === "settings" ? "active" : ""}`}
        aria-current={current === "settings" ? "page" : undefined}
        onClick={() => onSelect("settings")}
        data-tab="settings"
        title={t("Configurações")}
      >
        <span className="nav-icon" aria-hidden>
          <SettingsIcon />
        </span>
        <span className="nav-label">{t("Configurações")}</span>
      </button>
      {profile && (
        <button className="profile-chip" title={`${profile.name} · trocar de perfil`} onClick={onProfile} data-testid="profile-chip" aria-haspopup="menu">
          <span className="profile-ring"><Avatar profile={profile} /></span>
          <span className="nav-label">{profile.name}</span>
        </button>
      )}
    </nav>
  );
}

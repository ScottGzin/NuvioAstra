// Side navigation: Início, TV, Buscar, Biblioteca; Configurações at the bottom above the profile.
// Icons in the app icon's blue/cyan palette (`NavIcons.tsx`); the logo at the top is the app's own,
// centered on the same axis as the icons. The sync state is shown in Settings → Conta only.

import type { ReactNode } from "react";
import { t } from "../i18n";
import type { ProfileInfo } from "../types";
import { Avatar } from "./ProfileMenu";
import { HomeIcon, LibraryIcon, SearchIcon, SettingsIcon, TvIcon } from "./NavIcons";

export type Tab = "home" | "tv" | "search" | "library" | "settings";

const ITEMS: { tab: Tab; label: string; icon: ReactNode }[] = [
  { tab: "home", label: "Home", icon: <HomeIcon /> },
  { tab: "tv", label: "TV", icon: <TvIcon /> },
  { tab: "search", label: "Search", icon: <SearchIcon /> },
  { tab: "library", label: "Library", icon: <LibraryIcon /> },
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
    <nav className="sidebar" aria-label={t("Main navigation")}>
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
        title={t("Settings")}
      >
        <span className="nav-icon" aria-hidden>
          <SettingsIcon />
        </span>
        <span className="nav-label">{t("Settings")}</span>
      </button>
      {profile && (
        <button className="profile-chip" title={t("{name} · switch profile", { name: profile.name })} onClick={onProfile} data-testid="profile-chip" aria-haspopup="menu">
          <span className="profile-ring"><Avatar profile={profile} /></span>
          <span className="nav-label">{profile.name}</span>
        </button>
      )}
    </nav>
  );
}

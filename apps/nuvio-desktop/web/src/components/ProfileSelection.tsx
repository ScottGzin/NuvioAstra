// Full-screen profile picker ("Who's watching?"), matching the original app: brand wordmark, one
// row of avatars, and a Manage Profiles button that toggles the edit badges and the Add Profile card.

import { useState } from "react";
import { api, errorText } from "../api";
import type { ProfileInfo, ProfilesView } from "../types";
import { t } from "../i18n";
import { Avatar, ProfilesDialog } from "./ProfileMenu";
import { PinDialog } from "./PinDialog";

interface Props {
  view: ProfilesView;
  /** Opens the chosen profile (the app leaves this screen). */
  onOpen(view: ProfilesView): void;
  /** In-app profile management: start in edit mode and close with Done. */
  initialManage?: boolean;
  onDone?(): void;
  onChanged?(view: ProfilesView): void;
}

/** Which avatar colors the backdrop blends to while the pointer is over a profile. */
function colorOf(profile: ProfileInfo): string {
  return profile.avatarColorHex || "#4527a0";
}

export function ProfileSelection({ view: initial, onOpen, initialManage, onDone, onChanged }: Props) {
  const [view, setView] = useState(initial);
  const [editMode, setEditMode] = useState(!!initialManage);
  const [editing, setEditing] = useState<number | "add" | null>(null);
  const [hovered, setHovered] = useState<ProfileInfo | null>(null);
  const [pinFor, setPinFor] = useState<ProfileInfo | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const open = async (index: number) => {
    setBusy(true);
    setError(null);
    try {
      onOpen(await api.profileSwitch(index));
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  const choose = (profile: ProfileInfo) => {
    if (editMode) {
      setEditing(profile.index);
      return;
    }
    if (profile.pinEnabled) setPinFor(profile);
    else void open(profile.index);
  };

  return (
    <div className="profile-gate" data-testid="profile-gate">
      <div className="profile-gate-bg" style={{ background: hovered ? colorOf(hovered) : undefined }} aria-hidden />
      <div className="profile-gate-content">
        <img className="profile-gate-logo" src="/nuvio-astra-128.png" alt="Nuvio Astra" />
        <h1>{t("Quem está assistindo?")}</h1>
        <div className="profile-gate-list">
          {view.profiles.map((profile) => (
            <button
              key={profile.index}
              className="profile-gate-item"
              data-profile={profile.index}
              disabled={busy}
              onMouseEnter={() => setHovered(profile)}
              onMouseLeave={() => setHovered(null)}
              onClick={() => choose(profile)}
            >
              <span className="profile-gate-avatar">
                <Avatar profile={profile} size={96} />
                {editMode && <span className="profile-gate-pencil" aria-hidden>✎</span>}
              </span>
              <span className="profile-gate-name">{profile.name}</span>
              {profile.pinEnabled && <span className="profile-gate-lock" aria-label={t("Protegido por PIN")}>🔒</span>}
            </button>
          ))}
          {editMode && view.profiles.length < view.max && (
            <button className="profile-gate-item add" data-testid="profile-add" onClick={() => setEditing("add")}>
              <span className="profile-gate-avatar add" aria-hidden>+</span>
              <span className="profile-gate-name">{t("Adicionar perfil")}</span>
            </button>
          )}
        </div>
        {error && <div className="row-error" role="alert">{error}</div>}
        <button
          className="profile-gate-manage"
          data-testid="profiles-manage"
          onClick={() => {
            if (editMode && onDone) { onDone(); return; }
            setEditMode((on) => !on);
          }}
        >
          {editMode ? t("Concluído") : t("Gerenciar perfis")}
        </button>
        {onDone && (
          <button className="profile-gate-close" aria-label={t("Fechar")} onClick={onDone}>✕</button>
        )}
      </div>
      {pinFor && (
        <PinDialog
          profile={pinFor}
          onUnlocked={() => { setPinFor(null); void open(pinFor.index); }}
          onCancel={() => setPinFor(null)}
        />
      )}
      {editing !== null && (
        <ProfilesDialog
          view={view}
          initialIndex={editing === "add" ? undefined : editing}
          initialAdd={editing === "add"}
          fullscreen
          onChanged={(next) => { setView(next); onChanged?.(next); setEditing(null); if (!onDone) setEditMode(false); }}
          onClose={() => setEditing(null)}
        />
      )}
    </div>
  );
}

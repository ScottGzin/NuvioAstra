// The PIN asked before opening a protected profile. The lock at start is `screens/LockScreen`.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import type { ProfileInfo } from "../types";
import { PinPad } from "./PinPad";
import { Avatar } from "./ProfileMenu";
import { t } from "../i18n";

interface Props {
  profile: ProfileInfo;
  onUnlocked(): void;
  onCancel(): void;
}

export function PinDialog({ profile, onUnlocked, onCancel }: Props) {
  const [checking, setChecking] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      onCancel();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onCancel]);

  const recheck = async () => {
    setChecking(true);
    setNote(null);
    try {
      const view = await api.profilePinRefresh(profile.index);
      if (view.profiles.find((p) => p.index === profile.index)?.pinEnabled === false) return onUnlocked();
      setNote("Não deu para confirmar a remoção: é preciso estar com internet e conectado à conta dona deste perfil.");
    } catch (e) {
      setNote(errorText(e));
    } finally {
      setChecking(false);
    }
  };

  return (
    <div className="modal-backdrop" role="presentation">
      <div className="modal pin-modal" role="dialog" aria-modal="true" aria-label={`PIN de ${profile.name}`} data-testid="pin-dialog">
        <Avatar profile={profile} size={64} />
        <h2>{t("Insira o PIN para")} {profile.name}</h2>
        <PinPad profile={profile} onUnlocked={onUnlocked} />
        {note && <p className="muted small" role="status">{note}</p>}
        <div className="gate-actions">
          <button className="ghost" disabled={checking} onClick={() => void recheck()}>{checking ? t("Verificando…") : t("Já removi o PIN")}</button>
          <button className="ghost" data-testid="pin-cancel" onClick={onCancel}>{t("Cancelar")}</button>
        </div>
      </div>
    </div>
  );
}

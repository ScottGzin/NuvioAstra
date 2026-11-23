// Profile switcher (sidebar avatar) and profile editor, following the original's rules: up to 6
// profiles, a new one takes the lowest free slot, the primary profile (1) cannot be deleted, deleting
// removes the profile's data (Nuvio Astra keeps a backup), a secondary profile may use the primary
// profile's addons. Avatars come from the Nuvio account's catalog (`get_avatar_catalog`); without
// one, the profile shows its color and initial. PIN: set, changed and removed with the account.

import { useEffect, useRef, useState } from "react";
import { api, errorText } from "../api";
import { useAccount, useAvatars } from "../account";
import type { ProfileInfo, ProfilesView } from "../types";
import { t } from "../i18n";

export function Avatar({ profile, size = 34 }: { profile: Pick<ProfileInfo, "name" | "avatarColorHex"> & { avatarId?: string | null }; size?: number }) {
  const avatars = useAvatars();
  const [broken, setBroken] = useState(false);
  const image = profile.avatarId ? avatars.find((a) => a.id === profile.avatarId) : undefined;
  if (image && !broken) {
    return (
      <span className="avatar avatar-img" aria-hidden style={{ width: size, height: size, background: image.bgColor ?? "transparent" }}>
        <img src={image.imageUrl} alt="" draggable={false} onError={() => setBroken(true)} />
      </span>
    );
  }
  const color = profile.avatarColorHex || "#1E88E5";
  // Light colors (e.g. #E9E8F2) need dark text.
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(color.slice(i, i + 2), 16) || 0);
  const dark = r * 0.299 + g * 0.587 + b * 0.114 > 160;
  return (
    <span className="avatar" aria-hidden style={{ background: color, color: dark ? "#14161c" : "#fff", width: size, height: size, fontSize: size * 0.42 }}>
      {profile.name.slice(0, 1).toUpperCase() || "?"}
    </span>
  );
}

interface MenuProps {
  view: ProfilesView;
  busy: boolean;
  onSwitch(index: number): void;
  onManage(): void;
  onClose(): void;
}

export function ProfileMenu({ view, busy, onSwitch, onManage, onClose }: MenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const away = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && onClose();
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", away);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("mousedown", away);
      window.removeEventListener("keydown", esc);
    };
  }, [onClose]);
  return (
    <div className="profile-menu" ref={ref} role="menu" aria-label={t("Perfis")} data-testid="profile-menu">
      <div className="muted small profile-menu-title">{t("Quem está assistindo?")}</div>
      {view.profiles.map((p) => {
        const active = p.index === view.active;
        return (
          <button
            key={p.index}
            role="menuitemradio"
            aria-checked={active}
            className={`profile-option ${active ? "active" : ""}`}
            disabled={busy}
            data-profile={p.index}
            onClick={() => (active ? onClose() : onSwitch(p.index))}
          >
            <Avatar profile={p} />
            <span className="profile-option-name">{p.name}</span>
            {p.pinEnabled && <span className="profile-lock" title={t("Protegido por PIN")} aria-label={t("Protegido por PIN")}>🔒</span>}
            {active && <span className="profile-check" aria-label={t("Perfil ativo")}>✓</span>}
          </button>
        );
      })}
      <button className="profile-option manage" onClick={onManage} data-testid="profiles-manage">
        <span className="profile-manage-icon" aria-hidden>✎</span>
        <span>{t("Gerenciar perfis")}</span>
      </button>
    </div>
  );
}

interface EditorProps {
  view: ProfilesView;
  onChanged(view: ProfilesView): void;
  onClose(): void;
}

type Editing = { index: number | null; name: string; color: string; usesPrimaryAddons: boolean; avatarId: string | null };

export function ProfilesDialog({ view, onChanged, onClose }: EditorProps) {
  const [editing, setEditing] = useState<Editing | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const save = async () => {
    if (!editing) return;
    setBusy(true);
    setError(null);
    try {
      const v = editing.index === null
        ? await api.profileCreate(editing.name, editing.color, editing.usesPrimaryAddons)
        : await api.profileUpdate(editing.index, editing.name, editing.color, editing.usesPrimaryAddons, editing.avatarId ?? "");
      onChanged(v);
      setEditing(null);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!editing || editing.index === null) return;
    const name = view.profiles.find((p) => p.index === editing.index)?.name ?? editing.name;
    if (!window.confirm(t("Excluir perfil?\n\nTodos os dados de “{name}” serão excluídos permanentemente: favoritos, histórico, progresso, addons e configurações deste perfil. Uma cópia de segurança é guardada na pasta de dados do Nuvio Astra.", { name }))) return;
    setBusy(true);
    setError(null);
    try {
      onChanged(await api.profileDelete(editing.index));
      setEditing(null);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(e) => e.target === e.currentTarget && !busy && onClose()}>
      <div className="modal" role="dialog" aria-modal="true" aria-label={t("Gerenciar perfis")} data-testid="profiles-dialog">
        <header className="modal-header">
          <h2>{editing ? (editing.index === null ? t("Adicionar perfil") : t("Editar perfil")) : t("Gerenciar perfis")}</h2>
          <button className="icon-btn" aria-label={t("Fechar")} onClick={onClose} disabled={busy}>✕</button>
        </header>

        {!editing && (
          <>
            <div className="profiles-grid">
              {view.profiles.map((p) => (
                <button key={p.index} className="profile-tile" onClick={() => setEditing({ index: p.index, name: p.name, color: p.avatarColorHex, usesPrimaryAddons: p.usesPrimaryAddons, avatarId: p.avatarId })} data-profile={p.index}>
                  <Avatar profile={p} size={64} />
                  <span>{p.name}</span>
                  <span className="muted small">{p.index === view.active ? t("Ativo") : p.index === 1 ? t("Principal") : t("Perfil {index}", { index: p.index })}</span>
                </button>
              ))}
              {view.profiles.length < view.max && (
                <button className="profile-tile add" onClick={() => setEditing({ index: null, name: "", color: view.colors[view.profiles.length % view.colors.length], usesPrimaryAddons: false, avatarId: null })} data-testid="profile-add">
                  <span className="avatar add" aria-hidden style={{ width: 64, height: 64 }}>+</span>
                  <span>{t("Criar perfil")}</span>
                  <span className="muted small">{view.profiles.length} de {view.max}</span>
                </button>
              )}
            </div>
            <p className="muted small">{t("Cada perfil tem seus próprios favoritos, histórico, progresso, addons e configurações.")}</p>
          </>
        )}

        {editing && (
          <form className="profile-form" onSubmit={(e) => { e.preventDefault(); void save(); }}>
            <div className="profile-form-head">
              <Avatar profile={{ name: editing.name || "?", avatarColorHex: editing.color, avatarId: editing.avatarId }} size={72} />
              <input
                aria-label={t("Nome do perfil")}
                placeholder={t("Nome do perfil")}
                value={editing.name}
                maxLength={40}
                autoFocus
                data-testid="profile-name"
                onChange={(e) => setEditing({ ...editing, name: e.target.value })}
              />
            </div>
            <div className="muted small">{t("Cor do avatar")}</div>
            <div className="color-swatches" role="radiogroup" aria-label={t("Cor do avatar")}>
              {view.colors.map((c) => (
                <button
                  type="button"
                  key={c}
                  role="radio"
                  aria-checked={editing.color.toUpperCase() === c}
                  aria-label={c}
                  className={`swatch ${editing.color.toUpperCase() === c ? "active" : ""}`}
                  style={{ background: c }}
                  onClick={() => setEditing({ ...editing, color: c })}
                />
              ))}
            </div>
            {editing.index !== null && <AvatarPicker value={editing.avatarId} onChange={(avatarId) => setEditing({ ...editing, avatarId })} />}
            {editing.index !== null && (
              <PinSection profile={view.profiles.find((p) => p.index === editing.index)!} onChanged={onChanged} />
            )}
            {editing.index !== 1 && (
              <label className="setting-row profile-primary">
                <div className="setting-text">
                  <div className="setting-title">{t("Usar addons do perfil principal")}</div>
                  <div className="muted small">{t("Compartilhar a configuração de addon do perfil principal em vez de gerenciar uma lista separada.")}</div>
                </div>
                <input type="checkbox" checked={editing.usesPrimaryAddons} onChange={(e) => setEditing({ ...editing, usesPrimaryAddons: e.target.checked })} />
              </label>
            )}
            {error && <div className="row-error">{error}</div>}
            <div className="modal-actions">
              {editing.index !== null && editing.index > 1 && (
                <button type="button" className="chip danger" onClick={() => void remove()} disabled={busy} data-testid="profile-delete">{t("Excluir perfil")}</button>
              )}
              <span className="spacer" />
              <button type="button" className="chip" onClick={() => { setEditing(null); setError(null); }} disabled={busy}>{t("Voltar")}</button>
              <button type="submit" className="primary" disabled={busy || !editing.name.trim()} data-testid="profile-save">{busy ? t("Salvando…") : t("Salvar")}</button>
            </div>
          </form>
        )}
      </div>
    </div>
  );
}

/** Avatars of the account's catalog (`get_avatar_catalog`, by category and order). */
function AvatarPicker({ value, onChange }: { value: string | null; onChange(id: string | null): void }) {
  const avatars = useAvatars();
  const account = useAccount();
  if (!avatars.length) {
    return <div className="muted small">{account?.configured ? t("Os avatares da conta Nuvio aparecem aqui quando o catálogo for carregado (requer conexão na primeira vez).") : t("Avatares indisponíveis nesta versão: o perfil usa cor e inicial.")}</div>;
  }
  return (
    <div className="avatar-picker" role="radiogroup" aria-label={t("Avatar")}>
      <button type="button" role="radio" aria-checked={!value} className={`avatar-choice none ${!value ? "active" : ""}`} onClick={() => onChange(null)} title={t("Cor e inicial")}>Aa</button>
      {avatars.map((a) => (
        <button type="button" key={a.id} role="radio" aria-checked={value === a.id} title={a.displayName} className={`avatar-choice ${value === a.id ? "active" : ""}`} style={{ background: a.bgColor ?? undefined }} onClick={() => onChange(a.id)} data-avatar={a.id}>
          <img src={a.imageUrl} alt={a.displayName} loading="lazy" />
        </button>
      ))}
    </div>
  );
}

/** PIN of a profile: set / change / remove through the account (`set_profile_pin`, `clear_profile_pin`). */
function PinSection({ profile, onChanged }: { profile: ProfileInfo; onChanged(view: ProfilesView): void }) {
  const account = useAccount();
  const [mode, setMode] = useState<"idle" | "set" | "clear">("idle");
  const [current, setCurrent] = useState("");
  const [pin, setPin] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const digits = (v: string) => v.replace(/\D/g, "").slice(0, 4);
  const reset = () => { setMode("idle"); setCurrent(""); setPin(""); setConfirm(""); setError(null); };

  if (!account?.signedIn) {
    return (
      <div className="setting-row">
        <div className="setting-text">
          <div className="setting-title">{t("Bloqueio por PIN")}</div>
          <div className="muted small">{profile.pinEnabled ? `${t("Este perfil está protegido com um PIN.")} ` : ""}{t("Entre na conta Nuvio (Configurações → Conta) para definir, alterar ou remover o PIN.")}</div>
        </div>
      </div>
    );
  }

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      if (mode === "set") {
        if (pin.length !== 4 || pin !== confirm) throw new Error(pin.length !== 4 ? t("O PIN deve ter 4 dígitos.") : t("Os dois PINs não são iguais."));
        onChanged(await api.profilePinSet(profile.index, pin, profile.pinEnabled ? current : null));
      } else {
        onChanged(await api.profilePinClear(profile.index, current || null));
      }
      reset();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="pin-section" data-testid="pin-section">
      <div className="setting-title">{t("Bloqueio por PIN")}</div>
      <div className="muted small">{profile.pinEnabled ? t("Este perfil está protegido com um PIN.") : t("Adicione um PIN se quiser que este perfil seja bloqueado antes de alternar para ele.")}</div>
      {mode === "idle" && (
        <div className="account-actions">
          <button type="button" className="chip" onClick={() => setMode("set")}>{profile.pinEnabled ? t("Alterar PIN") : t("Definir bloqueio por PIN")}</button>
          {profile.pinEnabled && <button type="button" className="chip danger" onClick={() => setMode("clear")}>{t("Remover bloqueio por PIN")}</button>}
        </div>
      )}
      {mode !== "idle" && (
        <div className="pin-fields">
          {profile.pinEnabled && <input inputMode="numeric" type="password" placeholder={t("PIN atual")} aria-label={t("PIN atual")} value={current} onChange={(e) => setCurrent(digits(e.target.value))} />}
          {mode === "set" && (
            <>
              <input inputMode="numeric" type="password" placeholder={t("Novo PIN (4 dígitos)")} aria-label={t("Novo PIN")} value={pin} onChange={(e) => setPin(digits(e.target.value))} />
              <input inputMode="numeric" type="password" placeholder={t("Repita o novo PIN")} aria-label={t("Repita o novo PIN")} value={confirm} onChange={(e) => setConfirm(digits(e.target.value))} />
            </>
          )}
          <button type="button" className="primary" disabled={busy} onClick={() => void save()}>{busy ? t("Salvando…") : mode === "set" ? t("Salvar PIN") : t("Remover PIN")}</button>
          <button type="button" className="chip" onClick={reset}>{t("Cancelar")}</button>
        </div>
      )}
      {error && <div className="row-error">{error}</div>}
    </div>
  );
}

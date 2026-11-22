// Profile switcher (sidebar avatar) and profile editor, following the original's rules: up to 6
// profiles, a new one takes the lowest free slot, the primary profile (1) cannot be deleted, deleting
// removes the profile's data (Nuvio Astra keeps a backup), a secondary profile may use the primary
// profile's addons. Avatars come from the Nuvio account's catalog (`get_avatar_catalog`); without
// one, the profile shows its color and initial. PIN: set, changed and removed with the account.

import { useEffect, useRef, useState } from "react";
import { api, errorText } from "../api";
import { useAccount, useAvatars } from "../account";
import type { ProfileInfo, ProfilesView } from "../types";

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
    <div className="profile-menu" ref={ref} role="menu" aria-label="Profiles" data-testid="profile-menu">
      <div className="muted small profile-menu-title">Who's watching?</div>
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
            {p.pinEnabled && <span className="profile-lock" title="Protected by PIN" aria-label="Protected by PIN">🔒</span>}
            {active && <span className="profile-check" aria-label="Active profile">✓</span>}
          </button>
        );
      })}
      <button className="profile-option manage" onClick={onManage} data-testid="profiles-manage">
        <span className="profile-manage-icon" aria-hidden>✎</span>
        <span>Manage profiles</span>
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
    if (!window.confirm(`Delete profile?\n\nAll data from “${name}” will be permanently deleted: favorites, history, progress, addons, and settings of this profile. A backup is kept in the Nuvio Astra data folder.`)) return;
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
      <div className="modal" role="dialog" aria-modal="true" aria-label="Manage profiles" data-testid="profiles-dialog">
        <header className="modal-header">
          <h2>{editing ? (editing.index === null ? "Add profile" : "Edit profile") : "Manage profiles"}</h2>
          <button className="icon-btn" aria-label="Close" onClick={onClose} disabled={busy}>✕</button>
        </header>

        {!editing && (
          <>
            <div className="profiles-grid">
              {view.profiles.map((p) => (
                <button key={p.index} className="profile-tile" onClick={() => setEditing({ index: p.index, name: p.name, color: p.avatarColorHex, usesPrimaryAddons: p.usesPrimaryAddons, avatarId: p.avatarId })} data-profile={p.index}>
                  <Avatar profile={p} size={64} />
                  <span>{p.name}</span>
                  <span className="muted small">{p.index === view.active ? "Active" : p.index === 1 ? "Primary" : `Profile ${p.index}`}</span>
                </button>
              ))}
              {view.profiles.length < view.max && (
                <button className="profile-tile add" onClick={() => setEditing({ index: null, name: "", color: view.colors[view.profiles.length % view.colors.length], usesPrimaryAddons: false, avatarId: null })} data-testid="profile-add">
                  <span className="avatar add" aria-hidden style={{ width: 64, height: 64 }}>+</span>
                  <span>Create profile</span>
                  <span className="muted small">{view.profiles.length} of {view.max}</span>
                </button>
              )}
            </div>
            <p className="muted small">Each profile has its own favorites, history, progress, addons, and settings.</p>
          </>
        )}

        {editing && (
          <form className="profile-form" onSubmit={(e) => { e.preventDefault(); void save(); }}>
            <div className="profile-form-head">
              <Avatar profile={{ name: editing.name || "?", avatarColorHex: editing.color, avatarId: editing.avatarId }} size={72} />
              <input
                aria-label="Profile name"
                placeholder="Profile name"
                value={editing.name}
                maxLength={40}
                autoFocus
                data-testid="profile-name"
                onChange={(e) => setEditing({ ...editing, name: e.target.value })}
              />
            </div>
            <div className="muted small">Avatar color</div>
            <div className="color-swatches" role="radiogroup" aria-label="Avatar color">
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
                  <div className="setting-title">Use the main profile's addons</div>
                  <div className="muted small">Share the main profile's addon configuration instead of managing a separate list.</div>
                </div>
                <input type="checkbox" checked={editing.usesPrimaryAddons} onChange={(e) => setEditing({ ...editing, usesPrimaryAddons: e.target.checked })} />
              </label>
            )}
            {error && <div className="row-error">{error}</div>}
            <div className="modal-actions">
              {editing.index !== null && editing.index > 1 && (
                <button type="button" className="chip danger" onClick={() => void remove()} disabled={busy} data-testid="profile-delete">Delete profile</button>
              )}
              <span className="spacer" />
              <button type="button" className="chip" onClick={() => { setEditing(null); setError(null); }} disabled={busy}>Back</button>
              <button type="submit" className="primary" disabled={busy || !editing.name.trim()} data-testid="profile-save">{busy ? "Saving…" : "Save"}</button>
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
    return <div className="muted small">{account?.configured ? "Nuvio account avatars appear here once the catalog loads (requires a connection the first time)." : "Avatars unavailable in this version: the profile uses a color and initial."}</div>;
  }
  return (
    <div className="avatar-picker" role="radiogroup" aria-label="Avatar">
      <button type="button" role="radio" aria-checked={!value} className={`avatar-choice none ${!value ? "active" : ""}`} onClick={() => onChange(null)} title="Color and initial">Aa</button>
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
          <div className="setting-title">PIN lock</div>
          <div className="muted small">{profile.pinEnabled ? "This profile is protected with a PIN. " : ""}Sign in to the Nuvio account (Settings → Account) to set, change, or remove the PIN.</div>
        </div>
      </div>
    );
  }

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      if (mode === "set") {
        if (pin.length !== 4 || pin !== confirm) throw new Error(pin.length !== 4 ? "The PIN must have 4 digits." : "The two PINs do not match.");
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
      <div className="setting-title">PIN lock</div>
      <div className="muted small">{profile.pinEnabled ? "This profile is protected with a PIN." : "Add a PIN if you want this profile to be locked before switching to it."}</div>
      {mode === "idle" && (
        <div className="account-actions">
          <button type="button" className="chip" onClick={() => setMode("set")}>{profile.pinEnabled ? "Change PIN" : "Set PIN lock"}</button>
          {profile.pinEnabled && <button type="button" className="chip danger" onClick={() => setMode("clear")}>Remove PIN lock</button>}
        </div>
      )}
      {mode !== "idle" && (
        <div className="pin-fields">
          {profile.pinEnabled && <input inputMode="numeric" type="password" placeholder="Current PIN" aria-label="Current PIN" value={current} onChange={(e) => setCurrent(digits(e.target.value))} />}
          {mode === "set" && (
            <>
              <input inputMode="numeric" type="password" placeholder="New PIN (4 digits)" aria-label="New PIN" value={pin} onChange={(e) => setPin(digits(e.target.value))} />
              <input inputMode="numeric" type="password" placeholder="Repeat the new PIN" aria-label="Repeat the new PIN" value={confirm} onChange={(e) => setConfirm(digits(e.target.value))} />
            </>
          )}
          <button type="button" className="primary" disabled={busy} onClick={() => void save()}>{busy ? "Saving…" : mode === "set" ? "Save PIN" : "Remove PIN"}</button>
          <button type="button" className="chip" onClick={reset}>Cancel</button>
        </div>
      )}
      {error && <div className="row-error">{error}</div>}
    </div>
  );
}

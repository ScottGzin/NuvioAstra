// 4-digit PIN entry (`PinEntryDialog` of the original). The check happens in Rust
// (`profile_unlock`: the account first, the offline hash otherwise); this only collects the digits.

import { useEffect, useRef, useState } from "react";
import { api, errorText } from "../api";
import type { ProfileInfo } from "../types";
import { Avatar } from "./ProfileMenu";
import { t } from "../i18n";

interface Props {
  profile: ProfileInfo;
  onUnlocked(): void;
  /** Absent for the lock screen at start (no way around it). */
  onCancel?(): void;
}

export function PinDialog({ profile, onUnlocked, onCancel }: Props) {
  const [pin, setPin] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => input.current?.focus(), []);

  const submit = async (value: string) => {
    setBusy(true);
    setError(null);
    try {
      const r = await api.profileUnlock(profile.index, value);
      if (r.unlocked) onUnlocked();
      else {
        setError(r.message ?? t("Incorrect PIN"));
        setPin("");
      }
    } catch (e) {
      setError(errorText(e));
      setPin("");
    } finally {
      setBusy(false);
      input.current?.focus();
    }
  };

  return (
    <div className="modal-backdrop" role="presentation">
      <div className="modal pin-modal" role="dialog" aria-modal="true" aria-label={t("PIN for {name}", { name: profile.name })} data-testid="pin-dialog">
        <Avatar profile={profile} size={64} />
        <h2>{t("Enter the PIN for {name}", { name: profile.name })}</h2>
        <div className="pin-dots" aria-hidden>
          {[0, 1, 2, 3].map((i) => <span key={i} className={`pin-dot ${i < pin.length ? "filled" : ""} ${error ? "error" : ""}`} />)}
        </div>
        <input
          ref={input}
          className="pin-input"
          inputMode="numeric"
          autoComplete="off"
          aria-label={t("PIN")}
          data-testid="pin-input"
          value={pin}
          disabled={busy}
          onChange={(e) => {
            const v = e.target.value.replace(/\D/g, "").slice(0, 4);
            setPin(v);
            if (v.length === 4) void submit(v);
          }}
        />
        {error && <div className="row-error" role="alert">{error}</div>}
        {busy && <div className="muted small">{t("Verifying…")}</div>}
        {onCancel && <button className="chip" onClick={onCancel}>{t("Cancel")}</button>}
      </div>
    </div>
  );
}

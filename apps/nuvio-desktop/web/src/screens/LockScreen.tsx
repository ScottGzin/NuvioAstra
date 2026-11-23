// The app opened on a profile with a PIN. Nothing of the profile is behind this screen, and it has
// ways out: the PIN, a PIN removed in Nuvio (checked by itself and on demand), another profile,
// and signing in to the account the PIN belongs to.

import { useCallback, useEffect, useState } from "react";
import { api, errorText } from "../api";
import { set as setAccount, useAccount } from "../account";
import { PinPad } from "../components/PinPad";
import { ProfileChooser } from "../components/ProfileChooser";
import { Avatar } from "../components/ProfileMenu";
import { SignInForm } from "../components/SignInForm";
import type { ProfilesView } from "../types";
import { t } from "../i18n";

type Mode = "pin" | "profiles" | "account";

interface Props {
  view: ProfilesView;
  /** At startup show the profile chooser first, like the original app. */
  initialMode?: Mode;
  /** The lock is gone: the PIN was right, it was removed, or another profile was opened. */
  onOpen(view: ProfilesView): void;
}

export function LockScreen({ view, onOpen, initialMode }: Props) {
  const account = useAccount();
  const [mode, setMode] = useState<Mode>(initialMode ?? "pin");
  const [checking, setChecking] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const profile = view.profiles.find((p) => p.index === view.active) ?? view.profiles[0];

  const check = useCallback(
    async (asked: boolean) => {
      if (asked) {
        setChecking(true);
        setNote(null);
      }
      try {
        const next = await api.profilePinRefresh(view.active);
        if (!next.locked) return onOpen(next);
        if (asked) {
          setNote(
            account?.signedIn
              ? t("O PIN continua ativo ou não deu para falar com a conta. Se esta não é a conta dona do perfil, use outra conta.")
              : t("Entre na conta Nuvio dona deste perfil para confirmar a remoção."),
          );
        }
      } catch (e) {
        if (asked) setNote(errorText(e));
      } finally {
        if (asked) setChecking(false);
      }
    },
    [account?.signedIn, onOpen, view.active],
  );

  // Only the PIN gate looks for a lock removed in Nuvio; the chooser waits for the user to say who
  // is watching (checking there reopened the last profile by itself).
  useEffect(() => {
    if (mode !== "pin") return;
    void check(false);
    const onFocus = () => void check(false);
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [check, mode]);

  const openAccount = async () => {
    setNote(null);
    try {
      if (account?.signedIn) setAccount(await api.accountSignOut());
      setMode("account");
    } catch (e) {
      setNote(errorText(e));
    }
  };

  const unlocked = async () => {
    try {
      onOpen(await api.profilesList());
    } catch (e) {
      setNote(errorText(e));
    }
  };

  if (!profile) return null;

  return (
    <div className="gate" data-testid="lock-screen">
      <div className="gate-card">
        {mode === "pin" && (
          <>
            <Avatar profile={profile} size={72} />
            <h1>{t("Insira o PIN para")} {profile.name}</h1>
            <PinPad profile={profile} onUnlocked={() => void unlocked()} />
            {!account?.signedIn && account?.configured && (
              <p className="muted small" data-testid="lock-signed-out">
                {t("Este computador não está conectado à conta Nuvio. Se o PIN não for aceito ou você já o removeu, entre na conta")}.
              </p>
            )}
            {note && <p className="gate-note" role="status" data-testid="lock-note">{note}</p>}
            <div className="gate-actions">
              {account?.configured && (
                <button className={account.signedIn ? "ghost" : "primary"} data-testid="lock-account" onClick={() => void openAccount()}>
                  {account.signedIn ? t("Usar outra conta") : t("Entrar na conta Nuvio")}
                </button>
              )}
              <button className="ghost" disabled={checking} data-testid="lock-recheck" onClick={() => void check(true)}>{checking ? t("Verificando…") : t("Já removi o PIN")}</button>
              {view.profiles.length > 1 && <button className="ghost" data-testid="lock-profiles" onClick={() => setMode("profiles")}>{t("Trocar de perfil")}</button>}
            </div>
          </>
        )}
        {mode === "profiles" && (
          <>
            <h1>{t("Quem está assistindo?")}</h1>
            <ProfileChooser view={view} onChosen={onOpen} />
            <button className="gate-link" onClick={() => setMode("pin")}>{t("Voltar")}</button>
          </>
        )}
        {mode === "account" && (
          <>
            <h1>{t("Entre na conta Nuvio")}</h1>
            <p className="muted">{t("Use a conta dona de “{name}”. Seus dados deste computador continuam aqui.", { name: profile.name })}</p>
            <SignInForm
              onSignedIn={() => {
                setMode("pin");
                void check(false);
              }}
            />
            <button className="gate-link" data-testid="lock-back" onClick={() => setMode("pin")}>{t("Voltar")}</button>
          </>
        )}
      </div>
    </div>
  );
}

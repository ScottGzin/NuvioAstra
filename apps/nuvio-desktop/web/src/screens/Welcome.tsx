// First opening: the Nuvio account, the sync right after it and who is watching. Only for a
// computer never linked to an account, once; going on without an account is one click.

import { useCallback, useState } from "react";
import { api, errorText } from "../api";
import { set as setAccount, useAccount } from "../account";
import { ProfileChooser } from "../components/ProfileChooser";
import { SignInForm } from "../components/SignInForm";
import type { ProfilesView } from "../types";
import { t } from "../i18n";

type Step =
  | { name: "account" }
  | { name: "syncing" }
  | { name: "confirm"; question: string }
  | { name: "failed"; message: string }
  | { name: "profiles"; view: ProfilesView };

export function Welcome({ onDone }: { onDone(): void }) {
  const account = useAccount();
  const [step, setStep] = useState<Step>({ name: "account" });

  const finish = useCallback(async () => {
    await api.welcomeDone().catch(() => {});
    onDone();
  }, [onDone]);

  const connect = async (confirmOther = false) => {
    setStep({ name: "syncing" });
    try {
      const status = await api.accountConnect(confirmOther);
      setAccount(status);
      if (status.state === "needsReview") {
        setStep({ name: "confirm", question: status.linkQuestion ?? t("Confirme para sincronizar este computador com a conta.") });
        return;
      }
      if (status.state === "error" || status.state === "offline") {
        setStep({ name: "failed", message: status.lastError ?? t("Não foi possível sincronizar agora.") });
        return;
      }
      const view = await api.profilesList();
      if (view.profiles.length > 1) setStep({ name: "profiles", view });
      else await finish();
    } catch (e) {
      setStep({ name: "failed", message: errorText(e) });
    }
  };

  const otherAccount = async () => {
    try {
      setAccount(await api.accountSignOut());
      setStep({ name: "account" });
    } catch (e) {
      setStep({ name: "failed", message: errorText(e) });
    }
  };

  return (
    <div className="gate" data-testid="welcome" data-step={step.name}>
      <div className={`gate-card ${step.name === "profiles" ? "wide" : ""}`}>
        <img className="gate-logo" src="./nuvio-astra-128.png" alt="" width={72} height={72} draggable={false} />
        {step.name === "account" && (account?.signedIn ? (
          <>
            <h1>{t("Sincronizar com a conta Nuvio")}</h1>
            <p className="muted">{t("Você já está conectado como {email}, mas este computador ainda não foi sincronizado.", { email: account.email ?? "" })}</p>
            <button className="primary" data-testid="welcome-sync" onClick={() => void connect()}>{t("Sincronizar agora")}</button>
            <button className="gate-link" data-testid="welcome-skip" onClick={() => void finish()}>{t("Agora não")}</button>
          </>
        ) : (
          <>
            <h1>{t("Entre na sua conta Nuvio")}</h1>
            <p className="muted">{t("Perfis, addons, Continuar assistindo, favoritos, coleções, Início e configurações chegam sozinhos. O que já existe neste computador é mantido junto, com uma cópia de segurança antes.")}</p>
            <SignInForm signUp onSignedIn={() => void connect()} />
            <button className="gate-link" data-testid="welcome-skip" onClick={() => void finish()}>{t("Continuar sem conta")}</button>
          </>
        ))}
        {step.name === "syncing" && (
          <>
            <h1>{t("Sincronizando")}</h1>
            <p className="muted" role="status"><span className="spinner" aria-hidden /> {t("Recebendo os dados da sua conta…")}</p>
          </>
        )}
        {step.name === "confirm" && (
          <>
            <h1>{t("Dados de outra conta")}</h1>
            <p className="muted">{step.question}</p>
            <div className="gate-actions">
              <button className="primary" data-testid="welcome-confirm" onClick={() => void connect(true)}>{t("Sincronizar com esta conta")}</button>
              <button className="ghost" data-testid="welcome-other" onClick={() => void otherAccount()}>{t("Usar outra conta")}</button>
            </div>
            <button className="gate-link" onClick={() => void finish()}>{t("Agora não")}</button>
          </>
        )}
        {step.name === "failed" && (
          <>
            <h1>{t("A sincronização não terminou")}</h1>
            <p className="muted">{step.message}</p>
            <div className="gate-actions">
              <button className="primary" onClick={() => void connect()}>{t("Tentar de novo")}</button>
              <button className="ghost" onClick={() => void finish()}>{t("Abrir o app")}</button>
            </div>
          </>
        )}
        {step.name === "profiles" && (
          <>
            <h1>{t("Quem está assistindo?")}</h1>
            <ProfileChooser view={step.view} onChosen={() => void finish()} />
          </>
        )}
      </div>
    </div>
  );
}

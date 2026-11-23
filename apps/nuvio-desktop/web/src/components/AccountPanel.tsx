// Settings → Conta: Nuvio account (e-mail and password, as the original), sync state, last sync,
// changes waiting and sign-out (sync is automatic). Everything is done by Rust; this page shows the status
// the backend confirms (never a "done" before the backend answered).

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { set as setAccount, syncLabel, useAccount, when } from "../account";
import type { SyncPreview as Preview } from "../types";
import { SyncPreview } from "./SyncPreview";
import { t } from "../i18n";

export function AccountPanel() {
  const account = useAccount();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const reviewing = account?.signedIn && account.state === "needsReview";

  const loadPreview = async () => {
    setBusy("preview");
    setPreviewError(null);
    try {
      setPreview(await api.accountSyncPreview());
    } catch (e) {
      setPreviewError(errorText(e));
    } finally {
      setBusy(null);
    }
  };

  // First sync with this account: the read-only preview is loaded at once (nothing is written).
  useEffect(() => {
    if (reviewing && !preview && !previewError && busy === null) void loadPreview();
    if (!account?.signedIn) setPreview(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reviewing, account?.signedIn]);

  if (!account) return <div className="muted">{t("Carregando conta…")}</div>;
  if (!account.configured) {
    return (
      <div className="settings-group" data-testid="account">
        <p className="muted">{t("Esta versão do Nuvio Astra foi compilada sem o servidor da conta Nuvio, então login e sincronização não estão disponíveis. Todos os dados continuam funcionando neste computador.")}</p>
      </div>
    );
  }

  const run = async (label: string, action: () => Promise<void>) => {
    setBusy(label);
    setMessage(null);
    try {
      await action();
    } catch (e) {
      setMessage({ ok: false, text: errorText(e) });
    } finally {
      setBusy(null);
    }
  };

  /** Signed in: the sync starts by itself, except over another account's data. */
  const connect = async () => {
    const s = await api.accountConnect(false);
    setAccount(s);
    if (s.state === "needsReview") setMessage({ ok: true, text: t("Conectado. Os dados deste computador são de outra conta: confira e confirme para sincronizar.") });
    else if (s.state === "error" || s.state === "offline") setMessage({ ok: false, text: s.lastError ?? t("Não foi possível sincronizar agora.") });
    else setMessage({ ok: true, text: t("Conectado e sincronizado.") });
  };

  const signIn = () =>
    run("signin", async () => {
      setAccount(await api.accountSignIn(email, password));
      setPassword("");
      await connect();
    });

  const signUp = () =>
    run("signup", async () => {
      const r = await api.accountSignUp(email, password);
      setMessage({ ok: true, text: r.message });
      if (r.signedIn) {
        setAccount(await api.accountStatus());
        await connect();
      }
    });

  const startSync = () =>
    run("sync", async () => {
      const s = await api.accountSyncNow(true);
      setAccount(s);
      const r = s.lastReport;
      if (s.state === "needsReview") return;
      const ok = !!r && s.state !== "error" && s.state !== "offline";
      setMessage(
        ok
          ? { ok: true, text: t("Sincronização concluída.") }
          : { ok: false, text: s.lastError ?? t("Não foi possível sincronizar.") },
      );
      // A comparison shown in t("Ver detalhes") is refreshed; otherwise nothing more is requested.
      if (ok && preview) setPreview(await api.accountSyncPreview().catch(() => null));
    });

  const startFirstSync = () => {
    if (!preview) return;
    const text = t("Iniciar a primeira sincronização com {account}?\n\nSerão enviados {send} item(ns) e recebidos {receive}; {conflicts} conflito(s) resolvido(s) pelas regras mostradas. Nada é apagado, e uma cópia verificada dos dados deste computador é feita antes.", {
      account: account?.email ?? t("esta conta"),
      send: preview.toSend,
      receive: preview.toReceive,
      conflicts: preview.conflicts,
    });
    if (window.confirm(text)) void startSync();
  };

  const signOut = () => {
    const pending = account.pending > 0
      ? "\n\n" + t("{count} alteração(ões) ainda não foram enviadas. Elas continuam guardadas neste computador e serão enviadas quando você entrar de novo nesta conta.", { count: account.pending })
      : "";
    if (!window.confirm(t("Sair da conta {account}?\n\nFavoritos, histórico, progresso, addons e perfis continuam neste computador.{pending}", { account: account.email ?? "", pending }))) return;
    void run("signout", async () => {
      setAccount(await api.accountSignOut());
      setMessage({ ok: true, text: t("Você saiu da conta. Os dados locais foram mantidos.") });
    });
  };

  const label = syncLabel(account);
  const signedIn = account.signedIn;

  return (
    <div className="settings-group account" data-testid="account">
      {signedIn ? (
        <>
          <div className="account-card simple">
            <div className="account-id">
              <div className="account-avatar" aria-hidden>{(account.email ?? "?").slice(0, 1).toUpperCase()}</div>
              <div>
                <div className="setting-title" data-testid="account-email-shown">{account.email}</div>
                <div className={`sync-badge ${label.tone}`} data-testid="sync-state" data-state={account.state}>
                  <span className="sync-dot" aria-hidden /> {label.text}
                </div>
                <div className="muted small" data-testid="last-sync">{t("Última sincronização: {when}", { when: when(account.lastSyncEpochMs) })}</div>
                {!reviewing && (
                  <div className="muted small" data-testid="sync-automatic">
                    {account.state === "error" || account.state === "offline"
                      ? t("Vai tentar de novo sozinho.")
                      : t("Automática: recebe ao abrir e ao voltar para o app, envia a cada alteração.")}
                  </div>
                )}
              </div>
            </div>
            <div className="account-actions">
              <button className="ghost" onClick={signOut} disabled={busy !== null} data-testid="sign-out">{t("Sair")}</button>
            </div>
          </div>
          {reviewing && (
            <div className="banner review" data-testid="first-sync-review">
              <p>{t("Nenhum dado foi sincronizado ainda. Confira a prévia em “Ver detalhes”; a primeira sincronização só começa quando você iniciar.")}</p>
              {account.linkQuestion && <p data-testid="link-question">{account.linkQuestion}</p>}
              {busy === "preview" && <p className="muted">{t("Consultando a conta (somente leitura)…")}</p>}
              {previewError && <div className="row-error">{t("Não foi possível montar a prévia: {error}", { error: previewError })}</div>}
              <div className="account-actions">
                <button className="primary" onClick={startFirstSync} disabled={busy !== null || !preview || !preview.firstSync} data-testid="first-sync-start">
                  {busy === "sync" ? t("Sincronizando…") : t("Iniciar primeira sincronização")}
                </button>
              </div>
            </div>
          )}
          <details className="account-details" data-testid="account-details" open={reviewing || undefined}>
            <summary>{t("Ver detalhes")}</summary>
            <dl className="facts account-facts">
              <div className="fact"><dt>{t("Alterações aguardando envio")}</dt><dd>{account.pending}</dd></div>
              {account.lastReport && (
                <div className="fact"><dt>{t("Última execução")}</dt><dd>{account.lastReport.sent} enviado(s) · {account.lastReport.received} recebido(s)</dd></div>
              )}
            </dl>
            {account.lastError && <div className="row-error" data-testid="sync-error">{account.lastError}</div>}
            <div className="account-actions">
              <button className="ghost" onClick={() => void loadPreview()} disabled={busy !== null || account.state === "syncing"} data-testid={reviewing ? "preview-refresh" : "sync-compare"}>
                {busy === "preview" ? t("Comparando…") : reviewing ? t("Atualizar prévia") : t("Comparar com a conta")}
              </button>
            </div>
            {preview && <SyncPreview preview={preview} />}
            <p className="muted small">
              {t("Sincronizados com a conta Nuvio, por perfil: perfis (nome, cor, avatar, PIN), addons e a ordem deles, favoritos, continuar assistindo, títulos assistidos, coleções (recebidas), ordem e nomes do Início, e preferências de reprodução, áudio e legendas.")}{" "}
              {t("As preferências de reprodução são compartilhadas só entre computadores, como no Nuvio Desktop, e se a conta já tiver preferências salvas por ele.")}{" "}
              {t("Ficam só neste computador: Melhorar imagem, Movimento, Torrent/P2P, catálogos do destaque (o Nuvio também não envia essa escolha para a conta) e favoritos de canais.")}{" "}
              {t("Sem internet, as alterações ficam guardadas e são enviadas quando a conexão voltar.")}
            </p>
          </details>
        </>
      ) : (
        <form className="account-form" onSubmit={(e) => { e.preventDefault(); void signIn(); }} noValidate>
          {account.state === "expired" && <div className="banner">{t("Sua sessão expirou. Entre novamente — nada deste computador foi apagado.")}</div>}
          <p className="muted">{t("Entre com a mesma conta Nuvio que você usa nos outros aparelhos para sincronizar perfis, addons, favoritos e progresso.")}</p>
          <label className="field">
            <span>{t("E-mail")}</span>
            <input type="text" inputMode="email" autoComplete="username" value={email} onChange={(e) => setEmail(e.target.value)} data-testid="account-email" />
          </label>
          <label className="field">
            <span>{t("Senha")}</span>
            <input type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} data-testid="account-password" />
          </label>
          <div className="account-actions">
            <button className="primary" type="submit" disabled={busy !== null || !email.trim() || !password} data-testid="sign-in">{busy === "signin" ? t("Entrando…") : t("Entrar")}</button>
            <button className="ghost" type="button" disabled={busy !== null || !email.trim() || password.length < 6} onClick={() => void signUp()}>{t("Criar conta")}</button>
          </div>
          <p className="muted small">{t("A senha vai apenas para o servidor da conta Nuvio. A sessão fica guardada criptografada pelo Windows (DPAPI) e só este usuário deste computador consegue usá-la.")}</p>
        </form>
      )}
      {message && <div className={`addon-feedback ${message.ok ? "ok" : "error"}`} role="status" data-testid="account-message">{message.text}</div>}
    </div>
  );
}

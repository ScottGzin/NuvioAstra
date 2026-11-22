// Settings → Conta: Nuvio account (e-mail and password, as the original), sync state, last sync,
// changes waiting, manual sync and sign-out. Everything is done by Rust; this page shows the status
// the backend confirms (never a "done" before the backend answered).

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { set as setAccount, syncLabel, useAccount, when } from "../account";
import { translateBackendError, useI18n } from "../i18n";
import type { SyncPreview as Preview } from "../types";
import { SyncPreview } from "./SyncPreview";

export function AccountPanel() {
  const { t } = useI18n();
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

  if (!account) return <div className="muted">{t("account.loading")}</div>;
  if (!account.configured) {
    return (
      <div className="settings-group" data-testid="account">
        <p className="muted">{t("account.notConfigured")}</p>
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

  const signIn = () =>
    run("signin", async () => {
      setAccount(await api.accountSignIn(email, password));
      setPassword("");
      setMessage({ ok: true, text: t("account.signedIn") });
    });

  const signUp = () =>
    run("signup", async () => {
      const r = await api.accountSignUp(email, password);
      setMessage({ ok: true, text: translateBackendError(r.message) });
      if (r.signedIn) setAccount(await api.accountStatus());
    });

  const syncNow = (confirm = false) =>
    run("sync", async () => {
      const s = await api.accountSyncNow(confirm);
      setAccount(s);
      const r = s.lastReport;
      if (s.state === "needsReview") return;
      const ok = !!r && s.state !== "error" && s.state !== "offline";
      setMessage(
        ok
          ? { ok: true, text: t("account.syncDone") }
          : { ok: false, text: translateBackendError(s.lastError ?? t("account.syncFailed")) },
      );
      // A comparison shown in "Ver detalhes" is refreshed; otherwise nothing more is requested.
      if (ok && preview) setPreview(await api.accountSyncPreview().catch(() => null));
    });

  const startFirstSync = () => {
    if (!preview) return;
    const text = t("account.firstSyncConfirm", {
      account: account?.email ?? t("account.thisAccount"),
      send: preview.toSend,
      receive: preview.toReceive,
      conflicts: preview.conflicts,
    });
    if (window.confirm(text)) void syncNow(true);
  };

  const signOut = () => {
    const pending = account.pending > 0 ? t("account.signOutPending", { pending: account.pending }) : "";
    if (!window.confirm(t("account.signOutConfirm", { account: account.email ?? "", pending }))) return;
    void run("signout", async () => {
      setAccount(await api.accountSignOut());
      setMessage({ ok: true, text: t("account.signedOut") });
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
                <div className="muted small" data-testid="last-sync">{t("account.lastSync", { when: when(account.lastSyncEpochMs) })}</div>
              </div>
            </div>
            <div className="account-actions">
              {!reviewing && (
                <button className="primary" onClick={() => void syncNow()} disabled={busy !== null || account.state === "syncing"} data-testid="sync-now">
                  {busy === "sync" || account.state === "syncing" ? t("account.syncing") : t("account.syncNow")}
                </button>
              )}
              <button className="ghost" onClick={signOut} disabled={busy !== null} data-testid="sign-out">{t("account.signOut")}</button>
            </div>
          </div>
          {reviewing && (
            <div className="banner review" data-testid="first-sync-review">
              <p>{t("account.review")}</p>
              {account.linkQuestion && <p data-testid="link-question">{translateBackendError(account.linkQuestion)}</p>}
              {busy === "preview" && <p className="muted">{t("account.previewLoading")}</p>}
              {previewError && <div className="row-error">{t("account.previewError", { error: previewError })}</div>}
              <div className="account-actions">
                <button className="primary" onClick={startFirstSync} disabled={busy !== null || !preview || !preview.firstSync} data-testid="first-sync-start">
                  {busy === "sync" ? t("account.syncing") : t("account.firstSyncStart")}
                </button>
              </div>
            </div>
          )}
          <details className="account-details" data-testid="account-details" open={reviewing || undefined}>
            <summary>{t("account.details")}</summary>
            <dl className="facts account-facts">
              <div className="fact"><dt>{t("account.pending")}</dt><dd>{account.pending}</dd></div>
              {account.lastReport && (
                <div className="fact"><dt>{t("account.lastRun")}</dt><dd>{t("account.lastRunValues", { sent: account.lastReport.sent, received: account.lastReport.received })}</dd></div>
              )}
            </dl>
            {account.lastError && <div className="row-error" data-testid="sync-error">{translateBackendError(account.lastError)}</div>}
            <div className="account-actions">
              <button className="ghost" onClick={() => void loadPreview()} disabled={busy !== null || account.state === "syncing"} data-testid={reviewing ? "preview-refresh" : "sync-compare"}>
                {busy === "preview" ? t("account.comparing") : reviewing ? t("account.refreshPreview") : t("account.compare")}
              </button>
            </div>
            {preview && <SyncPreview preview={preview} />}
            <p className="muted small">{t("account.syncedDescription")}</p>
          </details>
        </>
      ) : (
        <form className="account-form" onSubmit={(e) => { e.preventDefault(); void signIn(); }} noValidate>
          {account.state === "expired" && <div className="banner">{t("account.sessionExpired")}</div>}
          <p className="muted">{t("account.signInDescription")}</p>
          <label className="field">
            <span>{t("account.email")}</span>
            <input type="text" inputMode="email" autoComplete="username" value={email} onChange={(e) => setEmail(e.target.value)} data-testid="account-email" />
          </label>
          <label className="field">
            <span>{t("account.password")}</span>
            <input type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} data-testid="account-password" />
          </label>
          <div className="account-actions">
            <button className="primary" type="submit" disabled={busy !== null || !email.trim() || !password} data-testid="sign-in">{busy === "signin" ? t("account.signingIn") : t("account.signIn")}</button>
            <button className="ghost" type="button" disabled={busy !== null || !email.trim() || password.length < 6} onClick={() => void signUp()}>{t("account.signUp")}</button>
          </div>
          <p className="muted small">{t("account.passwordNote")}</p>
        </form>
      )}
      {message && <div className={`addon-feedback ${message.ok ? "ok" : "error"}`} role="status" data-testid="account-message">{message.text}</div>}
    </div>
  );
}

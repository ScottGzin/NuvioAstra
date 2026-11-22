// Settings → Conta: Nuvio account (e-mail and password, as the original), sync state, last sync,
// changes waiting, manual sync and sign-out. Everything is done by Rust; this page shows the status
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

  if (!account) return <div className="muted">{t("Loading account…")}</div>;
  if (!account.configured) {
    return (
      <div className="settings-group" data-testid="account">
        <p className="muted">{t("This version of Nuvio Astra was built without the Nuvio account server, so sign-in and sync are unavailable. All data keeps working on this computer.")}</p>
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
      setMessage({ ok: true, text: t("Signed in. Nothing has been synced yet: review the preview and start the first sync whenever you like.") });
    });

  const signUp = () =>
    run("signup", async () => {
      const r = await api.accountSignUp(email, password);
      setMessage({ ok: true, text: r.message });
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
          ? { ok: true, text: t("Sync complete.") }
          : { ok: false, text: s.lastError ?? t("Could not sync.") },
      );
      // A comparison shown in "View details" is refreshed; otherwise nothing more is requested.
      if (ok && preview) setPreview(await api.accountSyncPreview().catch(() => null));
    });

  const startFirstSync = () => {
    if (!preview) return;
    const text = t(
      "Start the first sync with {email}?\n\n{sent} item(s) will be sent and {received} received; {conflicts} conflict(s) resolved by the rules shown. Nothing is deleted, and a verified copy of this computer's data is made first.",
      {
        email: account?.email ?? t("this account"),
        sent: preview.toSend,
        received: preview.toReceive,
        conflicts: preview.conflicts,
      },
    );
    if (window.confirm(text)) void syncNow(true);
  };

  const signOut = () => {
    const pending = account.pending > 0 ? "\n\n" + t("{n} change(s) have not been sent yet. They stay stored on this computer and will be sent when you sign in to this account again.", { n: account.pending }) : "";
    if (!window.confirm(t("Sign out of account {email}?\n\nFavorites, history, progress, addons, and profiles stay on this computer.", { email: account.email ?? "" }) + pending)) return;
    void run("signout", async () => {
      setAccount(await api.accountSignOut());
      setMessage({ ok: true, text: t("You signed out. Local data was kept.") });
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
                <div className="muted small" data-testid="last-sync">{t("Last sync: {time}", { time: when(account.lastSyncEpochMs) })}</div>
              </div>
            </div>
            <div className="account-actions">
              {!reviewing && (
                <button className="primary" onClick={() => void syncNow()} disabled={busy !== null || account.state === "syncing"} data-testid="sync-now">
                  {busy === "sync" || account.state === "syncing" ? t("Syncing…") : t("Sync now")}
                </button>
              )}
              <button className="ghost" onClick={signOut} disabled={busy !== null} data-testid="sign-out">{t("Sign out")}</button>
            </div>
          </div>
          {reviewing && (
            <div className="banner review" data-testid="first-sync-review">
              <p>{t("No data has been synced yet. Check the preview in \"View details\"; the first sync only starts when you start it.")}</p>
              {account.linkQuestion && <p data-testid="link-question">{account.linkQuestion}</p>}
              {busy === "preview" && <p className="muted">{t("Querying the account (read-only)…")}</p>}
              {previewError && <div className="row-error">{t("Could not build the preview: {error}", { error: previewError })}</div>}
              <div className="account-actions">
                <button className="primary" onClick={startFirstSync} disabled={busy !== null || !preview || !preview.firstSync} data-testid="first-sync-start">
                  {busy === "sync" ? t("Syncing…") : t("Start first sync")}
                </button>
              </div>
            </div>
          )}
          <details className="account-details" data-testid="account-details" open={reviewing || undefined}>
            <summary>{t("View details")}</summary>
            <dl className="facts account-facts">
              <div className="fact"><dt>{t("Changes waiting to be sent")}</dt><dd>{account.pending}</dd></div>
              {account.lastReport && (
                <div className="fact"><dt>{t("Last run")}</dt><dd>{t("{sent} sent · {received} received", { sent: account.lastReport.sent, received: account.lastReport.received })}</dd></div>
              )}
            </dl>
            {account.lastError && <div className="row-error" data-testid="sync-error">{account.lastError}</div>}
            <div className="account-actions">
              <button className="ghost" onClick={() => void loadPreview()} disabled={busy !== null || account.state === "syncing"} data-testid={reviewing ? "preview-refresh" : "sync-compare"}>
                {busy === "preview" ? t("Comparing…") : reviewing ? t("Refresh preview") : t("Compare with the account")}
              </button>
            </div>
            {preview && <SyncPreview preview={preview} />}
            <p className="muted small">
              {t("Synced with the Nuvio account, per profile: profiles (name, color, avatar, PIN), addons and their order, movie and series favorites, playback progress, and watched titles. Works offline: changes are stored and sent when the connection is back.")}
            </p>
          </details>
        </>
      ) : (
        <form className="account-form" onSubmit={(e) => { e.preventDefault(); void signIn(); }} noValidate>
          {account.state === "expired" && <div className="banner">{t("Your session expired. Sign in again — nothing on this computer was deleted.")}</div>}
          <p className="muted">{t("Sign in with the same Nuvio account you use on your other devices to sync profiles, addons, favorites, and progress.")}</p>
          <label className="field">
            <span>{t("E-mail")}</span>
            <input type="text" inputMode="email" autoComplete="username" value={email} onChange={(e) => setEmail(e.target.value)} data-testid="account-email" />
          </label>
          <label className="field">
            <span>{t("Password")}</span>
            <input type="password" autoComplete="current-password" value={password} onChange={(e) => setPassword(e.target.value)} data-testid="account-password" />
          </label>
          <div className="account-actions">
            <button className="primary" type="submit" disabled={busy !== null || !email.trim() || !password} data-testid="sign-in">{busy === "signin" ? t("Signing in…") : t("Sign in")}</button>
            <button className="ghost" type="button" disabled={busy !== null || !email.trim() || password.length < 6} onClick={() => void signUp()}>{t("Create account")}</button>
          </div>
          <p className="muted small">{t("The password goes only to the Nuvio account server. The session is stored encrypted by Windows (DPAPI) and only this user on this computer can use it.")}</p>
        </form>
      )}
      {message && <div className={`addon-feedback ${message.ok ? "ok" : "error"}`} role="status" data-testid="account-message">{message.text}</div>}
    </div>
  );
}

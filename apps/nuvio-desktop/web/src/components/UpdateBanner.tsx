// New version: a window in the middle of the screen with the version, what is new (the release notes,
// emojis and simple Markdown kept) and t("Atualizar agora") / t("Depois"). While updating it shows the
// download progress, the signature check and the installer. It waits while a video is open in full
// screen and appears when the player closes. t("Depois") hides it for that version until the app restarts.

import { useEffect, useRef } from "react";
import { clearInstallError, dismissUpdate, installUpdate, startAutomaticChecks, useUpdates } from "../updates";
import { decimal, t } from "../i18n";
import { ReleaseNotes } from "./ReleaseNotes";

export function megabytes(n: number) {
  return `${decimal(n / 1048576)} MB`;
}

export function UpdateBanner({ automatic, waiting = false }: { automatic: boolean; waiting?: boolean }) {
  const u = useUpdates();
  const now = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (automatic && u.status?.configured) startAutomaticChecks();
  }, [automatic, u.status?.configured]);

  const a = u.available;
  const p = u.installing;
  const shown = !!a && !(u.dismissed === a.version && !p) && !waiting;

  useEffect(() => {
    if (!shown) return;
    now.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !p) dismissUpdate();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [shown, p]);

  if (!shown || !a) return null;
  const pct = p?.total ? Math.min(100, Math.round((p.downloaded / p.total) * 100)) : null;

  return (
    <div className="update-modal-backdrop" data-testid="update-banner">
      <div className="update-modal" role="dialog" aria-modal="true" aria-labelledby="update-modal-title">
        <header className="update-modal-head">
          <span className="update-modal-badge">{t("Nova versão")}</span>
          <h2 id="update-modal-title">Nuvio Astra {a.version}</h2>
          <span className="muted small">{t("Você usa a {version}", { version: a.currentVersion })}</span>
        </header>

        <div className="update-modal-body">
          {a.notes ? <ReleaseNotes notes={a.notes} /> : <p className="muted">{t("Melhorias e correções.")}</p>}
        </div>

        <footer className="update-modal-foot">
          {!p && (
            <>
              <button className="ghost" onClick={dismissUpdate}>{t("Depois")}</button>
              <button className="primary" ref={now} onClick={() => void installUpdate()} data-testid="update-now">{t("Atualizar agora")}</button>
            </>
          )}
          {p && p.phase === "downloading" && (
            <div className="update-progress" role="status">
              <span>{t("Baixando")} {pct !== null ? `${pct}%` : megabytes(p.downloaded)}</span>
              <span className="update-bar"><span style={{ width: `${pct ?? 30}%` }} /></span>
            </div>
          )}
          {p && p.phase === "verifying" && <span role="status">{t("Verificando a assinatura…")}</span>}
          {p && p.phase === "installing" && <span role="status">{t("Assinatura válida. Instalando — o Nuvio Astra vai fechar e abrir de novo.")}</span>}
          {p && p.phase === "error" && (
            <>
              <span className="update-error" role="alert">{t(p.error ?? "")}</span>
              <button className="ghost" onClick={clearInstallError}>{t("Fechar")}</button>
            </>
          )}
        </footer>
      </div>
    </div>
  );
}

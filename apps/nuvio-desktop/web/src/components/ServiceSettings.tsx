// Settings → TorBox and Settings → Downloads.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { t } from "../i18n";
import { sizeText } from "../sources";
import type { DebridSettings, DownloadsFolder, TorboxStatus } from "../types";
import { Row, Toggle } from "./SettingControls";

export function TorboxSection({ settings, update }: { settings: DebridSettings; update(change: (s: DebridSettings) => DebridSettings): void }) {
  const [status, setStatus] = useState<TorboxStatus | null>(null);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    api.torboxStatus().then(setStatus, () => {});
  }, []);

  const save = async () => {
    setBusy(true);
    setMessage(null);
    try {
      setStatus(await api.torboxSaveKey(key));
      setKey("");
      setMessage({ ok: true, text: t("Chave conferida com o TorBox e guardada criptografada neste computador.") });
    } catch (e) {
      setMessage({ ok: false, text: errorText(e) });
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!window.confirm(t("Remover a chave do TorBox deste perfil?"))) return;
    setStatus(await api.torboxRemoveKey());
    setMessage({ ok: true, text: t("Chave removida.") });
  };

  return (
    <div className="settings-group" data-testid="torbox-settings">
      <p className="muted">
        {t("Com a sua chave do TorBox, as fontes torrent que já estão no cache dele aparecem como TB Instant e abrem na hora, como link direto, sem P2P. Funciona com addons de torrent configurados sem chave, e também com addons que já entregam fontes do TorBox.")}
      </p>
      {status?.configured ? (
        <Row title={t("Conta TorBox")} description={t("A chave fica criptografada pelo Windows neste computador e só é enviada ao próprio TorBox.")}>
          <span className="account-actions" style={{ marginTop: 0 }}>
            <span data-testid="torbox-account">{status.account}</span>
            <button className="ghost" onClick={() => void remove()}>{t("Remover chave")}</button>
          </span>
        </Row>
      ) : (
        <Row title={t("Chave de API do TorBox")} description={t("Em torbox.app: Settings → API Key. Cole aqui e clique em Salvar.")}>
          <form className="torbox-key" onSubmit={(e) => { e.preventDefault(); void save(); }}>
            <input type="password" autoComplete="off" spellCheck={false} placeholder={t("Cole a chave")} aria-label={t("Chave de API do TorBox")} value={key} onChange={(e) => setKey(e.target.value)} data-testid="torbox-key" />
            <button className="primary" type="submit" disabled={busy || key.trim().length < 8} data-testid="torbox-save">{busy ? t("Conferindo…") : t("Salvar")}</button>
          </form>
        </Row>
      )}
      <Row title={t("Usar o TorBox nas fontes")} description={t("Marca as fontes Instant e abre por ele. Desligado, os torrents continuam pelo P2P.")}>
        <Toggle label={t("Usar o TorBox nas fontes")} checked={settings.enabled} onChange={(v) => update((s) => ({ ...s, enabled: v }))} />
      </Row>
      {message && <div className={`addon-feedback ${message.ok ? "ok" : "error"}`} role="status" data-testid="torbox-message">{message.text}</div>}
    </div>
  );
}

export function DownloadsSection() {
  const [folder, setFolder] = useState<DownloadsFolder | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.downloadsFolder().then(setFolder, (e) => setError(errorText(e)));
  }, []);
  const choose = async () => {
    try {
      setFolder(await api.downloadsChooseFolder());
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <div className="settings-group" data-testid="downloads-settings">
      <Row title={t("Pasta dos downloads")} description={folder?.freeBytes != null ? t("{size} livres neste disco.", { size: sizeText(folder.freeBytes) }) : undefined}>
        <span className="account-actions" style={{ marginTop: 0 }}>
          <code className="path" data-testid="downloads-folder">{folder?.folder ?? "…"}</code>
          <button className="ghost" onClick={() => void choose()} data-testid="downloads-choose">{t("Escolher pasta")}</button>
        </span>
      </Row>
      <p className="muted small pad-top">
        {t("Para baixar, clique com o botão direito numa fonte (HTTP, TorBox ou torrent) ou use o botão de baixar no player. O arquivo é baixado em várias conexões ao mesmo tempo, continua de onde parou se a conexão cair, se você pausar ou fechar o app, e tenta de novo sozinho em erros do servidor como o 503. Os downloads ficam em Biblioteca → Downloads.")}
      </p>
      {error && <div className="row-error">{error}</div>}
    </div>
  );
}

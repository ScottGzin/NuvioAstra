// Settings → Debrid (TorBox and the other services) and Settings → Downloads.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { t } from "../i18n";
import { sizeText } from "../sources";
import type { DebridSettings, DebridStatus, DownloadsFolder, TorboxStatus } from "../types";
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
      {message && <div className={`addon-feedback ${message.ok ? "ok" : "error"}`} role="status" data-testid="torbox-message">{message.text}</div>}
      <OtherDebridServices />
      <Row title={t("Usar debrid nas fontes")} description={t("Marca as fontes Instant e abre pelos serviços conectados. Desligado, os torrents continuam pelo P2P.")}>
        <Toggle label={t("Usar debrid nas fontes")} checked={settings.enabled} onChange={(v) => update((s) => ({ ...s, enabled: v }))} />
      </Row>
    </div>
  );
}

const HELP: Record<string, string> = {
  realdebrid: "Em real-debrid.com/apitoken: copie o token da API.",
  alldebrid: "Em alldebrid.com/apikeys: crie uma chave e copie.",
  premiumize: "Em premiumize.me/account: copie a API key.",
  debridlink: "Em debrid-link.com/webapp/apikey: copie a chave da API.",
};

/** Real-Debrid, AllDebrid, Premiumize and Debrid-Link: one card each, plus which service is tried first. */
function OtherDebridServices() {
  const [status, setStatus] = useState<DebridStatus | null>(null);
  const [keys, setKeys] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<{ id: string; ok: boolean; text: string } | null>(null);

  useEffect(() => {
    api.debridStatus().then(setStatus, () => {});
  }, []);
  if (!status) return null;

  const save = async (id: string) => {
    setBusy(id);
    setMessage(null);
    try {
      setStatus(await api.debridSaveKey(id, keys[id] ?? ""));
      setKeys((k) => ({ ...k, [id]: "" }));
      setMessage({ id, ok: true, text: t("Chave conferida e guardada criptografada neste computador.") });
    } catch (e) {
      setMessage({ id, ok: false, text: errorText(e) });
    } finally {
      setBusy(null);
    }
  };
  const remove = async (id: string, label: string) => {
    if (!window.confirm(t("Remover a chave do {name} deste perfil?", { name: label }))) return;
    setStatus(await api.debridRemoveKey(id));
    setMessage({ id, ok: true, text: t("Chave removida.") });
  };
  const connected = status.services.filter((s) => s.configured);

  return (
    <>
      <h3 className="settings-subhead">{t("Outros serviços")}</h3>
      <p className="muted small">
        {t("O Premiumize mostra as fontes que já tem (PM Instant). Real-Debrid, AllDebrid e Debrid-Link não informam isso antes: as fontes torrent mostram a sigla do serviço e são pedidas a ele ao abrir. Se ele não tiver o torrent, o pedido é apagado da sua conta e a fonte abre por P2P.")}
      </p>
      {status.services.filter((s) => s.id !== "torbox").map((s) => (
        <div key={s.id} data-testid={`debrid-${s.id}`}>
          {s.configured ? (
            <Row title={s.label} description={t("A chave fica criptografada pelo Windows neste computador e só é enviada ao próprio {name}.", { name: s.label })}>
              <span className="account-actions" style={{ marginTop: 0 }}>
                <span data-testid={`debrid-account-${s.id}`}>{s.account}</span>
                <button className="ghost" onClick={() => void remove(s.id, s.label)}>{t("Remover chave")}</button>
              </span>
            </Row>
          ) : (
            <Row title={t("Chave de API do {name}", { name: s.label })} description={t(HELP[s.id] ?? "")}>
              <form className="torbox-key" onSubmit={(e) => { e.preventDefault(); void save(s.id); }}>
                <input
                  type="password"
                  autoComplete="off"
                  spellCheck={false}
                  placeholder={t("Cole a chave")}
                  aria-label={t("Chave de API do {name}", { name: s.label })}
                  value={keys[s.id] ?? ""}
                  onChange={(e) => setKeys((k) => ({ ...k, [s.id]: e.target.value }))}
                  data-testid={`debrid-key-${s.id}`}
                />
                <button className="primary" type="submit" disabled={busy !== null || (keys[s.id] ?? "").trim().length < 8} data-testid={`debrid-save-${s.id}`}>
                  {busy === s.id ? t("Conferindo…") : t("Salvar")}
                </button>
              </form>
            </Row>
          )}
          {message?.id === s.id && <div className={`addon-feedback ${message.ok ? "ok" : "error"}`} role="status">{message.text}</div>}
        </div>
      ))}
      {connected.length > 1 && (
        <Row title={t("Serviço preferido")} description={t("Tentado primeiro quando mais de um serviço está conectado.")}>
          <select aria-label={t("Serviço preferido")} value={status.preferred ?? ""} onChange={(e) => void api.debridSetPreferred(e.target.value).then(setStatus)} data-testid="debrid-preferred">
            {connected.map((s) => <option key={s.id} value={s.id}>{s.label}</option>)}
          </select>
        </Row>
      )}
    </>
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

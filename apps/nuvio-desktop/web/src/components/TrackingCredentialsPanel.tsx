// Trakt/Simkl credentials and authorization. Client ids are entered locally (or come from the
// gitignored build config); tokens stay in Rust and are never shown here.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { Row } from "./SettingControls";
import { t } from "../i18n";

interface DeviceStart { userCode: string; verificationUrl: string; intervalSecs: number }
interface View { traktConfigured: boolean; simklConfigured: boolean; traktConnected: boolean; simklConnected: boolean }
type Key = "trakt:clientId" | "trakt:clientSecret" | "simkl:clientId";

export function TrackingCredentialsPanel() {
  const [view, setView] = useState<View | null>(null);
  const [draft, setDraft] = useState<Record<Key, string>>({ "trakt:clientId": "", "trakt:clientSecret": "", "simkl:clientId": "" });
  const [error, setError] = useState<string | null>(null);
  const [device, setDevice] = useState<DeviceStart | null>(null);
  const [simklUrl, setSimklUrl] = useState<string | null>(null);
  const [callback, setCallback] = useState("");
  const [sync, setSync] = useState("");

  const reload = () => api.trackingCredentials().then(setView, (e) => setError(errorText(e)));
  useEffect(() => { reload(); }, []);

  // Poll Trakt's device token endpoint while the user authorizes in the browser.
  useEffect(() => {
    if (!device) return;
    let active = true;
    const timer = window.setInterval(() => {
      void api.traktDevicePoll().then((result) => {
        if (!active) return;
        if (result === "authorized") {
          setDevice(null);
          void reload();
          window.clearInterval(timer);
        } else if (result === "expired" || result === "denied") {
          setDevice(null);
          setError(t("Autorização expirada ou recusada."));
          window.clearInterval(timer);
        }
      }, (e) => { if (active) setError(errorText(e)); });
    }, Math.max(5, device.intervalSecs) * 1000);
    return () => { active = false; window.clearInterval(timer); };
  }, [device]);

  const save = async (key: Key, value: string) => {
    setError(null);
    try { setView(await api.trackingCredentialSet(key, value)); }
    catch (e) { setError(errorText(e)); }
  };

  const field = (key: Key, label: string, configured: boolean) => (
    <Row title={label} description={configured ? t("Configurado neste computador.") : t("Não configurado.")}>
      <span className="tmdb-key">
        <input type="password" aria-label={label} placeholder={configured ? "••••••••" : t("Cole o valor")} value={draft[key]} onChange={(e) => setDraft((d) => ({ ...d, [key]: e.target.value }))} />
        <button className="ghost" disabled={!draft[key].trim()} onClick={() => {
          const value = draft[key].trim();
          setDraft((d) => ({ ...d, [key]: "" }));
          void save(key, value);
        }}>{t("Salvar")}</button>
      </span>
    </Row>
  );

  const doSync = (provider: "trakt" | "simkl") => {
    setSync(t("Sincronizando…"));
    void api.trackingSync(provider).then((result) => {
      setSync(t("Sincronizados: {movies} filmes e {episodes} episódios.", { movies: result.movies, episodes: result.episodes }));
    }, (e) => { setSync(""); setError(errorText(e)); });
  };

  return (
    <>
      <h3 className="settings-subhead">Trakt</h3>
      {view && <>
        {field("trakt:clientId", t("ID do cliente Trakt"), view.traktConfigured)}
        {field("trakt:clientSecret", t("Segredo do cliente Trakt"), view.traktConfigured)}
        <div className="modal-actions">
          {view.traktConnected ? (
            <>
              <button className="chip" data-testid="trakt-sync" onClick={() => doSync("trakt")}>{t("Sincronizar assistidos")}</button>
              <button className="ghost" onClick={() => void api.trackingDisconnect("trakt").then(setView)}>{t("Desconectar")}</button>
            </>
          ) : (
            <button className="primary" data-testid="trakt-connect" disabled={!view.traktConfigured || !!device} onClick={() => {
              void api.traktDeviceStart().then(setDevice, (e) => setError(errorText(e)));
            }}>{t("Conectar com código")}</button>
          )}
        </div>
        {device && (
          <div className="settings-group" data-testid="trakt-device">
            <div className="setting-title">{device.userCode}</div>
            <a href={device.verificationUrl} target="_blank" rel="noreferrer">{device.verificationUrl}</a>
            <div className="muted small">{t("Aguardando autorização no navegador…")}</div>
          </div>
        )}
      </>}

      <h3 className="settings-subhead">Simkl</h3>
      {view && <>
        {field("simkl:clientId", t("ID do cliente Simkl"), view.simklConfigured)}
        <div className="modal-actions">
          {view.simklConnected ? (
            <>
              <button className="chip" data-testid="simkl-sync" onClick={() => doSync("simkl")}>{t("Sincronizar assistidos")}</button>
              <button className="ghost" onClick={() => void api.trackingDisconnect("simkl").then(setView)}>{t("Desconectar")}</button>
            </>
          ) : (
            <button className="primary" data-testid="simkl-connect" disabled={!view.simklConfigured} onClick={() => {
              void api.simklAuthorizeUrl().then((url) => { setSimklUrl(url); window.open(url, "_blank", "noopener"); }, (e) => setError(errorText(e)));
            }}>{t("Abrir autorização")}</button>
          )}
        </div>
        {simklUrl && !view.simklConnected && (
          <div className="settings-group">
            <p className="muted small">{t("Após autorizar, cole aqui o código e o state do endereço de retorno.")}</p>
            <input aria-label={t("Código de autorização")} placeholder={t("Código")} value={callback} onChange={(e) => setCallback(e.target.value)} />
            <button className="primary" disabled={!callback.includes("code=")} onClick={() => {
              const params = new URLSearchParams(callback.split("?")[1] ?? callback);
              const code = params.get("code") ?? "";
              const state = params.get("state") ?? "";
              void api.simklComplete(code, state).then(() => { setSimklUrl(null); setCallback(""); void reload(); }, (e) => setError(errorText(e)));
            }}>{t("Concluir autorização")}</button>
          </div>
        )}
      </>}
      {sync && <p className="muted small" role="status">{sync}</p>}
      {error && <div className="row-error" role="alert">{error}</div>}
      <p className="muted small">{t("Registre seu próprio aplicativo no Trakt/Simkl e cole os valores aqui. Eles ficam só neste computador e não aparecem mais na interface.")}</p>
    </>
  );
}

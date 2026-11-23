// Trakt/Simkl account cards, styled like the original app. Client ids are entered locally (or come
// from the gitignored build config); tokens stay in Rust and are never shown here.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { Row } from "./SettingControls";
import { t } from "../i18n";

interface DeviceStart { userCode: string; verificationUrl: string; intervalSecs: number }
interface View { traktConfigured: boolean; simklConfigured: boolean; traktConnected: boolean; simklConnected: boolean; traktFromBuild: boolean; simklFromBuild: boolean }
type Key = "trakt:clientId" | "trakt:clientSecret" | "simkl:clientId";

export function TrackingCredentialsPanel() {
  const [view, setView] = useState<View | null>(null);
  const [draft, setDraft] = useState<Record<Key, string>>({ "trakt:clientId": "", "trakt:clientSecret": "", "simkl:clientId": "" });
  const [error, setError] = useState<string | null>(null);
  const [device, setDevice] = useState<DeviceStart | null>(null);
  const [simklUrl, setSimklUrl] = useState<string | null>(null);
  const [callback, setCallback] = useState("");
  const [sync, setSync] = useState("");
  const [showTrakt, setShowTrakt] = useState(false);
  const [showSimkl, setShowSimkl] = useState(false);
  const [copied, setCopied] = useState(false);

  const reload = () => api.trackingCredentials().then(setView, (e) => setError(errorText(e)));
  useEffect(() => { reload(); }, []);

  useEffect(() => {
    const onCallback = (event: Event) => {
      const url = (event as CustomEvent<string>).detail;
      const params = new URLSearchParams(url.split("?")[1] ?? "");
      const code = params.get("code") ?? "";
      const state = params.get("state") ?? "";
      if (!code) return;
      void api.simklComplete(code, state).then(() => {
        setSimklUrl(null);
        setCallback("");
        void reload();
      }, (e) => setError(errorText(e)));
    };
    window.addEventListener("nuvio-simkl-callback", onCallback);
    return () => window.removeEventListener("nuvio-simkl-callback", onCallback);
  }, []);

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
      {view && <section className={`provider-card trakt ${device ? "authorizing" : ""}`} data-provider="trakt">
        <div className="provider-logo trakt-logo">trakt</div>
        <div className="provider-watermark" aria-hidden>✓</div>
        {!device ? (
          <>
            <p className="provider-desc">{t("Sincronize sua lista, progresso, continuar assistindo, scrobbles e listas pessoais com o Trakt.")}</p>
            {view.traktConnected ? (
              <div className="modal-actions">
                <button className="provider-action large" data-testid="trakt-sync" onClick={() => doSync("trakt")}>{t("Sincronizar assistidos")}</button>
                <button className="provider-action outline" onClick={() => void api.trackingDisconnect("trakt").then(setView)}>{t("Desconectar")}</button>
              </div>
            ) : (
              <button className="provider-action large" data-testid="trakt-connect" disabled={!view.traktConfigured} onClick={() => {
                void api.traktDeviceStart().then(setDevice, (e) => setError(errorText(e)));
              }}>{t("Conectar ao Trakt")}</button>
            )}
          </>
        ) : (
          <>
            <h3 className="provider-heading">{t("Digite este código no Trakt")}</h3>
            <p className="provider-desc">{t("Abra o link e digite este código para aprovar o Nuvio. Esta tela será atualizada automaticamente.")}</p>
            <div className="device-code-box" data-testid="trakt-device">
              <strong>{device.userCode}</strong>
              <button className="link" onClick={() => void api.openUrl(device.verificationUrl)}>{device.verificationUrl}</button>
              <button
                className="copy-code"
                data-testid="trakt-copy-code"
                onClick={() => {
                  void navigator.clipboard.writeText(device.userCode).then(() => {
                    setCopied(true);
                    window.setTimeout(() => setCopied(false), 1800);
                  }, () => {});
                }}
              >{copied ? t("تم النسخ ✓") : t("نسخ الكود")}</button>
            </div>
            <button className="provider-action large" data-testid="trakt-open-activate" onClick={() => void api.openUrl(device.verificationUrl).catch((e) => setError(errorText(e)))}>{t("Abrir ativação do Trakt")}</button>
            <button className="provider-action outline" onClick={() => setDevice(null)}>{t("Cancelar")}</button>
            <div className="device-status">{t("Aguardando aprovação no Trakt…")}</div>
          </>
        )}
        {(!view.traktFromBuild || showTrakt) && <div className="provider-credentials">
          {field("trakt:clientId", t("ID do cliente Trakt"), view.traktConfigured)}
          {field("trakt:clientSecret", t("Segredo do cliente Trakt"), view.traktConfigured)}
        </div>}
        {view.traktFromBuild && !showTrakt && <button className="provider-more" onClick={() => setShowTrakt(true)}>{t("Alterar credenciais")}</button>}
      </section>}

      {view && <section className={`provider-card simkl ${simklUrl && !view.simklConnected ? "authorizing" : ""}`} data-provider="simkl">
        <div className="provider-logo simkl-logo">SIMKL™</div>
        <div className="provider-watermark simkl-watermark" aria-hidden>S</div>
        <p className="provider-desc">{t("Conecte o Simkl para sincronizar listas, histórico, progresso e scrobbles.")}</p>
        {view.simklConnected ? (
          <div className="modal-actions">
            <button className="provider-action large" data-testid="simkl-sync" onClick={() => doSync("simkl")}>{t("Sincronizar assistidos")}</button>
            <button className="ghost" onClick={() => void api.trackingDisconnect("simkl").then(setView)}>{t("Desconectar")}</button>
          </div>
        ) : (
          <button className="provider-action large" data-testid="simkl-connect" disabled={!view.simklConfigured} onClick={() => {
            void api.simklAuthorizeUrl().then((url) => {
              setSimklUrl(url);
              void api.openUrl(url).catch((e) => setError(errorText(e)));
            }, (e) => setError(errorText(e)));
          }}>{t("Conectar ao Simkl")}</button>
        )}
        {simklUrl && !view.simklConnected && (
          <div className="provider-credentials">
            <p className="muted small">{t("Após autorizar, cole aqui o código e o state do endereço de retorno.")}</p>
            <input aria-label={t("Código de autorização")} placeholder={t("Código")} value={callback} onChange={(e) => setCallback(e.target.value)} />
            <button className="primary" disabled={!callback.includes("code=")} onClick={() => {
              const params = new URLSearchParams(callback.split("?")[1] ?? callback);
              void api.simklComplete(params.get("code") ?? "", params.get("state") ?? "")
                .then(() => { setSimklUrl(null); setCallback(""); void reload(); }, (e) => setError(errorText(e)));
            }}>{t("Concluir autorização")}</button>
          </div>
        )}
        {(!view.simklFromBuild || showSimkl) && <div className="provider-credentials">
          {field("simkl:clientId", t("ID do cliente Simkl"), view.simklConfigured)}
        </div>}
        {view.simklFromBuild && !showSimkl && <button className="provider-more" onClick={() => setShowSimkl(true)}>{t("Alterar credenciais")}</button>}
        <button className="provider-link" onClick={() => void api.openUrl("https://simkl.com")}>{t("Visitar Simkl")}</button>
      </section>}
      {sync && <p className="muted small" role="status">{sync}</p>}
      {error && <div className="row-error" role="alert">{error}</div>}
    </>
  );
}

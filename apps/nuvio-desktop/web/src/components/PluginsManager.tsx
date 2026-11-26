// Settings → Addons → Plugins: Nuvio scraper repositories (manifest.json), not VyseTV's repo.json.
// The manifest address never reaches the UI: Rust returns an opaque key for every repository.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { t } from "../i18n";
import type { PluginsView } from "../types";
import { Row, Toggle } from "./SettingControls";

type Feedback = { kind: "ok" | "error" | "busy"; text: string } | null;

const dateText = (ms: number) => (ms > 0 ? new Date(ms).toLocaleDateString() : "—");

export function PluginsManager() {
  const [view, setView] = useState<PluginsView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<Feedback>(null);
  const [address, setAddress] = useState("");

  useEffect(() => {
    api.pluginsList().then(setView, (e) => setError(errorText(e)));
  }, []);

  const run = async (busy: string, action: () => Promise<PluginsView>, done: string) => {
    setFeedback({ kind: "busy", text: busy });
    try {
      setView(await action());
      setFeedback({ kind: "ok", text: done });
    } catch (e) {
      setFeedback({ kind: "error", text: errorText(e) });
    }
  };

  if (error) return <div className="row-error">{t("Não foi possível ler os plugins: {error}", { error })}</div>;
  if (!view) return <div className="muted">{t("Carregando plugins…")}</div>;

  const scrapers = (key: string) => view.scrapers.filter((s) => s.repositoryKey === key);

  return (
    <div className="settings-group addons" data-testid="plugins">
      <h2 className="settings-subtitle">{t("Plugins de fontes")}</h2>
      <p className="muted small">
        {t("Repositórios de scrapers do Nuvio (manifest.json). Os scripts são validados antes de serem guardados e executados num ambiente isolado, sem acesso à sua rede local.")}
      </p>

      <Row title={t("Ativar plugins")} description={t("Adiciona as fontes encontradas pelos scrapers à lista de fontes de cada título.")}>
        <Toggle label={t("Ativar plugins")} checked={view.enabled} disabled={view.scrapers.length === 0} onChange={(v) => void run(t("Salvando…"), () => api.pluginsSetEnabled(v), t("Preferência salva."))} />
      </Row>

      <form className="addon-install-row" noValidate onSubmit={(e) => {
        e.preventDefault();
        const url = address.trim();
        if (!url) return;
        setAddress("");
        void run(t("Baixando e validando o repositório…"), () => api.pluginsAdd(url), t("Repositório adicionado."));
      }}>
        <input
          type="text"
          inputMode="url"
          placeholder={t("URL do repositório de plugins (manifest.json)")}
          aria-label={t("URL do repositório de plugins")}
          value={address}
          onChange={(e) => setAddress(e.target.value)}
        />
        <button className="primary" type="submit" disabled={!address.trim()}>{t("Adicionar")}</button>
      </form>

      {feedback && <div className={`addon-feedback ${feedback.kind === "error" ? "error" : feedback.kind === "ok" ? "ok" : ""}`} role="status">{feedback.text}</div>}
      {view.error && <div className="row-error">{view.error}</div>}

      {view.repositories.length === 0 ? (
        <p className="muted small">{t("Nenhum repositório adicionado.")}</p>
      ) : (
        view.repositories.map((repo) => {
          const items = scrapers(repo.key);
          return (
            <div className="plugin-repo" key={repo.key} data-testid={`plugin-repo-${repo.key}`}>
              <div className="addon-overview">
                <div><b>{repo.name}</b><span className="muted small">v{repo.version}</span></div>
                <div><b>{items.length}</b><span className="muted small">{t("Scrapers")}</span></div>
                <div><b>{dateText(repo.lastUpdated)}</b><span className="muted small">{t("Atualizado")}</span></div>
              </div>
              <div className="account-actions" style={{ marginTop: 0 }}>
                <button className="ghost" onClick={() => void run(t("Atualizando…"), () => api.pluginsRefresh(repo.key), t("Repositório atualizado."))}>{t("Atualizar")}</button>
                <button className="ghost" onClick={() => {
                  if (!window.confirm(t("Remover este repositório e todos os seus scrapers?"))) return;
                  void run(t("Removendo…"), () => api.pluginsRemove(repo.key), t("Repositório removido."));
                }}>{t("Remover")}</button>
              </div>
              {items.map((scraper) => (
                <Row key={scraper.key} title={scraper.name} description={`v${scraper.version} · ${scraper.supportedTypes.join(", ")}`}>
                  <Toggle
                    label={scraper.name}
                    checked={scraper.enabled}
                    disabled={!scraper.manifestEnabled}
                    onChange={(v) => void run(t("Salvando…"), () => api.pluginsSetScraperEnabled(scraper.key, v), t("Preferência salva."))}
                  />
                </Row>
              ))}
            </div>
          );
        })
      )}
      <p className="muted small settings-note">
        {t("O estado dos plugins fica neste perfil e não é sincronizado com a conta Nuvio.")}
      </p>
    </div>
  );
}

// Settings → Fontes e downloads → Plugins (`PluginsSettingsPage` of the original): repositories of
// scraper plugins (the Nuvio community `manifest.json` format), each plugin on or off, and the
// master switch. The scripts run isolated (no access to this computer or the local network).

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { Row, Toggle } from "./SettingControls";
import type { PluginsView } from "../types";
import { locale, t } from "../i18n";

export function PluginsSection() {
  const [view, setView] = useState<PluginsView | null>(null);
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.pluginsList().then(setView, (e) => setError(errorText(e)));
  }, []);

  const run = async (what: string, action: () => Promise<PluginsView>) => {
    setBusy(what);
    setError(null);
    try {
      setView(await action());
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(null);
    }
  };

  const add = () => run("add", async () => {
    const v = await api.pluginsAdd(url);
    setUrl("");
    return v;
  });

  return (
    <div className="settings-group" data-testid="plugins-settings">
      <p className="muted">
        {t("Plugins procuram fontes em sites, como os addons. Adicione o endereço de um repositório de plugins do Nuvio (o arquivo manifest.json). Eles rodam isolados: não acessam este computador nem a sua rede local. Precisam do TMDB configurado para títulos com código do IMDb.")}
      </p>
      <Row title={t("Usar plugins")} description={t("As fontes dos plugins aparecem no painel de fontes, depois das dos addons.")}>
        <Toggle label={t("Usar plugins")} checked={view?.enabled ?? false} onChange={(v) => void run("toggle", () => api.pluginsSetEnabled(v))} />
      </Row>
      <Row title={t("Adicionar repositório")}>
        <div className="torbox-key">
          <input
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && url.trim() && !busy && void add()}
            className="wide"
            placeholder="https://…/manifest.json"
            spellCheck={false}
            aria-label={t("Endereço do repositório")}
            data-testid="plugins-url"
          />
          <button className="primary" onClick={() => void add()} disabled={!url.trim() || busy !== null} data-testid="plugins-add">{busy === "add" ? t("Instalando…") : t("Adicionar")}</button>
        </div>
      </Row>
      {error && <div className="row-error" role="alert">{t(error)}</div>}
      {view?.warning && <div className="muted small" role="status" data-testid="plugins-warning">{t("Alguns plugins não foram instalados:")} {view.warning}</div>}

      {view?.repositories.map((r) => (
        <div key={r.key} className="plugin-repo" data-repo={r.key}>
          <div className="plugin-repo-head">
            <div className="plugin-repo-text">
              <div className="setting-title">{r.name}</div>
              <div className="muted small">
                {[r.origin, r.version && `v${r.version}`, t("{n} plugins", { n: r.scraperCount }), r.lastUpdated ? t("atualizado em {when}", { when: new Date(r.lastUpdated).toLocaleDateString(locale()) }) : null].filter(Boolean).join(" · ")}
              </div>
            </div>
            <button className="ghost" onClick={() => void run(`refresh:${r.key}`, () => api.pluginsRefresh(r.key))} disabled={busy !== null} data-testid="plugins-refresh">{busy === `refresh:${r.key}` ? t("Atualizando…") : t("Atualizar")}</button>
            <button
              className="ghost"
              onClick={() => { if (window.confirm(t("Remover o repositório “{name}” e os plugins dele?", { name: r.name }))) void run("remove", () => api.pluginsRemove(r.key)); }}
              disabled={busy !== null}
              data-testid="plugins-remove"
            >
              {t("Remover")}
            </button>
          </div>
          <div className="plugin-list">
            {view.scrapers.filter((s) => s.repositoryKey === r.key).map((s) => (
              <div key={s.key} className="plugin-item" data-plugin={s.key}>
                {s.logo && s.logo.startsWith("https://") ? <img src={s.logo} alt="" loading="lazy" /> : <span className="plugin-dot" aria-hidden>{s.name.slice(0, 1)}</span>}
                <div className="plugin-text">
                  <div>{s.name} <span className="muted small">v{s.version}</span></div>
                  <div className="muted small">{[s.supportedTypes.map((x) => (x === "movie" ? t("Filmes") : t("Séries"))).join(", "), s.description].filter(Boolean).join(" · ")}</div>
                </div>
                <Toggle label={s.name} checked={s.enabled} onChange={(v) => void run("scraper", () => api.pluginsSetScraperEnabled(s.key, v))} />
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

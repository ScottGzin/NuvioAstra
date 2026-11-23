// Settings → Addons: the active profile's addons, as in the original's AddonsScreen (install from a
// manifest address, enable/disable, order, refresh, configure on the addon's page, delete with
// confirmation). Every action goes through Rust; the manifest addresses never reach the UI.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import type { AddonPreview, AddonView, AddonsView } from "../types";
import { typeLabel } from "./PosterCard";
import { t } from "../i18n";
import { message } from "../rustErrors";

type Feedback = { kind: "ok" | "error" | "busy"; text: string } | null;

function AddonLogo({ addon }: { addon: AddonView }) {
  const [failed, setFailed] = useState(false);
  return (
    <div className="addon-logo" aria-hidden>
      {addon.logo && !failed ? <img src={addon.logo} alt="" loading="lazy" onError={() => setFailed(true)} /> : addon.name.slice(0, 1).toUpperCase()}
    </div>
  );
}

function Badges({ addon }: { addon: AddonView }) {
  return (
    <div className="chips addon-badges">
      {addon.status === "ok" && <span className="chip badge-ok">{t("Ativo")}</span>}
      {addon.status === "disabled" && <span className="chip">{t("Desativado")}</span>}
      {addon.status === "error" && <span className="chip badge-error">{t("Indisponível")}</span>}
      {addon.configurable && <span className="chip">{t("Configurável")}</span>}
      {addon.tracking && <span className="chip">{t("Rastreamento")}</span>}
      {addon.types.slice(0, 6).map((t) => <span key={t} className="chip muted">{typeLabel(t)}</span>)}
    </div>
  );
}

function Catalogs({ addon }: { addon: AddonView }) {
  const [open, setOpen] = useState(false);
  if (!addon.catalogCount) return null;
  return (
    <div className="addon-catalogs">
      <button className="link" onClick={() => setOpen(!open)} aria-expanded={open}>
        {t("{count} catálogo(s)", { count: addon.catalogCount })} {open ? "▴" : "▾"}
      </button>
      {open && (
        addon.catalogs.length ? (
          <ul>{addon.catalogs.map((c, i) => <li key={`${c.contentType}-${c.name}-${i}`}>{c.name} <span className="muted small">· {typeLabel(c.contentType)}</span></li>)}</ul>
        ) : (
          <div className="muted small">{t("Ative o addon para ver a lista de catálogos.")}</div>
        )
      )}
    </div>
  );
}

export function AddonsManager() {
  const [view, setView] = useState<AddonsView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [feedback, setFeedback] = useState<Feedback>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [address, setAddress] = useState("");
  const [preview, setPreview] = useState<AddonPreview | null>(null);
  const [checking, setChecking] = useState(false);
  const [consent, setConsent] = useState(false);

  useEffect(() => {
    api.addonsList().then(setView, (e) => setError(errorText(e)));
  }, []);

  const run = async (key: string | null, busy: string, action: () => Promise<AddonsView>, done: string | ((v: AddonsView) => string)): Promise<boolean> => {
    setBusyKey(key);
    setFeedback({ kind: "busy", text: busy });
    try {
      const v = await action();
      setView(v);
      setFeedback({ kind: "ok", text: typeof done === "string" ? done : done(v) });
      return true;
    } catch (e) {
      setFeedback({ kind: "error", text: errorText(e) });
      return false;
    } finally {
      setBusyKey(null);
    }
  };

  const check = async () => {
    setPreview(null);
    setConsent(false);
    setChecking(true);
    setFeedback({ kind: "busy", text: t("Validando a URL do manifesto e carregando detalhes do addon…") });
    try {
      const p = await api.addonPreview(address);
      setPreview(p);
      setFeedback(null);
    } catch (e) {
      setFeedback({ kind: "error", text: t("Falha na instalação: {error}", { error: errorText(e) }) });
    } finally {
      setChecking(false);
    }
  };

  const install = async () => {
    if (!preview) return;
    const name = preview.addon.name;
    if (await run(null, t("Instalando…"), () => api.addonInstall(preview.key, consent), t("{name} foi validado e adicionado com sucesso.", { name }))) {
      setPreview(null);
      setAddress("");
    }
  };

  const remove = (a: AddonView) => {
    if (!window.confirm(t("Excluir addon?\n\n“{name}” será removido deste perfil. Favoritos, histórico e progresso de títulos vindos dele não são apagados.", { name: a.name }))) return;
    void run(a.key, t("Removendo…"), () => api.addonRemove(a.key), t("{name} removido.", { name: a.name }));
  };

  const configure = async (a: AddonView) => {
    try {
      await api.addonConfigure(a.key);
      setFeedback({ kind: "ok", text: t("Página de configuração de {name} aberta no navegador. Ao terminar, instale o novo endereço que ela fornecer.", { name: a.name }) });
    } catch (e) {
      setFeedback({ kind: "error", text: errorText(e) });
    }
  };

  if (error) return <div className="row-error">{t("Não foi possível ler os addons: {error}", { error })}</div>;
  if (!view) return <div className="muted">{t("Carregando addons…")}</div>;
  const active = view.addons.filter((a) => a.status === "ok").length;
  const catalogs = view.addons.filter((a) => a.enabled).reduce((n, a) => n + a.catalogCount, 0);
  const ro = view.readOnly;

  return (
    <div className="settings-group addons" data-testid="addons">
      <div className="addon-overview">
        <div><b>{view.addons.length}</b><span className="muted small">{t("Addons")}</span></div>
        <div><b>{active}</b><span className="muted small">{t("Ativos")}</span></div>
        <div><b>{catalogs}</b><span className="muted small">{t("Catálogos")}</span></div>
      </div>

      {ro && <div className="banner">{t("Este perfil usa os addons do perfil principal. Para gerenciá-los, entre no perfil principal ou desative “Usar addons do perfil principal” na edição do perfil.")}</div>}

      {!ro && (
        <section className="addon-install">
          <h2 className="settings-subtitle">{t("Adicionar addon")}</h2>
          <form className="addon-install-row" noValidate onSubmit={(e) => { e.preventDefault(); void check(); }}>
            <input
              // Text, not `url`: the browser would block addresses without scheme, which the
              // original accepts (https:// is added by Rust, like `normalizeManifestUrl`).
              type="text"
              inputMode="url"
              placeholder={t("URL do addon (manifest.json)")}
              aria-label={t("URL do addon")}
              data-testid="addon-url"
              value={address}
              onChange={(e) => { setAddress(e.target.value); setPreview(null); }}
              spellCheck={false}
              autoComplete="off"
            />
            <button className="primary" type="submit" disabled={checking || !address.trim()} data-testid="addon-check">
              {checking ? t("Verificando…") : t("Verificar")}
            </button>
          </form>
          {preview && (
            <div className="addon-card preview" data-testid="addon-preview">
              <AddonLogo addon={preview.addon} />
              <div className="addon-main">
                <div className="addon-title">{preview.addon.name} <span className="muted small">{t("Versão {version}", { version: preview.addon.version })}</span></div>
                <div className="muted small">{preview.addon.host}{preview.addon.configured && ` · ${t("configurado")}`}</div>
                {preview.addon.description && <p className="addon-desc">{preview.addon.description}</p>}
                <Badges addon={preview.addon} />
                <Catalogs addon={preview.addon} />
                {preview.addon.configurationRequired && <div className="muted small">{t("Este addon pede configuração: depois de instalar, use “Configurar”.")}</div>}
                {preview.tracking && (
                  <label className="addon-consent">
                    <input type="checkbox" checked={consent} onChange={(e) => setConsent(e.target.checked)} data-testid="addon-consent" />
                    <span>
                      <b>{preview.addon.name}</b> {t("é um addon de rastreamento: ele recebe o que você reproduz, pausa e para (com a posição), o que entra ou sai da biblioteca e o que é marcado como assistido.")}{" "}
                      {t("Concordo em compartilhar minha atividade e biblioteca com este addon.")}
                    </span>
                  </label>
                )}
                <div className="addon-actions">
                  <button className="primary" onClick={() => void install()} disabled={preview.tracking && !consent} data-testid="addon-install">{t("Instalar addon")}</button>
                  <button className="chip" onClick={() => setPreview(null)}>{t("Cancelar")}</button>
                </div>
              </div>
            </div>
          )}
        </section>
      )}

      {feedback && (
        <div className={`addon-feedback ${feedback.kind}`} role="status" data-testid="addon-feedback">{feedback.text}</div>
      )}

      <h2 className="settings-subtitle">{t("Addons instalados")}</h2>
      {view.addons.length === 0 ? (
        <div className="muted" data-testid="addons-empty">{t("Nenhum addon instalado ainda. Adicione uma URL de manifesto para carregar catálogos, metadados e fontes.")}</div>
      ) : (
        <ul className="addon-list">
          {view.addons.map((a, i) => (
            <li key={a.key} className={`addon-card ${a.enabled ? "" : "off"}`} data-key={a.key} data-status={a.status}>
              <AddonLogo addon={a} />
              <div className="addon-main">
                <div className="addon-title">
                  {a.name} {a.version && <span className="muted small">{t("Versão {version}", { version: a.version })}</span>}
                </div>
                <div className="muted small">{a.host}{a.configured && ` · ${t("configurado")}`}</div>
                {a.description && <p className="addon-desc">{a.description}</p>}
                {a.error && <div className="row-error small">{message(a.error)}</div>}
                <Badges addon={a} />
                <Catalogs addon={a} />
              </div>
              <div className="addon-side">
                <button
                  role="switch"
                  aria-checked={a.enabled}
                  aria-label={`${a.enabled ? t("Desativar") : t("Ativar")} ${a.name}`}
                  className={`switch ${a.enabled ? "on" : ""}`}
                  disabled={ro || busyKey === a.key}
                  data-testid="addon-toggle"
                  onClick={() => void run(a.key, a.enabled ? t("Desativando…") : t("Ativando…"), () => api.addonSetEnabled(a.key, !a.enabled), t("{name} {state}.", { name: a.name, state: t(a.enabled ? "desativado" : "ativado") }))}
                >
                  <span className="knob" />
                </button>
                <div className="addon-buttons">
                  <button className="icon-btn" aria-label={t("Mover {name} para cima", { name: a.name })} title={t("Mover para cima")} disabled={ro || i === 0 || !!busyKey} onClick={() => void run(a.key, t("Reordenando…"), () => api.addonMove(a.key, -1), t("Ordem salva."))}>↑</button>
                  <button className="icon-btn" aria-label={t("Mover {name} para baixo", { name: a.name })} title={t("Mover para baixo")} disabled={ro || i === view.addons.length - 1 || !!busyKey} onClick={() => void run(a.key, t("Reordenando…"), () => api.addonMove(a.key, 1), t("Ordem salva."))}>↓</button>
                  <button
                    className="icon-btn"
                    aria-label={t("Atualizar {name}", { name: a.name })}
                    title={t("Atualizar addon")}
                    disabled={!a.enabled || busyKey === a.key}
                    data-testid="addon-refresh"
                    onClick={() => void run(a.key, t("Atualizando {name}…", { name: a.name }), () => api.addonRefresh(a.key), (v) => {
                      const now = v.addons.find((x) => x.key === a.key);
                      return now?.status === "ok" ? t("{name} atualizado (versão {version}).", { name: now.name, version: now.version }) : t("{name}: {error}", { name: a.name, error: now?.error ? message(now.error) : t("não foi possível atualizar") });
                    })}
                  >
                    ⟳
                  </button>
                  {a.configurable && (
                    <button className="icon-btn" aria-label={t("Configurar {name}", { name: a.name })} title={t("Configurar addon")} onClick={() => void configure(a)}>⚙</button>
                  )}
                  <button className="icon-btn danger" aria-label={t("Excluir {name}", { name: a.name })} title={t("Excluir addon")} disabled={ro || !!busyKey} data-testid="addon-remove" onClick={() => remove(a)}>🗑</button>
                </div>
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

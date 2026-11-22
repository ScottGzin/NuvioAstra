// Settings → Addons: the active profile's addons, as in the original's AddonsScreen (install from a
// manifest address, enable/disable, order, refresh, configure on the addon's page, delete with
// confirmation). Every action goes through Rust; the manifest addresses never reach the UI.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { translateBackendError, useI18n } from "../i18n";
import type { AddonPreview, AddonView, AddonsView } from "../types";
import { typeLabel } from "./PosterCard";

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
  const { t } = useI18n();
  return (
    <div className="chips addon-badges">
      {addon.status === "ok" && <span className="chip badge-ok">{t("addons.active")}</span>}
      {addon.status === "disabled" && <span className="chip">{t("addons.disabled")}</span>}
      {addon.status === "error" && <span className="chip badge-error">{t("addons.unavailable")}</span>}
      {addon.configurable && <span className="chip">{t("addons.configurable")}</span>}
      {addon.tracking && <span className="chip">{t("addons.tracking")}</span>}
      {addon.types.slice(0, 6).map((t) => <span key={t} className="chip muted">{typeLabel(t)}</span>)}
    </div>
  );
}

function Catalogs({ addon }: { addon: AddonView }) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  if (!addon.catalogCount) return null;
  return (
    <div className="addon-catalogs">
      <button className="link" onClick={() => setOpen(!open)} aria-expanded={open}>
        {t("addons.catalogsCount", { count: addon.catalogCount })} {open ? "▴" : "▾"}
      </button>
      {open && (
        addon.catalogs.length ? (
          <ul>{addon.catalogs.map((c, i) => <li key={`${c.contentType}-${c.name}-${i}`}>{c.name} <span className="muted small">· {typeLabel(c.contentType)}</span></li>)}</ul>
        ) : (
          <div className="muted small">{t("addons.enableToSeeCatalogs")}</div>
        )
      )}
    </div>
  );
}

export function AddonsManager() {
  const { t } = useI18n();
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
    setFeedback({ kind: "busy", text: t("addons.validating") });
    try {
      const p = await api.addonPreview(address);
      setPreview(p);
      setFeedback(null);
    } catch (e) {
      setFeedback({ kind: "error", text: t("addons.installFailed", { error: errorText(e) }) });
    } finally {
      setChecking(false);
    }
  };

  const install = async () => {
    if (!preview) return;
    const name = preview.addon.name;
    if (await run(null, t("addons.installing"), () => api.addonInstall(preview.key, consent), t("addons.installSuccess", { name }))) {
      setPreview(null);
      setAddress("");
    }
  };

  const remove = (a: AddonView) => {
    if (!window.confirm(t("addons.removeConfirm", { name: a.name }))) return;
    void run(a.key, t("addons.removing"), () => api.addonRemove(a.key), t("addons.removed", { name: a.name }));
  };

  const configure = async (a: AddonView) => {
    try {
      await api.addonConfigure(a.key);
      setFeedback({ kind: "ok", text: t("addons.configureOpened", { name: a.name }) });
    } catch (e) {
      setFeedback({ kind: "error", text: errorText(e) });
    }
  };

  if (error) return <div className="row-error">{t("addons.loadError", { error })}</div>;
  if (!view) return <div className="muted">{t("addons.loading")}</div>;
  const active = view.addons.filter((a) => a.status === "ok").length;
  const catalogs = view.addons.filter((a) => a.enabled).reduce((n, a) => n + a.catalogCount, 0);
  const ro = view.readOnly;

  return (
    <div className="settings-group addons" data-testid="addons">
      <div className="addon-overview">
        <div><b>{view.addons.length}</b><span className="muted small">{t("addons.title")}</span></div>
        <div><b>{active}</b><span className="muted small">{t("addons.activeCount")}</span></div>
        <div><b>{catalogs}</b><span className="muted small">{t("addons.catalogs")}</span></div>
      </div>

      {ro && <div className="banner">{t("addons.readOnly")}</div>}

      {!ro && (
        <section className="addon-install">
          <h2 className="settings-subtitle">{t("addons.add")}</h2>
          <form className="addon-install-row" noValidate onSubmit={(e) => { e.preventDefault(); void check(); }}>
            <input
              // Text, not `url`: the browser would block addresses without scheme, which the
              // original accepts (https:// is added by Rust, like `normalizeManifestUrl`).
              type="text"
              inputMode="url"
              placeholder={t("addons.urlPlaceholder")}
              aria-label={t("addons.urlAria")}
              data-testid="addon-url"
              value={address}
              onChange={(e) => { setAddress(e.target.value); setPreview(null); }}
              spellCheck={false}
              autoComplete="off"
            />
            <button className="primary" type="submit" disabled={checking || !address.trim()} data-testid="addon-check">
              {checking ? t("addons.checking") : t("addons.check")}
            </button>
          </form>
          {preview && (
            <div className="addon-card preview" data-testid="addon-preview">
              <AddonLogo addon={preview.addon} />
              <div className="addon-main">
                <div className="addon-title">{preview.addon.name} <span className="muted small">{t("addons.version", { version: preview.addon.version })}</span></div>
                <div className="muted small">{preview.addon.host}{preview.addon.configured && t("addons.configured")}</div>
                {preview.addon.description && <p className="addon-desc">{preview.addon.description}</p>}
                <Badges addon={preview.addon} />
                <Catalogs addon={preview.addon} />
                {preview.addon.configurationRequired && <div className="muted small">{t("addons.configurationRequired")}</div>}
                {preview.tracking && (
                  <label className="addon-consent">
                    <input type="checkbox" checked={consent} onChange={(e) => setConsent(e.target.checked)} data-testid="addon-consent" />
                    <span>{t("addons.trackingConsent", { name: preview.addon.name })}</span>
                  </label>
                )}
                <div className="addon-actions">
                  <button className="primary" onClick={() => void install()} disabled={preview.tracking && !consent} data-testid="addon-install">{t("addons.install")}</button>
                  <button className="chip" onClick={() => setPreview(null)}>{t("common.cancel")}</button>
                </div>
              </div>
            </div>
          )}
        </section>
      )}

      {feedback && (
        <div className={`addon-feedback ${feedback.kind}`} role="status" data-testid="addon-feedback">{feedback.text}</div>
      )}

      <h2 className="settings-subtitle">{t("addons.installed")}</h2>
      {view.addons.length === 0 ? (
        <div className="muted" data-testid="addons-empty">{t("addons.empty")}</div>
      ) : (
        <ul className="addon-list">
          {view.addons.map((a, i) => (
            <li key={a.key} className={`addon-card ${a.enabled ? "" : "off"}`} data-key={a.key} data-status={a.status}>
              <AddonLogo addon={a} />
              <div className="addon-main">
                <div className="addon-title">
                  {a.name} {a.version && <span className="muted small">{t("addons.version", { version: a.version })}</span>}
                </div>
                <div className="muted small">{a.host}{a.configured && t("addons.configured")}</div>
                {a.description && <p className="addon-desc">{a.description}</p>}
                {a.error && <div className="row-error small">{translateBackendError(a.error)}</div>}
                <Badges addon={a} />
                <Catalogs addon={a} />
              </div>
              <div className="addon-side">
                <button
                  role="switch"
                  aria-checked={a.enabled}
                  aria-label={a.enabled ? t("addons.disableAria", { name: a.name }) : t("addons.enableAria", { name: a.name })}
                  className={`switch ${a.enabled ? "on" : ""}`}
                  disabled={ro || busyKey === a.key}
                  data-testid="addon-toggle"
                  onClick={() => void run(a.key, a.enabled ? t("addons.disabling") : t("addons.enabling"), () => api.addonSetEnabled(a.key, !a.enabled), a.enabled ? t("addons.disabledDone", { name: a.name }) : t("addons.enabledDone", { name: a.name }))}
                >
                  <span className="knob" />
                </button>
                <div className="addon-buttons">
                  <button className="icon-btn" aria-label={t("addons.moveUpAria", { name: a.name })} title={t("addons.moveUp")} disabled={ro || i === 0 || !!busyKey} onClick={() => void run(a.key, t("addons.reordering"), () => api.addonMove(a.key, -1), t("addons.orderSaved"))}>↑</button>
                  <button className="icon-btn" aria-label={t("addons.moveDownAria", { name: a.name })} title={t("addons.moveDown")} disabled={ro || i === view.addons.length - 1 || !!busyKey} onClick={() => void run(a.key, t("addons.reordering"), () => api.addonMove(a.key, 1), t("addons.orderSaved"))}>↓</button>
                  <button
                    className="icon-btn"
                    aria-label={t("addons.refreshAria", { name: a.name })}
                    title={t("addons.refresh")}
                    disabled={!a.enabled || busyKey === a.key}
                    data-testid="addon-refresh"
                    onClick={() => void run(a.key, t("addons.refreshing", { name: a.name }), () => api.addonRefresh(a.key), (v) => {
                      const now = v.addons.find((x) => x.key === a.key);
                      return now?.status === "ok" ? t("addons.refreshSuccess", { name: now.name, version: now.version }) : t("addons.refreshFailed", { name: a.name, error: now?.error ?? t("addons.refreshUnknown") });
                    })}
                  >
                    ⟳
                  </button>
                  {a.configurable && (
                    <button className="icon-btn" aria-label={t("addons.configureAria", { name: a.name })} title={t("addons.configure")} onClick={() => void configure(a)}>⚙</button>
                  )}
                  <button className="icon-btn danger" aria-label={t("addons.removeAria", { name: a.name })} title={t("addons.remove")} disabled={ro || !!busyKey} data-testid="addon-remove" onClick={() => remove(a)}>🗑</button>
                </div>
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

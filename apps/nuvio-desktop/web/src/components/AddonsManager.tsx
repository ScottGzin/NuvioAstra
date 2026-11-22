// Settings → Addons: the active profile's addons, as in the original's AddonsScreen (install from a
// manifest address, enable/disable, order, refresh, configure on the addon's page, delete with
// confirmation). Every action goes through Rust; the manifest addresses never reach the UI.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
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
  return (
    <div className="chips addon-badges">
      {addon.status === "ok" && <span className="chip badge-ok">Active</span>}
      {addon.status === "disabled" && <span className="chip">Disabled</span>}
      {addon.status === "error" && <span className="chip badge-error">Unavailable</span>}
      {addon.configurable && <span className="chip">Configurable</span>}
      {addon.tracking && <span className="chip">Tracking</span>}
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
        {addon.catalogCount} catalog(s) {open ? "▴" : "▾"}
      </button>
      {open && (
        addon.catalogs.length ? (
          <ul>{addon.catalogs.map((c, i) => <li key={`${c.contentType}-${c.name}-${i}`}>{c.name} <span className="muted small">· {typeLabel(c.contentType)}</span></li>)}</ul>
        ) : (
          <div className="muted small">Enable the addon to see its catalog list.</div>
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
    setFeedback({ kind: "busy", text: "Validating the manifest URL and loading addon details…" });
    try {
      const p = await api.addonPreview(address);
      setPreview(p);
      setFeedback(null);
    } catch (e) {
      setFeedback({ kind: "error", text: `Install failed: ${errorText(e)}` });
    } finally {
      setChecking(false);
    }
  };

  const install = async () => {
    if (!preview) return;
    const name = preview.addon.name;
    if (await run(null, "Installing…", () => api.addonInstall(preview.key, consent), `${name} was validated and added successfully.`)) {
      setPreview(null);
      setAddress("");
    }
  };

  const remove = (a: AddonView) => {
    if (!window.confirm(`Delete addon?\n\n“${a.name}” will be removed from this profile. Favorites, history, and progress of titles from it are not deleted.`)) return;
    void run(a.key, "Removing…", () => api.addonRemove(a.key), `${a.name} removed.`);
  };

  const configure = async (a: AddonView) => {
    try {
      await api.addonConfigure(a.key);
      setFeedback({ kind: "ok", text: `${a.name}'s configuration page opened in the browser. When you are done, install the new address it gives you.` });
    } catch (e) {
      setFeedback({ kind: "error", text: errorText(e) });
    }
  };

  if (error) return <div className="row-error">Could not read the addons: {error}</div>;
  if (!view) return <div className="muted">Loading addons…</div>;
  const active = view.addons.filter((a) => a.status === "ok").length;
  const catalogs = view.addons.filter((a) => a.enabled).reduce((n, a) => n + a.catalogCount, 0);
  const ro = view.readOnly;

  return (
    <div className="settings-group addons" data-testid="addons">
      <div className="addon-overview">
        <div><b>{view.addons.length}</b><span className="muted small">Addons</span></div>
        <div><b>{active}</b><span className="muted small">Active</span></div>
        <div><b>{catalogs}</b><span className="muted small">Catalogs</span></div>
      </div>

      {ro && <div className="banner">This profile uses the main profile's addons. To manage them, sign in to the main profile or turn off “Use the main profile's addons” in the profile editor.</div>}

      {!ro && (
        <section className="addon-install">
          <h2 className="settings-subtitle">Add addon</h2>
          <form className="addon-install-row" noValidate onSubmit={(e) => { e.preventDefault(); void check(); }}>
            <input
              // Text, not `url`: the browser would block addresses without scheme, which the
              // original accepts (https:// is added by Rust, like `normalizeManifestUrl`).
              type="text"
              inputMode="url"
              placeholder="Addon URL (manifest.json)"
              aria-label="Addon URL"
              data-testid="addon-url"
              value={address}
              onChange={(e) => { setAddress(e.target.value); setPreview(null); }}
              spellCheck={false}
              autoComplete="off"
            />
            <button className="primary" type="submit" disabled={checking || !address.trim()} data-testid="addon-check">
              {checking ? "Checking…" : "Check"}
            </button>
          </form>
          {preview && (
            <div className="addon-card preview" data-testid="addon-preview">
              <AddonLogo addon={preview.addon} />
              <div className="addon-main">
                <div className="addon-title">{preview.addon.name} <span className="muted small">Version {preview.addon.version}</span></div>
                <div className="muted small">{preview.addon.host}{preview.addon.configured && " · configured"}</div>
                {preview.addon.description && <p className="addon-desc">{preview.addon.description}</p>}
                <Badges addon={preview.addon} />
                <Catalogs addon={preview.addon} />
                {preview.addon.configurationRequired && <div className="muted small">This addon requires configuration: after installing, use “Configure”.</div>}
                {preview.tracking && (
                  <label className="addon-consent">
                    <input type="checkbox" checked={consent} onChange={(e) => setConsent(e.target.checked)} data-testid="addon-consent" />
                    <span>
                      <b>{preview.addon.name}</b> is a tracking addon: it receives what you play, pause, and stop (with the position), what enters or leaves the library, and what is marked as watched.
                      <br />I agree to share my activity and library with this addon.
                    </span>
                  </label>
                )}
                <div className="addon-actions">
                  <button className="primary" onClick={() => void install()} disabled={preview.tracking && !consent} data-testid="addon-install">Install addon</button>
                  <button className="chip" onClick={() => setPreview(null)}>Cancel</button>
                </div>
              </div>
            </div>
          )}
        </section>
      )}

      {feedback && (
        <div className={`addon-feedback ${feedback.kind}`} role="status" data-testid="addon-feedback">{feedback.text}</div>
      )}

      <h2 className="settings-subtitle">Installed addons</h2>
      {view.addons.length === 0 ? (
        <div className="muted" data-testid="addons-empty">No addons installed yet. Add a manifest URL to load catalogs, metadata, and sources.</div>
      ) : (
        <ul className="addon-list">
          {view.addons.map((a, i) => (
            <li key={a.key} className={`addon-card ${a.enabled ? "" : "off"}`} data-key={a.key} data-status={a.status}>
              <AddonLogo addon={a} />
              <div className="addon-main">
                <div className="addon-title">
                  {a.name} {a.version && <span className="muted small">Version {a.version}</span>}
                </div>
                <div className="muted small">{a.host}{a.configured && " · configured"}</div>
                {a.description && <p className="addon-desc">{a.description}</p>}
                {a.error && <div className="row-error small">{a.error}</div>}
                <Badges addon={a} />
                <Catalogs addon={a} />
              </div>
              <div className="addon-side">
                <button
                  role="switch"
                  aria-checked={a.enabled}
                  aria-label={`${a.enabled ? "Disable" : "Enable"} ${a.name}`}
                  className={`switch ${a.enabled ? "on" : ""}`}
                  disabled={ro || busyKey === a.key}
                  data-testid="addon-toggle"
                  onClick={() => void run(a.key, a.enabled ? "Disabling…" : "Enabling…", () => api.addonSetEnabled(a.key, !a.enabled), `${a.name} ${a.enabled ? "disabled" : "enabled"}.`)}
                >
                  <span className="knob" />
                </button>
                <div className="addon-buttons">
                  <button className="icon-btn" aria-label={`Move ${a.name} up`} title="Move up" disabled={ro || i === 0 || !!busyKey} onClick={() => void run(a.key, "Reordering…", () => api.addonMove(a.key, -1), "Order saved.")}>↑</button>
                  <button className="icon-btn" aria-label={`Move ${a.name} down`} title="Move down" disabled={ro || i === view.addons.length - 1 || !!busyKey} onClick={() => void run(a.key, "Reordering…", () => api.addonMove(a.key, 1), "Order saved.")}>↓</button>
                  <button
                    className="icon-btn"
                    aria-label={`Refresh ${a.name}`}
                    title="Refresh addon"
                    disabled={!a.enabled || busyKey === a.key}
                    data-testid="addon-refresh"
                    onClick={() => void run(a.key, `Refreshing ${a.name}…`, () => api.addonRefresh(a.key), (v) => {
                      const now = v.addons.find((x) => x.key === a.key);
                      return now?.status === "ok" ? `${now.name} updated (version ${now.version}).` : `${a.name}: ${now?.error ?? "could not update"}`;
                    })}
                  >
                    ⟳
                  </button>
                  {a.configurable && (
                    <button className="icon-btn" aria-label={`Configure ${a.name}`} title="Configure addon" onClick={() => void configure(a)}>⚙</button>
                  )}
                  <button className="icon-btn danger" aria-label={`Delete ${a.name}`} title="Delete addon" disabled={ro || !!busyKey} data-testid="addon-remove" onClick={() => remove(a)}>🗑</button>
                </div>
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

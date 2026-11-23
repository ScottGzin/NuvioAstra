// Trakt/Simkl credentials. The original's client ids are private app registrations: they are
// entered locally and kept in a Rust-only document, never committed and never shown back to the UI.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { Row } from "./SettingControls";
import { t } from "../i18n";

interface View { traktConfigured: boolean; simklConfigured: boolean }
type Key = "trakt:clientId" | "trakt:clientSecret" | "simkl:clientId";

export function TrackingCredentialsPanel() {
  const [view, setView] = useState<View | null>(null);
  const [draft, setDraft] = useState<Record<Key, string>>({ "trakt:clientId": "", "trakt:clientSecret": "", "simkl:clientId": "" });
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.trackingCredentials().then(setView, (e) => setError(errorText(e)));
  }, []);

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

  return (
    <>
      <h3 className="settings-subhead">Trakt</h3>
      {view && <>
        {field("trakt:clientId", t("ID do cliente Trakt"), view.traktConfigured)}
        {field("trakt:clientSecret", t("Segredo do cliente Trakt"), view.traktConfigured)}
      </>}
      <h3 className="settings-subhead">Simkl</h3>
      {view && field("simkl:clientId", t("ID do cliente Simkl"), view.simklConfigured)}
      {error && <div className="row-error" role="alert">{error}</div>}
      <p className="muted small">{t("Registre seu próprio aplicativo no Trakt/Simkl e cole os valores aqui. Eles ficam só neste computador e não aparecem mais na interface.")}</p>
    </>
  );
}

// Jellyfin/Emby servers: connect with the user's own credentials, then browse a library with the
// same poster cards and paging as Home. Tokens never leave Rust.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { PosterCard } from "./PosterCard";
import type { MetaPreview, ServerConnection, ServerLibrary } from "../types";
import { t } from "../i18n";

export function ServersPanel() {
  const [connections, setConnections] = useState<ServerConnection[] | null>(null);
  const [provider, setProvider] = useState("jellyfin");
  const [address, setAddress] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [libraries, setLibraries] = useState<{ connection: ServerConnection; items: ServerLibrary[] } | null>(null);
  const [items, setItems] = useState<MetaPreview[]>([]);
  const [library, setLibrary] = useState<ServerLibrary | null>(null);
  const [nextSkip, setNextSkip] = useState<number | null>(null);

  const reload = () => api.serverList().then(setConnections, (e) => setError(errorText(e)));
  useEffect(() => { reload(); }, []);

  const add = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.serverAdd(provider, address, username, password);
      setAddress(""); setUsername(""); setPassword("");
      await reload();
    } catch (e) { setError(errorText(e)); }
    finally { setBusy(false); }
  };

  const openLibraries = async (connection: ServerConnection) => {
    setError(null);
    try { setLibraries({ connection, items: await api.serverLibraries(connection.id) }); }
    catch (e) { setError(errorText(e)); }
  };

  const openLibrary = async (connection: ServerConnection, selected: ServerLibrary) => {
    setLibrary(selected);
    setItems([]);
    setNextSkip(null);
    try {
      const page = await api.serverCatalog(connection.id, selected.id, 0);
      setItems(page.items);
      setNextSkip(page.nextSkip);
    } catch (e) { setError(errorText(e)); }
  };

  return (
    <div className="servers-panel">
      {error && <div className="row-error" role="alert">{error}</div>}
      <div className="settings-group">
        <div className="setting-title">{t("Conectar servidor")}</div>
        <div className="server-form">
          <select aria-label={t("Tipo de servidor")} value={provider} onChange={(e) => setProvider(e.target.value)}>
            <option value="jellyfin">Jellyfin</option>
            <option value="emby">Emby</option>
          </select>
          <input aria-label={t("Endereço do servidor")} placeholder={t("Endereço do servidor")} value={address} onChange={(e) => setAddress(e.target.value)} />
          <input aria-label={t("Usuário")} placeholder={t("Usuário")} value={username} onChange={(e) => setUsername(e.target.value)} />
          <input aria-label={t("Senha")} type="password" placeholder={t("Senha")} value={password} onChange={(e) => setPassword(e.target.value)} />
          <button className="primary" disabled={busy || !address.trim() || !username.trim() || !password} onClick={() => void add()}>{t("Conectar")}</button>
        </div>
        <p className="muted small">{t("As credenciais são enviadas somente ao seu servidor; o token fica no app e nunca aparece aqui.")}</p>
      </div>
      {connections?.map((connection) => (
        <div key={connection.id} className="settings-group server-row">
          <div>
            <div className="setting-title">{connection.name}</div>
            <div className="muted small">{connection.provider} · {connection.address}</div>
          </div>
          <button className="chip" onClick={() => void openLibraries(connection)}>{t("Bibliotecas")}</button>
          <button className="chip" aria-label={t("Remover")} onClick={() => void api.serverRemove(connection.id).then(setConnections)}>✕</button>
        </div>
      ))}
      {libraries && (
        <div className="settings-group">
          <div className="setting-title">{t("Bibliotecas de {server}", { server: libraries.connection.name })}</div>
          <div className="chip-select">
            {libraries.items.map((item) => (
              <button key={item.id} className={`chip ${library?.id === item.id ? "active" : ""}`} onClick={() => void openLibrary(libraries.connection, item)}>{item.name}</button>
            ))}
          </div>
          {library && (
            <>
              <div className="grid discover-grid">
                {items.map((item) => <PosterCard key={`${item.contentType}/${item.id}`} item={item} onOpen={() => {}} />)}
              </div>
              {nextSkip !== null && (
                <button className="ghost" onClick={() => void api.serverCatalog(libraries.connection.id, library.id, nextSkip).then((page) => {
                  setItems((current) => [...current, ...page.items]);
                  setNextSkip(page.nextSkip);
                })}>{t("Carregar mais")}</button>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}

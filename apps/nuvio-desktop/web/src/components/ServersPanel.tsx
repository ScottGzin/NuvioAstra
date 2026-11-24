// Media servers: Jellyfin/Emby (password), Plex (PIN) and Silo (password or device code). Browse a
// library with the same poster cards and paging as Home. Tokens never leave Rust.

import { useEffect, useRef, useState } from "react";
import { api, errorText } from "../api";
import { PosterCard } from "./PosterCard";
import type { MetaPreview, PlexPin, ServerConnection, ServerLibrary, SiloCode, SiloInfo } from "../types";
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
  const [pin, setPin] = useState<PlexPin | null>(null);
  const [siloInfo, setSiloInfo] = useState<SiloInfo | null>(null);
  const [siloCode, setSiloCode] = useState<SiloCode | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const poll = useRef<number>(0);

  const reload = () => api.serverList().then(setConnections, (e) => setError(errorText(e)));
  useEffect(() => {
    reload();
    return () => window.clearInterval(poll.current);
  }, []);

  const openUrl = (url: string) => void api.openUrl(url);

  const startPlex = async () => {
    setBusy(true); setError(null); setNotice(null);
    try {
      const started = await api.serverPlexPinStart();
      setPin(started);
      window.clearInterval(poll.current);
      poll.current = window.setInterval(async () => {
        try {
          const token = await api.serverPlexPinPoll(started.id);
          if (!token) return;
          window.clearInterval(poll.current);
          await api.serverPlexConnect(token);
          setPin(null);
          setNotice(t("Plex conectado."));
          await reload();
        } catch (e) { setError(errorText(e)); }
      }, 2000);
    } catch (e) { setError(errorText(e)); }
    finally { setBusy(false); }
  };

  const discoverSilo = async () => {
    setBusy(true); setError(null); setNotice(null); setSiloCode(null);
    try { setSiloInfo(await api.serverSiloDiscover(address)); }
    catch (e) { setError(errorText(e)); }
    finally { setBusy(false); }
  };

  const addSiloPassword = async () => {
    setBusy(true); setError(null);
    try {
      await api.serverSiloLogin(address, username, password);
      setSiloInfo(null); setAddress(""); setUsername(""); setPassword("");
      setNotice(t("Silo conectado."));
      await reload();
    } catch (e) { setError(errorText(e)); }
    finally { setBusy(false); }
  };

  const startSiloCode = async () => {
    setBusy(true); setError(null);
    try {
      const code = await api.serverSiloCodeStart(siloInfo?.address ?? address);
      setSiloCode(code);
      window.clearInterval(poll.current);
      const interval = Math.max(1000, code.interval * 1000);
      poll.current = window.setInterval(async () => {
        try {
          const connected = await api.serverSiloCodeConnect(siloInfo?.address ?? address, code.deviceCode);
          if (!connected) return;
          window.clearInterval(poll.current);
          setSiloCode(null); setSiloInfo(null);
          setNotice(t("Silo conectado."));
          await reload();
        } catch (e) { setError(errorText(e)); }
      }, interval);
    } catch (e) { setError(errorText(e)); }
    finally { setBusy(false); }
  };

  const play = async (item: MetaPreview) => {
    const parts = item.id.split(":");
    if (parts[0] !== "srv1" || !libraries) return;
    try {
      const key = await api.serverStream(libraries.connection.id, parts.slice(2).join(":"));
      await api.playStream(key, null, null);
    } catch (e) { setError(errorText(e)); }
  };

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
          <select aria-label={t("Tipo de servidor")} value={provider} onChange={(e) => { setProvider(e.target.value); setPin(null); setSiloInfo(null); setSiloCode(null); setNotice(null); }}>
            <option value="jellyfin">Jellyfin</option>
            <option value="emby">Emby</option>
            <option value="plex">Plex</option>
            <option value="silo">Silo</option>
          </select>
          {provider !== "plex" && (
            <input aria-label={t("Endereço do servidor")} placeholder={t("Endereço do servidor")} value={address} onChange={(e) => setAddress(e.target.value)} />
          )}
          {(provider === "jellyfin" || provider === "emby" || provider === "silo") && (
            <>
              <input aria-label={t("Usuário")} placeholder={t("Usuário")} value={username} onChange={(e) => setUsername(e.target.value)} />
              <input aria-label={t("Senha")} type="password" placeholder={t("Senha")} value={password} onChange={(e) => setPassword(e.target.value)} />
            </>
          )}
          {provider === "plex" ? (
            <button className="primary" disabled={busy} onClick={() => void startPlex()}>{t("Entrar com Plex")}</button>
          ) : provider === "silo" ? (
            <>
              <button className="primary" disabled={busy || !address.trim()} onClick={() => void discoverSilo()}>{t("Verificar servidor")}</button>
              {siloInfo?.passwordLogin && (
                <button className="primary" disabled={busy || !username.trim() || !password} onClick={() => void addSiloPassword()}>{t("Conectar com senha")}</button>
              )}
              {siloInfo?.deviceCode && (
                <button className="ghost" disabled={busy} onClick={() => void startSiloCode()}>{t("Entrar com código")}</button>
              )}
            </>
          ) : (
            <button className="primary" disabled={busy || !address.trim() || !username.trim() || !password} onClick={() => void add()}>{t("Conectar")}</button>
          )}
        </div>
        {notice && <p className="muted small" role="status">{notice}</p>}
        {pin && (
          <div className="settings-group">
            <div className="setting-title">{t("Código Plex")}</div>
            <p className="muted small">{t("Abra a página de autorização e digite este código, depois aguarde.")}</p>
            <code className="path">{pin.code}</code>
            <button className="ghost" onClick={() => openUrl(pin.authUrl)}>{t("Abrir autorização Plex")}</button>
          </div>
        )}
        {siloCode && (
          <div className="settings-group">
            <div className="setting-title">{t("Código Silo")}</div>
            <code className="path">{siloCode.userCode}</code>
            {siloCode.verificationUri && <button className="ghost" onClick={() => openUrl(siloCode.verificationUri)}>{t("Abrir autorização Silo")}</button>}
          </div>
        )}
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
                {items.map((item) => <PosterCard key={`${item.contentType}/${item.id}`} item={item} onOpen={() => void play(item)} />)}
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

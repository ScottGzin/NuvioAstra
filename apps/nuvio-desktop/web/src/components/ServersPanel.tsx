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
    return () => window.clearTimeout(poll.current);
  }, []);

  const openUrl = (url: string) => void api.openUrl(url);

  const startPlex = async () => {
    setBusy(true); setError(null); setNotice(null);
    try {
      const started = await api.serverPlexPinStart();
      setPin(started);
      // The original opens plex.tv right away: the user approves in the browser while we poll.
      openUrl(started.authUrl);
      const startedAt = Date.now();
      let backoff = 1000;
      const pollPin = async () => {
        if (Date.now() - startedAt > 120_000) {
          setPin(null);
          setError(t("A autorização Plex expirou. Tente novamente."));
          return;
        }
        try {
          const token = await api.serverPlexPinPoll(started.id);
          if (token) {
            await api.serverPlexConnect(token);
            setPin(null);
            setNotice(t("Plex conectado."));
            await reload();
            return;
          }
        } catch {
          // A dropped probe is inconclusive: the browser sign-in may still be in progress.
        }
        poll.current = window.setTimeout(() => void pollPin(), backoff);
        backoff = Math.min(backoff * 2, 5000);
      };
      poll.current = window.setTimeout(() => void pollPin(), backoff);
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
      window.clearTimeout(poll.current);
      const interval = Math.max(1000, code.interval * 1000);
      const pollSilo = async () => {
        try {
          const connected = await api.serverSiloCodeConnect(siloInfo?.address ?? address, code.deviceCode);
          if (connected) {
            setSiloCode(null); setSiloInfo(null);
            setNotice(t("Silo conectado."));
            await reload();
            return;
          }
        } catch { /* keep polling: the code may not be approved yet */ }
        poll.current = window.setTimeout(() => void pollSilo(), interval);
      };
      poll.current = window.setTimeout(() => void pollSilo(), interval);
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
      <div className="settings-group servers-connect">
        <div className="setting-title">{t("Conectar servidor")}</div>
        <div className="server-providers">
          {([
            ["jellyfin", "Jellyfin", "◆"],
            ["emby", "Emby", "▶"],
            ["plex", "Plex", "›"],
            ["silo", "Silo", "▤"],
          ] as const).map(([id, label, glyph]) => (
            <button
              key={id}
              className={`server-provider ${id} ${provider === id ? "active" : ""}`}
              data-provider={id}
              aria-pressed={provider === id}
              onClick={() => { setProvider(id); setPin(null); setSiloInfo(null); setSiloCode(null); setNotice(null); }}
            >
              <span className="server-provider-icon" aria-hidden>{glyph}</span>
              <span>{label}</span>
            </button>
          ))}
        </div>
        {provider === "plex" ? (
          <div className="server-plex">
            <p className="muted small">{t("Entre no plex.tv no navegador. Sua senha do Plex nunca é digitada aqui.")}</p>
            <button className="server-primary plex" disabled={busy} onClick={() => void startPlex()}>
              <span className="plex-chevron" aria-hidden>›</span> Continue with Plex
            </button>
            {pin && (
              <div className="server-pin">
                <span className="setting-title">{t("Código Plex")}</span>
                <code className="path">{pin.code}</code>
                <button className="ghost" onClick={() => openUrl(pin.authUrl)}>{t("Abrir autorização Plex")}</button>
                <p className="muted small">{t("Aguardando autorização no navegador…")}</p>
              </div>
            )}
          </div>
        ) : (
          <div className="server-form">
            <input aria-label={t("Endereço do servidor")} placeholder={t("Endereço do servidor")} value={address} onChange={(e) => setAddress(e.target.value)} />
            {(provider === "jellyfin" || provider === "emby" || provider === "silo") && (
              <>
                <input aria-label={t("Usuário")} placeholder={t("Usuário")} value={username} onChange={(e) => setUsername(e.target.value)} />
                <input aria-label={t("Senha")} type="password" placeholder={t("Senha")} value={password} onChange={(e) => setPassword(e.target.value)} />
              </>
            )}
            {provider === "silo" ? (
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
        )}
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

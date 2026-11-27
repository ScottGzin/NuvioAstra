// Live TV → "Listas M3U": add a playlist (an address or a .m3u file on this computer) or remove
// one. Its channels show up with the addons' ones, under the playlist's name. The addresses are
// kept encrypted by Rust; only the site's name comes back here.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import type { M3uPlaylist } from "../types";
import { t } from "../i18n";

export function M3uDialog({ onClose, onChanged }: { onClose(): void; onChanged(): void }) {
  const [list, setList] = useState<M3uPlaylist[] | null>(null);
  const [name, setName] = useState("");
  const [source, setSource] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.m3uList().then(setList, (e) => setError(errorText(e)));
  }, []);

  const add = async () => {
    setBusy(true);
    setError(null);
    try {
      setList(await api.m3uAdd(name, source));
      setName("");
      setSource("");
      onChanged();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (p: M3uPlaylist) => {
    if (!window.confirm(t("Remover a lista “{name}”?", { name: p.name }))) return;
    try {
      setList(await api.m3uRemove(p.id));
      onChanged();
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(e) => e.target === e.currentTarget && !busy && onClose()}>
      <div className="modal m3u-modal" role="dialog" aria-modal="true" aria-label={t("Listas M3U")} data-testid="m3u-dialog">
        <header className="modal-header">
          <h2>{t("Listas M3U")}</h2>
          <button className="icon-btn" aria-label={t("Fechar")} onClick={onClose} disabled={busy}>✕</button>
        </header>
        <p className="muted small">{t("Adicione uma lista de canais (M3U/M3U8) pelo endereço ou pelo caminho de um arquivo neste computador. Os canais aparecem junto com os dos addons, com o nome da lista. O endereço fica guardado criptografado.")}</p>
        <div className="m3u-list">
          {list === null && !error && <div className="muted small">{t("Carregando…")}</div>}
          {list?.length === 0 && <div className="muted small">{t("Nenhuma lista ainda.")}</div>}
          {list?.map((p) => (
            <div key={p.id} className="m3u-item" data-playlist={p.id}>
              <div className="m3u-text">
                <div className="setting-title">{p.name}</div>
                <div className={`small ${p.error ? "error-text" : "muted"}`}>
                  {p.origin}
                  {p.channels != null && ` · ${t("{n} canais", { n: p.channels })}`}
                  {p.error && ` · ${t(p.error)}`}
                </div>
              </div>
              <button className="ghost" onClick={() => void remove(p)} disabled={busy} data-testid="m3u-remove">{t("Remover")}</button>
            </div>
          ))}
        </div>
        <div className="m3u-form torbox-key">
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder={t("Nome (opcional)")} aria-label={t("Nome da lista")} data-testid="m3u-name" />
          <input
            value={source}
            onChange={(e) => setSource(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && source.trim() && !busy && void add()}
            placeholder={t("https://… ou C:\\…\\lista.m3u")}
            aria-label={t("Endereço ou arquivo da lista")}
            spellCheck={false}
            data-testid="m3u-source"
          />
          <button className="primary" onClick={() => void add()} disabled={!source.trim() || busy} data-testid="m3u-add">{busy ? t("Lendo a lista…") : t("Adicionar")}</button>
        </div>
        {error && <div className="row-error" role="alert">{t(error)}</div>}
      </div>
    </div>
  );
}

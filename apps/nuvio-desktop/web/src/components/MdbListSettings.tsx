// Settings → Integrações → MDBList (`MdbListSettingsPage` of the original): the user's free API key
// (kept encrypted by Rust, never shown again) and which ratings the details page shows.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { Row, Toggle } from "./SettingControls";
import type { MdbListSettings, MdbListStatus } from "../types";
import { t } from "../i18n";

export const RATING_LABELS: Record<string, string> = {
  imdb: "IMDb",
  tmdb: "TMDB",
  tomatoes: "Rotten Tomatoes",
  audience: "Rotten Tomatoes (público)",
  metacritic: "Metacritic",
  trakt: "Trakt",
  letterboxd: "Letterboxd",
  mal: "MyAnimeList",
};

const SOURCES: [keyof MdbListSettings, string][] = [
  ["useImdb", "imdb"],
  ["useTmdb", "tmdb"],
  ["useTomatoes", "tomatoes"],
  ["useAudience", "audience"],
  ["useMetacritic", "metacritic"],
  ["useTrakt", "trakt"],
  ["useLetterboxd", "letterboxd"],
  ["useMal", "mal"],
];

export function MdbListSection({ settings, onChange }: { settings: MdbListSettings; onChange(change: Partial<MdbListSettings>): void }) {
  const [status, setStatus] = useState<MdbListStatus | null>(null);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    api.mdblistStatus().then(setStatus, (e) => setError(errorText(e)));
  }, []);

  const save = async () => {
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      setStatus(await api.mdblistSaveKey(key));
      setKey("");
      setSaved(true);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!window.confirm(t("Remover a chave do MDBList deste computador?"))) return;
    setError(null);
    setSaved(false);
    try {
      setStatus(await api.mdblistRemoveKey());
    } catch (e) {
      setError(errorText(e));
    }
  };

  const change = (c: Partial<MdbListSettings>) => {
    onChange(c);
    api.mdblistChanged();
  };

  return (
    <div className="settings-group" data-testid="mdblist-settings">
      <p className="muted">
        {t("O MDBList mostra na tela de detalhes as notas de vários sites ao mesmo tempo: IMDb, Rotten Tomatoes, Metacritic, Letterboxd e outros. Basta a sua chave gratuita do MDBList.")}
      </p>
      <Row
        title={t("Chave do MDBList")}
        description={status?.hasKey
          ? (status.username ? t("Chave da conta {name}, salva neste computador (guardada criptografada).", { name: status.username }) : t("Chave salva neste computador (guardada criptografada)."))
          : t("Entre no mdblist.com, abra Preferências e copie a “API key”.")}
      >
        <div className="torbox-key">
          {!status?.hasKey && <button className="ghost" onClick={() => void api.mdblistOpenKeyPage().catch((e) => setError(errorText(e)))} data-testid="mdblist-key-page">{t("Pegar minha chave")}</button>}
          <input
            type="password"
            autoComplete="off"
            spellCheck={false}
            placeholder={status?.hasKey ? t("Trocar a chave…") : t("Cole a chave aqui")}
            value={key}
            onChange={(e) => { setKey(e.target.value); setSaved(false); }}
            onKeyDown={(e) => e.key === "Enter" && key.trim() && !busy && void save()}
            aria-label={t("Chave do MDBList")}
            data-testid="mdblist-key"
          />
          <button className="primary" disabled={!key.trim() || busy} onClick={() => void save()} data-testid="mdblist-save">{busy ? t("Verificando…") : t("Salvar")}</button>
          {status?.hasKey && <button className="ghost" onClick={() => void remove()} data-testid="mdblist-remove">{t("Remover")}</button>}
        </div>
      </Row>
      {error && <div className="row-error" role="alert">{t(error)}</div>}
      {saved && <div className="muted small" role="status" data-testid="mdblist-saved">{t("Chave verificada e salva.")}</div>}

      <Row title={t("Mostrar as notas")} description={t("Na tela de detalhes, abaixo do título.")}>
        <Toggle label={t("Mostrar as notas do MDBList")} checked={settings.enabled} onChange={(v) => change({ enabled: v })} />
      </Row>
      <h3 className="settings-subtitle">{t("Quais notas")}</h3>
      {SOURCES.map(([field, source]) => (
        <Row key={field} title={t(RATING_LABELS[source])}>
          <Toggle label={t(RATING_LABELS[source])} checked={Boolean(settings[field])} onChange={(v) => change({ [field]: v } as Partial<MdbListSettings>)} />
        </Row>
      ))}
    </div>
  );
}

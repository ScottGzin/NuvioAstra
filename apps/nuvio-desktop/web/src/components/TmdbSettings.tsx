// Settings → TMDB (`TmdbSettingsPage` of the original): the user's own key (kept encrypted by Rust,
// never shown again), the language and which parts of the details page come from TMDB.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { Row, Toggle } from "./SettingControls";
import type { TmdbSettings, TmdbStatus } from "../types";
import { t } from "../i18n";

const LANGUAGES: [string, string][] = [
  ["", "Mesmo idioma do app"],
  ["pt-BR", "Português (Brasil)"],
  ["pt-PT", "Português (Portugal)"],
  ["en-US", "English"],
  ["es-ES", "Español (España)"],
  ["es-MX", "Español (Latinoamérica)"],
  ["fr-FR", "Français"],
  ["de-DE", "Deutsch"],
  ["it-IT", "Italiano"],
  ["ja-JP", "日本語"],
  ["ko-KR", "한국어"],
  ["zh-CN", "中文"],
  ["ar-SA", "العربية"],
];

const PARTS: [keyof TmdbSettings, string, string][] = [
  ["useArtwork", "Imagens", "Fundo, pôster e logo do título."],
  ["useBasicInfo", "Título, sinopse e gêneros", "No idioma escolhido, quando o TMDB tiver."],
  ["useDetails", "Detalhes", "Classificação indicativa, situação, duração, país e idioma."],
  ["useCredits", "Elenco e equipe", "Com fotos; clique numa pessoa para ver os trabalhos dela."],
  ["useProductions", "Estúdios", "Produtoras do título, com a página de cada uma."],
  ["useNetworks", "Canais", "Emissoras das séries, com a página de cada uma."],
  ["useEpisodes", "Episódios", "Título, sinopse e imagem de cada episódio."],
  ["useSeasonPosters", "Pôster das temporadas", "Mostrado ao escolher a temporada."],
  ["useMoreLikeThis", "Títulos semelhantes", "A fileira “Mais como este”."],
  ["useCollections", "Coleções", "A saga do filme e as pastas de coleções com fontes do TMDB."],
  ["useTrailers", "Trailers", "Os vídeos do TMDB para o botão de trailer."],
];

export function TmdbSection({ settings, onChange }: { settings: TmdbSettings; onChange(change: Partial<TmdbSettings>): void }) {
  const [status, setStatus] = useState<TmdbStatus | null>(null);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    api.tmdbStatus().then(setStatus, (e) => setError(errorText(e)));
  }, [settings.language, settings.enabled]);

  const save = async () => {
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      setStatus(await api.tmdbSaveKey(key));
      setKey("");
      setSaved(true);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!window.confirm(t("Remover a chave do TMDB deste computador?"))) return;
    setError(null);
    setSaved(false);
    try {
      setStatus(await api.tmdbRemoveKey());
    } catch (e) {
      setError(errorText(e));
    }
  };

  const change = (c: Partial<TmdbSettings>) => {
    onChange(c);
    api.tmdbChanged();
  };

  const hasKey = status !== null && status.keySource !== "none";
  return (
    <div className="settings-group" data-testid="tmdb-settings">
      <p className="muted">
        {t("Com uma chave do TMDB, a tela de detalhes ganha imagens, elenco com fotos, episódios, títulos semelhantes, a saga do filme e páginas de atores, estúdios e canais. A chave é gratuita: crie uma conta em themoviedb.org e copie a “Chave da API” ou o “Token de leitura” em Configurações → API.")}
      </p>
      <Row
        title={t("Chave do TMDB")}
        description={
          status?.keySource === "astra"
            ? t("Chave salva neste computador (guardada criptografada).")
            : status?.keySource === "original"
              ? t("Usando a chave do Nuvio original deste perfil. Salve uma aqui para usar a sua.")
              : t("Nenhuma chave. Os recursos do TMDB ficam desligados.")
        }
      >
        <div className="torbox-key">
          <input
            type="password"
            autoComplete="off"
            spellCheck={false}
            placeholder={hasKey ? t("Trocar a chave…") : t("Cole a chave aqui")}
            value={key}
            onChange={(e) => { setKey(e.target.value); setSaved(false); }}
            onKeyDown={(e) => e.key === "Enter" && key.trim() && !busy && void save()}
            aria-label={t("Chave do TMDB")}
            data-testid="tmdb-key"
          />
          <button className="primary" disabled={!key.trim() || busy} onClick={() => void save()} data-testid="tmdb-save">
            {busy ? t("Verificando…") : t("Salvar")}
          </button>
          {status?.keySource === "astra" && <button className="ghost" onClick={() => void remove()} data-testid="tmdb-remove">{t("Remover")}</button>}
        </div>
      </Row>
      {error && <div className="row-error" role="alert">{t(error)}</div>}
      {saved && <div className="muted small" role="status" data-testid="tmdb-saved">{t("Chave verificada e salva.")}</div>}

      <Row title={t("Usar o TMDB")} description={t("Desligado, o app usa só o que os addons informam.")}>
        <Toggle label={t("Usar o TMDB")} checked={settings.enabled} onChange={(v) => change({ enabled: v })} />
      </Row>
      <Row title={t("Idioma")} description={status ? t("Em uso: {lang}", { lang: status.language }) : undefined}>
        <select aria-label={t("Idioma do TMDB")} value={settings.language} onChange={(e) => change({ language: e.target.value })} data-setting="tmdbLanguage">
          {LANGUAGES.map(([code, label]) => <option key={code} value={code}>{code ? label : t(label)}</option>)}
        </select>
      </Row>
      <h3 className="settings-subtitle">{t("O que vem do TMDB")}</h3>
      {PARTS.map(([field, title, description]) => (
        <Row key={field} title={t(title)} description={t(description)}>
          <Toggle label={t(title)} checked={Boolean(settings[field])} onChange={(v) => change({ [field]: v } as Partial<TmdbSettings>)} />
        </Row>
      ))}
    </div>
  );
}

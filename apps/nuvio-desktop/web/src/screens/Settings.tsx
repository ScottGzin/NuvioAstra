// Settings of the active profile. Only options that NextGen applies for real are shown; each one
// is saved through Rust (`settings_set`, validated into the original's ranges) and read back.
// The original's other settings pages are listed under t("Ainda não disponível") (no controls).

import { ReleaseNotes } from "../components/ReleaseNotes";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import {
  AccountLine as AccountIcon, AddonsLine as AddonsIcon, AudioLine as AudioIcon, HomeLine as HomeIcon, P2pLine as P2pIcon, TvLine as DetailsIcon,
  PendingLine as PendingIcon, PlaybackLine as PlaybackIcon, ProfileDataLine as ProfileDataIcon, SubtitlesLine as SubtitlesIcon, UpdatesLine as UpdatesIcon,
  ServicesLine as ServicesIcon,
} from "../components/SidebarIcons";
import { api, errorText, onRemoteSettingsChanged } from "../api";
import { AccountPanel } from "../components/AccountPanel";
import { AddonsManager } from "../components/AddonsManager";
import { PluginsManager } from "../components/PluginsManager";
import { checkForUpdates, installUpdate, useUpdates } from "../updates";
import { Avatar } from "../components/ProfileMenu";
import { LANGUAGES, languageLabel } from "../languages";
import { P2P_CONSENT_TEXT, megabytes } from "../components/TorrentStart";
import { ANIME_MODES, ENHANCEMENTS, MOTIONS } from "../player/EnhanceMenu";
import { Row, Toggle } from "../components/SettingControls";
import { SubtitleAppearance } from "../components/SubtitleAppearance";
import { HeroCatalogs } from "../components/HeroCatalogs";
import { AudioSettings } from "../components/AudioSettings";
import { BufferSettings } from "../components/BufferSettings";
import { CollectionsManager } from "../components/CollectionsManager";
import { ServersPanel } from "../components/ServersPanel";
import { PosterCardStyle } from "../components/PosterCardStyle";
import { SourceSettings } from "../components/SourceSettings";
import { StreamsSettingsPanel } from "../components/StreamsSettingsPanel";
import { TrackingCredentialsPanel } from "../components/TrackingCredentialsPanel";
import { WatchingSettings } from "../components/WatchingSettings";
import { refreshLibraryViews } from "../library";
import type { CatalogChoice, ContinueWatchingSettings, DebridSettingsInput, DebridSettingsView, MdblistSettingsInput, MdblistSettingsView, MetaScreenSettings, P2pCacheName, P2pProfileName, P2pSettings, PlayerSettings, ProfileSettings, AnimeModeName, EnhancementName, MotionName, ResizeModeName, SettingsView, SubtitleSettings, TmdbSettingsInput, TmdbSettingsView } from "../types";
import { LANGUAGES_UI, addonLabel, localizeCatalogTitle, locale, setLang, t, useLang, type Lang } from "../i18n";
import { message } from "../rustErrors";

type Section = "account" | "home" | "layout" | "details" | "collections" | "sources" | "addons" | "playback" | "p2p" | "audio" | "subtitles" | "services" | "data" | "updates" | "pending";

const SECTIONS: { id: Section; label: string; icon: ReactNode }[] = [
  { id: "account", label: "Conta", icon: <AccountIcon /> },
  { id: "home", label: "Início", icon: <HomeIcon /> },
  { id: "layout", label: "Layout", icon: <HomeIcon /> },
  { id: "details", label: "Detalhes", icon: <DetailsIcon /> },
  { id: "collections", label: "Coleções", icon: <DetailsIcon /> },
  { id: "sources", label: "Fontes", icon: <AddonsIcon /> },
  { id: "addons", label: "Addons", icon: <AddonsIcon /> },
  { id: "playback", label: "Reprodução", icon: <PlaybackIcon /> },
  { id: "p2p", label: "Torrent / P2P", icon: <P2pIcon /> },
  { id: "audio", label: "Áudio", icon: <AudioIcon /> },
  { id: "subtitles", label: "Legendas", icon: <SubtitlesIcon /> },
  { id: "services", label: "Serviços", icon: <ServicesIcon /> },
  { id: "data", label: "Perfil e dados", icon: <ProfileDataIcon /> },
  { id: "updates", label: "Atualizações", icon: <UpdatesIcon /> },
  { id: "pending", label: "Ainda não disponível", icon: <PendingIcon /> },
];

/** Extra words that lead to a Settings section, so the search finds more than section titles. */
const SETTING_KEYWORDS: Partial<Record<Section, string>> = {
  home: "início catalogo catálogo destaque continuar assistindo gênero genre hero",
  details: "detalhes seções episódios elenco fundo pôster capítulos blur",
  sources: "fontes streams regex automático fallback reutilizar link youtube addons",
  addons: "addon instalar configurar manifest",
  playback: "reprodução vídeo imagem decodificação mpv próximo episódio buffer cache velocidade",
  p2p: "torrent p2p engine cache upload",
  audio: "áudio idioma som volume amplificação downmix canais atraso",
  subtitles: "legendas subtitle legenda estilo sdsh cor tamanho",
  services: "serviços tmdb mdblist debrid torbox discord trakt simkl integrações",
  data: "perfil dados conta sincronização conta avatares idioma",
  updates: "atualizações versão github",
  pending: "não disponível recursos futuros downloads",
};

type Status = { kind: "idle" } | { kind: "saving" } | { kind: "saved"; note?: string } | { kind: "error"; message: string };

// Languages offered by the TMDB page (the original accepts any BCP-47 tag; these are the usual ones).
const TMDB_LANGUAGES: [string, string][] = [
  ["en", "English"], ["en-US", "English (US)"], ["pt-BR", "Português (Brasil)"], ["ar", "العربية"],
  ["es-ES", "Español"], ["fr-FR", "Français"], ["de-DE", "Deutsch"], ["tr-TR", "Türkçe"],
  ["hi-IN", "हिन्दी"], ["ja-JP", "日本語"], ["ko-KR", "한국어"], ["zh-CN", "中文"],
];

// ---------- small controls ----------

function LanguageSelect({ value, onChange, special, label }: { value: string | null; onChange(v: string | null): void; special: [string, string][]; label: string }) {
  const lang = useLang();
  const known = LANGUAGES.some(([c]) => c.toLowerCase() === (value ?? "").toLowerCase()) || special.some(([c]) => c === (value ?? ""));
  return (
    <select aria-label={label} value={value ?? ""} onChange={(e) => onChange(e.target.value || null)}>
      {special.map(([code, name]) => <option key={code} value={code}>{name}</option>)}
      {!known && value && <option value={value}>{value}</option>}
      {LANGUAGES.map(([code, name]) => <option key={code} value={code}>{languageLabel(code, lang) || name}</option>)}
    </select>
  );
}

// ---------- page ----------

export function Settings() {
  const lang = useLang();
  const [view, setView] = useState<SettingsView | null>(null);
  const [draft, setDraft] = useState<ProfileSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [section, setSection] = useState<Section>("home");
  const [settingSearch, setSettingSearch] = useState("");
  const [layoutPage, setLayoutPage] = useState<"menu" | "poster" | "streams">("menu");
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  // Services → Integrations → TMDB (the only integration ported so far).
  const [servicePage, setServicePage] = useState<"menu" | "integrations" | "tracking" | "tmdb" | "debrid" | "mdblist" | "servers">("menu");
  const [tmdb, setTmdb] = useState<TmdbSettingsView | null>(null);
  const [tmdbError, setTmdbError] = useState<string | null>(null);
  const [tmdbStatus, setTmdbStatus] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [apiKeyDraft, setApiKeyDraft] = useState("");
  const [debrid, setDebrid] = useState<DebridSettingsView | null>(null);
  const [debridError, setDebridError] = useState<string | null>(null);
  const [debridStatus, setDebridStatus] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [debridDrafts, setDebridDrafts] = useState<Record<string, string>>({});
  const [mdblist, setMdblist] = useState<MdblistSettingsView | null>(null);
  const [mdblistError, setMdblistError] = useState<string | null>(null);
  const [mdblistStatus, setMdblistStatus] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [mdblistKeyDraft, setMdblistKeyDraft] = useState("");
  const timer = useRef<number>(0);
  const pending = useRef<{ next: ProfileSettings; note?: string } | null>(null);
  /** Latest settings, including changes not rendered yet (several quick changes in a row). */
  const draftRef = useRef<ProfileSettings | null>(null);
  /** What the draft was based on: only the difference from it is saved. */
  const baseRef = useRef<ProfileSettings | null>(null);
  const profileRef = useRef<number | undefined>(undefined);
  const saving = useRef(false);

  useEffect(() => {
    if (!view) return;
    const cards = view.settings.player.cards;
    if (cards) {
      document.documentElement.style.setProperty("--card-width", `${cards.width}px`);
      document.documentElement.style.setProperty("--card-radius", `${cards.radius}px`);
    }
  }, [view]);

  useEffect(() => {
    api.settingsGet().then(
      (v) => {
        setView(v);
        baseRef.current = v.settings;
        profileRef.current = v.profile.index;
        draftRef.current = v.settings;
        setDraft(v.settings);
      },
      (e) => setError(errorText(e)),
    );
  }, []);

  // TMDB settings of the profile, loaded with the Services page.
  useEffect(() => {
    if (section !== "services") return;
    let cancelled = false;
    setTmdbError(null);
    api.tmdbSettings().then(
      (value) => !cancelled && setTmdb(value),
      (e) => !cancelled && setTmdbError(errorText(e)),
    );
    api.debridSettings().then(
      (value) => !cancelled && setDebrid(value),
      (e) => !cancelled && setDebridError(errorText(e)),
    );
    api.mdblistSettings().then(
      (value) => !cancelled && setMdblist(value),
      (e) => !cancelled && setMdblistError(errorText(e)),
    );
    return () => {
      cancelled = true;
    };
  }, [section, view?.profile.index]);

  const saveTmdb = useCallback(async (change: Partial<TmdbSettingsInput>) => {
    if (!tmdb) return;
    setTmdbStatus("saving");
    try {
      const saved = await api.tmdbSettingsSet({
        enabled: tmdb.enabled,
        language: tmdb.language,
        useMoreLikeThis: tmdb.useMoreLikeThis,
        useCollections: tmdb.useCollections,
        useArtwork: tmdb.useArtwork,
        useBasicInfo: tmdb.useBasicInfo,
        useDetails: tmdb.useDetails,
        useCredits: tmdb.useCredits,
        useEpisodes: tmdb.useEpisodes,
        useTrailers: tmdb.useTrailers,
        ...change,
      });
      setTmdb(saved);
      setTmdbError(null);
      setTmdbStatus("saved");
      window.setTimeout(() => setTmdbStatus((current) => (current === "saved" ? "idle" : current)), 1600);
    } catch (e) {
      setTmdbStatus("error");
      setTmdbError(errorText(e));
    }
  }, [tmdb]);

  const saveDebrid = useCallback(async (change: Partial<DebridSettingsInput>) => {
    if (!debrid) return;
    setDebridStatus("saving");
    try {
      const saved = await api.debridSettingsSet({ enabled: debrid.enabled, ...change });
      setDebrid(saved);
      setDebridError(null);
      setDebridStatus("saved");
      window.setTimeout(() => setDebridStatus((current) => (current === "saved" ? "idle" : current)), 1600);
    } catch (e) {
      setDebridStatus("error");
      setDebridError(errorText(e));
    }
  }, [debrid]);

  const saveMdblist = useCallback(async (change: Partial<MdblistSettingsInput>) => {
    if (!mdblist) return;
    setMdblistStatus("saving");
    try {
      const saved = await api.mdblistSettingsSet({
        enabled: mdblist.enabled,
        useImdb: mdblist.useImdb,
        useTmdb: mdblist.useTmdb,
        useTomatoes: mdblist.useTomatoes,
        useMetacritic: mdblist.useMetacritic,
        useTrakt: mdblist.useTrakt,
        useLetterboxd: mdblist.useLetterboxd,
        useAudience: mdblist.useAudience,
        useMal: mdblist.useMal,
        ...change,
      });
      setMdblist(saved);
      setMdblistError(null);
      setMdblistStatus("saved");
      window.setTimeout(() => setMdblistStatus((current) => (current === "saved" ? "idle" : current)), 1600);
    } catch (e) {
      setMdblistStatus("error");
      setMdblistError(errorText(e));
    }
  }, [mdblist]);

  // The account sent something: shown at once. An edit still waiting keeps the screen as it is;
  // its save writes only that edit and returns both.
  useEffect(
    () =>
      onRemoteSettingsChanged(() => {
        api.settingsGet().then((v) => {
          setView(v);
          if (pending.current || saving.current) return;
          baseRef.current = v.settings;
          draftRef.current = v.settings;
          setDraft(v.settings);
        }, () => {});
      }),
    [],
  );

  const flush = useCallback(async () => {
    const job = pending.current;
    if (!job || saving.current) return; // a running save sends it when it ends
    pending.current = null;
    saving.current = true;
    setStatus({ kind: "saving" });
    let ok = false;
    try {
      const watchedBefore = baseRef.current?.player.watchedPercent;
      const saved = await api.settingsSet(job.next, baseRef.current, profileRef.current);
      if (saved.player.watchedPercent !== watchedBefore) refreshLibraryViews();
      ok = true;
      // Rust may have corrected a value or kept one the account sent: show what was stored, unless
      // the user changed something else meanwhile (that change is based on `job.next`, saved next).
      if (draftRef.current === job.next) {
        baseRef.current = saved;
        draftRef.current = saved;
        setDraft(saved);
      } else {
        baseRef.current = job.next;
      }
      if (!pending.current) setStatus({ kind: "saved", note: job.note });
    } catch (e) {
      setStatus({ kind: "error", message: errorText(e) });
    } finally {
      saving.current = false;
    }
    if (ok && pending.current) void flush();
  }, []);

  // Leaving the page (or the app tab) never drops a change still waiting for its debounce.
  useEffect(
    () => () => {
      window.clearTimeout(timer.current);
      void flush();
    },
    [flush],
  );

  /** Applies a change locally and saves it (debounced for sliders and typing). */
  const update = useCallback(
    (change: (s: ProfileSettings) => ProfileSettings, note?: string, delay = 300) => {
      const current = pending.current?.next ?? draftRef.current;
      if (!current) return;
      const next = change(current);
      draftRef.current = next;
      pending.current = { next, note };
      setDraft(next);
      setStatus({ kind: "saving" });
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => void flush(), delay);
    },
    [flush],
  );

  const player = (change: (p: PlayerSettings) => PlayerSettings) =>
    update((s) => ({ ...s, player: change(s.player) }), t("Vale a partir da próxima reprodução."));
  const subtitle = (change: (p: SubtitleSettings) => SubtitleSettings) => player((p) => ({ ...p, subtitle: change(p.subtitle) }));

  const resetSection = (what: Section) => {
    if (!view || !draft) return;
    const name = t(SECTIONS.find((s) => s.id === what)?.label ?? "");
    if (!window.confirm(t("Restaurar os padrões de “{name}” para o perfil {profile}? As alterações desta seção serão descartadas.", { name, profile: view.profile.name }))) return;
    const d = view.defaults;
    update((s) => {
      switch (what) {
        case "home": return { ...s, home: { ...d.home } };
        case "playback": return { ...s, player: { ...s.player, resizeMode: d.player.resizeMode, decoderPriority: d.player.decoderPriority, nvidiaRtxSuperResolution: d.player.nvidiaRtxSuperResolution, nvidiaRtxHdr: d.player.nvidiaRtxHdr, motionSmoothing: d.player.motionSmoothing, videoEnhancement: d.player.videoEnhancement, picture: { ...d.player.picture }, skipSegmentsEnabled: d.player.skipSegmentsEnabled, autoSkipSegments: [...d.player.autoSkipSegments], nextEpisodeThresholdMode: d.player.nextEpisodeThresholdMode, nextEpisodeThresholdPercent: d.player.nextEpisodeThresholdPercent, nextEpisodeThresholdMinutes: d.player.nextEpisodeThresholdMinutes, watchedPercent: d.player.watchedPercent } };
        case "audio": return { ...s, player: { ...s.player, preferredAudioLanguage: d.player.preferredAudioLanguage, secondaryPreferredAudioLanguage: d.player.secondaryPreferredAudioLanguage } };
        case "subtitles": return { ...s, player: { ...s.player, preferredSubtitleLanguage: d.player.preferredSubtitleLanguage, secondaryPreferredSubtitleLanguage: d.player.secondaryPreferredSubtitleLanguage, subtitle: { ...d.player.subtitle } } };
        default: return s;
      }
    }, t("Padrões restaurados."), 0);
  };

  if (error) return <div className="screen-message">{t("Não foi possível abrir as configurações:")} {error}</div>;
  if (!view || !draft) return <div className="screen-message">{t("Carregando configurações…")}</div>;
  const p = draft.player;
  const sub = p.subtitle;

  return (
    <div className="settings">
      <nav className="settings-nav" aria-label={t("Categorias de configurações")}>
        <div className="settings-profile">
          <Avatar profile={view.profile} />
          <div>
            <div className="setting-title">{view.profile.name}</div>
            <div className="muted small">{t("Configurações deste perfil")}</div>
          </div>
        </div>
        <input
          className="settings-search"
          type="search"
          aria-label={t("Buscar nas configurações")}
          placeholder={t("Buscar nas configurações")}
          data-testid="settings-search"
          value={settingSearch}
          onChange={(e) => setSettingSearch(e.target.value)}
        />
        {SECTIONS.filter((s) => {
          const q = settingSearch.trim().toLocaleLowerCase();
          if (!q) return true;
          const label = t(s.label).toLocaleLowerCase();
          const keywords = SETTING_KEYWORDS[s.id] ?? "";
          return label.includes(q) || keywords.includes(q);
        }).map((s) => (
          <button key={s.id} className={`nav-item ${section === s.id ? "active" : ""}`} data-section={s.id} onClick={() => { if (s.id === "services") setServicePage("menu"); setSection(s.id); }}>
            <span className="nav-icon" aria-hidden>{s.icon}</span>
            <span>{t(s.label)}</span>
          </button>
        ))}
      </nav>

      <div className="settings-body" key={section}>
        <header className="settings-header">
          <h1>{t(SECTIONS.find((s) => s.id === section)?.label ?? "")}</h1>
          <span className={`save-status ${status.kind}`} role="status" data-testid="save-status">
            {status.kind === "saving" && t("Salvando…")}
            {status.kind === "saved" && `${t("✓ Salvo")}${status.note ? ` · ${status.note}` : ""}`}
            {status.kind === "error" && t("Não foi possível salvar: {error}", { error: message(status.message) })}
          </span>
          {["home", "playback", "audio", "subtitles"].includes(section) && (
            <button className="ghost" onClick={() => resetSection(section)}>{t("Restaurar padrões")}</button>
          )}
        </header>

        {section === "home" && (
          <HomeSection settings={draft} catalogs={view.catalogs} update={update} />
        )}
        {section === "layout" && (layoutPage === "menu" ? (
          <div className="settings-group">
            <button className="integration-row" data-service="poster" onClick={() => setLayoutPage("poster")}>
              <span className="integration-icon link">▧</span>
              <span className="integration-text">
                <span className="setting-title">{t("Poster Card Style")}</span>
                <span className="muted small">{t("Tune card width and corner radius.")}</span>
              </span>
              <span className="integration-arrow" aria-hidden>›</span>
            </button>
            <button className="integration-row" data-service="streams" onClick={() => setLayoutPage("streams")}>
              <span className="integration-icon link">≡</span>
              <span className="integration-text">
                <span className="setting-title">{t("Streams")}</span>
                <span className="muted small">{t("Stream result display and badge URL rules.")}</span>
              </span>
              <span className="integration-arrow" aria-hidden>›</span>
            </button>
          </div>
        ) : layoutPage === "streams" ? (
          <>
            <div className="settings-subhead">
              <button className="ghost" onClick={() => setLayoutPage("menu")}>← {t("Layout")}</button>
              <h2>{t("Streams")}</h2>
            </div>
            <StreamsSettingsPanel />
          </>
        ) : (
          <>
            <div className="settings-subhead">
              <button className="ghost" onClick={() => setLayoutPage("menu")}>← {t("Layout")}</button>
              <h2>{t("Poster Card Style")}</h2>
            </div>
            {draft && <PosterCardStyle value={draft.player} onChange={(change) => update((s) => ({ ...s, player: change(s.player) }))} />}
          </>
        ))}

        {section === "details" && (
          <DetailsSection
            settings={draft.metaScreen}
            update={(change) => update((s) => ({ ...s, metaScreen: change(s.metaScreen) }), t("A tela de detalhes é atualizada."))}
          />
        )}

        {section === "account" && <AccountPanel />}
        {section === "collections" && <CollectionsManager />}
        {section === "sources" && (
          <div className="settings-group">
            <SourceSettings value={draft.player} addons={[...new Set(view.catalogs.map((c) => c.addonName))].sort()} onChange={player} />
          </div>
        )}
        {section === "addons" && <><AddonsManager /><PluginsManager /></>}

        {section === "playback" && (
          <div className="settings-group">
            <Row title={t("Modo de imagem padrão")} description={t("Como o vídeo ocupa a tela ao iniciar (pode ser trocado durante a reprodução).")}>
              <select aria-label={t("Modo de imagem padrão")} data-setting="resizeMode" value={p.resizeMode} onChange={(e) => player((x) => ({ ...x, resizeMode: e.target.value as ResizeModeName }))}>
                <option value="Fit">{t("Ajustar")}</option>
                <option value="Fill">{t("Preencher")}</option>
                <option value={t("Zoom")}>{t("Zoom")}</option>
                <option value="Stretch">{t("Esticar")}</option>
              </select>
            </Row>
            <Row title={t("Decodificação de vídeo")} description={t("Prioridade do decodificador (libmpv).")}>
              <select aria-label={t("Decodificação de vídeo")} data-setting="decoderPriority" value={p.decoderPriority} onChange={(e) => player((x) => ({ ...x, decoderPriority: Number(e.target.value) }))}>
                <option value={1}>{t("Hardware, com software como reserva (padrão)")}</option>
                <option value={0}>{t("Somente hardware")}</option>
                <option value={2}>{t("Somente software")}</option>
              </select>
            </Row>
            <Row title={t("Melhorar imagem")} description={t("Filmes e episódios. O Anime4K muda bastante em 480p e 720p e é sutil em 1080p numa tela 1080p. Se a placa não aguentar, o nível cai sozinho.")}>
              <select aria-label={t("Melhorar imagem")} data-setting="videoEnhancement" value={p.videoEnhancement} onChange={(e) => player((x) => ({ ...x, videoEnhancement: e.target.value as EnhancementName }))}>
                {ENHANCEMENTS.map((o) => <option key={o.value} value={o.value}>{t(o.label)}</option>)}
              </select>
            </Row>
            <Row title={t("Modo do Anime4K")} description={`${t(ANIME_MODES.find((m) => m.value === p.picture.mode)?.hint ?? "")} ${t("Efeitos de traço, nitidez e cores ficam no botão Melhorar imagem, dentro do player.")}`}>
              <select aria-label={t("Modo do Anime4K")} data-setting="animeMode" value={p.picture.mode} onChange={(e) => player((x) => ({ ...x, picture: { ...x.picture, mode: e.target.value as AnimeModeName } }))}>
                {ANIME_MODES.map((o) => <option key={o.value} value={o.value}>{t(o.label)}</option>)}
              </select>
            </Row>
            <Row title={t("Movimento do vídeo")} description={t("“Sem tremidas” ajusta os 24 quadros ao monitor. “Fluido” mistura quadros vizinhos e pode deixar rastros. Com qualquer um dos dois, a saída HDR (inclusive RTX Video HDR) vira SDR.")}>
              <select aria-label={t("Movimento do vídeo")} data-setting="motionSmoothing" value={p.motionSmoothing} onChange={(e) => player((x) => ({ ...x, motionSmoothing: e.target.value as MotionName }))}>
                {MOTIONS.map((o) => <option key={o.value} value={o.value}>{t(o.label)}</option>)}
              </select>
            </Row>
            <Row title={t("NVIDIA RTX Video Super Resolution")} description={t("Requer placa NVIDIA RTX e driver compatível; sem efeito em outras placas. Nos níveis Anime, o Anime4K fica no lugar dela.")}>
              <Toggle label={t("NVIDIA RTX Video Super Resolution")} checked={p.nvidiaRtxSuperResolution} onChange={(v) => player((x) => ({ ...x, nvidiaRtxSuperResolution: v }))} />
            </Row>
            <Row title="NVIDIA RTX Video HDR" description={t("Converte SDR em HDR em telas HDR com placa RTX.")}>
              <Toggle label="NVIDIA RTX Video HDR" checked={p.nvidiaRtxHdr} onChange={(v) => player((x) => ({ ...x, nvidiaRtxHdr: v }))} />
            </Row>
            <WatchingSettings value={p} onChange={player} />
            <BufferSettings value={p} onChange={player} />
          </div>
        )}

        {section === "p2p" && <P2pSection settings={draft.p2p} update={(change) => update((s) => ({ ...s, p2p: change(s.p2p) }), t("Vale a partir da próxima fonte torrent."), 0)} />}

        {section === "audio" && (
          <div className="settings-group">
            <Row title={t("Idioma de áudio preferido")} description={t("“Idioma do dispositivo” = {list} neste PC.", { list: view.deviceLanguages.join(", ") || t("não identificado") })}>
              <LanguageSelect label={t("Idioma de áudio preferido")} value={p.preferredAudioLanguage} special={[["device", t("Idioma do dispositivo")], ["default", t("Padrão do arquivo")], ["original", t("Idioma original")]]} onChange={(v) => player((x) => ({ ...x, preferredAudioLanguage: v ?? "device" }))} />
            </Row>
            <Row title={t("Idioma de áudio secundário")} description={t("Usado quando o preferido não existe no arquivo.")}>
              <LanguageSelect label={t("Idioma de áudio secundário")} value={p.secondaryPreferredAudioLanguage} special={[["", t("Nenhum")]]} onChange={(v) => player((x) => ({ ...x, secondaryPreferredAudioLanguage: v }))} />
            </Row>
            <AudioSettings value={p} onChange={player} />
            <p className="muted small pad-top">{t("“Idioma original” usa o idioma do dispositivo enquanto o Nuvio Astra não conhece o idioma original do título (mesma regra do Nuvio original quando essa informação falta).")}</p>
          </div>
        )}

        {section === "subtitles" && (
          <div className="settings-group">
            <Row title={t("Idioma de legenda preferido")}>
              <LanguageSelect label={t("Idioma de legenda preferido")} value={p.preferredSubtitleLanguage} special={[["none", t("Nenhuma (só forçadas)")], ["device", t("Idioma do dispositivo")], ["forced", t("Somente forçadas")]]} onChange={(v) => player((x) => ({ ...x, preferredSubtitleLanguage: v ?? "none" }))} />
            </Row>
            <Row title={t("Idioma de legenda secundário")}>
              <LanguageSelect label={t("Idioma de legenda secundário")} value={p.secondaryPreferredSubtitleLanguage} special={[["", t("Nenhum")]]} onChange={(v) => player((x) => ({ ...x, secondaryPreferredSubtitleLanguage: v }))} />
            </Row>
            <SubtitleAppearance value={sub} onChange={subtitle} />
            <Row title={t("Remover descrições para surdos (SDH)")} description={t("Oculta trechos como [música] e (risos).")}>
              <Toggle label={t("Remover SDH")} checked={sub.stripSdh} onChange={(v) => subtitle((s) => ({ ...s, stripSdh: v }))} />
            </Row>
            <Row title={t("Sincronização automática da legenda")} description={t("Compara a legenda escolhida com outras disponíveis e aplica o atraso encontrado. Sem chave, sem análise de áudio e só quando há correspondência confiável.")}>
              <Toggle label={t("Sincronizar ao escolher uma legenda")} checked={sub.autoSyncEnabled} onChange={(v) => subtitle((s) => ({ ...s, autoSyncEnabled: v }))} />
            </Row>
          </div>
        )}

        {section === "services" && (servicePage === "menu" ? (
          <div className="settings-group">
            <button className="integration-row" data-service="integrations" onClick={() => setServicePage("integrations")}>
              <span className="integration-icon link">↔</span>
              <span className="integration-text">
                <span className="setting-title">{t("Integrações")}</span>
                <span className="muted small">{t("Gerenciar integrações disponíveis")}</span>
              </span>
              <span className="integration-arrow" aria-hidden>›</span>
            </button>
            <button className="integration-row" data-service="tracking" onClick={() => setServicePage("tracking")}>
              <span className="integration-icon link">♻</span>
              <span className="integration-text">
                <span className="setting-title">{t("Rastreamento")}</span>
                <span className="muted small">{t("Conecte provedores de rastreamento e escolha qual alimenta sua Biblioteca e Continuar assistindo.")}</span>
              </span>
              <span className="integration-arrow" aria-hidden>›</span>
            </button>
          </div>
        ) : servicePage === "tracking" ? (
          <>
            <div className="settings-subhead">
              <button className="ghost" onClick={() => setServicePage("menu")}>← {t("Serviços")}</button>
              <h2>{t("Rastreamento")}</h2>
            </div>
            <TrackingCredentialsPanel settings={draft} update={update} />
          </>
        ) : servicePage === "integrations" ? (
          <>
          <div className="settings-subhead">
            <button className="ghost" data-testid="integrations-back" onClick={() => setServicePage("menu")}>← {t("Serviços")}</button>
            <h2>{t("Integrações")}</h2>
          </div>
          <div className="settings-group">
            <button className="integration-row" onClick={() => setServicePage("tmdb")} data-service="tmdb">
              <span className="integration-icon tmdb">TMDB</span>
              <span className="integration-text">
                <span className="setting-title">{t("Enriquecimento TMDB")}</span>
                <span className="muted small">{t("Controles de enriquecimento de metadados")}</span>
              </span>
              <span className="integration-arrow" aria-hidden>›</span>
            </button>
            <button className="integration-row" data-service="mdblist" data-testid="mdblist-open" onClick={() => setServicePage("mdblist")}>
              <span className="integration-icon mdb">M</span>
              <span className="integration-text">
                <span className="setting-title">{t("MDBList Ratings")}</span>
                <span className="muted small">{t("Avaliações externas")}</span>
              </span>
              <span className="integration-arrow" aria-hidden>›</span>
            </button>
            <button className="integration-row" onClick={() => setServicePage("servers")} data-service="servers">
              <span className="integration-icon link">▣</span>
              <span className="integration-text">
                <span className="setting-title">{t("Servidores de mídia")}</span>
                <span className="muted small">{t("Jellyfin, Emby, Plex e Silo")}</span>
              </span>
              <span className="integration-arrow" aria-hidden>›</span>
            </button>
            <button className="integration-row" onClick={() => setServicePage("debrid")} data-service="debrid">
              <span className="integration-icon link">↔</span>
              <span className="integration-text">
                <span className="setting-title">{t("Serviços conectados")}</span>
                <span className="muted small">{t("TorBox, Premiumize e Real-Debrid (debrid)")}</span>
              </span>
              <span className="integration-arrow" aria-hidden>›</span>
            </button>
          <Row title={t("Presença no Discord")} description={t("Mostra o que você está assistindo no Discord.")}>
            <Toggle
              label={t("Presença no Discord")}
              checked={draft.discordRpcEnabled}
              onChange={(v) => {
                update((x) => ({ ...x, discordRpcEnabled: v }));
                if (!v) void api.discordActivity(null);
              }}
            />
          </Row>
          </div>
          </>
        ) : servicePage === "servers" ? (
          <>
            <div className="settings-subhead">
              <button className="ghost" onClick={() => setServicePage("menu")}>← {t("Integrações")}</button>
              <h2>{t("Servidores de mídia")}</h2>
            </div>
            <ServersPanel />
          </>
        ) : servicePage === "mdblist" ? (
          <>
            <div className="settings-subhead">
              <button className="ghost" onClick={() => setServicePage("menu")}>← {t("Integrações")}</button>
              <h2>{t("MDBList Ratings")}</h2>
              <span className={`save-status ${mdblistStatus}`} role="status" data-testid="mdblist-save-status">
                {mdblistStatus === "saving" && t("Salvando…")}
                {mdblistStatus === "saved" && t("✓ Salvo")}
              </span>
            </div>
            {mdblistError && <div className="row-error">{mdblistError}</div>}
            {!mdblist ? (
              <div className="muted">{t("Carregando…")}</div>
            ) : (
              <div className="settings-group">
                <Row
                  title={t("Ativar avaliações externas")}
                  description={
                    mdblist.apiKeyFromAccount
                      ? t("Chave sincronizada da conta Nuvio.")
                      : mdblist.apiKeyFromOriginal
                        ? t("Chave do Nuvio original em uso.")
                        : mdblist.apiKeySet
                          ? t("Uma chave está configurada.")
                          : t("Sem chave: as avaliações ficam indisponíveis.")
                  }
                >
                  <Toggle label={t("Ativar avaliações externas")} checked={mdblist.enabled} onChange={(v) => void saveMdblist({ enabled: v })} />
                </Row>
                <Row title={t("Chave do MDBList")} description={t("Chave da API do MDBList (mdblist.com).")}>
                  <span className="tmdb-key">
                    <input
                      type="password"
                      value={mdblistKeyDraft}
                      placeholder={mdblist.apiKeySet ? "••••••••" : t("Cole a chave")}
                      aria-label={t("Chave do MDBList")}
                      onChange={(e) => setMdblistKeyDraft(e.target.value)}
                    />
                    <button className="ghost" disabled={!mdblistKeyDraft.trim()} onClick={() => { const key = mdblistKeyDraft.trim(); if (key) { setMdblistKeyDraft(""); void saveMdblist({ apiKey: key }); } }}>
                      {t("Salvar")}
                    </button>
                  </span>
                </Row>
                {([
                  ["useImdb", "IMDb"],
                  ["useTmdb", "TMDB"],
                  ["useTomatoes", "Rotten Tomatoes"],
                  ["useMetacritic", "Metacritic"],
                  ["useTrakt", "Trakt"],
                  ["useLetterboxd", "Letterboxd"],
                  ["useAudience", "Audience"],
                  ["useMal", "MyAnimeList"],
                ] as const).map(([key, label]) => (
                  <Row key={key} title={label}>
                    <Toggle label={label} checked={mdblist[key]} disabled={!mdblist.enabled} onChange={(v) => void saveMdblist({ [key]: v } as Partial<MdblistSettingsInput>)} />
                  </Row>
                ))}
                <p className="muted small settings-note">{t("As chaves são sincronizadas com a sua conta Nuvio; as avaliações aparecem no ecrã de detalhes.")}</p>
              </div>
            )}
          </>
        ) : servicePage === "debrid" ? (
          <>
            <div className="settings-subhead">
              <button className="ghost" onClick={() => setServicePage("menu")}>← {t("Integrações")}</button>
              <h2>{t("Serviços conectados")}</h2>
              <span className={`save-status ${debridStatus}`} role="status" data-testid="debrid-save-status">
                {debridStatus === "saving" && t("Salvando…")}
                {debridStatus === "saved" && t("✓ Salvo")}
              </span>
            </div>
            {debridError && <div className="row-error">{debridError}</div>}
            {!debrid ? (
              <div className="muted">{t("Carregando…")}</div>
            ) : (
              <div className="settings-group">
                <Row title={t("Ativar resolução de links")} description={t("Resolve as fontes de debrid para uma ligação direta antes de reproduzir.")}>
                  <Toggle label={t("Ativar resolução de links")} checked={debrid.enabled} disabled={!debrid.active} onChange={(v) => void saveDebrid({ enabled: v })} />
                </Row>
                {debrid.providers.map((provider) => (
                  <Row
                    key={provider.id}
                    title={provider.label}
                    description={
                      provider.fromAccount
                        ? t("Chave sincronizada da conta Nuvio.")
                        : provider.fromOriginal
                          ? t("Chave do Nuvio original em uso.")
                          : provider.connected
                            ? t("Ligado{account}.", { account: provider.account ? ` · ${provider.account}` : "" })
                            : t("Sem chave: cole a sua chave de API.")
                    }
                  >
                    <span className="tmdb-key">
                      <input
                        type="password"
                        value={debridDrafts[provider.id] ?? ""}
                        placeholder={provider.connected ? "••••••••" : t("Cole a chave")}
                        aria-label={provider.label}
                        onChange={(e) => setDebridDrafts((drafts) => ({ ...drafts, [provider.id]: e.target.value }))}
                      />
                      <button
                        className="ghost"
                        disabled={!(debridDrafts[provider.id] ?? "").trim()}
                        onClick={() => {
                          const key = (debridDrafts[provider.id] ?? "").trim();
                          if (!key) return;
                          setDebridDrafts((drafts) => ({ ...drafts, [provider.id]: "" }));
                          void saveDebrid({ keys: { [provider.id]: key }, ...(debrid.active ? {} : { active: provider.id }) });
                        }}
                      >
                        {t("Salvar")}
                      </button>
                      {provider.connected && debrid.active !== provider.id && (
                        <button className="ghost" onClick={() => void saveDebrid({ active: provider.id })}>
                          {t("Usar")}
                        </button>
                      )}
                      {provider.connected && (
                        <button className="ghost" onClick={() => void saveDebrid({ keys: { [provider.id]: "" }, active: debrid.active === provider.id ? "" : debrid.active })}>
                          {t("Remover chave")}
                        </button>
                      )}
                      {debrid.active === provider.id && <span className="integration-tag">{t("Em uso")}</span>}
                    </span>
                  </Row>
                ))}
                <p className="muted small settings-note">{t("As chaves são sincronizadas com a sua conta Nuvio (ProviderCredentialSync) por perfil. O primeiro serviço ligado é usado por padrão.")}</p>
              </div>
            )}
          </>
        ) : (
          <>
            <div className="settings-subhead">
              <button className="ghost" onClick={() => setServicePage("menu")}>← {t("Integrações")}</button>
              <h2>{t("Enriquecimento TMDB")}</h2>
              <span className={`save-status ${tmdbStatus}`} role="status" data-testid="tmdb-save-status">
                {tmdbStatus === "saving" && t("Salvando…")}
                {tmdbStatus === "saved" && t("✓ Salvo")}
              </span>
            </div>
            {tmdbError && <div className="row-error">{tmdbError}</div>}
            {!tmdb ? (
              <div className="muted">{t("Carregando…")}</div>
            ) : (
              <div className="settings-group">
                <Row title={t("Ativar enriquecimento TMDB")} description={t("Fontes TMDB nas coleções e “Mais como este”.")}>
                  <Toggle label={t("Ativar enriquecimento TMDB")} checked={tmdb.enabled} onChange={(v) => void saveTmdb({ enabled: v })} />
                </Row>
                <Row
                  title={t("Chave da API do TMDB")}
                  description={
                    tmdb.apiKeyFromAccount
                      ? t("Chave sincronizada da conta Nuvio.")
                      : tmdb.apiKeyFromOriginal
                        ? t("A chave do Nuvio original está em uso.")
                        : tmdb.apiKeySet
                        ? t("Uma chave está configurada.")
                        : t("Sem chave: os recursos TMDB ficam indisponíveis.")
                  }
                >
                  <span className="tmdb-key">
                    <input
                      type="password"
                      value={apiKeyDraft}
                      placeholder={tmdb.apiKeySet ? "••••••••" : t("Cole a chave")}
                      aria-label={t("Chave da API do TMDB")}
                      onChange={(e) => setApiKeyDraft(e.target.value)}
                    />
                    <button className="ghost" disabled={!apiKeyDraft.trim()} onClick={() => { const key = apiKeyDraft.trim(); if (key) { setApiKeyDraft(""); void saveTmdb({ apiKey: key }); } }}>
                      {t("Salvar")}
                    </button>
                    {tmdb.apiKeySet && !tmdb.apiKeyFromOriginal && (
                      <button className="ghost" onClick={() => void saveTmdb({ apiKey: "" })}>{t("Remover")}</button>
                    )}
                  </span>
                </Row>
                <Row title={t("Idioma preferido")} description={t("Idioma dos títulos e sinopses do TMDB.")}>
                  <select aria-label={t("Idioma preferido")} value={tmdb.language} onChange={(e) => void saveTmdb({ language: e.target.value })}>
                    {!TMDB_LANGUAGES.some(([code]) => code === tmdb.language) && <option value={tmdb.language}>{tmdb.language}</option>}
                    {TMDB_LANGUAGES.map(([code, name]) => <option key={code} value={code}>{name}</option>)}
                  </select>
                </Row>
                <Row title={t("Imagens (arte)")} description={t("Cartazes, fundos e logotipos do TMDB.")}>
                  <Toggle label={t("Imagens (arte)")} checked={tmdb.useArtwork} disabled={!tmdb.enabled} onChange={(v) => void saveTmdb({ useArtwork: v })} />
                </Row>
                <Row title={t("Informações básicas")} description={t("Título, sinopse, avaliação e géneros.")}>
                  <Toggle label={t("Informações básicas")} checked={tmdb.useBasicInfo} disabled={!tmdb.enabled} onChange={(v) => void saveTmdb({ useBasicInfo: v })} />
                </Row>
                <Row title={t("Detalhes")} description={t("Estado, classificação etária, duração, país e idioma.")}>
                  <Toggle label={t("Detalhes")} checked={tmdb.useDetails} disabled={!tmdb.enabled} onChange={(v) => void saveTmdb({ useDetails: v })} />
                </Row>
                <Row title={t("Elenco e equipa")} description={t("Elenco, realização e argumento do TMDB.")}>
                  <Toggle label={t("Elenco e equipa")} checked={tmdb.useCredits} disabled={!tmdb.enabled} onChange={(v) => void saveTmdb({ useCredits: v })} />
                </Row>
                <Row title={t("Episódios")} description={t("Títulos, sinopses e imagens dos episódios.")}>
                  <Toggle label={t("Episódios")} checked={tmdb.useEpisodes} disabled={!tmdb.enabled} onChange={(v) => void saveTmdb({ useEpisodes: v })} />
                </Row>
                <Row title={t("Trailers")} description={t("Trailers do TMDB no ecrã de detalhes.")}>
                  <Toggle label={t("Trailers")} checked={tmdb.useTrailers} disabled={!tmdb.enabled} onChange={(v) => void saveTmdb({ useTrailers: v })} />
                </Row>
                <Row title={t("Mais como este")} description={t("Recomendações do TMDB no ecrã de detalhes.")}>
                  <Toggle label={t("Mais como este")} checked={tmdb.useMoreLikeThis} disabled={!tmdb.enabled} onChange={(v) => void saveTmdb({ useMoreLikeThis: v })} />
                </Row>
                <Row title={t("Fontes TMDB nas coleções")} description={t("Catálogos TMDB dos folders das coleções do perfil.")}>
                  <Toggle label={t("Fontes TMDB nas coleções")} checked={tmdb.useCollections} disabled={!tmdb.enabled} onChange={(v) => void saveTmdb({ useCollections: v })} />
                </Row>
                <p className="muted small settings-note">{t("Os módulos de produtoras, redes, cartazes de temporada e coleções do TMDB ainda não foram portados.")}</p>
              </div>
            )}
          </>
        ))}

        {section === "data" && (
          <div className="settings-group">
            <div className="about" data-testid="about">
              <img src="./nuvio-astra-128.png" alt="" width={56} height={56} draggable={false} />
              <div>
                <div className="about-name">Nuvio Astra</div>
                <div className="muted small">{t("Versão")} {view.appVersion} · {t("Windows x64 · fork do Nuvio Desktop, compatível com seus addons e dados")}</div>
              </div>
            </div>
            <Row title={t("Idioma da interface")} description={t("Aplicado imediatamente a todo o aplicativo.")}>
              <select aria-label={t("Idioma da interface")} value={lang} onChange={(e) => setLang(e.target.value as Lang)}>
                {LANGUAGES_UI.map(([code, name]) => <option key={code} value={code}>{name}</option>)}
              </select>
            </Row>
            <Row title={t("Perfil ativo")} description={t("Cada perfil tem suas próprias configurações, como no Nuvio original.")}>
              <span>{view.profile.name}</span>
            </Row>
            <Row title={t("Pasta de dados do Nuvio Astra")} description={t("Gravação atômica com cópia .bak para recuperação. O nome técnico da pasta foi mantido para preservar os dados existentes.")}>
              <code className="path">{view.dataDir}</code>
            </Row>
            {view.migration && (
              <>
                <Row title={t("Migração do Nuvio original")} description={t("Os arquivos originais não são alterados.")}>
                  <span>
                    {t("versão {version} · {files} arquivos em backup · {profiles} perfis · configurações de {settings} perfil(is)", {
                      version: view.migration.version,
                      files: view.migration.backupFiles,
                      profiles: view.migration.profiles,
                      settings: view.migration.settingsProfiles ?? 0,
                    })}
                  </span>
                </Row>
                <Row title={t("Backup verificado")}><code className="path">{view.migration.backupDir}</code></Row>
              </>
            )}
          </div>
        )}

        {section === "updates" && <UpdatesSection />}

        {section === "pending" && (
          <div className="settings-group">
            <p className="muted">{t("Recursos do Nuvio original que o Nuvio Astra ainda não tem. Eles aparecem nas Configurações quando forem adicionados — nenhuma opção fica visível sem funcionar.")}</p>
            <p className="muted small">{t("Já disponíveis: Trakt e Simkl (com IDs locais), conexão MDBList, retomada pelo serviço escolhido, downloads, embaralhamento, servidores Jellyfin/Emby, editor de coleções, busca nas configurações, seleção automática e regex, fallback, reutilização de link, links externos e YouTube, capítulos, áudio, buffer/cache, Discord, Descobrir na pesquisa, títulos semelhantes, marcar assistidos, estilo e ordenação de Continuar assistindo, MDBList, TMDB, chaves de serviço e TorBox.")}</p>
            <ul className="pending-list">
              <li><b>{t("Debrid")}</b>{t("— biblioteca na nuvem (os serviços de debrid e a resolução de links já funcionam).")}</li>
              <li><b>{t("Conta e perfis")}</b>{t("— entrar com outro dispositivo (QR).")}</li>
            </ul>
          </div>
        )}
      </div>
    </div>
  );
}

// ---------- Atualizações ----------

function UpdatesSection() {
  const u = useUpdates();
  const s = u.status;
  const when = u.lastCheck ? new Date(u.lastCheck).toLocaleString(locale(), { hour: "2-digit", minute: "2-digit", day: "2-digit", month: "2-digit" }) : null;
  return (
    <div className="settings-group" data-testid="updates-settings">
      <Row title={t("Versão instalada")} description={s?.repository ? t("Atualizações publicadas em github.com/{repository}", { repository: s.repository }) : undefined}>
        <span>{s?.currentVersion ?? "…"}</span>
      </Row>
      {s && !s.configured && <p className="muted">{t("Esta compilação não tem atualizações automáticas configuradas (compilação local). As versões publicadas pelo GitHub têm.")}</p>}
      {s?.configured && (
        <>
          <Row title={t("Procurar atualizações")} description={t("O Nuvio Astra também procura sozinho ao abrir e a cada 6 horas. Toda atualização é verificada pela assinatura antes de ser instalada.")}>
            <button className="ghost" disabled={u.checking || !!u.installing} onClick={() => void checkForUpdates()} data-testid="update-check">
              {u.checking ? t("Procurando…") : t("Verificar agora")}
            </button>
          </Row>
          {u.checkError && <div className="row-error">{u.checkError}</div>}
          {!u.checking && !u.checkError && u.lastCheck && !u.available && <div className="addon-feedback ok" role="status">{t("Você já está na versão mais recente (verificado às")} {when}).</div>}
          {u.available && (
            <div className="update-card">
              <div className="setting-title">{t("Nova versão")} {u.available.version}</div>
              {u.available.notes && <div className="update-notes"><ReleaseNotes notes={u.available.notes} /></div>}
              <div className="account-actions">
                <button className="primary" disabled={!!u.installing && u.installing.phase !== "error"} onClick={() => void installUpdate()}>
                  {u.installing && u.installing.phase !== "error" ? t("Atualizando…") : t("Atualizar agora")}
                </button>
              </div>
              {u.installing?.phase === "error" && <div className="row-error">{message(u.installing.error ?? "")}</div>}
            </div>
          )}
          <p className="muted small">{t("Atualizar mantém perfis, addons, configurações, biblioteca e a sessão da conta: o instalador troca só os arquivos do programa.")}</p>
        </>
      )}
    </div>
  );
}

// ---------- Torrent / P2P ----------

function P2pSection({ settings, update }: { settings: P2pSettings; update(change: (s: P2pSettings) => P2pSettings): void }) {
  const [cache, setCache] = useState<number | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    api.p2pCacheSize().then(setCache, () => setCache(null));
  }, []);

  const enable = (on: boolean) => {
    if (on && !window.confirm(P2P_CONSENT_TEXT.map((l) => t(l)).join("\n\n"))) return;
    update((s) => ({ ...s, enabled: on }));
  };
  const clear = async () => {
    setBusy(true);
    setMessage(null);
    try {
      const freed = await api.p2pClearCache();
      setMessage(t("Cache limpo: {size} liberados.", { size: megabytes(freed) }));
      setCache(await api.p2pCacheSize());
    } catch (e) {
      setMessage(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="settings-group" data-testid="p2p-settings">
      <Row title={t("Streaming P2P")} description={t("Reproduz fontes torrent dos addons no próprio player, baixando por partes enquanto você assiste.")}>
        <Toggle label={t("Streaming P2P")} checked={settings.enabled} onChange={enable} />
      </Row>
      <Row title={t("Enviar dados (upload)")} description={t("Compartilha com outros peers as partes já baixadas enquanto o vídeo toca.")}>
        <Toggle label={t("Enviar dados")} checked={settings.enableUpload} onChange={(v) => update((s) => ({ ...s, enableUpload: v }))} />
      </Row>
      <Row title={t("Ocultar estatísticas do torrent")} description={t("Peers, velocidade e progresso sobre o player.")}>
        <Toggle label={t("Ocultar estatísticas")} checked={settings.hideTorrentStats} onChange={(v) => update((s) => ({ ...s, hideTorrentStats: v }))} />
      </Row>
      <Row title={t("Perfil de conexão")} description={t("Quantas conexões e requisições o motor abre ao mesmo tempo.")}>
        <select aria-label={t("Perfil de conexão")} value={settings.torrentProfile} onChange={(e) => update((s) => ({ ...s, torrentProfile: e.target.value as P2pProfileName }))}>
          <option value={t("SOFT")}>{t("Leve")}</option>
          <option value={t("BALANCED")}>{t("Equilibrado (padrão)")}</option>
          <option value={t("FAST")}>{t("Rápido")}</option>
        </select>
      </Row>
      <Row title={t("Tamanho do cache")} description={t("Espaço em disco para partes já baixadas (reaproveitadas ao assistir de novo).")}>
        <select aria-label={t("Tamanho do cache")} value={settings.cacheSize} onChange={(e) => update((s) => ({ ...s, cacheSize: e.target.value as P2pCacheName }))}>
          <option value={t("NONE")}>{t("Sem cache")}</option>
          <option value={t("GB_2")}>{t("2 GB (padrão)")}</option>
          <option value={t("GB_5")}>{t("5 GB")}</option>
          <option value={t("GB_10")}>{t("10 GB")}</option>
        </select>
      </Row>
      <Row title={t("Cache em uso")} description={t("Não pode ser limpo durante uma reprodução P2P.")}>
        <span className="account-actions" style={{ marginTop: 0 }}>
          <span>{cache === null ? "—" : megabytes(cache)}</span>
          <button className="ghost" disabled={busy} onClick={() => void clear()}>{busy ? t("Limpando…") : t("Limpar cache")}</button>
        </span>
      </Row>
      {message && <div className="addon-feedback ok" role="status">{message}</div>}
    </div>
  );
}

// ---------- Início ----------

/** The original's MetaScreenSettings: which details sections appear, their order and look. */
function DetailsSection({ settings, update }: {
  settings: MetaScreenSettings;
  update(change: (s: MetaScreenSettings) => MetaScreenSettings): void;
}) {
  const sections = [...settings.sections].sort((a, b) => a.order - b.order);
  const patch = (change: Partial<MetaScreenSettings>) => update((s) => ({ ...s, ...change }));
  const setSection = (key: string, enabled: boolean) =>
    patch({ sections: settings.sections.map((s) => (s.key === key ? { ...s, enabled } : s)) });
  const move = (index: number, dir: -1 | 1) => {
    const next = [...sections];
    const [item] = next.splice(index, 1);
    next.splice(index + dir, 0, item);
    patch({ sections: next.map((s, i) => ({ ...s, order: i })) });
  };
  return (
    <>
      <div className="settings-group">
        <Row title={t("Seções exibidas")} description={t("Escolha e ordene as seções da tela de detalhes.")}>
          <span className="muted small">{t("A tela de detalhes é atualizada.")}</span>
        </Row>
        {sections.map((s, i) => (
          <div className="setting-row meta-section-row" key={s.key}>
            <div className="setting-text">
              <div className="setting-title">{t(metaSectionLabel(s.key))}</div>
            </div>
            <div className="setting-control meta-section-controls">
              <Toggle label={t(metaSectionLabel(s.key))} checked={s.enabled} onChange={(v) => setSection(s.key, v)} />
              <button className="icon-btn" aria-label={t("Subir")} disabled={i === 0} onClick={() => move(i, -1)}>↑</button>
              <button className="icon-btn" aria-label={t("Descer")} disabled={i === sections.length - 1} onClick={() => move(i, 1)}>↓</button>
            </div>
          </div>
        ))}
      </div>
      <div className="settings-group">
        <Row title={t("Comentários do Trakt")} description={t("Mostra avaliações da comunidade quando a conta Trakt está conectada.")}>
          <Toggle label={t("Comentários do Trakt")} checked={settings.traktCommentsEnabled !== false} onChange={(v) => patch({ traktCommentsEnabled: v })} />
        </Row>
        <Row title={t("Fundo da tela de detalhes")}>
          <select aria-label={t("Fundo da tela de detalhes")} value={settings.backgroundMode} onChange={(e) => patch({ backgroundMode: e.target.value })}>
            <option value="normal">{t("Normal")}</option>
            <option value="cinematic">{t("Cinematográfico")}</option>
            <option value="dominant_color">{t("Cor dominante")}</option>
          </select>
        </Row>
        <Row title={t("Cartões de episódio")}>
          <select aria-label={t("Cartões de episódio")} value={settings.episodeCardStyle} onChange={(e) => patch({ episodeCardStyle: e.target.value })}>
            <option value="horizontal">{t("Horizontal")}</option>
            <option value="list">{t("Lista")}</option>
          </select>
        </Row>
        <Row title={t("Desfocar episódios não assistidos")}>
          <Toggle label={t("Desfocar episódios não assistidos")} checked={settings.blurUnwatchedEpisodes} onChange={(v) => patch({ blurUnwatchedEpisodes: v })} />
        </Row>
        <Row title={t("Mostrar avaliações gerais")}>
          <Toggle label={t("Mostrar avaliações gerais")} checked={settings.showOverallRatings} onChange={(v) => patch({ showOverallRatings: v })} />
        </Row>
        <Row title={t("Reproduzir trailer no destaque")}>
          <Toggle label={t("Reproduzir trailer no destaque")} checked={settings.heroTrailerPlayback} onChange={(v) => patch({ heroTrailerPlayback: v })} />
        </Row>
        <Row title={t("Layout em abas")} description={t("Agrupa as seções em abas na tela de detalhes.")}>
          <Toggle label={t("Layout em abas")} checked={settings.tabLayout} onChange={(v) => patch({ tabLayout: v })} />
        </Row>
      </div>
    </>
  );
}

const META_SECTION_LABELS: Record<string, string> = {
  actions: "Ações", overview: "Sinopse", episodes: "Episódios", cast: "Elenco", details: "Detalhes",
  production: "Produção", trailers: "Trailers", comments: "Comentários", collection: "Coleção", moreLikeThis: "Títulos semelhantes",
};
function metaSectionLabel(key: string): string { return META_SECTION_LABELS[key] ?? key; }

function HomeSection({ settings, catalogs, update }: {
  settings: ProfileSettings;
  catalogs: CatalogChoice[];
  update(change: (s: ProfileSettings) => ProfileSettings, note?: string, delay?: number): void;
}) {
  const note = t("A tela Início é atualizada.");
  const home = settings.home;
  // Current order: saved preferences first (by order), then catalogs without one.
  const ordered = [...catalogs].sort((a, b) => {
    const pa = home.items.find((i) => i.key === a.key)?.order ?? Number.MAX_SAFE_INTEGER;
    const pb = home.items.find((i) => i.key === b.key)?.order ?? Number.MAX_SAFE_INTEGER;
    return pa - pb || catalogs.indexOf(a) - catalogs.indexOf(b);
  });
  const pref = (c: CatalogChoice) => home.items.find((i) => i.key === c.key);
  const cw = settings.continueWatching;
  const setCw = (change: Partial<ContinueWatchingSettings>) =>
    update((s) => ({ ...s, continueWatching: { ...s.continueWatching, ...change } }), note);

  /** Rewrites the preference list in `order`, keeping hero-source flags. Rows not listed now (an
   *  addon that is off or did not answer) keep their saved position, as in the original. */
  const saveCatalogs = (order: CatalogChoice[], patch?: { key: string; enabled?: boolean; customTitle?: string }, delay?: number) =>
    update(
      (s) => ({
        ...s,
        home: {
          ...s.home,
          items: [...order.map((c, index) => {
            const old = s.home.items.find((i) => i.key === c.key);
            const base = { key: c.key, customTitle: old?.customTitle ?? c.customTitle, enabled: old?.enabled ?? c.enabled, heroSourceEnabled: old?.heroSourceEnabled ?? true, order: index };
            return patch && patch.key === c.key ? { ...base, ...(patch.enabled !== undefined && { enabled: patch.enabled }), ...(patch.customTitle !== undefined && { customTitle: patch.customTitle }) } : base;
          }), ...s.home.items.filter((i) => !order.some((c) => c.key === i.key))],
        },
      }),
      note,
      delay,
    );

  const move = (index: number, dir: -1 | 1) => {
    const next = [...ordered];
    const [item] = next.splice(index, 1);
    next.splice(index + dir, 0, item);
    saveCatalogs(next, undefined, 0);
  };

  return (
    <>
      <div className="settings-group">
        <Row title={t("Destaque no topo")} description={t("Mostra o título em destaque no início da tela.")}>
          <Toggle label={t("Destaque no topo")} checked={home.heroEnabled} onChange={(v) => update((s) => ({ ...s, home: { ...s.home, heroEnabled: v } }), note)} />
        </Row>
        {home.heroEnabled && (
          <HeroCatalogs home={home} catalogs={ordered} onChange={(heroCatalogs) => update((s) => ({ ...s, home: { ...s.home, heroCatalogs } }), note, 0)} />
        )}
        <Row title={t("Mostrar o tipo no nome do catálogo")} description={t("Ex.: “Em Alta - Filmes”.")}>
          <Toggle label={t("Mostrar o tipo no nome do catálogo")} checked={home.showCatalogType} onChange={(v) => update((s) => ({ ...s, home: { ...s.home, showCatalogType: v } }), note)} />
        </Row>
        <Row title={t("Ocultar conteúdo ainda não lançado")} description={t("Remove dos catálogos títulos com data de lançamento futura.")}>
          <Toggle label={t("Ocultar conteúdo ainda não lançado")} checked={home.hideUnreleasedContent} onChange={(v) => update((s) => ({ ...s, home: { ...s.home, hideUnreleasedContent: v } }), note)} />
        </Row>
      </div>
      <h2 className="settings-subtitle">{t("Continuar assistindo")}</h2>
      <div className="settings-group">
        <Row title={t("Mostrar a seção")} description={t("Esconde a fila de Continuar assistindo na tela Início.")}>
          <Toggle label={t("Mostrar a seção")} checked={cw.visible} onChange={(v) => setCw({ visible: v })} />
        </Row>
        <Row title={t("Estilo dos cartões")}>
          <select aria-label={t("Estilo dos cartões")} value={cw.style} onChange={(e) => setCw({ style: e.target.value })}>
            <option value="card">{t("Cartão")}</option>
            <option value="wide">{t("Largo")}</option>
            <option value="poster">{t("Pôster")}</option>
          </select>
        </Row>
        <Row title={t("Ordenação")}>
          <select aria-label={t("Ordenação")} value={cw.sortMode} onChange={(e) => setCw({ sortMode: e.target.value })}>
            <option value="default">{t("Padrão")}</option>
            <option value="streaming">{t("Estilo streaming")}</option>
            <option value="splitUpcoming">{t("Separar próximos episódios")}</option>
          </select>
        </Row>
        <Row title={t("Ao clicar")}>
          <select aria-label={t("Ao clicar")} value={cw.clickAction} onChange={(e) => setCw({ clickAction: e.target.value })}>
            <option value="stream">{t("Reproduzir direto")}</option>
            <option value="details">{t("Abrir detalhes")}</option>
            <option value="detailsForUpcoming">{t("Detalhes só nos próximos episódios")}</option>
          </select>
        </Row>
        <Row title={t("Aviso ao abrir o app")} description={t("Pergunta se quer continuar assistindo ao iniciar o aplicativo.")}>
          <Toggle label={t("Aviso ao abrir o app")} checked={cw.showResumePromptOnLaunch} onChange={(v) => setCw({ showResumePromptOnLaunch: v })} />
        </Row>
        <Row title={t("Mostrar episódios ainda não exibidos")} description={t("Próximos episódios com data futura na fila.")}>
          <Toggle label={t("Mostrar episódios ainda não exibidos")} checked={cw.showUnairedNextUp} onChange={(v) => setCw({ showUnairedNextUp: v })} />
        </Row>
        <Row title={t("Desfocar próximos episódios")}>
          <Toggle label={t("Desfocar próximos episódios")} checked={cw.blurNextUp} onChange={(v) => setCw({ blurNextUp: v })} />
        </Row>
      </div>
      <h2 className="settings-subtitle">{t("Catálogos ({shown} de {total} visíveis)", { shown: ordered.filter((c) => pref(c)?.enabled ?? c.enabled).length, total: ordered.length })}</h2>
      <p className="muted small">{t("Com uma conta Nuvio, a ordem, os nomes e o que fica visível sincronizam sozinhos com os outros aparelhos.")}</p>
      <div className="settings-group catalogs">
        {ordered.map((c, index) => {
          const enabled = pref(c)?.enabled ?? c.enabled;
          const title = pref(c)?.customTitle ?? c.customTitle;
          return (
            <div key={c.key} className={`catalog-pref ${enabled ? "" : "off"}`} data-key={c.key}>
              <Toggle label={t("Mostrar {title}", { title: localizeCatalogTitle(c.defaultTitle) })} checked={enabled} onChange={(v) => saveCatalogs(ordered, { key: c.key, enabled: v }, 0)} />
              <div className="catalog-pref-text">
                <input
                  className="catalog-title-input"
                  value={title}
                  placeholder={localizeCatalogTitle(c.defaultTitle)}
                  aria-label={t("Nome de {title}", { title: localizeCatalogTitle(c.defaultTitle) })}
                  onChange={(e) => saveCatalogs(ordered, { key: c.key, customTitle: e.target.value }, 600)}
                />
                <span className="muted small">{addonLabel(c.addonName)}</span>
              </div>
              <button className="icon-btn" aria-label={t("Subir {title}", { title: localizeCatalogTitle(c.defaultTitle) })} disabled={index === 0} onClick={() => move(index, -1)}>↑</button>
              <button className="icon-btn" aria-label={t("Descer {title}", { title: localizeCatalogTitle(c.defaultTitle) })} disabled={index === ordered.length - 1} onClick={() => move(index, 1)}>↓</button>
            </div>
          );
        })}
      </div>
    </>
  );
}

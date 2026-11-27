// Settings of the active profile. Only options that NextGen applies for real are shown; each one
// is saved through Rust (`settings_set`, validated into the original's ranges) and read back.
// The original's other settings pages are listed under t("Ainda não disponível") (no controls).

import { ReleaseNotes } from "../components/ReleaseNotes";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import {
  AccountLine as AccountIcon, AddonsLine as AddonsIcon, AudioLine as AudioIcon, HomeLine as HomeIcon, P2pLine as P2pIcon,
  PendingLine as PendingIcon, TorboxLine as TorboxIcon, DownloadsLine as DownloadsIcon, PlaybackLine as PlaybackIcon, ProfileDataLine as ProfileDataIcon, SubtitlesLine as SubtitlesIcon, UpdatesLine as UpdatesIcon,
} from "../components/SidebarIcons";
import { api, errorText, onRemoteSettingsChanged } from "../api";
import { AccountPanel } from "../components/AccountPanel";
import { AddonsManager } from "../components/AddonsManager";
import { checkForUpdates, installUpdate, useUpdates } from "../updates";
import { Avatar } from "../components/ProfileMenu";
import { LANGUAGES, languageLabel } from "../languages";
import { P2P_CONSENT_TEXT, megabytes } from "../components/TorrentStart";
import { ANIME_MODES, ENHANCEMENTS, MOTIONS } from "../player/EnhanceMenu";
import { Row, Toggle } from "../components/SettingControls";
import { SubtitleAppearance } from "../components/SubtitleAppearance";
import { HeroCatalogs } from "../components/HeroCatalogs";
import { WatchingSettings } from "../components/WatchingSettings";
import { DownloadsSection, TorboxSection } from "../components/ServiceSettings";
import { SourcesSection } from "../components/SourceSettings";
import { applyCardStyle, CARD_RADII, CARD_WIDTHS } from "../cardStyle";
import { refreshLibraryViews } from "../library";
import { SHORTCUTS } from "../player/PlayerExtras";
import { screensChanged } from "../screens";
import { CollectionsEditor } from "../components/CollectionsEditor";
import { TmdbSection } from "../components/TmdbSettings";
import { SimklSection } from "../components/SimklSettings";
import { DiscordSection } from "../components/DiscordSettings";
import { PluginsSection } from "../components/PluginsSettings";
import { MdbListSection } from "../components/MdbListSettings";
import { chooserOnLaunch, setChooserOnLaunch } from "../components/LaunchScreens";
import type { ScreenSettings, PlaybackExtras, CatalogChoice, P2pCacheName, P2pProfileName, P2pSettings, PlayerSettings, ProfileSettings, AnimeModeName, EnhancementName, MotionName, ResizeModeName, SettingsView, SubtitleSettings } from "../types";
import { LANGUAGES_UI, addonLabel, localizeCatalogTitle, locale, t, useLang, type Lang } from "../i18n";
import { chooseAppLanguage, chosenLanguage } from "../appLanguage";
import { message } from "../rustErrors";

type Section = "account" | "home" | "plugins" | "tmdb" | "simkl" | "mdblist" | "discord" | "collections" | "details" | "continue" | "posters" | "addons" | "playback" | "watching" | "controls" | "sources" | "p2p" | "torbox" | "downloads" | "audio" | "subtitles" | "data" | "updates" | "pending";

// The menu: a few groups, each opening into its pages (only one group open at a time).
const GROUPS: { id: string; label: string; icon: ReactNode; sections: { id: Section; label: string; icon: ReactNode }[] }[] = [
  { id: "profile", label: "Conta e perfil", icon: <AccountIcon />, sections: [
    { id: "account", label: "Conta", icon: <AccountIcon /> },
    { id: "data", label: "Perfil e dados", icon: <ProfileDataIcon /> },
  ] },
  { id: "content", label: "Conteúdo", icon: <HomeIcon />, sections: [
    { id: "home", label: "Início", icon: <HomeIcon /> },
    { id: "continue", label: "Continuar assistindo", icon: <HomeIcon /> },
    { id: "collections", label: "Coleções", icon: <HomeIcon /> },
    { id: "details", label: "Tela de detalhes", icon: <HomeIcon /> },
    { id: "posters", label: "Pôsteres", icon: <HomeIcon /> },
    { id: "addons", label: "Addons", icon: <AddonsIcon /> },
  ] },
  { id: "player", label: "Player", icon: <PlaybackIcon />, sections: [
    { id: "playback", label: "Imagem e vídeo", icon: <PlaybackIcon /> },
    { id: "watching", label: "Episódios e pular", icon: <PlaybackIcon /> },
    { id: "controls", label: "Controles e buffer", icon: <PlaybackIcon /> },
    { id: "audio", label: "Áudio", icon: <AudioIcon /> },
    { id: "subtitles", label: "Legendas", icon: <SubtitlesIcon /> },
  ] },
  { id: "sources", label: "Fontes e downloads", icon: <DownloadsIcon />, sections: [
    { id: "sources", label: "Fontes", icon: <DownloadsIcon /> },
    { id: "plugins", label: "Plugins", icon: <AddonsIcon /> },
    { id: "p2p", label: "Torrent / P2P", icon: <P2pIcon /> },
    { id: "torbox", label: "Debrid", icon: <TorboxIcon /> },
    { id: "downloads", label: "Downloads", icon: <DownloadsIcon /> },
  ] },
  { id: "integrations", label: "Integrações", icon: <AddonsIcon />, sections: [
    { id: "tmdb", label: "TMDB", icon: <AddonsIcon /> },
    { id: "simkl", label: "Simkl", icon: <AddonsIcon /> },
    { id: "mdblist", label: "MDBList", icon: <AddonsIcon /> },
    { id: "discord", label: "Discord", icon: <AddonsIcon /> },
  ] },
  { id: "app", label: "Sobre o app", icon: <UpdatesIcon />, sections: [
    { id: "updates", label: "Atualizações", icon: <UpdatesIcon /> },
    { id: "pending", label: "Ainda não disponível", icon: <PendingIcon /> },
  ] },
];
const SECTIONS = GROUPS.flatMap((g) => g.sections);

/** What each page has, for the search box (labels of its options, in Portuguese). */
const KEYWORDS: Partial<Record<Section, string>> = {
  account: "conta nuvio entrar sair sincronizar email",
  data: "perfil idioma interface pasta dados migração versão quem está assistindo",
  home: "início destaque catálogos cards tamanho cantos paisagem nomes tipo lançado",
  continue: "continuar assistindo estilo ordem clique retomar próximo episódio",
  collections: "coleções pastas criar editar catálogos",
  plugins: "plugins repositório scrapers fontes manifest provedores",
  discord: "discord rich presence status perfil assistindo integrações",
  simkl: "simkl conta conectar scrobble histórico assistidos importar anime integrações",
  mdblist: "mdblist notas avaliações imdb rotten tomatoes metacritic letterboxd chave integrações",
  tmdb: "tmdb chave api metadados elenco atores estúdios canais semelhantes saga coleção idioma imagens episódios trailers integrações",
  details: "detalhes elenco informações episódios desfocar spoiler fundo grade",
  posters: "pôster prévia mouse rpdb endereço personalizado",
  addons: "addons instalar configurar remover ordem",
  playback: "imagem vídeo decodificação anime4k movimento rtx hdr modo",
  watching: "assistido próximo episódio pular introdução créditos",
  controls: "segurar acelerar avisos conteúdo buffer cache disco atalhos teclado",
  audio: "áudio idioma amplificação downmix estéreo trailer",
  subtitles: "legendas idioma fonte tamanho cor contorno sombra sdh",
  sources: "fontes automática regex falhar último link emblemas",
  p2p: "torrent p2p upload cache conexão",
  torbox: "debrid torbox real-debrid alldebrid premiumize debrid-link chave",
  downloads: "downloads pasta baixar",
  updates: "atualizações versão",
  pending: "ainda não disponível",
};
const groupOf = (s: Section) => GROUPS.find((g) => g.sections.some((x) => x.id === s))!.id;

function Chevron({ open }: { open: boolean }) {
  return (
    <svg className={`nav-chevron ${open ? "open" : ""}`} viewBox="0 0 24 24" width="14" height="14" aria-hidden fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M9 6l6 6-6 6" />
    </svg>
  );
}

type Status = { kind: "idle" } | { kind: "saving" } | { kind: "saved"; note?: string } | { kind: "error"; message: string };

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
  useLang(); // re-renders the page when the language changes
  const [view, setView] = useState<SettingsView | null>(null);
  const [draft, setDraft] = useState<ProfileSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [section, setSection] = useState<Section>("home");
  const [openGroup, setOpenGroup] = useState<string | null>(() => groupOf("home"));
  const [query, setQuery] = useState("");
  const fold = (v: string) => v.toLowerCase().normalize("NFD").replace(/[\u0300-\u036f]/g, "");
  const found = query.trim()
    ? SECTIONS.filter((s) => {
        const words = fold(query).split(/\s+/).filter(Boolean);
        const text = fold(`${t(s.label)} ${s.label} ${KEYWORDS[s.id] ?? ""}`);
        return words.every((w) => text.includes(w));
      })
    : [];
  const choose = (s: Section) => {
    setSection(s);
    setOpenGroup(groupOf(s));
  };
  const toggleGroup = (id: string) => {
    if (openGroup === id) return setOpenGroup(null);
    setOpenGroup(id);
    const group = GROUPS.find((g) => g.id === id)!;
    if (!group.sections.some((s) => s.id === section)) setSection(group.sections[0].id);
  };
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const timer = useRef<number>(0);
  const pending = useRef<{ next: ProfileSettings; note?: string } | null>(null);
  /** Latest settings, including changes not rendered yet (several quick changes in a row). */
  const draftRef = useRef<ProfileSettings | null>(null);
  /** What the draft was based on: only the difference from it is saved. */
  const baseRef = useRef<ProfileSettings | null>(null);
  const profileRef = useRef<number | undefined>(undefined);
  const saving = useRef(false);

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
  const screen = (change: Partial<ScreenSettings>) =>
    update((s) => {
      const next = { ...s, screens: { ...s.screens, ...change } };
      screensChanged(next.screens);
      return next;
    }, undefined, 0);
  const [chooser, setChooser] = useState(chooserOnLaunch);
  const extras = (change: Partial<PlaybackExtras>) => update((s) => ({ ...s, playback: { ...s.playback, ...change } }), t("Vale a partir da próxima reprodução."), 0);

  const resetSection = (what: Section) => {
    if (!view || !draft) return;
    const name = t(SECTIONS.find((s) => s.id === what)?.label ?? "");
    if (!window.confirm(t("Restaurar os padrões de “{name}” para o perfil {profile}? As alterações desta seção serão descartadas.", { name, profile: view.profile.name }))) return;
    const d = view.defaults;
    update((s) => {
      switch (what) {
        case "home": return { ...s, home: { ...d.home } };
        case "playback": return { ...s, player: { ...s.player, resizeMode: d.player.resizeMode, decoderPriority: d.player.decoderPriority, nvidiaRtxSuperResolution: d.player.nvidiaRtxSuperResolution, nvidiaRtxHdr: d.player.nvidiaRtxHdr, motionSmoothing: d.player.motionSmoothing, videoEnhancement: d.player.videoEnhancement, picture: { ...d.player.picture } } };
        case "watching": return { ...s, player: { ...s.player, skipSegmentsEnabled: d.player.skipSegmentsEnabled, autoSkipSegments: [...d.player.autoSkipSegments], nextEpisodeThresholdMode: d.player.nextEpisodeThresholdMode, nextEpisodeThresholdPercent: d.player.nextEpisodeThresholdPercent, nextEpisodeThresholdMinutes: d.player.nextEpisodeThresholdMinutes, watchedPercent: d.player.watchedPercent } };
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
          placeholder={t("Buscar nas configurações")}
          aria-label={t("Buscar nas configurações")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          data-testid="settings-search"
        />
        {query.trim() && (
          <div className="settings-search-results" data-testid="settings-search-results">
            {found.length === 0 && <div className="muted small">{t("Nada encontrado.")}</div>}
            {found.map((s) => (
              <button key={s.id} className={`nav-item nav-sub ${section === s.id ? "active" : ""}`} data-result={s.id} onClick={() => { choose(s.id); setQuery(""); }}>
                <span>{t(s.label)}</span>
              </button>
            ))}
          </div>
        )}
        {!query.trim() && GROUPS.map((g) => {
          const open = openGroup === g.id;
          const current = g.sections.some((s) => s.id === section);
          return (
            <div key={g.id} className={`nav-group ${open ? "open" : ""}`}>
              <button className={`nav-item nav-group-head ${current ? "current" : ""}`} data-group={g.id} aria-expanded={open} onClick={() => toggleGroup(g.id)}>
                <span className="nav-icon" aria-hidden>{g.icon}</span>
                <span className="nav-group-label">{t(g.label)}</span>
                <Chevron open={open} />
              </button>
              <div className="nav-sub-list" hidden={!open}>
                {g.sections.map((s) => (
                  <button key={s.id} className={`nav-item nav-sub ${section === s.id ? "active" : ""}`} data-section={s.id} onClick={() => choose(s.id)}>
                    <span>{t(s.label)}</span>
                  </button>
                ))}
              </div>
            </div>
          );
        })}
      </nav>

      <div className="settings-body" key={section}>
        <header className="settings-header">
          <h1>{t(SECTIONS.find((s) => s.id === section)?.label ?? "")}</h1>
          <span className={`save-status ${status.kind}`} role="status" data-testid="save-status">
            {status.kind === "saving" && t("Salvando…")}
            {status.kind === "saved" && `${t("✓ Salvo")}${status.note ? ` · ${status.note}` : ""}`}
            {status.kind === "error" && t("Não foi possível salvar: {error}", { error: message(status.message) })}
          </span>
          {["home", "playback", "watching", "audio", "subtitles"].includes(section) && (
            <button className="ghost" onClick={() => resetSection(section)}>{t("Restaurar padrões")}</button>
          )}
        </header>

        {section === "home" && (
          <HomeSection settings={draft} catalogs={view.catalogs} update={update} />
        )}

        {section === "account" && <AccountPanel />}
        {section === "addons" && <AddonsManager />}

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
          </div>
        )}

        {section === "watching" && (
          <div className="settings-group">
            <WatchingSettings value={p} onChange={player} />
          </div>
        )}

        {section === "torbox" && <TorboxSection settings={draft.debrid} update={(change) => update((s) => ({ ...s, debrid: change(s.debrid) }), t("Vale para as próximas fontes."), 0)} />}
        {section === "downloads" && <DownloadsSection />}
        {section === "sources" && <SourcesSection value={draft.sources} onChange={(change) => update((s) => ({ ...s, sources: change(s.sources) }), t("Vale para as próximas fontes."), 400)} />}
        {section === "p2p" && <P2pSection settings={draft.p2p} update={(change) => update((s) => ({ ...s, p2p: change(s.p2p) }), t("Vale a partir da próxima fonte torrent."), 0)} />}

        {section === "controls" && (
          <div className="settings-group" data-testid="controls-settings">
            <h3 className="settings-subhead">{t("Controles")}</h3>
            <Row title={t("Segurar para acelerar")} description={t("Segurar o clique no vídeo, o botão +10 s ou a tecla L toca mais rápido enquanto segura.")}>
              <Toggle label={t("Segurar para acelerar")} checked={draft.playback.holdToSpeedEnabled} onChange={(v) => extras({ holdToSpeedEnabled: v })} />
            </Row>
            {draft.playback.holdToSpeedEnabled && (
              <Row title={t("Velocidade ao segurar")}>
                <select aria-label={t("Velocidade ao segurar")} value={draft.playback.holdToSpeedValue} onChange={(e) => extras({ holdToSpeedValue: Number(e.target.value) })}>
                  {[1.5, 2, 2.5, 3].map((v) => <option key={v} value={v}>{`${String(v).replace(".", ",")}x`}</option>)}
                </select>
              </Row>
            )}
            <Row title={t("Avisos de conteúdo")} description={t("Ao começar um filme ou episódio, mostra por alguns segundos os avisos do guia para pais do IMDb (violência, linguagem…). Só o código IMDb do título é enviado.")}>
              <Toggle label={t("Avisos de conteúdo")} checked={draft.playback.showParentalGuide} onChange={(v) => extras({ showParentalGuide: v })} />
            </Row>
            <h3 className="settings-subhead">{t("Buffer e cache")}</h3>
            <Row title={t("Buffer à frente")} description={t("Quanto do vídeo fica guardado adiante. Mais ajuda em internet instável e usa mais memória.")}>
              <select aria-label={t("Buffer à frente")} value={draft.playback.cacheMb} onChange={(e) => extras({ cacheMb: Number(e.target.value) })}>
                {[50, 100, 150, 300, 500, 1000, 2000].map((v) => <option key={v} value={v}>{v >= 1000 ? `${v / 1000} GB` : `${v} MB`}{v === 150 ? ` (${t("padrão")})` : ""}</option>)}
              </select>
            </Row>
            <Row title={t("Buffer para voltar")} description={t("Quanto do que já passou fica guardado para voltar sem baixar de novo.")}>
              <select aria-label={t("Buffer para voltar")} value={draft.playback.backCacheMb} onChange={(e) => extras({ backCacheMb: Number(e.target.value) })}>
                {[25, 50, 100, 200, 500].map((v) => <option key={v} value={v}>{`${v} MB`}{v === 50 ? ` (${t("padrão")})` : ""}</option>)}
              </select>
            </Row>
            <Row title={t("Cache em disco")} description={t("Guarda o buffer no disco em vez da memória (útil com buffers grandes). Fica na pasta de dados do app.")}>
              <Toggle label={t("Cache em disco")} checked={draft.playback.diskCache} onChange={(v) => extras({ diskCache: v })} />
            </Row>
            <h3 className="settings-subhead">{t("Atalhos do player")}</h3>
            <dl className="shortcut-list">
              {SHORTCUTS.map(([keys, action]) => (
                <div key={keys} className="shortcut"><dt>{t(keys)}</dt><dd>{t(action)}</dd></div>
              ))}
            </dl>
          </div>
        )}

        {section === "audio" && (
          <div className="settings-group">
            <Row title={t("Amplificação")} description={t("Aumenta o volume além do máximo (útil em vídeos com áudio baixo). Também ajustável no player.")}>
              <select aria-label={t("Amplificação")} data-setting="audioGainDb" value={draft.playback.audioGainDb} onChange={(e) => extras({ audioGainDb: Number(e.target.value) })}>
                {[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map((v) => <option key={v} value={v}>{v === 0 ? t("Desligada") : `+${v} dB`}</option>)}
              </select>
            </Row>
            <Row title={t("Converter para estéreo (downmix)")} description={t("Mistura áudio 5.1/7.1 em dois canais, para fones e caixas estéreo (diálogos mais audíveis).")}>
              <Toggle label={t("Converter para estéreo (downmix)")} checked={draft.playback.downmix} onChange={(v) => extras({ downmix: v })} />
            </Row>
            <Row title={t("Idioma dos trailers")} description={t("O “Assistir trailer” procura no YouTube o trailer nesse idioma (ex.: dublado) ao ser clicado. Sem um vídeo adequado, toca o trailer do addon.")}>
              <select aria-label={t("Idioma dos trailers")} data-setting="trailerLanguage" value={draft.trailer.language} onChange={(e) => update((s) => ({ ...s, trailer: { language: e.target.value } }), t("Vale para o próximo trailer."), 0)}>
                <option value="original">{t("Original do addon")}</option>
                <option value="pt-BR">{t("Português (Brasil) — dublado")}</option>
                <option value="pt-PT">{t("Português (Portugal)")}</option>
                <option value="en">{t("Inglês")}</option>
                <option value="es">{t("Espanhol (Espanha)")}</option>
                <option value="es-419">{t("Espanhol (América Latina)")}</option>
                <option value="fr">{t("Francês")}</option>
                <option value="de">{t("Alemão")}</option>
                <option value="it">{t("Italiano")}</option>
                <option value="ja">{t("Japonês")}</option>
              </select>
            </Row>
            <Row title={t("Idioma de áudio preferido")} description={t("“Idioma do dispositivo” = {list} neste PC.", { list: view.deviceLanguages.join(", ") || t("não identificado") })}>
              <LanguageSelect label={t("Idioma de áudio preferido")} value={p.preferredAudioLanguage} special={[["device", t("Idioma do dispositivo")], ["default", t("Padrão do arquivo")], ["original", t("Idioma original")]]} onChange={(v) => player((x) => ({ ...x, preferredAudioLanguage: v ?? "device" }))} />
            </Row>
            <Row title={t("Idioma de áudio secundário")} description={t("Usado quando o preferido não existe no arquivo.")}>
              <LanguageSelect label={t("Idioma de áudio secundário")} value={p.secondaryPreferredAudioLanguage} special={[["", t("Nenhum")]]} onChange={(v) => player((x) => ({ ...x, secondaryPreferredAudioLanguage: v }))} />
            </Row>
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
          </div>
        )}

        {section === "collections" && <CollectionsEditor />}

        {section === "discord" && <DiscordSection />}

        {section === "plugins" && <PluginsSection />}

        {section === "simkl" && <SimklSection settings={draft.simkl} onChange={(change) => update((s) => ({ ...s, simkl: { ...s.simkl, ...change } }), undefined, 0)} />}

        {section === "mdblist" && <MdbListSection settings={draft.mdblist} onChange={(change) => update((s) => ({ ...s, mdblist: { ...s.mdblist, ...change } }), undefined, 0)} />}

        {section === "tmdb" && <TmdbSection settings={draft.tmdb} onChange={(change) => update((s) => ({ ...s, tmdb: { ...s.tmdb, ...change } }), undefined, 0)} />}

        {section === "details" && (
          <div className="settings-group" data-testid="details-settings">
            <Row title={t("Elenco")} description={t("Mostra o elenco na tela de detalhes.")}>
              <Toggle label={t("Elenco")} checked={draft.screens.detailsShowCast} onChange={(v) => screen({ detailsShowCast: v })} />
            </Row>
            <Row title={t("Informações")} description={t("Classificação, roteiro, país, idioma, prêmios e site.")}>
              <Toggle label={t("Informações")} checked={draft.screens.detailsShowInfo} onChange={(v) => screen({ detailsShowInfo: v })} />
            </Row>
            <Row title={t("Episódios")} description={t("Lista com sinopse ou grade de imagens.")}>
              <select aria-label={t("Episódios")} value={draft.screens.detailsEpisodeStyle} onChange={(e) => screen({ detailsEpisodeStyle: e.target.value as "list" | "grid" })} data-setting="detailsEpisodeStyle">
                <option value="list">{t("Lista")}</option>
                <option value="grid">{t("Grade")}</option>
              </select>
            </Row>
            <Row title={t("Desfocar episódios não assistidos")} description={t("Vale para o app todo: esconde a imagem e a sinopse dos episódios que você ainda não viu, na tela de detalhes e no Continuar assistindo (evita spoilers).")}>
              <Toggle label={t("Desfocar episódios não assistidos")} checked={draft.screens.detailsBlurUnwatched} onChange={(v) => screen({ detailsBlurUnwatched: v })} />
            </Row>
            <Row title={t("Fundo")}>
              <select aria-label={t("Fundo")} value={draft.screens.detailsBackground} onChange={(e) => screen({ detailsBackground: e.target.value as "normal" | "cinematic" | "none" })}>
                <option value="normal">{t("Imagem do título")}</option>
                <option value="cinematic">{t("Cinematográfico (mais escuro e desfocado)")}</option>
                <option value="none">{t("Sem imagem")}</option>
              </select>
            </Row>
          </div>
        )}

        {section === "continue" && (
          <div className="settings-group" data-testid="continue-settings">
            <Row title={t("Mostrar no Início")}>
              <Toggle label={t("Mostrar Continuar assistindo no Início")} checked={draft.screens.cwVisible} onChange={(v) => screen({ cwVisible: v })} />
            </Row>
            <Row title={t("Estilo")}>
              <select aria-label={t("Estilo")} value={draft.screens.cwStyle} onChange={(e) => screen({ cwStyle: e.target.value as "card" | "wide" | "poster" })} data-setting="cwStyle">
                <option value="card">{t("Cartão")}</option>
                <option value="wide">{t("Largo, com informações")}</option>
                <option value="poster">{t("Pôster")}</option>
              </select>
            </Row>
            <Row title={t("Imagem do episódio")} description={t("Usa a imagem do próprio episódio em vez da imagem da série.")}>
              <Toggle label={t("Imagem do episódio")} checked={draft.screens.cwEpisodeThumbnails} onChange={(v) => screen({ cwEpisodeThumbnails: v })} />
            </Row>
            <Row title={t("Episódios ainda não lançados")} description={t("Mostra o próximo episódio mesmo antes da estreia, com a data.")}>
              <Toggle label={t("Episódios ainda não lançados")} checked={draft.screens.cwShowUnaired} onChange={(v) => screen({ cwShowUnaired: v })} />
            </Row>
            <Row title={t("Ordem")}>
              <select aria-label={t("Ordem")} value={draft.screens.cwSort} onChange={(e) => screen({ cwSort: e.target.value as "default" | "streaming" | "split" })} data-setting="cwSort">
                <option value="default">{t("Padrão (mais recentes)")}</option>
                <option value="streaming">{t("Estilo streaming (começados primeiro)")}</option>
                <option value="split">{t("Não lançados no fim")}</option>
              </select>
            </Row>
            <Row title={t("Ao clicar")}>
              <select aria-label={t("Ao clicar")} value={draft.screens.cwClick} onChange={(e) => screen({ cwClick: e.target.value as "stream" | "details" | "detailsForUpcoming" })} data-setting="cwClick">
                <option value="stream">{t("Abrir as fontes para continuar")}</option>
                <option value="details">{t("Abrir a tela de detalhes")}</option>
                <option value="detailsForUpcoming">{t("Detalhes só para episódios não lançados")}</option>
              </select>
            </Row>
            <Row title={t("Perguntar ao abrir o app")} description={t("Ao abrir, oferece continuar o último filme ou episódio que ficou pela metade.")}>
              <Toggle label={t("Perguntar ao abrir o app")} checked={draft.screens.cwResumePrompt} onChange={(v) => screen({ cwResumePrompt: v })} />
            </Row>
          </div>
        )}

        {section === "posters" && (
          <div className="settings-group" data-testid="posters-settings">
            <Row title={t("Prévia ao passar o mouse")} description={t("Mostra um cartão maior com sinopse, ano e nota quando o mouse para sobre um pôster.")}>
              <Toggle label={t("Prévia ao passar o mouse")} checked={draft.screens.hoverPreview} onChange={(v) => screen({ hoverPreview: v })} />
            </Row>
            {draft.screens.hoverPreview && (
              <Row title={t("Tempo até a prévia")}>
                <select aria-label={t("Tempo até a prévia")} value={draft.screens.hoverDelayMs} onChange={(e) => screen({ hoverDelayMs: Number(e.target.value) })}>
                  {[500, 1000, 1500, 2000, 2500, 3000, 4000, 5000].map((v) => <option key={v} value={v}>{`${String(v / 1000).replace(".", ",")} s`}</option>)}
                </select>
              </Row>
            )}
            <Row title={t("Endereço de pôster personalizado")} description={t("Ex.: RPDB — https://api.ratingposterdb.com/SUA-CHAVE/imdb/poster-default/{imdb_id}.jpg. Aceita {imdb_id}, {tmdb_id}, {tvdb_id}, {kitsu_id}, {type}, {id}; {x?} é opcional e {a|b} usa o primeiro disponível. Sem o código pedido, fica o pôster do addon.")}>
              <input
                className="poster-pattern"
                spellCheck={false}
                placeholder="https://…/{imdb_id}.jpg"
                aria-label={t("Endereço de pôster personalizado")}
                defaultValue={draft.screens.posterPattern}
                onBlur={(e) => screen({ posterPattern: e.target.value.trim() })}
                data-setting="posterPattern"
              />
            </Row>
          </div>
        )}

        {section === "data" && (
          <div className="settings-group">
            <Row title={t("“Quem está assistindo?” ao abrir")} description={t("Com mais de um perfil, pergunta quem vai assistir sempre que o app abre (vale para este computador).")}>
              <Toggle label={t("“Quem está assistindo?” ao abrir")} checked={chooser} onChange={(v) => { setChooser(v); setChooserOnLaunch(v); }} />
            </Row>
            <div className="about" data-testid="about">
              <img src="./nuvio-astra-128.png" alt="" width={56} height={56} draggable={false} />
              <div>
                <div className="about-name">Nuvio Astra</div>
                <div className="muted small">{t("Versão")} {view.appVersion} · {t("Windows x64 · fork do Nuvio Desktop, compatível com seus addons e dados")}</div>
              </div>
            </div>
            <Row title={t("Idioma da interface")} description={t("Aplicado na hora e salvo na sua conta Nuvio: os outros computadores com o Nuvio Astra abrem no mesmo idioma.")}>
              <select aria-label={t("Idioma da interface")} data-setting="appLanguage" value={chosenLanguage(draft.player.appLanguage)} onChange={(e) => {
                const choice = e.target.value as Lang | "device";
                setDraft((d) => (d ? { ...d, player: { ...d.player, appLanguage: choice === "device" ? "device" : choice === "pt" ? "pt-BR" : choice === "pt-PT" ? "pt" : choice } } : d));
                void chooseAppLanguage(choice);
              }}>
                <option value="device">{t("Idioma do dispositivo")}</option>
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
            <ul className="pending-list">
              <li><b>{t("Tela de detalhes")}</b> {t("— comentários (dependem do Trakt).")}</li>
              <li><b>{t("Fontes")}</b> {t("— emblemas personalizados (Fusion).")}</li>
              <li><b>{t("Próximo episódio")}</b> {t("— tocar sozinho sem escolher a fonte, pré-carregar as fontes e preferir o mesmo grupo de fontes.")}</li>
              <li><b>{t("Pular introdução e créditos")}</b> {t("— Anime-Skip como serviço extra e envio de novos tempos ao IntroDB (estes dois exigem chave pessoal). Pular com IntroDB e AniSkip já funciona sem chave.")}</li>
              <li><b>{t("Debrid")}</b> {t("— entrar no TorBox sem copiar a chave e biblioteca na nuvem.")}</li>
              <li><b>{t("Integrações")}</b> {t("— Trakt.")}</li>
              <li><b>{t("Servidores de mídia")}</b> {t("— Plex, Jellyfin, Emby e Silo.")}</li>
              <li><b>{t("Watch Party")}</b> {t("— assistir junto com outras pessoas.")}</li>
              <li><b>{t("Coleções")}</b> {t("— pastas com listas do Trakt (dependem desse serviço).")}</li>
              <li><b>{t("Conta e perfis")}</b> {t("— entrar com outro dispositivo (QR), sincronizar chaves de serviços e avatares exclusivos de membros.")}</li>
              <li><b>{t("Aparência")}</b> {t("— efeito de profundidade dos cards.")}</li>
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

/** Format of the poster cards (`PosterCardStyleRepository`), synced with the account. */
function CardRows({ cards, update }: { cards: ProfileSettings["player"]["cards"]; update(cards: ProfileSettings["player"]["cards"]): void }) {
  return (
    <>
      <Row title={t("Tamanho dos cards")} description={t("Largura dos pôsteres no Início, na pesquisa e na biblioteca.")}>
        <select aria-label={t("Tamanho dos cards")} data-setting="cardWidth" value={cards.width} onChange={(e) => update({ ...cards, width: Number(e.target.value) })}>
          {CARD_WIDTHS.map(([v, label]) => <option key={v} value={v}>{t(label)}</option>)}
        </select>
      </Row>
      <Row title={t("Cantos dos cards")}>
        <select aria-label={t("Cantos dos cards")} data-setting="cardRadius" value={cards.radius} onChange={(e) => update({ ...cards, radius: Number(e.target.value) })}>
          {CARD_RADII.map(([v, label]) => <option key={v} value={v}>{t(label)}</option>)}
        </select>
      </Row>
      <Row title={t("Pôsteres em paisagem")} description={t("Usa a imagem larga do título nos catálogos, quando o addon tiver.")}>
        <Toggle label={t("Pôsteres em paisagem")} checked={cards.landscape} onChange={(v) => update({ ...cards, landscape: v })} />
      </Row>
      <Row title={t("Ocultar nomes")} description={t("Mostra só a imagem nos cards.")}>
        <Toggle label={t("Ocultar nomes")} checked={cards.hideLabels} onChange={(v) => update({ ...cards, hideLabels: v })} />
      </Row>
    </>
  );
}

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
        <CardRows cards={settings.player.cards} update={(cards) => { applyCardStyle(cards); update((s) => ({ ...s, player: { ...s.player, cards } }), note, 0); }} />
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

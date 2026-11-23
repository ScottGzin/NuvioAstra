// Settings of the active profile. Only options that NextGen applies for real are shown; each one
// is saved through Rust (`settings_set`, validated into the original's ranges) and read back.
// The original's other settings pages are listed under t("Ainda não disponível") (no controls).

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import {
  AccountLine as AccountIcon, AddonsLine as AddonsIcon, AudioLine as AudioIcon, HomeLine as HomeIcon, P2pLine as P2pIcon,
  PendingLine as PendingIcon, PlaybackLine as PlaybackIcon, ProfileDataLine as ProfileDataIcon, SubtitlesLine as SubtitlesIcon, UpdatesLine as UpdatesIcon,
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
import { refreshLibraryViews } from "../library";
import type { CatalogChoice, P2pCacheName, P2pProfileName, P2pSettings, PlayerSettings, ProfileSettings, AnimeModeName, EnhancementName, MotionName, ResizeModeName, SettingsView, SubtitleSettings } from "../types";
import { LANGUAGES_UI, addonLabel, localizeCatalogTitle, locale, setLang, t, useLang, type Lang } from "../i18n";
import { message } from "../rustErrors";

type Section = "account" | "home" | "addons" | "playback" | "p2p" | "audio" | "subtitles" | "data" | "updates" | "pending";

const SECTIONS: { id: Section; label: string; icon: ReactNode }[] = [
  { id: "account", label: "Conta", icon: <AccountIcon /> },
  { id: "home", label: "Início", icon: <HomeIcon /> },
  { id: "addons", label: "Addons", icon: <AddonsIcon /> },
  { id: "playback", label: "Reprodução", icon: <PlaybackIcon /> },
  { id: "p2p", label: "Torrent / P2P", icon: <P2pIcon /> },
  { id: "audio", label: "Áudio", icon: <AudioIcon /> },
  { id: "subtitles", label: "Legendas", icon: <SubtitlesIcon /> },
  { id: "data", label: "Perfil e dados", icon: <ProfileDataIcon /> },
  { id: "updates", label: "Atualizações", icon: <UpdatesIcon /> },
  { id: "pending", label: "Ainda não disponível", icon: <PendingIcon /> },
];

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
  const lang = useLang();
  const [view, setView] = useState<SettingsView | null>(null);
  const [draft, setDraft] = useState<ProfileSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [section, setSection] = useState<Section>("home");
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
        {SECTIONS.map((s) => (
          <button key={s.id} className={`nav-item ${section === s.id ? "active" : ""}`} data-section={s.id} onClick={() => setSection(s.id)}>
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
            <WatchingSettings value={p} onChange={player} />
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
            <ul className="pending-list">
              <li><b>{t("Início e catálogos")}</b>{t("— “Ver todos” de um catálogo, Descobrir na pesquisa, prévia ao passar o mouse e fonte de pôsteres personalizada.")}</li>
              <li><b>{t("Tela de detalhes")}</b>{t("— escolher as seções exibidas, títulos semelhantes, comentários e desfocar episódios não assistidos.")}</li>
              <li><b>{t("Assistidos")}</b>{t("— marcar ou desmarcar filmes e episódios como assistidos manualmente.")}</li>
              <li><b>{t("Continuar assistindo")}</b>{t("— estilo, ordenação, ação do clique, aviso de retomada ao abrir e opções para ocultar a fileira ou os episódios futuros.")}</li>
              <li><b>{t("Fontes")}</b>{t("— seleção automática, filtros por regex, outra fonte em caso de falha, reutilizar o último link, emblemas, links externos e fontes do YouTube.")}</li>
              <li><b>{t("Player")}</b>{t("— trocar de fonte e escolher qualquer episódio sem sair do vídeo, painel de capítulos, avisos de conteúdo e segurar para acelerar.")}</li>
              <li><b>{t("Próximo episódio")}</b>{t("— tocar sozinho sem escolher a fonte, pré-carregar as fontes, preferir o mesmo grupo de fontes e ordem aleatória.")}</li>
              <li><b>{t("Pular introdução e créditos")}</b>{t("— Anime-Skip como serviço extra e envio de novos tempos ao IntroDB (estes dois exigem chave pessoal). Pular com IntroDB e AniSkip já funciona sem chave.")}</li>
              <li><b>{t("Legendas")}</b>{t("— sincronização automática com o áudio.")}</li>
              <li><b>{t("Áudio")}</b>{t("— ajuste de atraso no player, amplificação e downmix.")}</li>
              <li><b>{t("Buffer e cache")}</b>{t("— ajustes de buffer e cache em disco para streams HTTP.")}</li>
              <li><b>{t("Downloads")}</b>{t("— baixar filmes e episódios para assistir depois.")}</li>
              <li><b>{t("Debrid")}</b>{t("— Real-Debrid, Premiumize, TorBox e biblioteca na nuvem.")}</li>
              <li><b>{t("Integrações")}</b>{t("— Trakt, Simkl, MDBList, TMDB e Discord Rich Presence.")}</li>
              <li><b>{t("Servidores de mídia")}</b>{t("— Plex, Jellyfin, Emby e Silo.")}</li>
              <li><b>{t("Plugins e Watch Party")}</b>{t("— plugins de fontes e assistir junto com outras pessoas.")}</li>
              <li><b>{t("Coleções")}</b>{t("— criar e editar no app e pastas com catálogos do TMDB ou do Trakt.")}</li>
              <li><b>{t("Conta e perfis")}</b>{t("— entrar com outro dispositivo (QR), sincronizar chaves de serviços e avatares exclusivos de membros.")}</li>
              <li><b>{t("Aplicativo")}</b>{t("— idioma do app e busca dentro das Configurações.")}</li>
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
              {u.available.notes && <pre className="update-notes">{u.available.notes}</pre>}
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

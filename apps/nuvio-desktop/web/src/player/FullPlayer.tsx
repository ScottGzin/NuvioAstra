// Whole-window player: on Windows/macOS the video uses the full client area (rect `null`) under
// the transparent page and the controls fade out after 3 s without mouse movement. On Linux the
// video is a native X11 window above the WebView, so the page keeps it inside the rectangle the
// chrome leaves free (see `useNativeVideoRect`). Keyboard: Space, ←/→, ↑/↓, F, M, Esc.

import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { api } from "../api";
import { listen } from "../tauri";
import type { CssRect, MetaVideo, PlayerSettings } from "../types";
import { playbackItem } from "../library";
import { usePlayer } from "./PlayerContext";
import { P2pStats } from "./P2pStats";
import { EnhanceNotice } from "./EnhanceMenu";
import { ChaptersPanel } from "./ChaptersPanel";
import { InPlayerSources } from "./InPlayerSources";
import { PlayerControls } from "./PlayerControls";
import { airDate, episodeTitle, hasAired, nextEpisode } from "./episodes";
import { NextEpisodePanel } from "./NextEpisodePanel";
import { PlaybackMoments, reachedNextEpisode, useMoments } from "./PlaybackMoments";
import { SubtitleMenu } from "./SubtitleMenu";
import { SubmitIntroDialog } from "./SubmitIntroDialog";
import { PartyPanel } from "../components/PartyPanel";
import { t } from "../i18n";

interface VideoLayout {
  /** `null` = the whole window. */
  rect: CssRect | null;
  /** Parts of `rect` the page needs for its chrome (the video is cut out there on Linux). */
  holes: CssRect[];
}

/**
 * The picture occupies the whole window, like the Windows backend: the chrome is cut out of the
 * native video window where it is visible, so no empty bands are left around the picture while
 * the controls are hidden. A dropdown menu is the exception — it is tall, so the picture is
 * letterboxed above it instead of being cut through.
 */
function videoLayout(root: HTMLElement, chrome: boolean): VideoLayout {
  const width = window.innerWidth;
  const height = window.innerHeight;
  const holes: CssRect[] = [];
  const cut = (nodes: NodeListOf<HTMLElement>) => {
    for (const node of nodes) {
      const rect = node.getBoundingClientRect();
      if (rect.height > 0) {
        holes.push({ x: Math.round(rect.left), y: Math.round(rect.top), width: Math.round(rect.width), height: Math.round(rect.height) });
      }
    }
  };
  // Everything the picture must not cover: the panels opened inside the player (sources, chapters,
  // next episode) and the skip/next cards. The video keeps playing around them; nothing is panned
  // or resized, so opening a panel never pushes the picture aside.
  cut(root.querySelectorAll<HTMLElement>(".chapters-panel, .next-panel, .player-moments"));
  if (chrome) {
    // Dropdown menus fade out with the control bar they belong to, so they only need a hole while
    // the chrome is shown.
    cut(root.querySelectorAll<HTMLElement>(".pc-enhance-menu"));
    let top = 0;
    for (const node of root.querySelectorAll<HTMLElement>(".full-player-top, .enhance-notice")) {
      const rect = node.getBoundingClientRect();
      if (rect.height > 0) top = Math.max(top, rect.bottom);
    }
    if (top > 0) holes.push({ x: 0, y: 0, width, height: Math.round(top) });
    const controls = root.querySelector<HTMLElement>(".controls");
    if (controls) {
      const top = controls.getBoundingClientRect().top;
      if (top < height) holes.push({ x: 0, y: Math.round(top), width, height: Math.round(height - top) });
    }
  }
  return { rect: null, holes };
}

/**
 * Linux draws mpv in a native X11 window above the WebView, so HTML can never paint over the
 * picture: the video fills the window and the areas the chrome needs are cut out of it. `pip`
 * leaves the layout alone (the picture lives in the PiP window and Rust keeps the last one).
 */
function useNativeVideoRect(
  root: RefObject<HTMLDivElement | null>,
  chrome: boolean,
  pip: boolean,
) {
  const [native, setNative] = useState<boolean | null>(null);
  useEffect(() => {
    let cancelled = false;
    api.platform().then((os) => !cancelled && setNative(os === "linux"), () => setNative(false));
    return () => { cancelled = true; };
  }, []);
  useEffect(() => {
    document.documentElement.classList.toggle("native-video", native === true);
    return () => document.documentElement.classList.remove("native-video");
  }, [native]);
  useEffect(() => {
    if (native === null || pip) return;
    if (!native) { void api.playerRect(null); return; }
    const element = root.current;
    if (!element) return;
    let frame = 0;
    let sent = "";
    const send = () => {
      frame = 0;
      const layout = videoLayout(element, chrome);
      const key = JSON.stringify(layout);
      if (key === sent) return;
      sent = key;
      void api.playerRect(layout.rect, layout.holes);
    };
    const schedule = () => { if (!frame) frame = requestAnimationFrame(send); };
    schedule();
    const sizes = new ResizeObserver(schedule);
    sizes.observe(element);
    element.querySelectorAll(".full-player-top, .controls").forEach((node) => sizes.observe(node));
    // Menus (subtitles, enhancements) and the skip/next cards mount and change class on demand.
    const changes = new MutationObserver(schedule);
    changes.observe(element, { childList: true, subtree: true, attributes: true, attributeFilter: ["class"] });
    window.addEventListener("resize", schedule);
    return () => {
      cancelAnimationFrame(frame);
      sizes.disconnect();
      changes.disconnect();
      window.removeEventListener("resize", schedule);
    };
  }, [native, chrome, pip, root]);
}

export function FullPlayer() {
  const player = usePlayer();
  const { session, snapshot } = player;
  const [visible, setVisible] = useState(true);
  const [pip, setPip] = useState(false);
  const hideTimer = useRef<number>(0);
  const live = session?.live === true;
  const moments = useMoments(snapshot, !live);
  const episode = session?.episode;
  const next = useMemo(() => (episode ? nextEpisode(episode) : null), [episode]);
  const nextAired = next !== null && hasAired(next.released);
  const [choosingNext, setChoosingNext] = useState(false);
  const [playerSettings, setPlayerSettings] = useState<PlayerSettings | null>(null);
  const [chaptersOpen, setChaptersOpen] = useState(false);
  const [sourcesOpen, setSourcesOpen] = useState(false);
  const [partyOpen, setPartyOpen] = useState(false);
  const [submitOpen, setSubmitOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  useNativeVideoRect(rootRef, visible || choosingNext, pip);
  // Another episode started: its panel is gone.
  useEffect(() => { setChoosingNext(false); setChaptersOpen(false); setSourcesOpen(false); }, [episode]);

  // Player settings are read once per session: hold-to-speed, chapters and next-episode autoplay.
  useEffect(() => {
    let cancelled = false;
    api.settingsGet().then((view) => !cancelled && setPlayerSettings(view.settings.player), () => {});
    return () => { cancelled = true; };
  }, []);

  // Discord presence follows the active playback (the original's PresenceSnapshot.Player).
  useEffect(() => {
    if (!session) {
      void api.discordActivity(null);
      return;
    }
    const paused = snapshot?.paused ?? false;
    const episode = session.episode ? episodeTitle(session.episode.video) : null;
    void api.discordActivity({
      title: session.title,
      state: episode ? `${episode}${paused ? ` · ${t("Pausado")}` : ""}` : (paused ? t("Pausado") : null),
      poster: session.episode?.meta.poster ?? null,
      playing: !paused,
      positionMs: snapshot ? Math.round(snapshot.timePos * 1000) : null,
      durationMs: snapshot ? Math.round(snapshot.duration * 1000) : null,
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session?.title, session?.episode?.video.id, snapshot?.paused]);

  const openNext = useCallback(() => {
    if (!nextAired) return;
    if (player.session && snapshot?.pictureInPicture) void api.pictureInPicture(false, "").catch(() => {});
    setChoosingNext(true);
  }, [nextAired, player.session, snapshot?.pictureInPicture]);

  const leave = useCallback(async () => {
    if (player.windowFullscreen) await player.setWindowFullscreen(false);
    if (session?.returnTo) player.setView(session.returnTo);
    else await player.close();
  }, [player, session]);

  const toggleFullscreen = useCallback(() => {
    if (session?.returnTo && player.windowFullscreen) void leave();
    else void player.setWindowFullscreen(!player.windowFullscreen);
  }, [leave, player, session]);

  const poke = useCallback(() => {
    setVisible(true);
    window.clearTimeout(hideTimer.current);
    hideTimer.current = window.setTimeout(() => setVisible(false), 3000);
  }, []);
  useEffect(() => {
    poke();
    return () => window.clearTimeout(hideTimer.current);
  }, [poke]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).tagName === "INPUT" || (e.target as HTMLElement).tagName === "SELECT") return;
      if (choosingNext) {
        if (e.key === "Escape") setChoosingNext(false);
        return;
      }
      poke();
      switch (e.key) {
        case " ": case "k": e.preventDefault(); void api.playerCommand({ type: "togglePause" }); break;
        case "ArrowLeft": if (!live) void api.playerCommand({ type: "seek", seconds: -10, relative: true }); break;
        case "ArrowRight": if (!live) void api.playerCommand({ type: "seek", seconds: 10, relative: true }); break;
        case "ArrowUp": void api.playerCommand({ type: "setVolume", volume: Math.min(200, (snapshot?.volume ?? 100) + 5) }); break;
        case "ArrowDown": void api.playerCommand({ type: "setVolume", volume: Math.max(0, (snapshot?.volume ?? 100) - 5) }); break;
        case "m": void api.playerCommand({ type: "setMute", mute: !snapshot?.mute }); break;
        case "f": toggleFullscreen(); break;
        case "n": openNext(); break;
        case "Escape": void leave(); break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [choosingNext, leave, live, openNext, poke, snapshot, toggleFullscreen]);

  // Also changed from the PiP window itself, or by closing it.
  useEffect(() => listen<boolean>("player-pip", setPip), []);

  const togglePip = async () => {
    try {
      await api.pictureInPicture(!pip, session?.title ?? "Nuvio");
      setPip(!pip);
    } catch {
      /* the player reports its own error state */
    }
  };

  return (
    <div ref={rootRef} className={`full-player ${visible || choosingNext ? "" : "chrome-hidden"}`} onMouseMove={poke}>
      <div className="full-player-top">
        <button className="back" onClick={() => void leave()} aria-label={t("Voltar")}>←</button>
        <div>
          <div className="full-player-title">{session?.title}</div>
          {session?.subtitle && <div className="muted">{session.subtitle}</div>}
        </div>
        <P2pStats />
        {snapshot && playerSettings?.introSubmitEnabled && session?.episode && (
          <button className="chip" data-testid="submit-intro-open" aria-label={t("Enviar tempo à IntroDB")} onClick={() => setSubmitOpen(true)}>{t("Enviar tempo")}</button>
        )}
        <button className="chip" data-testid="party-button" onClick={() => setPartyOpen((v) => !v)} aria-label={t("Watch Party")}>{t("Watch Party")}</button>
        {partyOpen && (
          <PartyPanel
            status={player.party.status}
            room={player.party.room}
            onJoin={(room) => player.party.join(room)}
            onLeave={() => player.party.leave()}
            configured={player.party.configured}
            t={t}
          />
        )}
      </div>
      {pip && (
        <div className="pip-placeholder" data-testid="pip-placeholder">
          <p>{t("Reproduzindo em picture-in-picture")}</p>
          <button className="ghost" onClick={() => void togglePip()}>{t("Trazer de volta")}</button>
        </div>
      )}
      <EnhanceNotice key={`notice-${snapshot?.session}`} snapshot={snapshot} />
      <PlayerControls
        snapshot={snapshot}
        live={live}
        fullscreen={player.windowFullscreen}
        onToggleFullscreen={toggleFullscreen}
        onPictureInPicture={() => void togglePip()}
        subtitles={snapshot ? <SubtitleMenu key={`subtitles-${snapshot.session}`} snapshot={snapshot} /> : undefined}
        holdSpeed={{ enabled: playerSettings?.holdToSpeedEnabled ?? true, value: playerSettings?.holdToSpeedValue ?? 2 }}
        onChapters={snapshot && snapshot.chapters.length > 0 ? () => setChaptersOpen(true) : undefined}
        onSources={episode ? () => setSourcesOpen(true) : undefined}
        next={
          next
            ? {
                label: nextAired ? `${episodeTitle(next)} (N)` : t("Não exibido · {date}", { date: airDate(next.released) }),
                disabled: !nextAired,
                open: openNext,
              }
            : undefined
        }
      />
      {snapshot && !live && !pip && (
        <PlaybackMoments
          key={`moments-${snapshot.session}`}
          snapshot={snapshot}
          moments={moments}
          canSubmit={playerSettings?.introSubmitEnabled ?? false}
          next={next}
          covered={choosingNext}
          chromeVisible={visible}
          onNext={openNext}
        />
      )}
      {sourcesOpen && episode && (
        <InPlayerSources
          context={episode}
          onClose={() => setSourcesOpen(false)}
          onPickEpisode={(chosen: MetaVideo) => {
            void (async () => {
              const groups = await api.streams(episode.meta.contentType, chosen.id);
              const stream = groups.flatMap((group) => group.streams)
                .find((candidate) => ["direct", "torrent", "debrid", "youTube"].includes(candidate.kind));
              if (!stream) return;
              await player.play(() => api.playStream(stream.key, null, playbackItem(episode.meta, chosen)), {
                title: episode.meta.name,
                subtitle: episodeTitle(chosen),
                view: "full",
                returnTo: null,
                episode: { meta: episode.meta, video: chosen },
                torrent: stream.kind === "torrent" && !stream.debridPlayable,
              });
            })();
          }}
        />
      )}
      {chaptersOpen && snapshot && snapshot.chapters.length > 0 && (
        <ChaptersPanel chapters={snapshot.chapters} onClose={() => setChaptersOpen(false)} />
      )}
      {choosingNext && episode && next && (
        <NextEpisodePanel
          key={next.id}
          context={episode}
          next={next}
          finished={() => reachedNextEpisode(snapshot, moments)}
          onClose={() => setChoosingNext(false)}
          autoPlay={playerSettings}
        />
      )}
      {submitOpen && snapshot && (
        <SubmitIntroDialog
          session={snapshot.session}
          position={snapshot.timePos}
          duration={snapshot.duration}
          onClose={() => setSubmitOpen(false)}
          onSubmitted={() => setSubmitOpen(false)}
        />
      )}
    </div>
  );
}

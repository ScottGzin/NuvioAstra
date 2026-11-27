// Whole-window player: the video uses the full client area (rect `null`) and the controls fade
// out after 3 s without mouse movement.
// - Without leaving the video: another source ("Fontes", O), another episode ("Episódios", E), the
//   next episode (N), chapters, the audio menu (track, delay, amplification, downmix).
// - Keyboard and mouse: see `SHORTCUTS` ("?"). A click on the video pauses, a double click is full
//   screen, holding the click (or L) plays faster while held, the wheel changes the volume.
// - Content warnings (IMDb parents guide) for a few seconds when a movie or episode starts.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import { listen } from "../tauri";
import { usePlayer } from "./PlayerContext";
import { P2pStats } from "./P2pStats";
import { EnhanceNotice } from "./EnhanceMenu";
import { PlayerControls } from "./PlayerControls";
import { airDate, episodeTitle, hasAired, nextEpisode } from "./episodes";
import { NextEpisodePanel } from "./NextEpisodePanel";
import { PlaybackMoments, reachedNextEpisode, useMoments } from "./PlaybackMoments";
import { SubtitleMenu } from "./SubtitleMenu";
import { AudioMenu, ContentWarnings, EpisodePicker, ShortcutsHelp } from "./PlayerExtras";
import { startDownload } from "../sources";
import type { MetaVideo, PlaybackExtras } from "../types";
import { t } from "../i18n";

const SPEEDS = [0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 2.5, 3];

type Panel = { kind: "next" } | { kind: "sources" } | { kind: "episodes" } | { kind: "episode"; video: MetaVideo };

export function FullPlayer() {
  const player = usePlayer();
  const { session, snapshot } = player;
  const [visible, setVisible] = useState(true);
  const [pip, setPip] = useState(false);
  const hideTimer = useRef<number>(0);
  const live = session?.live === true;
  const moments = useMoments(snapshot, !live);
  const episode = session?.episode;
  const meta = episode?.meta ?? session?.movie ?? null;
  const next = useMemo(() => (episode ? nextEpisode(episode) : null), [episode]);
  const nextAired = next !== null && hasAired(next.released);
  const [panel, setPanel] = useState<Panel | null>(null);
  const [help, setHelp] = useState(false);
  const [extras, setExtras] = useState<PlaybackExtras | null>(null);
  const [gainDb, setGainDb] = useState(0);
  const [downmix, setDownmix] = useState(false);
  /** Source switching: where the new source starts. */
  const [switchAt, setSwitchAt] = useState<number | null>(null);
  const choosingNext = panel !== null;

  useEffect(() => {
    api.settingsGet().then((v) => {
      setExtras(v.settings.playback);
      setGainDb(v.settings.playback.audioGainDb);
      setDownmix(v.settings.playback.downmix);
    }, () => {});
  }, []);

  // Another episode or source started: its panel is gone.
  useEffect(() => {
    setPanel(null);
  }, [episode, snapshot?.session]);
  const openNext = useCallback(() => {
    if (!nextAired) return;
    if (player.session && snapshot?.pictureInPicture) void api.pictureInPicture(false, "").catch(() => {});
    setPanel({ kind: "next" });
  }, [nextAired, player.session, snapshot?.pictureInPicture]);
  const openSources = useCallback(() => {
    if (!meta || live) return;
    setSwitchAt(snapshot?.timePos ?? null);
    setPanel({ kind: "sources" });
    // The player's own position (the one shown may lag while paused).
    api.playerSnapshot().then((s) => setSwitchAt(s.timePos), () => {});
  }, [meta, live, snapshot?.timePos]);
  const openEpisodes = useCallback(() => {
    if (!episode) return;
    setPanel({ kind: "episodes" });
  }, [episode]);

  useEffect(() => {
    void api.playerRect(null);
  }, []);

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

  const togglePip = useCallback(async () => {
    try {
      await api.pictureInPicture(!pip, session?.title ?? "Nuvio");
      setPip(!pip);
    } catch {
      /* the player reports its own error state */
    }
  }, [pip, session?.title]);

  // ---- hold to speed up (mouse on the video, or L held)
  const holdSpeed = extras?.holdToSpeedEnabled && !live ? extras.holdToSpeedValue : 0;
  const held = useRef<{ speed: number } | null>(null);
  const beginHold = useCallback(() => {
    if (!holdSpeed || held.current) return;
    held.current = { speed: snapshot?.speed ?? 1 };
    void api.playerCommand({ type: "setSpeed", speed: holdSpeed });
  }, [holdSpeed, snapshot?.speed]);
  const endHold = useCallback(() => {
    const h = held.current;
    held.current = null;
    if (h) void api.playerCommand({ type: "setSpeed", speed: h.speed });
    return !!h;
  }, []);

  const cycleAudio = useCallback(() => {
    const audio = snapshot?.tracks.filter((tr) => tr.kind === "audio") ?? [];
    if (audio.length < 2) return;
    const at = audio.findIndex((tr) => String(tr.id) === snapshot?.aid);
    void api.playerCommand({ type: "setAudioTrack", id: String(audio[(at + 1) % audio.length].id) });
  }, [snapshot]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).tagName === "INPUT" || (e.target as HTMLElement).tagName === "SELECT") return;
      if (help) {
        if (e.key === "Escape" || e.key === "?") setHelp(false);
        return;
      }
      if (choosingNext) {
        if (e.key === "Escape") setPanel(null);
        return;
      }
      poke();
      const seek = (seconds: number) => !live && void api.playerCommand({ type: "seek", seconds, relative: true });
      const duration = snapshot?.duration ?? 0;
      if (/^[0-9]$/.test(e.key) && !e.ctrlKey && !e.altKey && !live && duration > 0) {
        void api.playerCommand({ type: "seek", seconds: (Number(e.key) / 10) * duration, relative: false });
        return;
      }
      switch (e.key) {
        case " ": case "k": case "K": e.preventDefault(); void api.playerCommand({ type: "togglePause" }); break;
        case "ArrowLeft": case "j": case "J": seek(e.shiftKey ? -30 : -10); break;
        case "ArrowRight": seek(e.shiftKey ? 30 : 10); break;
        case "l": case "L":
          // Held: faster while held; a single press seeks.
          if (e.repeat) beginHold();
          break;
        case "Home": if (!live) void api.playerCommand({ type: "seek", seconds: 0, relative: false }); break;
        case ",": if (!live) { void api.playerCommand({ type: "setPaused", paused: true }); void api.playerCommand({ type: "frameStep", back: true }); } break;
        case ".": if (!live) { void api.playerCommand({ type: "setPaused", paused: true }); void api.playerCommand({ type: "frameStep", back: false }); } break;
        case "<": case ">": {
          if (live) break;
          const now = snapshot?.speed ?? 1;
          const at = SPEEDS.findIndex((v) => Math.abs(v - now) < 0.01);
          const nextSpeed = SPEEDS[Math.max(0, Math.min(SPEEDS.length - 1, (at < 0 ? 3 : at) + (e.key === ">" ? 1 : -1)))];
          void api.playerCommand({ type: "setSpeed", speed: nextSpeed });
          break;
        }
        case "ArrowUp": e.preventDefault(); void api.playerCommand({ type: "setVolume", volume: Math.min(200, (snapshot?.volume ?? 100) + 5) }); break;
        case "ArrowDown": e.preventDefault(); void api.playerCommand({ type: "setVolume", volume: Math.max(0, (snapshot?.volume ?? 100) - 5) }); break;
        case "m": case "M": void api.playerCommand({ type: "setMute", mute: !snapshot?.mute }); break;
        case "c": case "C": void api.playerCommand({ type: "setSubtitleVisible", visible: !(snapshot?.subVisible ?? true) }); break;
        case "a": case "A": cycleAudio(); break;
        case "f": case "F": toggleFullscreen(); break;
        case "p": case "P": void togglePip(); break;
        case "n": case "N": openNext(); break;
        case "e": case "E": openEpisodes(); break;
        case "o": case "O": openSources(); break;
        case "?": setHelp(true); break;
        case "Escape": void leave(); break;
      }
    };
    const onKeyUp = (e: KeyboardEvent) => {
      if (e.key !== "l" && e.key !== "L") return;
      // A held L ends its speed-up; a short press is a 10 s jump.
      if (!endHold() && !choosingNext && !help) void api.playerCommand({ type: "seek", seconds: 10, relative: true });
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKeyUp);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onKeyUp);
    };
  }, [beginHold, choosingNext, cycleAudio, endHold, help, leave, live, openEpisodes, openNext, openSources, poke, snapshot, toggleFullscreen, togglePip]);

  // ---- mouse on the video: click = pause, double click = full screen, hold = faster, wheel = volume
  const press = useRef<{ timer: number; clickTimer: number } | null>(null);
  const clickTimer = useRef(0);
  const onVideoDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    const timer = window.setTimeout(beginHold, 400);
    press.current = { timer, clickTimer: 0 };
  };
  const onVideoUp = (e: React.PointerEvent) => {
    if (e.button !== 0 || !press.current) return;
    window.clearTimeout(press.current.timer);
    press.current = null;
    if (endHold()) return;
    if (clickTimer.current) {
      // Second click: full screen (the first one's pause is cancelled).
      window.clearTimeout(clickTimer.current);
      clickTimer.current = 0;
      toggleFullscreen();
      return;
    }
    clickTimer.current = window.setTimeout(() => {
      clickTimer.current = 0;
      void api.playerCommand({ type: "togglePause" });
    }, 230);
  };
  const onWheel = (e: React.WheelEvent) => {
    poke();
    const step = e.deltaY < 0 ? 5 : -5;
    void api.playerCommand({ type: "setVolume", volume: Math.max(0, Math.min(200, (snapshot?.volume ?? 100) + step)) });
  };

  // Also changed from the PiP window itself, or by closing it.
  useEffect(() => listen<boolean>("player-pip", setPip), []);

  const applyGain = (db: number) => {
    setGainDb(db);
    void api.playerCommand({ type: "setAudioGain", db });
  };
  const applyDownmix = (stereo: boolean) => {
    setDownmix(stereo);
    void api.playerCommand({ type: "setDownmix", stereo });
  };

  const panelMeta = meta;
  const tabs = episode && panel && panel.kind !== "next" ? (
    <div className="player-panel-tabs">
      <button className={`chip ${panel.kind === "sources" ? "active" : ""}`} onClick={openSources}>{t("Fontes")}</button>
      <button className={`chip ${panel.kind !== "sources" ? "active" : ""}`} onClick={openEpisodes}>{t("Episódios")}</button>
    </div>
  ) : null;

  return (
    <div className={`full-player ${visible || choosingNext ? "" : "chrome-hidden"}`} onMouseMove={poke}>
      <div className="player-hit" onPointerDown={onVideoDown} onPointerUp={onVideoUp} onPointerLeave={() => { if (press.current) { window.clearTimeout(press.current.timer); press.current = null; endHold(); } }} onWheel={onWheel} data-testid="player-hit" />
      <div className="full-player-top">
        <button className="back" onClick={() => void leave()} aria-label={t("Voltar")}>←</button>
        <div>
          <div className="full-player-title">{session?.title}</div>
          {session?.subtitle && <div className="muted">{session.subtitle}</div>}
        </div>
        <P2pStats />
        <span className="spacer" />
        <button className="icon-btn player-help-btn" aria-label={t("Atalhos do player")} title={t("Atalhos do player (?)")} onClick={() => setHelp(true)}>?</button>
      </div>
      {pip && (
        <div className="pip-placeholder" data-testid="pip-placeholder">
          <p>{t("Reproduzindo em picture-in-picture")}</p>
          <button className="ghost" onClick={() => void togglePip()}>{t("Trazer de volta")}</button>
        </div>
      )}
      <EnhanceNotice key={`notice-${snapshot?.session}`} snapshot={snapshot} />
      {meta && !live && extras?.showParentalGuide !== false && (
        <ContentWarnings key={`warnings-${snapshot?.session}`} id={meta.id} started={!!snapshot?.loaded} />
      )}
      <PlayerControls
        snapshot={snapshot}
        live={live}
        fullscreen={player.windowFullscreen}
        onToggleFullscreen={toggleFullscreen}
        onPictureInPicture={() => void togglePip()}
        onDownload={session?.download ? () => void startDownload(session.download!) : undefined}
        subtitles={snapshot ? <SubtitleMenu key={`subtitles-${snapshot.session}`} snapshot={snapshot} /> : undefined}
        audio={snapshot && !live ? <AudioMenu snapshot={snapshot} gainDb={gainDb} onGain={applyGain} downmix={downmix} onDownmix={applyDownmix} /> : undefined}
        onSources={meta && !live ? openSources : undefined}
        onEpisodes={episode ? openEpisodes : undefined}
        holdSpeed={holdSpeed}
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
          next={next}
          covered={choosingNext}
          chromeVisible={visible}
          onNext={openNext}
        />
      )}
      {panel?.kind === "next" && episode && next && (
        <NextEpisodePanel key={next.id} context={episode} next={next} finished={() => reachedNextEpisode(snapshot, moments)} onClose={() => setPanel(null)} />
      )}
      {panel?.kind === "sources" && panelMeta && (
        <NextEpisodePanel
          key={`switch-${episode?.video.id ?? panelMeta.id}`}
          context={{ meta: panelMeta }}
          next={episode?.video ?? null}
          mode="switch"
          resumeFrom={switchAt}
          finished={() => false}
          onClose={() => setPanel(null)}
          top={tabs}
        />
      )}
      {panel?.kind === "episode" && episode && (
        <NextEpisodePanel
          key={`episode-${panel.video.id}`}
          context={episode}
          next={panel.video}
          mode="episode"
          finished={() => reachedNextEpisode(snapshot, moments)}
          onClose={() => setPanel(null)}
          top={tabs}
        />
      )}
      {panel?.kind === "episodes" && episode && (
        <div className="next-panel-layer" data-testid="episodes-panel">
          <button className="next-panel-outside" aria-label={t("Fechar")} tabIndex={-1} onClick={() => setPanel(null)} />
          <aside className="next-panel" aria-label={t("Episódios")}>
            {tabs}
            <header className="sources-header">
              <div>
                <h2>{t("Episódios")}</h2>
                <div className="muted small">{episode.meta.name}</div>
              </div>
              <button className="icon-btn" aria-label={t("Fechar")} onClick={() => setPanel(null)}>✕</button>
            </header>
            <div className="sources-body">
              <EpisodePicker meta={episode.meta} current={episode.video} onPick={(video) => (video.id === episode.video.id ? openSources() : setPanel({ kind: "episode", video }))} />
            </div>
          </aside>
        </div>
      )}
      {help && <ShortcutsHelp onClose={() => setHelp(false)} />}
    </div>
  );
}

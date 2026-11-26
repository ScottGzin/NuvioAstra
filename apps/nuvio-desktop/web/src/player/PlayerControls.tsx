// Controls drawn over the video (same commands as the validated test UI). Every control acts on
// the real player through `player_command`; nothing here is simulated.
//
// Look: cyan accents after the app icon. Timeline = track, buffered part (position + mpv's demuxer
// cache ahead, the same for HTTP and P2P sources), played part with glow and a moving dot; an
// invisible range input on top handles dragging and keyboard.

import { useEffect, useRef, useState, type ReactNode } from "react";
import { api } from "../api";
import type { PlayerSnapshot, ResizeMode, Track } from "../types";
import { EnhanceMenu } from "./EnhanceMenu";
import { reachedEnd } from "./episodes";
import { t } from "../i18n";

export function formatTime(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds || 0));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  return (h ? `${h}:${String(m).padStart(2, "0")}` : `${m}`) + `:${String(r).padStart(2, "0")}`;
}

export function trackLabel(t: Track): string {
  return (t.external ? "📄 " : "") + ([t.title, t.lang, t.codec].filter(Boolean).join(" · ") || `#${t.id}`);
}

const svg = (children: ReactNode) => (
  <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    {children}
  </svg>
);

const Icons = {
  play: svg(<path d="M7 4.5v15l12-7.5z" />),
  pause: svg(<><path d="M8 5v14" /><path d="M16 5v14" /></>),
  back: svg(<><path d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3" /><path d="M4 3.5v4h4" /></>),
  forward: svg(<><path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3" /><path d="M20 3.5v4h-4" /></>),
  volume: svg(<><path d="M4 9.5v5h3.5L12 18.5v-13L7.5 9.5z" /><path d="M15.5 9a4 4 0 0 1 0 6" /><path d="M18 6.5a7.5 7.5 0 0 1 0 11" /></>),
  muted: svg(<><path d="M4 9.5v5h3.5L12 18.5v-13L7.5 9.5z" /><path d="M16 9.5l5 5" /><path d="M21 9.5l-5 5" /></>),
  pip: svg(<><rect x="3" y="5" width="18" height="14" rx="2.5" /><rect x="12" y="11" width="6.5" height="5.5" rx="1.2" /></>),
  fullscreen: svg(<><path d="M4 9V4h5" /><path d="M20 9V4h-5" /><path d="M4 15v5h5" /><path d="M20 15v5h-5" /></>),
  exitFullscreen: svg(<><path d="M9 4v5H4" /><path d="M15 4v5h5" /><path d="M9 20v-5H4" /><path d="M15 20v-5h5" /></>),
  next: svg(<><path d="M5.5 5v14l10-7z" /><path d="M19 5v14" /></>),
  close: svg(<><path d="M6 6l12 12" /><path d="M18 6L6 18" /></>),
};

interface Props {
  snapshot: PlayerSnapshot | null;
  /** Live streams have no useful timeline. */
  live?: boolean;
  fullscreen: boolean;
  onToggleFullscreen(): void;
  /** Shown as an ✕ only when given (embedded preview); the full player closes with ← / Esc. */
  onClose?(): void;
  onPictureInPicture?(): void;
  /** Full player: the subtitle menu, instead of the plain list of the file's tracks. */
  subtitles?: ReactNode;
  /** A series with a following episode: opens its sources. */
  next?: { label: string; disabled: boolean; open(): void };
  /** Hold the forward button to play faster (the original's holdToSpeed). */
  holdSpeed?: { enabled: boolean; value: number };
  /** Opens the chapters panel when the video provides chapters. */
  onChapters?(): void;
  /** Opens the sources of the current video without leaving the player. */
  onSources?(): void;
}

export function PlayerControls({ snapshot: s, live, fullscreen, onToggleFullscreen, onClose, onPictureInPicture, subtitles, next, holdSpeed, onChapters, onSources }: Props) {
  const holdTimer = useRef<number>(0);
  const holdActive = useRef(false);
  const holdSpeedBefore = useRef<number>(1);
  const startHold = () => {
    if (!holdSpeed?.enabled) return;
    holdSpeedBefore.current = s?.speed ?? 1;
    holdActive.current = false;
    window.clearTimeout(holdTimer.current);
    holdTimer.current = window.setTimeout(() => {
      holdActive.current = true;
      void api.playerCommand({ type: "setSpeed", speed: holdSpeed.value });
    }, 350);
  };
  const endHold = () => {
    window.clearTimeout(holdTimer.current);
    if (holdActive.current) {
      holdActive.current = false;
      void api.playerCommand({ type: "setSpeed", speed: holdSpeedBefore.current });
    } else if (holdSpeed?.enabled) {
      void api.playerCommand({ type: "seek", seconds: 10, relative: true });
    }
  };
  const [seekDraft, setSeekDraft] = useState<number | null>(null);
  const [mode, setMode] = useState<ResizeMode>("fit");
  const duration = s?.duration ?? 0;
  const timePos = s?.timePos ?? 0;
  const position = seekDraft ?? (duration > 0 ? timePos / duration : 0);
  const buffered = duration > 0 ? Math.min(1, Math.max(position, (timePos + Math.max(0, s?.cacheSeconds ?? 0)) / duration)) : 0;
  const audio = s?.tracks.filter((t) => t.kind === "audio") ?? [];
  const subs = s?.tracks.filter((t) => t.kind === "sub") ?? [];
  const volume = Math.round(s?.volume ?? 100);

  useEffect(() => setSeekDraft(null), [s?.session]);

  const status = !s
    ? t("Abrindo…")
    : s.error
      ? t("Erro: {error}", { error: s.error })
      : s.buffering || !s.loaded
        ? t("Carregando…")
        : s.ended
          ? reachedEnd(s) || live ? t("Fim") : t("A transmissão parou antes do fim. Seu ponto ficou salvo.")
          : null;

  const pct = (v: number) => `${(v * 100).toFixed(3)}%`;

  const volumeControl = (
    <label className="volume">
      <button className="pc-icon" aria-label={s?.mute ? t("Ativar som") : t("Silenciar")} onClick={() => void api.playerCommand({ type: "setMute", mute: !s?.mute })}>
        {s?.mute ? Icons.muted : Icons.volume}
      </button>
      <input
        className="pc-volume"
        type="range"
        min={0}
        max={200}
        value={volume}
        aria-label={t("Volume")}
        style={{ ["--fill" as string]: `${(volume / 200) * 100}%` }}
        onChange={(e) => void api.playerCommand({ type: "setVolume", volume: Number(e.target.value) })}
      />
    </label>
  );

  return (
    <div className="controls" onDoubleClick={onToggleFullscreen}>
      {status && <div className="player-status">{status}</div>}
      {!live && duration > 0 && (
        <div className="timeline" onDoubleClick={(e) => e.stopPropagation()}>
          <div className="timeline-track" />
          <div className="timeline-buffered" style={{ width: pct(buffered) }} />
          <div className="timeline-played" style={{ width: pct(position) }} />
          <div className="timeline-dot" style={{ left: pct(position) }} />
          <input
            className="seek"
            type="range"
            min={0}
            max={1000}
            value={Math.round(position * 1000)}
            aria-label={t("Posição")}
            onChange={(e) => setSeekDraft(Number(e.target.value) / 1000)}
            onPointerUp={() => {
              if (seekDraft !== null) void api.playerCommand({ type: "seek", seconds: seekDraft * duration, relative: false });
              setSeekDraft(null);
            }}
            onKeyUp={() => {
              if (seekDraft !== null) void api.playerCommand({ type: "seek", seconds: seekDraft * duration, relative: false });
              setSeekDraft(null);
            }}
          />
        </div>
      )}
      <div className="row" onDoubleClick={(e) => e.stopPropagation()}>
        <button className="pc-btn pc-play" aria-label={s?.paused ? t("Reproduzir") : t("Pausar")} onClick={() => void api.playerCommand({ type: "togglePause" })}>
          {s?.paused ? Icons.play : Icons.pause}
        </button>
        {!live && (
          <>
            <button className="pc-btn pc-skip" aria-label={t("Voltar 10 s")} onClick={() => void api.playerCommand({ type: "seek", seconds: -10, relative: true })}>
              {Icons.back}<span>10</span>
            </button>
            <button
              className={`pc-btn pc-skip ${holdActive.current ? "holding" : ""}`}
              aria-label={holdSpeed?.enabled ? t("Avançar 10 s (segure para acelerar)") : t("Avançar 10 s")}
              data-testid="forward-10"
              onPointerDown={holdSpeed?.enabled ? startHold : undefined}
              onPointerUp={holdSpeed?.enabled ? endHold : undefined}
              onPointerLeave={holdSpeed?.enabled ? endHold : undefined}
              onPointerCancel={holdSpeed?.enabled ? endHold : undefined}
              onClick={holdSpeed?.enabled ? undefined : () => void api.playerCommand({ type: "seek", seconds: 10, relative: true })}
            >
              <span>10</span>{Icons.forward}
            </button>
            <span className="time">
              <b>{formatTime(timePos)}</b> <span className="time-sep">/</span> {formatTime(duration)}
            </span>
          </>
        )}
        {/* Live: play/pause, volume, then the AO VIVO mark. */}
        {live && <span className="pc-live-volume">{volumeControl}</span>}
        {live && <span className="live-dot">{t("AO VIVO")}</span>}
        {next && (
          <button className="pc-btn pc-next" aria-label={t("Próximo episódio")} title={next.label} disabled={next.disabled} data-testid="next-episode" onClick={next.open}>
            {Icons.next}<span>{t("Próximo episódio")}</span>
          </button>
        )}
        {onSources && (
          <button className="pc-btn pc-chapters" aria-label={t("Fontes no player")} data-testid="player-sources" onClick={onSources}>
            {t("Fontes")}
          </button>
        )}
        {onChapters && (
          <button className="pc-btn pc-chapters" aria-label={t("Capítulos")} data-testid="chapters" onClick={onChapters}>
            {t("Capítulos")}
          </button>
        )}
        <span className="spacer" />
        <div className="pc-panel">
          {!live && volumeControl}
          {audio.length > 1 && (
            <select className="pc-select" aria-label={t("Faixa de áudio")} value={s?.aid ?? "no"} onChange={(e) => void api.playerCommand({ type: "setAudioTrack", id: e.target.value })}>
              {audio.map((t) => <option key={t.id} value={String(t.id)}>{trackLabel(t)}</option>)}
            </select>
          )}
          {subtitles}
          {!subtitles && subs.length > 0 && (
            <select className="pc-select" aria-label={t("Legenda")} value={s?.sid && s.sid !== "no" ? s.sid : "no"} onChange={(e) => void api.playerCommand({ type: "setSubtitleTrack", id: e.target.value })}>
              <option value="no">{t("Sem legenda")}</option>
              {subs.map((t) => <option key={t.id} value={String(t.id)}>{trackLabel(t)}</option>)}
            </select>
          )}
          {!live && (
            <select className="pc-select pc-plain" aria-label={t("Velocidade")} value={String(s?.speed ?? 1)} onChange={(e) => void api.playerCommand({ type: "setSpeed", speed: Number(e.target.value) })}>
              {[0.5, 0.75, 1, 1.25, 1.5, 2].map((v) => <option key={v} value={String(v)}>{v}x</option>)}
            </select>
          )}
          {!live && (
            <select className="pc-select pc-pill" aria-label={t("Modo de imagem")} value={mode} onChange={(e) => {
              const next = e.target.value as ResizeMode;
              setMode(next);
              void api.playerCommand({ type: "setResizeMode", mode: next });
            }}>
              <option value="fit">{t("Ajustar")}</option>
              <option value="fill">{t("Preencher")}</option>
              <option value="zoom">{t("Zoom")}</option>
              <option value="stretch">{t("Esticar")}</option>
            </select>
          )}
          {!live && s && <EnhanceMenu key={`enhance-${s.session}`} snapshot={s} />}
          {onPictureInPicture && <button className="pc-icon" aria-label={t("Picture-in-picture")} onClick={onPictureInPicture}>{Icons.pip}</button>}
          {onClose && <button className="pc-icon" aria-label={t("Fechar")} onClick={onClose}>{Icons.close}</button>}
          <button className="pc-icon" aria-label={fullscreen ? t("Sair da tela cheia") : t("Tela cheia")} onClick={onToggleFullscreen}>
            {fullscreen ? Icons.exitFullscreen : Icons.fullscreen}
          </button>
        </div>
      </div>
    </div>
  );
}

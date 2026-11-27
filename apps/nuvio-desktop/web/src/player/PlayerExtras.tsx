// Player extras of the original, over the video:
// - EpisodePicker: the series' episodes by season, to switch without leaving the player;
// - ChaptersMenu: the file's chapters (also marked on the timeline);
// - AudioMenu: audio track, delay, amplification and downmix;
// - ContentWarnings: IMDb parents guide for a few seconds at the start;
// - ShortcutsHelp: the keyboard and mouse controls ("?").

import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { MetaDetails, MetaVideo, ParentalWarning, PlayerSnapshot } from "../types";
import { decimal, t } from "../i18n";
import { formatTime, trackLabel } from "./PlayerControls";
import { episodeTitle, hasAired } from "./episodes";

// ---------------------------------------------------------------- popover helper

function usePopover() {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key !== "Escape") return;
      if (e instanceof PointerEvent && ref.current?.contains(e.target as Node)) return;
      if (e instanceof KeyboardEvent) e.stopPropagation();
      setOpen(false);
    };
    window.addEventListener("pointerdown", close, true);
    window.addEventListener("keydown", close, true);
    return () => {
      window.removeEventListener("pointerdown", close, true);
      window.removeEventListener("keydown", close, true);
    };
  }, [open]);
  return { open, setOpen, ref };
}

const icon = (d: React.ReactNode) => (
  <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round" aria-hidden>{d}</svg>
);

// ---------------------------------------------------------------- episodes

export function EpisodePicker({ meta, current, onPick }: { meta: MetaDetails; current: MetaVideo | null; onPick(video: MetaVideo): void }) {
  const seasons = [...new Set(meta.videos.filter((v) => v.season != null).map((v) => v.season!))].sort((a, b) => (a === 0 ? 1 : b === 0 ? -1 : a - b));
  const [season, setSeason] = useState<number | null>(current?.season ?? seasons.find((s) => s > 0) ?? seasons[0] ?? null);
  const list = meta.videos.filter((v) => v.season === season).sort((a, b) => (a.episode ?? 0) - (b.episode ?? 0));
  const currentRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    currentRef.current?.scrollIntoView({ block: "center" });
  }, [season]);
  return (
    <div className="player-episodes" data-testid="player-episodes">
      <div className="chips player-seasons">
        {seasons.map((s) => (
          <button key={s} className={`chip ${s === season ? "active" : ""}`} onClick={() => setSeason(s)}>
            {s === 0 ? t("Especiais") : t("Temporada {n}", { n: s })}
          </button>
        ))}
      </div>
      <div className="player-episode-list">
        {list.map((v) => {
          const aired = hasAired(v.released);
          const isCurrent = current?.id === v.id;
          return (
            <button
              key={v.id}
              ref={isCurrent ? currentRef : undefined}
              className={`player-episode ${isCurrent ? "current" : ""}`}
              disabled={!aired}
              onClick={() => onPick(v)}
              data-episode={`${v.season}:${v.episode}`}
            >
              {v.thumbnail ? <img src={v.thumbnail} alt="" loading="lazy" /> : <span className="player-episode-thumb" />}
              <span className="player-episode-text">
                <b>{episodeTitle(v)}</b>
                {isCurrent && <span className="muted small">{t("Tocando agora")}</span>}
                {!aired && <span className="muted small">{t("Ainda não exibido")}</span>}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- chapters

export function ChaptersMenu({ snapshot }: { snapshot: PlayerSnapshot }) {
  const { open, setOpen, ref } = usePopover();
  const chapters = snapshot.chapters;
  if (chapters.length < 2) return null;
  const now = snapshot.timePos;
  const currentIndex = chapters.reduce((at, c, i) => (c.time <= now + 0.5 ? i : at), 0);
  return (
    <div className="pc-popover-wrap" ref={ref}>
      <button className="pc-icon" aria-label={t("Capítulos")} title={t("Capítulos")} aria-expanded={open} onClick={() => setOpen(!open)} data-testid="player-chapters">
        {icon(<><path d="M4 6h11" /><path d="M4 12h16" /><path d="M4 18h9" /><circle cx="19" cy="6" r="1.4" /></>)}
      </button>
      {open && (
        <div className="pc-popover chapters-pop" role="menu">
          <div className="pc-popover-title">{t("Capítulos")}</div>
          {chapters.map((c, i) => (
            <button
              key={`${i}-${c.time}`}
              role="menuitem"
              className={`chapter-item ${i === currentIndex ? "current" : ""}`}
              onClick={() => {
                void api.playerCommand({ type: "seek", seconds: c.time, relative: false });
                setOpen(false);
              }}
            >
              <span className="chapter-time">{formatTime(c.time)}</span>
              <span>{c.title || t("Capítulo {n}", { n: i + 1 })}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** Chapter marks on the timeline. */
export function ChapterTicks({ snapshot }: { snapshot: PlayerSnapshot }) {
  if (snapshot.duration <= 0 || snapshot.chapters.length < 2) return null;
  return (
    <>
      {snapshot.chapters.slice(1).map((c, i) => (
        <span key={i} className="timeline-chapter" style={{ left: `${(c.time / snapshot.duration) * 100}%` }} aria-hidden />
      ))}
    </>
  );
}

// ---------------------------------------------------------------- audio

export function AudioMenu({ snapshot, gainDb, onGain, downmix, onDownmix }: { snapshot: PlayerSnapshot; gainDb: number; onGain(db: number): void; downmix: boolean; onDownmix(v: boolean): void }) {
  const { open, setOpen, ref } = usePopover();
  const audio = snapshot.tracks.filter((tr) => tr.kind === "audio");
  const delay = (ms: number) => void api.playerCommand({ type: "setAudioDelay", ms: Math.max(-10000, Math.min(10000, ms)) });
  return (
    <div className="pc-popover-wrap" ref={ref}>
      <button className="pc-icon" aria-label={t("Áudio")} title={t("Áudio")} aria-expanded={open} onClick={() => setOpen(!open)} data-testid="player-audio">
        {icon(<><path d="M3 10v4" /><path d="M7 7v10" /><path d="M11 4v16" /><path d="M15 8v8" /><path d="M19 11v2" /></>)}
      </button>
      {open && (
        <div className="pc-popover audio-pop" role="dialog" aria-label={t("Áudio")}>
          {audio.length > 0 && (
            <>
              <div className="pc-popover-title">{t("Faixa de áudio")}</div>
              <div className="audio-tracks">
                {audio.map((tr) => (
                  <button key={tr.id} className={`chapter-item ${String(tr.id) === snapshot.aid ? "current" : ""}`} onClick={() => void api.playerCommand({ type: "setAudioTrack", id: String(tr.id) })}>
                    {trackLabel(tr)}
                  </button>
                ))}
              </div>
            </>
          )}
          <div className="pc-popover-title">{t("Atraso do áudio")}</div>
          <div className="pc-delay">
            <button className="pc-level" onClick={() => delay(snapshot.audioDelayMs - 100)}>−0,1 s</button>
            <span className="pc-delay-value" data-testid="audio-delay">{decimal(snapshot.audioDelayMs / 1000, 1)} s</span>
            <button className="pc-level" onClick={() => delay(snapshot.audioDelayMs + 100)}>+0,1 s</button>
            {snapshot.audioDelayMs !== 0 && <button className="link" onClick={() => delay(0)}>{t("Zerar")}</button>}
          </div>
          <div className="pc-popover-title">{t("Amplificação")}</div>
          <label className="audio-gain">
            <input type="range" min={0} max={10} step={1} value={gainDb} aria-label={t("Amplificação")} onChange={(e) => onGain(Number(e.target.value))} data-testid="audio-gain" />
            <output>+{gainDb} dB</output>
          </label>
          <label className="audio-downmix">
            <input type="checkbox" checked={downmix} onChange={(e) => onDownmix(e.target.checked)} data-testid="audio-downmix" />
            {t("Converter para estéreo (downmix)")}
          </label>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- content warnings

const CATEGORY: Record<string, string> = {
  nudity: "Nudez",
  violence: "Violência",
  profanity: "Linguagem",
  alcohol: "Álcool e drogas",
  frightening: "Cenas assustadoras",
};
const SEVERITY: Record<string, string> = { severe: "Forte", moderate: "Moderado", mild: "Leve" };

export function ContentWarnings({ id, started }: { id: string; started: boolean }) {
  const [warnings, setWarnings] = useState<ParentalWarning[]>([]);
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    let cancelled = false;
    api.parentalGuide(id).then((w) => !cancelled && setWarnings(w), () => {});
    return () => {
      cancelled = true;
    };
  }, [id]);
  useEffect(() => {
    if (!started || !warnings.length) return;
    setVisible(true);
    const timer = window.setTimeout(() => setVisible(false), 7000);
    return () => window.clearTimeout(timer);
  }, [started, warnings]);
  if (!visible) return null;
  return (
    <div className="content-warnings" role="status" data-testid="content-warnings">
      <div className="content-warnings-title">{t("Avisos de conteúdo")}</div>
      {warnings.map((w) => (
        <div key={w.category} className={`content-warning ${w.severity}`}>
          <span>{t(CATEGORY[w.category] ?? w.category)}</span>
          <b>{t(SEVERITY[w.severity] ?? w.severity)}</b>
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------- shortcuts

export const SHORTCUTS: [string, string][] = [
  ["Espaço / K / clique no vídeo", "Reproduzir ou pausar"],
  ["← → / J L", "Voltar ou avançar 10 s"],
  ["Shift + ← →", "Voltar ou avançar 30 s"],
  ["0 – 9", "Ir para 0% – 90% do vídeo"],
  ["Home", "Voltar ao início"],
  [", .", "Quadro anterior ou seguinte (pausado)"],
  ["< >", "Diminuir ou aumentar a velocidade"],
  ["Segurar o clique no vídeo / segurar L", "Acelerar enquanto segura"],
  ["↑ ↓ / roda do mouse", "Volume"],
  ["M", "Silenciar"],
  ["C", "Mostrar ou ocultar a legenda"],
  ["A", "Próxima faixa de áudio"],
  ["F / clique duplo", "Tela cheia"],
  ["P", "Picture-in-picture"],
  ["N", "Próximo episódio"],
  ["E", "Lista de episódios"],
  ["O", "Trocar fonte"],
  ["?", "Esta ajuda"],
  ["Esc", "Sair do player"],
];

export function ShortcutsHelp({ onClose }: { onClose(): void }) {
  return (
    <div className="shortcuts-layer" onClick={onClose} data-testid="shortcuts-help">
      <div className="shortcuts" onClick={(e) => e.stopPropagation()} role="dialog" aria-label={t("Atalhos do player")}>
        <header>
          <h2>{t("Atalhos do player")}</h2>
          <button className="icon-btn" aria-label={t("Fechar")} onClick={onClose}>✕</button>
        </header>
        <dl>
          {SHORTCUTS.map(([keys, action]) => (
            <div key={keys} className="shortcut">
              <dt>{t(keys)}</dt>
              <dd>{t(action)}</dd>
            </div>
          ))}
        </dl>
      </div>
    </div>
  );
}

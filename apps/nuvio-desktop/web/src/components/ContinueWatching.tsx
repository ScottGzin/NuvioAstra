// Continue watching: started, unfinished movies, the latest episode of each series and, when that
// one is finished, the next episode (the original's rules, computed in Rust). The row draws at
// once from what is known; the episode lists of the next episodes complete it.

import { memo, useEffect, useState } from "react";
import { api } from "../api";
import { clock, episodeCode, lastWatched, remaining, removeHistory, useLibraryVersion } from "../library";
import type { ContinueWatchingSettings, HistoryEntry } from "../types";
import { airDate } from "../player/episodes";
import { t } from "../i18n";

interface CardProps {
  entry: HistoryEntry;
  onResume(entry: HistoryEntry): void;
  onRemove?(entry: HistoryEntry): void;
  blurred?: boolean;
  style?: string;
}

export const ContinueCard = memo(function ContinueCard({ entry, onResume, onRemove, blurred, style }: CardProps) {
  const image = entry.background ?? entry.poster;
  const code = episodeCode(entry);
  const left = remaining(entry);
  const fresh = entry.nextUp && !entry.inProgress;
  return (
    <div className={`cw-wrap ${blurred ? "cw-blurred" : ""} ${style === "wide" ? "cw-wide-card" : ""}`}>
      <button className={`cw-card ${style === "wide" ? "cw-wide-layout" : ""}`} onClick={() => onResume(entry)} data-key={entry.key} data-next-up={entry.nextUp || undefined} title={fresh ? t("Próximo episódio de {title}", { title: entry.title }) : t("Continuar {title}", { title: entry.title })}>
        <div className="cw-art">
          {image ? <img src={image} alt="" loading="eager" decoding="async" draggable={false} /> : <span className="card-fallback">{entry.title}</span>}
          <span className="cw-play" aria-hidden>▶</span>
          {entry.nextUp && <span className="cw-badge">{entry.airs ? t("Estreia em {date}", { date: airDate(entry.airs) }) : t("Próximo episódio")}</span>}
          {style !== "wide" && !fresh && (
            <div className="progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(entry.fraction * 100)}>
              <div style={{ width: `${Math.max(2, entry.fraction * 100)}%` }} />
            </div>
          )}
        </div>
        <div className="cw-text">
          <div className="card-title">{entry.title}</div>
          <div className="card-sub">
            {[code && `${code}${entry.episodeTitle ? ` · ${entry.episodeTitle}` : ""}`].filter(Boolean).join("") || entry.metaType}
          </div>
          <div className="card-sub">
            {[entry.positionMs > 0 && t("{time} assistidos", { time: clock(entry.positionMs) }), left].filter(Boolean).join(" · ")}
          </div>
          <div className="card-sub">{lastWatched(entry.lastWatchedEpochMs)}</div>
          {style === "wide" && !fresh && (
            <div className="progress cw-progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(entry.fraction * 100)}>
              <div style={{ width: `${Math.max(2, entry.fraction * 100)}%` }} />
            </div>
          )}
        </div>
      </button>
      {onRemove && (
        <button className="fav-btn icon" aria-label={t("Remover {title} de Continuar assistindo", { title: entry.title })} title={t("Remover")} data-testid="cw-remove" onClick={() => onRemove(entry)}>
          ✕
        </button>
      )}
    </div>
  );
});

/** Reads the row at once and again when the next episodes are known. */
export function loadContinueWatching(show: (items: HistoryEntry[]) => void, failed?: (e: unknown) => void): () => void {
  let cancelled = false;
  void (async () => {
    try {
      const known = await api.libraryContinueWatching();
      if (cancelled) return;
      show(known);
    } catch (e) {
      if (!cancelled) failed?.(e);
      return;
    }
    try {
      const complete = await api.libraryNextUp();
      if (!cancelled) show(complete);
    } catch {
      /* the addons did not answer: what is known stays */
    }
  })();
  return () => {
    cancelled = true;
  };
}

const DEFAULT_PREFS: ContinueWatchingSettings = {
  visible: true, style: "wide", upNextFromFurthestEpisode: true, useEpisodeThumbnails: true,
  showUnairedNextUp: true, blurNextUp: false, showResumePromptOnLaunch: false, sortMode: "default",
  clickAction: "stream",
};

/** Home row (hidden when empty), like the original's Continue watching section at the top. */
export function ContinueWatchingRow({
  onResume,
  prefs = DEFAULT_PREFS,
  onOpenDetails,
}: {
  onResume(entry: HistoryEntry): void;
  prefs?: ContinueWatchingSettings;
  onOpenDetails?(entry: HistoryEntry): void;
}) {
  const version = useLibraryVersion();
  const [items, setItems] = useState<HistoryEntry[]>([]);
  useEffect(() => loadContinueWatching(setItems), [version]);
  if (!prefs.visible) return null;
  let shown = items;
  if (!prefs.showUnairedNextUp) {
    const now = Date.now();
    shown = shown.filter((entry) => !entry.nextUp || !entry.airs || new Date(entry.airs).getTime() <= now);
  }
  if (prefs.sortMode === "streaming" || prefs.sortMode === "splitUpcoming") {
    const released = shown.filter((entry) => !entry.nextUp || !entry.airs || new Date(entry.airs).getTime() <= Date.now());
    const upcoming = shown.filter((entry) => !released.includes(entry));
    upcoming.sort((a, b) => (a.airs ?? "9999").localeCompare(b.airs ?? "9999"));
    shown = [...released, ...upcoming];
  }
  if (!shown.length) return null;
  const play = (entry: HistoryEntry) => {
    const details = prefs.clickAction === "details" || (prefs.clickAction === "detailsForUpcoming" && entry.nextUp);
    if (details && onOpenDetails) onOpenDetails(entry);
    else onResume(entry);
  };
  return (
    <section className={`row-section cw-${prefs.style}`} data-testid="home-continue">
      <header className="row-header">
        <h2>{t("Continuar assistindo")}</h2>
      </header>
      <div className="row-track">
        {shown.map((e) => (
          <ContinueCard key={e.key} entry={e} onResume={play} onRemove={(x) => void removeHistory(x.key)} blurred={prefs.blurNextUp && e.nextUp} style={prefs.style} />
        ))}
      </div>
    </section>
  );
}

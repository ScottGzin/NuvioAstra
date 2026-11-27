// Continue watching: started, unfinished movies, the latest episode of each series and, when that
// one is finished, the next episode (the original's rules, computed in Rust). The row draws at
// once from what is known; the episode lists of the next episodes complete it.
// Settings → Continuar assistindo: show the row, card style, episode pictures, episodes not out
// yet (shown, blurred or hidden), order and what a click does.

import { memo, useEffect, useState } from "react";
import { api } from "../api";
import { clock, episodeCode, lastWatched, remaining, removeHistory, useLibraryVersion } from "../library";
import type { HistoryEntry, ScreenSettings } from "../types";
import { airDate } from "../player/episodes";
import { useScreens } from "../screens";
import { t } from "../i18n";

interface CardProps {
  entry: HistoryEntry;
  onResume(entry: HistoryEntry): void;
  onRemove?(entry: HistoryEntry): void;
  /** Settings → Continuar assistindo (default look when absent). */
  look?: Pick<ScreenSettings, "cwStyle" | "cwEpisodeThumbnails" | "detailsBlurUnwatched">;
}

export const ContinueCard = memo(function ContinueCard({ entry, onResume, onRemove, look }: CardProps) {
  const style = look?.cwStyle ?? "card";
  const image = style === "poster"
    ? entry.poster ?? entry.background
    : (look?.cwEpisodeThumbnails !== false ? entry.episodeThumbnail : null) ?? entry.background ?? entry.poster;
  // "Desfocar episódios não assistidos" (one option for every screen): a next episode not started.
  const blurred = !!look?.detailsBlurUnwatched && entry.nextUp && !entry.inProgress;
  const code = episodeCode(entry);
  const left = remaining(entry);
  const fresh = entry.nextUp && !entry.inProgress;
  return (
    <div className={`cw-wrap cw-style-${style}`}>
      <button className={`cw-card ${blurred ? "cw-blurred" : ""}`} onClick={() => onResume(entry)} data-key={entry.key} data-next-up={entry.nextUp || undefined} title={fresh ? t("Próximo episódio de {title}", { title: entry.title }) : t("Continuar {title}", { title: entry.title })}>
        <div className="cw-art">
          {image ? <img src={image} alt="" loading="lazy" decoding="async" draggable={false} /> : <span className="card-fallback">{entry.title}</span>}
          <span className="cw-play" aria-hidden>▶</span>
          {entry.nextUp && <span className="cw-badge">{entry.airs ? t("Estreia em {date}", { date: airDate(entry.airs) }) : t("Próximo episódio")}</span>}
          {!fresh && (
            <div className="progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(entry.fraction * 100)}>
              <div style={{ width: `${Math.max(2, entry.fraction * 100)}%` }} />
            </div>
          )}
        </div>
        <div className="card-title">{entry.title}</div>
        <div className="card-sub">
          {[code && `${code}${entry.episodeTitle ? ` · ${entry.episodeTitle}` : ""}`].filter(Boolean).join("")}
        </div>
        <div className="card-sub">
          {[entry.positionMs > 0 && t("{time} assistidos", { time: clock(entry.positionMs) }), left].filter(Boolean).join(" · ")}
        </div>
        <div className="card-sub">{lastWatched(entry.lastWatchedEpochMs)}</div>
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

const upcoming = (e: HistoryEntry) => e.nextUp && !!e.airs;

/** The row's entries with the settings applied: episodes not out yet and the order
 *  (`ContinueWatchingSortMode`: DEFAULT, STREAMING_STYLE — started first, then next episodes —
 *  and SPLIT_UPCOMING — episodes not out yet at the end). */
export function arrangeContinueWatching(items: HistoryEntry[], s: Pick<ScreenSettings, "cwShowUnaired" | "cwSort">): HistoryEntry[] {
  const shown = s.cwShowUnaired ? items : items.filter((e) => !upcoming(e));
  if (s.cwSort === "streaming") {
    const started = shown.filter((e) => !e.nextUp || e.inProgress);
    const next = shown.filter((e) => e.nextUp && !e.inProgress);
    return [...started.sort((a, b) => b.lastWatchedEpochMs - a.lastWatchedEpochMs), ...next];
  }
  if (s.cwSort === "split") return [...shown.filter((e) => !upcoming(e)), ...shown.filter(upcoming)];
  return shown;
}

/** Home row (hidden when empty), like the original's Continue watching section at the top. */
export function ContinueWatchingRow({ onResume, onOpenEntry }: { onResume(entry: HistoryEntry): void; onOpenEntry?(entry: HistoryEntry): void }) {
  const version = useLibraryVersion();
  const screens = useScreens();
  const [all, setItems] = useState<HistoryEntry[]>([]);
  useEffect(() => loadContinueWatching(setItems), [version]);
  const items = arrangeContinueWatching(all, screens);
  if (!screens.cwVisible || !items.length) return null;
  // `ContinueWatchingClickAction`: STREAM (sources, at the saved point), DETAILS, or DETAILS only
  // for episodes not out yet.
  const click = (e: HistoryEntry) => {
    const details = screens.cwClick === "details" || (screens.cwClick === "detailsForUpcoming" && upcoming(e));
    if (details && onOpenEntry) onOpenEntry(e);
    else onResume(e);
  };
  return (
    <section className="row-section" data-testid="home-continue">
      <header className="row-header">
        <h2>{t("Continuar assistindo")}</h2>
      </header>
      <div className={`row-track cw-track-${screens.cwStyle}`}>
        {items.map((e) => (
          <ContinueCard key={e.key} entry={e} onResume={click} onRemove={(x) => void removeHistory(x.key)} look={screens} />
        ))}
      </div>
    </section>
  );
}

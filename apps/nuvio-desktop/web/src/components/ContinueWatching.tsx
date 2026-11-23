// Continue watching: started, unfinished movies and the latest episode of each series (the
// original's rule, computed in Rust from NextGen's own data). A click resumes from the saved
// position; nothing is asked to the addons until the user picks the source.

import { memo, useEffect, useState } from "react";
import { api } from "../api";
import { clock, episodeCode, lastWatched, remaining, removeHistory, useLibraryVersion } from "../library";
import type { HistoryEntry } from "../types";
import { t } from "../i18n";

interface CardProps {
  entry: HistoryEntry;
  onResume(entry: HistoryEntry): void;
  onRemove?(entry: HistoryEntry): void;
}

export const ContinueCard = memo(function ContinueCard({ entry, onResume, onRemove }: CardProps) {
  const image = entry.background ?? entry.poster;
  const code = episodeCode(entry);
  const left = remaining(entry);
  return (
    <div className="cw-wrap">
      <button className="cw-card" onClick={() => onResume(entry)} data-key={entry.key} title={`Continuar ${entry.title}`}>
        <div className="cw-art">
          {image ? <img src={image} alt="" loading="lazy" decoding="async" draggable={false} /> : <span className="card-fallback">{entry.title}</span>}
          <span className="cw-play" aria-hidden>▶</span>
          <div className="progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(entry.fraction * 100)}>
            <div style={{ width: `${Math.max(2, entry.fraction * 100)}%` }} />
          </div>
        </div>
        <div className="card-title">{entry.title}</div>
        <div className="card-sub">
          {[code && `${code}${entry.episodeTitle ? ` · ${entry.episodeTitle}` : ""}`].filter(Boolean).join("")}
        </div>
        <div className="card-sub">
          {[entry.positionMs > 0 && `${clock(entry.positionMs)} assistidos`, left].filter(Boolean).join(" · ")}
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

/** Home row (hidden when empty), like the original's Continue watching section at the top. */
export function ContinueWatchingRow({ onResume }: { onResume(entry: HistoryEntry): void }) {
  const version = useLibraryVersion();
  const [items, setItems] = useState<HistoryEntry[]>([]);
  useEffect(() => {
    let cancelled = false;
    api.libraryContinueWatching().then((i) => !cancelled && setItems(i), () => {});
    return () => {
      cancelled = true;
    };
  }, [version]);
  if (!items.length) return null;
  return (
    <section className="row-section" data-testid="home-continue">
      <header className="row-header">
        <h2>{t("Continuar assistindo")}</h2>
      </header>
      <div className="row-track">
        {items.map((e) => (
          <ContinueCard key={e.key} entry={e} onResume={onResume} onRemove={(x) => void removeHistory(x.key)} />
        ))}
      </div>
    </section>
  );
}

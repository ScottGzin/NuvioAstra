// Chapters of the current video, over the player (the original's chapter list). Seeking uses the
// same command as the timeline; the panel closes when a chapter is chosen or Escape is pressed.

import { useEffect } from "react";
import { api } from "../api";
import type { Chapter } from "../types";
import { formatTime } from "./PlayerControls";
import { t } from "../i18n";

interface Props {
  chapters: Chapter[];
  onClose(): void;
}

export function ChaptersPanel({ chapters, onClose }: Props) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="chapters-overlay" role="dialog" aria-modal="true" aria-label={t("Capítulos")} data-testid="chapters-panel">
      <div className="chapters-panel">
        <header className="modal-header">
          <h2>{t("Capítulos")}</h2>
          <button className="icon-btn" aria-label={t("Fechar")} onClick={onClose}>✕</button>
        </header>
        <div className="chapters-list">
          {chapters.map((chapter, index) => (
            <button
              key={`${chapter.time}-${index}`}
              className="chapter-row"
              onClick={() => { void api.playerCommand({ type: "seek", seconds: chapter.time, relative: false }); onClose(); }}
            >
              <span className="chapter-time">{formatTime(chapter.time)}</span>
              <span>{chapter.title || `${t("Capítulo")} ${index + 1}`}</span>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

// "Resume prompt on launch" (the original's ContinueWatchingPreferences): asks once per app start
// whether to continue the most recently watched title.

import type { HistoryEntry } from "../types";
import { clock } from "../library";
import { t } from "../i18n";

interface Props {
  entry: HistoryEntry;
  onResume(): void;
  onDismiss(): void;
}

export function ResumePrompt({ entry, onResume, onDismiss }: Props) {
  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(e) => e.target === e.currentTarget && onDismiss()}>
      <div className="modal resume-prompt" role="dialog" aria-modal="true" aria-label={t("Continuar assistindo?")} data-testid="resume-prompt">
        <header className="modal-header">
          <h2>{t("Continuar assistindo?")}</h2>
          <button className="icon-btn" aria-label={t("Fechar")} onClick={onDismiss}>✕</button>
        </header>
        {entry.poster && <img className="resume-poster" src={entry.poster} alt="" />}
        <div className="setting-title">{entry.title}</div>
        <div className="muted small">{t("Continue de {time}", { time: clock(entry.positionMs) })}</div>
        <div className="modal-actions">
          <button className="primary" data-testid="resume-prompt-continue" onClick={onResume}>{t("Continuar")}</button>
          <button className="ghost" onClick={onDismiss}>{t("Agora não")}</button>
        </div>
      </div>
    </div>
  );
}

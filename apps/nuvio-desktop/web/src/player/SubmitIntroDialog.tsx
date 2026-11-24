// Manual IntroDB submission (the original's SubmitIntroDialog): capture start/end from the current
// playback position, choose the segment type, and send it through Rust (the API key stays there).

import { useState } from "react";
import { api, errorText } from "../api";
import { t } from "../i18n";

type Kind = "intro" | "recap" | "outro";

const parseTime = (value: string): number | null => {
  const raw = value.trim().replace(",", ".");
  if (!raw) return null;
  if (raw.includes(":")) {
    const parts = raw.split(":");
    if (parts.length !== 2) return null;
    const minutes = Number(parts[0]);
    const seconds = Number(parts[1]);
    if (!Number.isFinite(minutes) || !Number.isFinite(seconds) || seconds < 0 || seconds >= 60) return null;
    return minutes * 60 + seconds;
  }
  const seconds = Number(raw);
  return Number.isFinite(seconds) ? seconds : null;
};

const formatTime = (seconds: number): string => {
  const whole = Math.max(0, Math.floor(seconds));
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, "0")}`;
};

interface Props {
  session: number;
  position: number;
  duration: number;
  onClose(): void;
  onSubmitted(): void;
}

export function SubmitIntroDialog({ session, position, duration, onClose, onSubmitted }: Props) {
  const [kind, setKind] = useState<Kind>("intro");
  const [start, setStart] = useState(formatTime(position));
  const [end, setEnd] = useState(formatTime(Math.min(position + 90, duration)));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);
  const [captureField, setCaptureField] = useState<"start" | "end" | null>(null);

  const submit = async () => {
    const from = parseTime(start);
    const to = parseTime(end);
    if (from === null || to === null || to <= from || from < 0) {
      setError(t("Informe tempos válidos (MM:SS) com o fim depois do início."));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const ok = await api.skipSubmitTiming(session, kind, from, to);
      if (!ok) {
        setError(t("A IntroDB não aceitou o tempo."));
        return;
      }
      setDone(true);
      onSubmitted();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="comment-sheet-backdrop" role="presentation" onClick={onClose}>
      <div className="settings-group submit-intro-dialog" role="dialog" aria-modal="true" data-testid="submit-intro-dialog" onClick={(event) => event.stopPropagation()}>
        <div className="sources-header">
          <h2>{t("Enviar tempo à IntroDB")}</h2>
          <button className="icon-btn" aria-label={t("Fechar")} onClick={onClose}>✕</button>
        </div>
        <div className="pc-motion">
          {(["intro", "recap", "outro"] as Kind[]).map((option) => (
            <button key={option} className={`chip ${kind === option ? "active" : ""}`} onClick={() => setKind(option)}>
              {t(option === "intro" ? "Introdução" : option === "recap" ? "Recapitulação" : "Encerramento")}
            </button>
          ))}
        </div>
        <div className="pc-motion">
          <label className="pc-sub-slider">
            <span>{t("Início do trecho")}</span>
            <input value={start} aria-label={t("Início do trecho")} onChange={(e) => setStart(e.target.value)} />
            <button className="ghost" onClick={() => setCaptureField("start")}>{t("Capturar posição")}</button>
          </label>
          <label className="pc-sub-slider">
            <span>{t("Fim do trecho")}</span>
            <input value={end} aria-label={t("Fim do trecho")} onChange={(e) => setEnd(e.target.value)} />
            <button className="ghost" onClick={() => setCaptureField("end")}>{t("Capturar posição")}</button>
          </label>
        </div>
        {captureField && (
          <p className="muted small">
            {t("Posição atual")}: {formatTime(position)} ·
            <button className="link" onClick={() => {
              if (captureField === "start") setStart(formatTime(position));
              else setEnd(formatTime(position));
              setCaptureField(null);
            }}>{t("Usar")}</button>
          </p>
        )}
        {error && <p className="row-error" role="alert">{error}</p>}
        {done && <p className="muted small" role="status">{t("Tempo enviado à IntroDB.")}</p>}
        <div className="modal-actions">
          <button className="primary" data-testid="submit-intro-commit" disabled={busy} onClick={() => void submit()}>{t("Enviar")}</button>
          <button className="ghost" onClick={onClose}>{done ? t("Fechar") : t("Cancelar")}</button>
        </div>
      </div>
    </div>
  );
}

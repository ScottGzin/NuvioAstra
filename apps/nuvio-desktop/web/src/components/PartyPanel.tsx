// Watch-party overlay. Presentation only: the parent owns the connection and passes callbacks.

import { useEffect, useState } from "react";
import { randomRoomCode } from "../player/party";

interface PartyPanelProps {
  status: string;
  room: string;
  onJoin: (room: string) => void;
  onLeave: () => void;
  t: (s: string) => string;
  /** False hides join controls and explains that Supabase Realtime isn't configured. */
  configured?: boolean;
}

const dotFor = (status: string): string =>
  status === "joined" || status === "connected" ? "ok"
    : status === "connecting" || status === "reconnecting" ? "busy"
      : status === "error" ? "bad" : "off";

export function PartyPanel({ status, room, onJoin, onLeave, t, configured = true }: PartyPanelProps) {
  const [code, setCode] = useState(room);
  const [copied, setCopied] = useState(false);

  useEffect(() => { setCode(room); }, [room]);

  const join = (value: string) => {
    const next = value.trim().toUpperCase();
    if (!next) return;
    setCode(next);
    onJoin(next);
  };

  const copyLink = () => {
    try {
      const url = new URL(window.location.href);
      url.searchParams.set("party", room);
      void navigator.clipboard.writeText(url.toString()).then(() => {
        setCopied(true);
      }).catch(() => { setCopied(false); });
    } catch { setCopied(false); }
  };

  const inRoom = Boolean(room);

  return (
    <div className="pc-enhance-menu" data-testid="party-panel" data-status={status}>
      <div className="pc-enhance-title">{t("Assistir junto")}</div>
      <div className="pc-motion">
        <span
          className={`party-dot ${dotFor(status)}`}
          data-testid="party-status"
          data-status={status}
          aria-hidden
          style={{ width: 8, height: 8, borderRadius: "50%", display: "inline-block", background: dotFor(status) === "ok" ? "#4ade80" : dotFor(status) === "busy" ? "#fbbf24" : dotFor(status) === "bad" ? "#f87171" : "#9ca3af" }}
        />
        <span className="muted small">{t("Status")}: {t(status)}</span>
      </div>

      {!configured || status === "unconfigured" ? (
        <p className="muted small" data-testid="party-unconfigured">
          {t("Salas compartilhadas exigem a configuração do Supabase Realtime.")}
        </p>
      ) : (
        <>
          <div className="pc-motion">
            <input
              className="pc-level"
              data-testid="party-room-input"
              value={code}
              placeholder={t("Código da sala")}
              autoComplete="off"
              maxLength={12}
              onChange={(event) => setCode(event.target.value.toUpperCase())}
              onKeyDown={(event) => { if (event.key === "Enter") join(code); }}
            />
            <button className="pc-level" data-testid="party-join" onClick={() => join(code)}>{t("Entrar na sala")}</button>
          </div>
          <div className="pc-motion">
            <button className="pc-level" data-testid="party-create" onClick={() => join(randomRoomCode())}>{t("Criar sala")}</button>
            <button className="pc-level" data-testid="party-copy" disabled={!inRoom} onClick={copyLink}>
              {copied ? t("Link copiado") : t("Copiar link")}
            </button>
            <button className="pc-level" data-testid="party-leave" disabled={!inRoom} onClick={() => { onLeave(); setCode(""); }}>{t("Sair da sala")}</button>
          </div>
        </>
      )}

      <p className="muted small">
        {inRoom ? `${t("Código da sala")}: ${room}` : t("Crie uma sala ou entre com um código para assistir junto.")}
      </p>
    </div>
  );
}

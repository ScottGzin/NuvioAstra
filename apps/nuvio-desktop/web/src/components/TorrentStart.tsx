// Starting a torrent source (as the original's player does before playback): the P2P notice the
// first time (P2P is off until accepted, per profile), then the engine's phases with peers and
// speed until the selected file can be read; t("Cancelar") ends the P2P session. The video itself
// opens in the normal player.

import { useEffect, useRef, useState } from "react";
import { api, errorText } from "../api";
import { listen } from "../tauri";
import type { P2pStatus, StreamEntry, TorrentPrepared } from "../types";
import { decimal, t } from "../i18n";

export const P2P_CONSENT_TEXT = [
  "Esta stream utiliza tecnologia ponto a ponto (P2P). Ao ativar o P2P, você reconhece e concorda que:",
  "• Seu endereço IP ficará visível para outros peers na rede",
  "• Você é o único responsável pelo conteúdo que acessa",
  "• Você confirma que tem o direito legal de transmitir este conteúdo em sua jurisdição",
  "• O Nuvio não hospeda, distribui nem controla qualquer conteúdo P2P",
  "• O Nuvio não assume responsabilidade por quaisquer consequências legais resultantes do uso de streaming P2P",
  "Você utiliza este recurso inteiramente por sua conta e risco. O P2P pode ser desativado a qualquer momento nas Configurações.",
];

const PHASES: Record<string, string> = {
  engine: "Iniciando o motor P2P…",
  metadata: "Procurando peers e baixando as informações do torrent…",
  preparing: "Preparando o arquivo de vídeo…",
};

export function speed(bytes: number): string {
  if (bytes >= 1_048_576) return `${decimal(bytes / 1_048_576)} MB/s`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB/s`;
  return `${bytes} B/s`;
}

export function megabytes(bytes: number): string {
  return bytes >= 1_073_741_824 ? `${decimal(bytes / 1_073_741_824, 2)} GB` : `${decimal(bytes / 1_048_576)} MB`;
}

interface Props {
  stream: StreamEntry;
  onReady(file: TorrentPrepared): void;
  onCancel(): void;
}

export function TorrentStart({ stream, onReady, onCancel }: Props) {
  const [consent, setConsent] = useState(false);
  const [status, setStatus] = useState<P2pStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const live = useRef(true);

  useEffect(() => listen<P2pStatus>("p2p-status", (s) => live.current && setStatus(s)), []);
  useEffect(
    () => () => {
      live.current = false;
    },
    [],
  );

  useEffect(() => {
    let cancelled = false;
    setError(null);
    setConsent(false);
    api.torrentPrepare(stream.key).then(
      (file) => !cancelled && onReady(file),
      (e) => {
        if (cancelled) return;
        const text = errorText(e);
        if (text.includes("P2P_CONSENT_REQUIRED")) setConsent(true);
        else if (text !== "cancelado") setError(text);
      },
    );
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stream.key, attempt]);

  const cancel = () => {
    void api.torrentStop();
    onCancel();
  };

  const accept = async () => {
    try {
      const view = await api.settingsGet();
      await api.settingsSet({ ...view.settings, p2p: { ...view.settings.p2p, enabled: true } }, view.settings, view.profile.index);
      setAttempt((a) => a + 1);
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true" aria-labelledby="torrent-title" data-testid="torrent-start">
      <div className="modal torrent-start">
        {consent ? (
          <>
            <h2 id="torrent-title">{t("Streaming P2P")}</h2>
            <div className="p2p-consent">{P2P_CONSENT_TEXT.map((l, i) => <p key={i}>{t(l)}</p>)}</div>
            <div className="account-actions">
              <button className="primary" onClick={() => void accept()} data-testid="p2p-accept">{t("Ativar P2P")}</button>
              <button className="ghost" onClick={onCancel}>{t("Cancelar")}</button>
            </div>
          </>
        ) : (
          <>
            <h2 id="torrent-title">{stream.name || "Torrent"}</h2>
            {error ? (
              <div className="row-error" data-testid="torrent-error">{t("Não foi possível iniciar o torrent: {error}", { error })}</div>
            ) : (
              <>
                <div className="torrent-phase" data-testid="torrent-phase">
                  <span className="spinner" aria-hidden /> {t(PHASES[status?.phase || "engine"] ?? "Conectando…")}
                </div>
                <dl className="facts account-facts">
                  <div className="fact"><dt>{t("Peers")}</dt><dd>{status?.peers ?? 0} ({status?.seeds ?? 0} seeds)</dd></div>
                  <div className="fact"><dt>{t("Velocidade")}</dt><dd>{speed(status?.downloadSpeed ?? 0)}</dd></div>
                </dl>
                <p className="muted small">{t("A reprodução começa assim que o início do arquivo estiver disponível; o restante é baixado enquanto você assiste.")}</p>
              </>
            )}
            <div className="account-actions">
              {error && <button className="primary" onClick={() => setAttempt((a) => a + 1)}>{t("Tentar novamente")}</button>}
              <button className="ghost" onClick={cancel} data-testid="torrent-cancel">{t("Cancelar")}</button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

// Starting a torrent source (as the original's player does before playback): the P2P notice the
// first time (P2P is off until accepted, per profile), then the engine's phases with peers and
// speed until the selected file can be read; "Cancelar" ends the P2P session. The video itself
// opens in the normal player.

import { useEffect, useRef, useState } from "react";
import { api, errorText } from "../api";
import { listen } from "../tauri";
import type { P2pStatus, StreamEntry, TorrentPrepared } from "../types";

export const P2P_CONSENT_TEXT = [
  "This stream uses peer-to-peer (P2P) technology. By enabling P2P, you acknowledge and agree that:",
  "• Your IP address will be visible to other peers on the network",
  "• You are solely responsible for the content you access",
  "• You confirm that you have the legal right to stream this content in your jurisdiction",
  "• Nuvio does not host, distribute, or control any P2P content",
  "• Nuvio is not liable for any legal consequences resulting from the use of P2P streaming",
  "You use this feature entirely at your own risk. P2P can be disabled at any time in Settings.",
];

const PHASES: Record<string, string> = {
  engine: "Starting the P2P engine…",
  metadata: "Looking for peers and downloading torrent metadata…",
  preparing: "Preparing the video file…",
};

export function speed(bytes: number): string {
  if (bytes >= 1_048_576) return `${(bytes / 1_048_576).toFixed(1)} MB/s`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB/s`;
  return `${bytes} B/s`;
}

export function megabytes(bytes: number): string {
  return bytes >= 1_073_741_824 ? `${(bytes / 1_073_741_824).toFixed(2)} GB` : `${(bytes / 1_048_576).toFixed(1)} MB`;
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
        else if (text !== "canceled") setError(text);
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
      await api.settingsSet({ ...view.settings, p2p: { ...view.settings.p2p, enabled: true } });
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
            <h2 id="torrent-title">Streaming P2P</h2>
            <div className="p2p-consent">{P2P_CONSENT_TEXT.map((l, i) => <p key={i}>{l}</p>)}</div>
            <div className="account-actions">
              <button className="primary" onClick={() => void accept()} data-testid="p2p-accept">Enable P2P</button>
              <button className="ghost" onClick={onCancel}>Cancel</button>
            </div>
          </>
        ) : (
          <>
            <h2 id="torrent-title">{stream.name || "Torrent"}</h2>
            {error ? (
              <div className="row-error" data-testid="torrent-error">Could not start the torrent: {error}</div>
            ) : (
              <>
                <div className="torrent-phase" data-testid="torrent-phase">
                  <span className="spinner" aria-hidden /> {PHASES[status?.phase || "engine"] ?? "Connecting…"}
                </div>
                <dl className="facts account-facts">
                  <div className="fact"><dt>Peers</dt><dd>{status?.peers ?? 0} ({status?.seeds ?? 0} seeds)</dd></div>
                  <div className="fact"><dt>Speed</dt><dd>{speed(status?.downloadSpeed ?? 0)}</dd></div>
                </dl>
                <p className="muted small">Playback starts as soon as the beginning of the file is available; the rest downloads while you watch.</p>
              </>
            )}
            <div className="account-actions">
              {error && <button className="primary" onClick={() => setAttempt((a) => a + 1)}>Try again</button>}
              <button className="ghost" onClick={cancel} data-testid="torrent-cancel">Cancel</button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

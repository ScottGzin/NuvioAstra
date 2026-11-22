// Starting a torrent source (as the original's player does before playback): the P2P notice the
// first time (P2P is off until accepted, per profile), then the engine's phases with peers and
// speed until the selected file can be read; "Cancelar" ends the P2P session. The video itself
// opens in the normal player.

import { useEffect, useRef, useState } from "react";
import { api, errorText } from "../api";
import { locale, translate, useI18n, type TranslationKey } from "../i18n";
import { listen } from "../tauri";
import type { P2pStatus, StreamEntry, TorrentPrepared } from "../types";

export function p2pConsentText(): string[] {
  return [
    translate("p2p.consentIntro"),
    translate("p2p.consentIp"),
    translate("p2p.consentResponsibility"),
    translate("p2p.consentRights"),
    translate("p2p.consentHosting"),
    translate("p2p.consentLiability"),
    translate("p2p.consentRisk"),
  ];
}

const PHASES: Record<string, TranslationKey> = {
  engine: "p2p.phaseEngine",
  metadata: "p2p.phaseMetadata",
  preparing: "p2p.phasePreparing",
};

export function speed(bytes: number): string {
  const format = (value: number, digits: number) =>
    value.toLocaleString(locale(), { minimumFractionDigits: digits, maximumFractionDigits: digits });
  if (bytes >= 1_048_576) return `${format(bytes / 1_048_576, 1)} MB/s`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB/s`;
  return `${bytes} B/s`;
}

export function megabytes(bytes: number): string {
  return bytes >= 1_073_741_824
    ? `${(bytes / 1_073_741_824).toLocaleString(locale(), { minimumFractionDigits: 2, maximumFractionDigits: 2 })} GB`
    : `${(bytes / 1_048_576).toLocaleString(locale(), { minimumFractionDigits: 1, maximumFractionDigits: 1 })} MB`;
}

interface Props {
  stream: StreamEntry;
  onReady(file: TorrentPrepared): void;
  onCancel(): void;
}

export function TorrentStart({ stream, onReady, onCancel }: Props) {
  const { t } = useI18n();
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
        else if (text !== "cancelado" && text !== "cancelled") setError(text);
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
            <h2 id="torrent-title">{t("p2p.title")}</h2>
            <div className="p2p-consent">{p2pConsentText().map((l, i) => <p key={i}>{l}</p>)}</div>
            <div className="account-actions">
              <button className="primary" onClick={() => void accept()} data-testid="p2p-accept">{t("p2p.activate")}</button>
              <button className="ghost" onClick={onCancel}>{t("common.cancel")}</button>
            </div>
          </>
        ) : (
          <>
            <h2 id="torrent-title">{stream.name || t("p2p.torrent")}</h2>
            {error ? (
              <div className="row-error" data-testid="torrent-error">{t("p2p.startError", { error })}</div>
            ) : (
              <>
                <div className="torrent-phase" data-testid="torrent-phase">
                  <span className="spinner" aria-hidden /> {t(PHASES[status?.phase || "engine"]) ?? t("p2p.connecting")}
                </div>
                <dl className="facts account-facts">
                  <div className="fact"><dt>{t("p2p.peers")}</dt><dd>{status?.peers ?? 0} ({status?.seeds ?? 0} seeds)</dd></div>
                  <div className="fact"><dt>{t("p2p.speed")}</dt><dd>{speed(status?.downloadSpeed ?? 0)}</dd></div>
                </dl>
                <p className="muted small">{t("p2p.startsHint")}</p>
              </>
            )}
            <div className="account-actions">
              {error && <button className="primary" onClick={() => setAttempt((a) => a + 1)}>{t("p2p.retry")}</button>}
              <button className="ghost" onClick={cancel} data-testid="torrent-cancel">{t("common.cancel")}</button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

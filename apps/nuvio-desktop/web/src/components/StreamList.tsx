// Sources of one title/episode, asked to the addons only when shown. Direct and torrent (P2P)
// streams play in the app's player; the other kinds are listed with the reason they cannot play
// yet (no fake buttons).

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import type { StreamEntry, StreamGroup, StreamKind } from "../types";

const PENDING: Record<Exclude<StreamKind, "direct" | "torrent">, string> = {
  debrid: "Debrid — not supported in this version yet",
  external: "External link — opens outside the player",
  youTube: "YouTube — not supported in this version yet",
};

interface Props {
  contentType: string;
  id: string;
  onPlay(stream: StreamEntry): void;
}

export function StreamList({ contentType, id, onPlay }: Props) {
  const [groups, setGroups] = useState<StreamGroup[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setGroups(null);
    setError(null);
    api.streams(contentType, id).then(
      (g) => !cancelled && setGroups(g),
      (e) => !cancelled && setError(errorText(e)),
    );
    return () => {
      cancelled = true;
    };
  }, [contentType, id]);

  if (error) return <div className="row-error">Failed to fetch sources: {error}</div>;
  if (!groups) return <div className="muted">Fetching sources from addons…</div>;
  if (!groups.length) return <div className="muted">No installed addon provides sources for this title.</div>;

  return (
    <div className="streams">
      {groups.map((group) => (
        <div key={group.addonName} className="stream-group">
          <div className="stream-group-title">
            {group.addonName}
            <span className="muted small"> · {group.error ? `error: ${group.error}` : `${group.streams.length} source(s)`}</span>
          </div>
          {group.streams.map((s) => {
            const playable = s.kind === "direct" || s.kind === "torrent";
            return (
              <button key={s.key} className="stream" data-kind={s.kind} disabled={!playable} onClick={() => playable && onPlay(s)}>
                <span className="stream-name">{s.name || "Source"}{s.kind === "torrent" && <span className="stream-tag">P2P</span>}</span>
                <span className="stream-desc">{s.description}</span>
                {!playable && <span className="stream-pending">{PENDING[s.kind as Exclude<StreamKind, "direct" | "torrent">]}</span>}
              </button>
            );
          })}
        </div>
      ))}
    </div>
  );
}

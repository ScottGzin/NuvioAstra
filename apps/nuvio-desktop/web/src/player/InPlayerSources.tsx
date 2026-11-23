// Sources of the video currently playing, over the player (the original's in-player source
// switching). Picking a source starts it in the same session; picking an episode switches the
// episode without leaving the video.

import { useEffect, useState } from "react";
import { api } from "../api";
import { StreamList } from "../components/StreamList";
import { playbackItem } from "../library";
import type { MetaVideo, StreamEntry } from "../types";
import { usePlayer } from "./PlayerContext";
import { episodeTitle, type EpisodeContext } from "./episodes";
import { t } from "../i18n";

interface Props {
  context: EpisodeContext;
  onClose(): void;
  onPickEpisode(video: MetaVideo): void;
}

export function InPlayerSources({ context, onClose, onPickEpisode }: Props) {
  const player = usePlayer();
  const { meta, video } = context;
  const [starting, setStarting] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const play = async (stream: StreamEntry) => {
    if (starting) return;
    setStarting(true);
    const ok = await player.play(() => api.playStream(stream.key, null, playbackItem(meta, video)), {
      title: meta.name,
      subtitle: episodeTitle(video),
      view: "full",
      returnTo: null,
      episode: { meta, video },
      torrent: stream.kind === "torrent" && !stream.debridPlayable,
    });
    setStarting(false);
    if (ok) onClose();
  };

  const episodes = meta.videos
    .filter((v) => v.season != null && v.episode != null)
    .sort((a, b) => a.season! - b.season! || a.episode! - b.episode!);

  return (
    <div className="chapters-overlay" role="dialog" aria-modal="true" aria-label={t("Fontes no player")} data-testid="in-player-sources">
      <div className="chapters-panel in-player-sources-panel">
        <header className="modal-header">
          <h2>{t("Fontes no player")}</h2>
          <button className="icon-btn" aria-label={t("Fechar")} onClick={onClose}>✕</button>
        </header>
        {episodes.length > 1 && (
          <div className="in-player-episodes">
            <div className="settings-subhead">{t("Episódios")}</div>
            <div className="chip-select">
              {episodes.map((episode) => (
                <button
                  key={episode.id}
                  className={`chip ${episode.id === video.id ? "active" : ""}`}
                  onClick={() => { onPickEpisode(episode); onClose(); }}
                >
                  {episodeTitle(episode)}
                </button>
              ))}
            </div>
          </div>
        )}
        <StreamList contentType={meta.contentType} id={video.id} onPlay={(stream) => void play(stream)} />
      </div>
    </div>
  );
}

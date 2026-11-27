// Sources over the video, which keeps playing until one is chosen (the original's episode
// sources panel and its in-player source switching):
// - `mode: "next"`: the next episode (its saved position);
// - `mode: "switch"`: another source of what is playing, from the current position;
// - `mode: "episode"`: an episode chosen in the player's episode list (its saved position).

import { useEffect, useState, type ReactNode } from "react";
import { api, errorText } from "../api";
import { StreamList } from "../components/StreamList";
import { TorrentStart } from "../components/TorrentStart";
import { startDownload, viaTorbox, type TorboxStates } from "../sources";
import { clock, playbackItem } from "../library";
import type { MetaDetails, MetaVideo, StreamEntry } from "../types";
import { episodeTitle } from "./episodes";
import { usePlayer } from "./PlayerContext";
import { t } from "../i18n";

interface Props {
  context: { meta: MetaDetails };
  /** The episode whose sources are shown; `null`: the movie. */
  next: MetaVideo | null;
  mode?: "next" | "switch" | "episode";
  /** `switch`: where the new source starts (seconds). */
  resumeFrom?: number | null;
  /** The current episode reached its end (credits): it counts as watched when the next one starts. */
  finished(): boolean;
  onClose(): void;
  /** Above the list (the player's tabs). */
  top?: ReactNode;
}

export function NextEpisodePanel({ context, next, mode = "next", resumeFrom = null, finished, onClose, top }: Props) {
  const player = usePlayer();
  const { meta } = context;
  const itemTitle = next ? episodeTitle(next) : meta.releaseInfo ?? meta.name;
  const [torrent, setTorrent] = useState<StreamEntry | null>(null);
  const [starting, setStarting] = useState(false);
  const [resumeAt, setResumeAt] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    if (mode === "switch") {
      setResumeAt(resumeFrom !== null && resumeFrom > 5 ? resumeFrom * 1000 : null);
      return;
    }
    api.libraryTitleProgress(meta.id).then((progress) => {
      const saved = progress.entries.find((e) => (next ? e.season === next.season && e.episode === next.episode : e.season == null && e.episode == null));
      if (!cancelled && saved?.inProgress) setResumeAt(saved.positionMs);
    }, () => {});
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [meta.id, next, mode, resumeFrom]);

  // Preparing a torrent: only its own Cancel ends it.
  useEffect(() => {
    if (!torrent) return;
    const block = (e: KeyboardEvent) => e.key === "Escape" && e.stopPropagation();
    window.addEventListener("keydown", block, true);
    return () => window.removeEventListener("keydown", block, true);
  }, [torrent]);

  const downloadRequest = (stream: StreamEntry) => ({
    key: stream.key, title: meta.name, subtitle: next ? episodeTitle(next) : null, poster: meta.poster,
    season: next?.season ?? null, episode: next?.episode ?? null, playback: playbackItem(meta, next),
  });

  const play = async (stream: StreamEntry, prepared = false, torbox: TorboxStates = {}) => {
    if (starting) return;
    if (stream.kind === "external") {
      api.streamOpenExternal(stream.key).catch((e) => window.alert(errorText(e)));
      return;
    }
    let direct = stream.kind === "direct";
    if (!prepared && stream.kind !== "direct") {
      setStarting(true);
      try {
        direct = await viaTorbox(stream, torbox, { season: next?.season ?? null, episode: next?.episode ?? null });
      } catch (e) {
        setStarting(false);
        window.alert(errorText(e));
        return;
      }
      setStarting(false);
    }
    if (stream.kind === "torrent" && !prepared && !direct) {
      void api.playerCommand({ type: "setPaused", paused: true });
      setTorrent(stream);
      return;
    }
    setStarting(true);
    const start = resumeAt !== null ? resumeAt / 1000 : null;
    const previousFinished = mode === "next" && finished();
    const begin = stream.kind === "youTube"
      ? () => api.streamPlayYoutube(stream.key, start, playbackItem(meta, next))
      : () => api.playStream(stream.key, start, playbackItem(meta, next), previousFinished);
    const ok = await player.play(begin, {
      title: meta.name,
      subtitle: next ? episodeTitle(next) : meta.releaseInfo ?? undefined,
      view: "full",
      returnTo: null,
      episode: next ? { meta, video: next } : undefined,
      movie: next ? undefined : meta,
      torrent: stream.kind === "torrent" && !direct,
      download: downloadRequest(stream),
    });
    if (ok) return onClose();
    setStarting(false);
    if (stream.kind === "torrent" && !direct) void api.torrentStop();
  };

  const cancelTorrent = () => {
    setTorrent(null);
    // A torrent that was playing lost its session when the next one was prepared.
    if (player.session?.torrent) void player.close();
    else void api.playerCommand({ type: "setPaused", paused: false });
  };

  return (
    <div className="next-panel-layer" data-testid="next-panel">
      <button className="next-panel-outside" aria-label={t("Fechar fontes")} tabIndex={-1} onClick={onClose} />
      <aside className="next-panel" aria-label={mode === "next" ? t("Fontes do próximo episódio") : t("Fontes")}>
        {top}
        <header className="sources-header">
          <div>
            <h2>{mode === "next" ? t("Próximo episódio") : mode === "switch" ? t("Trocar fonte") : t("Fontes")}</h2>
            <div className="muted small">
              {itemTitle}
              {resumeAt !== null && ` · ${t("continua de {time}", { time: clock(resumeAt) })}`}
            </div>
          </div>
          <button className="icon-btn" aria-label={t("Fechar fontes")} onClick={onClose}>✕</button>
        </header>
        <div className="sources-body">
          {player.error && <div className="row-error">{t("Falha ao reproduzir:")} {player.error}</div>}
          {starting ? (
            <div className="muted"><span className="spinner" aria-hidden /> {mode === "next" ? t("Abrindo o próximo episódio…") : t("Abrindo a fonte…")}</div>
          ) : (
            <StreamList contentType={meta.contentType} id={next ? next.id : meta.id} onPlay={(stream, torbox) => void play(stream, false, torbox)} onDownload={(stream) => void startDownload(downloadRequest(stream))} />
          )}
        </div>
      </aside>
      {torrent && (
        <TorrentStart
          stream={torrent}
          onCancel={cancelTorrent}
          onReady={() => {
            const stream = torrent;
            setTorrent(null);
            void play(stream, true);
          }}
        />
      )}
    </div>
  );
}

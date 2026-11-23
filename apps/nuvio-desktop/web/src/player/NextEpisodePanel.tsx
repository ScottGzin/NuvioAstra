// The next episode's sources over the video, which keeps playing until one is chosen (the
// original's episode sources panel).

import { useEffect, useState } from "react";
import { api } from "../api";
import { StreamList } from "../components/StreamList";
import { TorrentStart } from "../components/TorrentStart";
import { clock, playbackItem } from "../library";
import type { MetaVideo, PlayerSettings, StreamEntry } from "../types";
import { episodeTitle, type EpisodeContext } from "./episodes";
import { usePlayer } from "./PlayerContext";
import { t } from "../i18n";

interface Props {
  context: EpisodeContext;
  next: MetaVideo;
  /** The current episode reached its end (credits): it counts as watched when the next one starts. */
  finished(): boolean;
  onClose(): void;
  /** Settings → Reprodução: autoplay of the next episode with source fallback. */
  autoPlay?: PlayerSettings | null;
}

export function NextEpisodePanel({ context, next, finished, onClose, autoPlay }: Props) {
  const player = usePlayer();
  const { meta } = context;
  const [torrent, setTorrent] = useState<StreamEntry | null>(null);
  const [starting, setStarting] = useState(false);
  const [resumeAt, setResumeAt] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    api.libraryTitleProgress(meta.id).then((progress) => {
      const saved = progress.entries.find((e) => e.season === next.season && e.episode === next.episode);
      if (!cancelled && saved?.inProgress) setResumeAt(saved.positionMs);
    }, () => {});
    return () => {
      cancelled = true;
    };
  }, [meta.id, next]);

  // Preparing a torrent: only its own Cancel ends it.
  useEffect(() => {
    if (!torrent) return;
    const block = (e: KeyboardEvent) => e.key === "Escape" && e.stopPropagation();
    window.addEventListener("keydown", block, true);
    return () => window.removeEventListener("keydown", block, true);
  }, [torrent]);

  // `PlayerNextEpisodeAutoPlay`: after a short countdown, start the first eligible source. If it
  // cannot start, try the next one (up to `autoPlayFallbackMaxSources`) before leaving the picker.
  useEffect(() => {
    if (!autoPlay?.autoPlayNextEpisodeEnabled) return;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const groups = await api.streams(meta.contentType, next.id);
          let candidates = groups.flatMap((group) => group.streams).filter((stream) =>
            ["direct", "torrent", "debrid", "youTube"].includes(stream.kind));
          if (autoPlay.autoPlayMode === "regexMatch" && autoPlay.autoPlayRegex.trim()) {
            let re: RegExp | null = null;
            try { re = new RegExp(autoPlay.autoPlayRegex.trim(), "i"); } catch { re = null; }
            if (re) candidates = candidates.filter((stream) => re!.test([stream.name, stream.description, stream.filename].filter(Boolean).join(" ")));
          }
          for (const stream of candidates.slice(0, Math.max(1, autoPlay.autoPlayFallbackMaxSources))) {
            if (cancelled) return;
            const resume = resumeAt !== null ? resumeAt / 1000 : null;
            const ok = await player.play(() => api.playStream(stream.key, resume, playbackItem(meta, next), finished()), {
              title: meta.name,
              subtitle: episodeTitle(next),
              view: "full",
              returnTo: null,
              episode: { meta, video: next },
              torrent: stream.kind === "torrent" && !stream.debridPlayable,
            });
            if (ok) return; // the panel unmounts with the new session
          }
        } catch {
          // Keep the manual picker on screen when no source could start.
        }
      })();
    }, 3000);
    return () => { cancelled = true; window.clearTimeout(timer); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoPlay?.autoPlayNextEpisodeEnabled, autoPlay?.autoPlayMode, autoPlay?.autoPlayRegex, autoPlay?.autoPlayFallbackMaxSources, meta.id, next.id, resumeAt]);

  const play = async (stream: StreamEntry, prepared = false) => {
    if (starting) return;
    if (stream.kind === "torrent" && !prepared && !stream.debridPlayable) {
      void api.playerCommand({ type: "setPaused", paused: true });
      setTorrent(stream);
      return;
    }
    setStarting(true);
    const start = resumeAt !== null ? resumeAt / 1000 : null;
    const previousFinished = finished();
    const ok = await player.play(() => api.playStream(stream.key, start, playbackItem(meta, next), previousFinished), {
      title: meta.name,
      subtitle: episodeTitle(next),
      view: "full",
      returnTo: null,
      episode: { meta, video: next },
      torrent: stream.kind === "torrent",
    });
    if (ok) return onClose();
    setStarting(false);
    if (stream.kind === "torrent") void api.torrentStop();
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
      <aside className="next-panel" aria-label={t("Fontes do próximo episódio")}>
        <header className="sources-header">
          <div>
            <h2>{t("Próximo episódio")}</h2>
            <div className="muted small">
              {episodeTitle(next)}
              {resumeAt !== null && ` · ${t("continua de {time}", { time: clock(resumeAt) })}`}
            </div>
          </div>
          <button className="icon-btn" aria-label={t("Fechar fontes")} onClick={onClose}>✕</button>
        </header>
        <div className="sources-body">
          {player.error && <div className="row-error">{t("Falha ao reproduzir:")} {player.error}</div>}
          {starting ? (
            <div className="muted"><span className="spinner" aria-hidden /> {t("Abrindo o próximo episódio…")}</div>
          ) : (
            <StreamList contentType={meta.contentType} id={next.id} onPlay={(stream) => void play(stream)} />
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

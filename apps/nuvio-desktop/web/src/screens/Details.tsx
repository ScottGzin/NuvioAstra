// Details of a movie or series: artwork, info, genres, cast, seasons/episodes and the sources of
// the chosen title or episode (asked to the addons only when the user opens them).

import { useEffect, useMemo, useRef, useState } from "react";
import { api, errorText } from "../api";
import { FavoriteButton } from "../components/FavoriteButton";
import { ResultRow } from "../components/ResultRow";
import { StreamList } from "../components/StreamList";
import { TorrentStart } from "../components/TorrentStart";
import { clock, episodeCode, libraryItemFromDetails, playbackItem, refreshLibraryViews, remaining, useLibraryVersion } from "../library";
import { usePlayer } from "../player/PlayerContext";
import type { Details as DetailsData, HistoryEntry, MetaPreview, MetaRef, MetaVideo, MetaScreenSettings, ParentalGuide, StreamEntry, TitleProgress } from "../types";
import { addonLabel, t } from "../i18n";

interface Props {
  /** Type and id exactly as the catalog item gave them (validated by `metaRef`); `resume` when
   *  opened from Continue watching / history (selects that movie/episode and resumes). */
  item: MetaRef;
  onBack(): void;
  /** Opens another title ("More Like This"); the screen remounts for it. */
  onOpen(item: MetaPreview): void;
}

/** Short labels of the MDBList sources (`MetaExternalRating` in the original). */
function ratingSource(source: string): string {
  switch (source) {
    case "imdb": return "IMDb";
    case "tmdb": return "TMDB";
    case "tomatoes": return "RT";
    case "metacritic": return "MC";
    case "trakt": return "Trakt";
    case "letterboxd": return "LB";
    case "audience": return "Audience";
    case "mal": return "MAL";
    default: return source;
  }
}

function ratingValue(rating: { source: string; value: number }): string {
  switch (rating.source) {
    case "imdb":
    case "mal":
      return ` ${rating.value.toFixed(1)}`;
    case "letterboxd":
      return ` ${rating.value.toFixed(1)}/5`;
    default:
      return ` ${Math.round(rating.value)}%`;
  }
}

const GUIDE_LABELS: Record<string, string> = {
  nudity: "Nu", violence: "Violência", profanity: "Linguagem", alcohol: "Álcool e drogas", frightening: "Tensão",
};

function episodeLabel(v: MetaVideo): string {
  return v.season != null && v.episode != null ? t("T{season} E{episode}", { season: v.season, episode: v.episode }) : "";
}

export function Details({ item, onBack, onOpen }: Props) {
  const { contentType, id } = item;
  const player = usePlayer();
  const [data, setData] = useState<DetailsData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [season, setSeason] = useState<number | null>(null);
  const [selected, setSelected] = useState<MetaVideo | "movie" | null>(null);
  const [fromStart, setFromStart] = useState(false);
  /** Torrent source being started by the P2P engine (before the player opens). */
  const [torrent, setTorrent] = useState<StreamEntry | null>(null);
  const [trailerBusy, setTrailerBusy] = useState(false);
  const [moreLikeThis, setMoreLikeThis] = useState<MetaPreview[]>([]);
  const [watchedKeys, setWatchedKeys] = useState<string[]>([]);
  const [metaSettings, setMetaSettings] = useState<MetaScreenSettings | null>(null);
  const [guide, setGuide] = useState<ParentalGuide | null>(null);
  const [showGuide, setShowGuide] = useState(false);
  const [progress, setProgress] = useState<TitleProgress | null>(null);
  const version = useLibraryVersion();
  const sources = useRef<HTMLElement>(null);

  // Saved progress of this title (NextGen data only; refreshed after each save).
  useEffect(() => {
    let cancelled = false;
    api.libraryTitleProgress(id).then((p) => !cancelled && setProgress(p), () => {});
    return () => {
      cancelled = true;
    };
  }, [id, version]);

  useEffect(() => setWatchedKeys(data?.watchedKeys ?? []), [data?.watchedKeys]);

  // Discord presence on the details screen (the original's PresenceSnapshot.Details).
  useEffect(() => {
    if (!data) return;
    void api.discordActivity({ title: data.meta.name, state: t("Vendo {title}", { title: data.meta.name }), poster: data.meta.poster, playing: false });
    return () => { void api.discordActivity(null); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data?.meta.id]);


  // Details screen settings (the original's MetaScreenSettingsRepository).
  useEffect(() => {
    api.settingsGet().then((v) => {
      setMetaSettings(v.settings.metaScreen);
      setShowGuide(v.settings.player.showParentalGuide);
    }, () => {});
  }, [version]);

  useEffect(() => {
    if (!showGuide) return;
    let cancelled = false;
    api.parentalGuide(id).then((value) => !cancelled && setGuide(value), () => !cancelled && setGuide(null));
    return () => { cancelled = true; };
  }, [showGuide, id]);
  const sectionOn = (key: string) => metaSettings?.sections.find((s) => s.key === key)?.enabled ?? true;

  const watchedKey = (contentType: string, id: string, season: number | null, episode: number | null) => {
    const raw = contentType.trim().toLowerCase();
    const kind = ["series", "tv", "show", "tvshow", "anime"].includes(raw) ? "series" : contentType.trim();
    return `${kind}:${id}:${season ?? -1}:${episode ?? -1}`;
  };

  /** Manually mark/unmark a movie or an episode as watched (synced with the account). */
  const toggleWatched = (contentType: string, id: string, season: number | null, episode: number | null, name: string) => {
    const key = watchedKey(contentType, id, season, episode);
    const wasWatched = watchedKeys.includes(key);
    setWatchedKeys((prev) => (wasWatched ? prev.filter((k) => k !== key) : [...prev, key]));
    void api
      .libraryToggleWatched({ id, contentType, name, poster: m.poster, releaseInfo: m.releaseInfo, season, episode, videoId: null, markedAtEpochMs: Date.now() })
      .then((added) => {
        if (added !== !wasWatched) setWatchedKeys(data?.watchedKeys ?? []);
        refreshLibraryViews();
      })
      .catch(() => setWatchedKeys(data?.watchedKeys ?? []));
  };

  // TMDB recommendations of this title (the original's "More Like This", powered by TMDB).
  useEffect(() => {
    let cancelled = false;
    setMoreLikeThis([]);
    api.moreLikeThis(contentType, id).then(
      (items) => !cancelled && setMoreLikeThis(items),
      () => {},
    );
    return () => {
      cancelled = true;
    };
  }, [contentType, id]);

  useEffect(() => {
    let cancelled = false;
    setData(null);
    setError(null);
    setSelected(null);
    api.details(contentType, id).then(
      (d) => {
        if (cancelled) return;
        setData(d);
        const resume = item.resume;
        const target = resume && d.seasons.length > 0
          ? d.meta.videos.find((v) => (resume.videoId && v.id === resume.videoId) || (v.season === resume.season && v.episode === resume.episode))
          : undefined;
        setSeason(target?.season ?? d.seasons.find((s) => s > 0) ?? d.seasons[0] ?? null);
        if (resume) setSelected(d.seasons.length > 0 ? target ?? null : "movie");
      },
      (e) => !cancelled && setError(errorText(e)),
    );
    return () => {
      cancelled = true;
    };
    // `item.resume` is read once per title (the component is keyed by type/id).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [contentType, id]);

  useEffect(() => {
    // The sources panel is a modal over the details: focus it so Escape and the keyboard work.
    if (selected) requestAnimationFrame(() => sources.current?.focus());
  }, [selected]);

  useEffect(() => {
    if (!selected) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setSelected(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selected]);

  const episodes = useMemo(
    () => (data?.meta.videos ?? []).filter((v) => v.season === season).sort((a, b) => (a.episode ?? 0) - (b.episode ?? 0)),
    [data, season],
  );
  const isSeries = (data?.seasons.length ?? 0) > 0;

  /** Progress of the movie or of one episode (each episode has its own). */
  const entryFor = (target: MetaVideo | "movie" | null): HistoryEntry | undefined => {
    if (!target || !progress) return undefined;
    return target === "movie"
      ? progress.entries.find((e) => e.season == null && e.episode == null)
      : progress.entries.find((e) => e.season === target.season && e.episode === target.episode);
  };
  const selectedEntry = entryFor(selected);
  const resumeAt = !fromStart && selectedEntry?.inProgress ? selectedEntry.positionMs : null;

  const choose = (target: MetaVideo | "movie", start = false) => {
    setFromStart(start);
    setSelected(target);
  };

  /** Trailer in the app's player; closing it returns here. Not saved to history or progress. */
  const playTrailer = async () => {
    if (!data?.trailer) return;
    setTrailerBusy(true);
    await player.play(() => api.trailerPlay(data.trailer!.key), { title: data.meta.name, subtitle: t("Trailer · {name}", { name: data.trailer.name }), view: "full" });
    setTrailerBusy(false);
  };

  const play = (stream: StreamEntry, prepared = false) => {
    if (!data) return;
    if (stream.kind === "torrent" && !prepared && !stream.debridPlayable) {
      setTorrent(stream);
      return;
    }
    const video = selected && selected !== "movie" ? selected : null;
    const subtitle = video ? `${episodeLabel(video)} · ${video.title}` : data.meta.releaseInfo ?? undefined;
    const start = resumeAt !== null ? resumeAt / 1000 : null;
    void player
      .play(() => api.playStream(stream.key, start, playbackItem(data.meta, video)), {
        title: data.meta.name,
        subtitle,
        view: "full",
        episode: video ? { meta: data.meta, video } : undefined,
        torrent: stream.kind === "torrent",
      })
      .then((ok) => {
        // A torrent whose player could not open: its P2P session ends too.
        if (!ok && stream.kind === "torrent") void api.torrentStop();
      });
  };

  if (error) {
    return (
      <div className="screen-message">
        <button className="back" onClick={onBack}>←</button> {t("Não foi possível abrir os detalhes:")} {error}
      </div>
    );
  }
  if (!data) return <div className="screen-message">{t("Carregando detalhes…")}</div>;
  const m = data.meta;
  const movieEntry = entryFor("movie");
  const resumeVideo = progress?.resume ? m.videos.find((v) => v.season === progress.resume!.season && v.episode === progress.resume!.episode) : undefined;

  return (
    <div className="details" data-id={m.id} data-content-type={m.contentType}>
      <div className="details-backdrop" style={m.background ? { backgroundImage: `url("${m.background}")` } : undefined} />
      <div className="details-body">
        <button className="back" onClick={onBack} aria-label={t("Voltar")}>←</button>
        <div className={`details-layout ${selected ? "with-sources" : ""}`}>
        <div className="details-main">
        <div className="details-head">
          {m.poster && <img className="details-poster" src={m.poster} alt="" />}
          <div className="details-info">
            {m.logo ? <img className="details-logo" src={m.logo} alt={m.name} /> : <h1>{m.name}</h1>}
            <div className="meta-line">
              {[m.releaseInfo, m.runtime, m.imdbRating && `★ ${m.imdbRating}`, m.ageRating].filter(Boolean).join("   ")}
            </div>
            {metaSettings?.showOverallRatings !== false && data.ratings.length > 0 && (
              <div className="external-ratings" aria-label={t("Avaliações externas")}>
                {data.ratings.map((rating) => (
                  <span key={rating.source} className={`rating-chip ${rating.certified ? "certified" : ""}`} title={rating.source}>
                    <b>{ratingSource(rating.source)}</b>
                    {ratingValue(rating)}
                  </span>
                ))}
              </div>
            )}
            {m.genres.length > 0 && <div className="chips">{m.genres.map((g) => <span key={g} className="chip">{g}</span>)}</div>}
            {guide && (
              <div className="chips warning-chips" aria-label={t("Avisos de conteúdo")}>
                {(Object.entries(guide) as [string, string | null][])
                  .filter(([, severity]) => !!severity)
                  .map(([label, severity]) => (
                    <span key={label} className={`chip warning ${severity}`} title={t("Avisos de conteúdo")}>
                      {t(GUIDE_LABELS[label] ?? label)}: {t(severity ?? "")}
                    </span>
                  ))}
              </div>
            )}
            {sectionOn("overview") && m.description && <p className="details-desc">{m.description}</p>}
            {m.director.length > 0 && <div className="muted">{t("Direção:")} {m.director.join(", ")}</div>}
            {sectionOn("actions") && <div className="details-actions">
              {!isSeries && (movieEntry?.inProgress ? (
                <>
                  <button className="primary" data-testid="watch" onClick={() => choose("movie")}>▶ {t("Continuar de {time}", { time: clock(movieEntry.positionMs) })}</button>
                  <button className="chip" data-testid="watch-from-start" onClick={() => choose("movie", true)}>{t("Assistir do início")}</button>
                </>
              ) : (
                <button className="primary" data-testid="watch" onClick={() => choose("movie", true)}>{t("▶ Assistir")}</button>
              ))}
              {isSeries && resumeVideo && progress?.resume && (
                <button className="primary" data-testid="resume-episode" onClick={() => { setSeason(resumeVideo.season); choose(resumeVideo); }}>
                  {t("▶ Continuar")} {episodeCode(progress.resume)} · {clock(progress.resume.positionMs)}
                </button>
              )}
              {sectionOn("trailers") && data.trailer && (
                <button className="chip" data-testid="watch-trailer" disabled={trailerBusy} onClick={() => void playTrailer()}>
                  {trailerBusy ? t("Carregando trailer…") : t("▶ Assistir trailer")}
                </button>
              )}
              <FavoriteButton item={libraryItemFromDetails(m)} variant="pill" />
              {!isSeries && (
                <button
                  className={`chip ${watchedKeys.includes(watchedKey(m.contentType, m.id, null, null)) ? "active" : ""}`}
                  data-testid="movie-watched"
                  title={t("Marcar como assistido")}
                  onClick={() => toggleWatched(m.contentType, m.id, null, null, m.name)}
                >
                  {watchedKeys.includes(watchedKey(m.contentType, m.id, null, null)) ? t("✓ Assistido") : t("Marcar como assistido")}
                </button>
              )}
            </div>}
            {!isSeries && movieEntry && (
              <div className="muted small" data-testid="movie-progress">
                {movieEntry.completed ? t("✓ Assistido") : [t("{time} assistidos", { time: clock(movieEntry.positionMs) }), remaining(movieEntry)].filter(Boolean).join(" · ")}
              </div>
            )}
            {sectionOn("production") && <div className="muted small">{t("Detalhes por")} {addonLabel(data.addonName)}</div>}
            {sectionOn("details") && <dl className="facts">
              {(
                [
                  [t("Classificação"), m.ageRating],
                  [t("Situação"), m.status],
                  [t("Roteiro"), m.writer.join(", ")],
                  [t("País"), m.country],
                  [t("Idioma"), m.language],
                  [t("Prêmios"), m.awards],
                  [t("Site"), m.website],
                ] as const
              )
                .filter(([, v]) => v)
                .map(([k, v]) => (
                  <div key={k} className="fact"><dt>{k}</dt><dd>{v}</dd></div>
                ))}
            </dl>}
          </div>
        </div>

        {player.error && <div className="row-error">{t("Falha ao reproduzir:")} {player.error}</div>}

        {isSeries && sectionOn("episodes") && (
          <section className={`details-section ${metaSettings?.episodeCardStyle === "list" ? "episode-list" : ""}`}>
            <div className="season-tabs" role="tablist">
              {data.seasons.map((s) => (
                <button key={s} role="tab" aria-selected={season === s} className={`chip ${season === s ? "active" : ""}`} onClick={() => setSeason(s)}>
                  {s === 0 ? t("Especiais") : t("Temporada {season}", { season: s })}
                </button>
              ))}
            </div>
            <div className="episodes">
              {episodes.map((v) => {
                const e = entryFor(v);
                const epKey = watchedKey(m.contentType, m.id, v.season ?? null, v.episode ?? null);
                return (
                  <div key={v.id} className={`episode-wrap ${metaSettings?.blurUnwatchedEpisodes && !e?.completed ? "episode-blurred" : ""}`}>
                    <button className={`episode ${selected !== "movie" && selected?.id === v.id ? "active" : ""}`} onClick={() => choose(v)} data-episode={`${v.season}:${v.episode}`}>
                    <div className="episode-thumb">
                      {v.thumbnail && <img src={v.thumbnail} alt="" loading="lazy" decoding="async" />}
                      {e && !e.completed && e.fraction > 0 && <div className="progress"><div style={{ width: `${Math.max(2, e.fraction * 100)}%` }} /></div>}
                    </div>
                    <div className="episode-text">
                      <div className="episode-title">{episodeLabel(v)} · {v.title}</div>
                      {v.released && <div className="muted small">{v.released.slice(0, 10)}</div>}
                      {e && (
                        <div className="small episode-progress" data-testid="episode-progress">
                          {e.completed ? t("✓ Assistido") : t("{time} assistidos", { time: clock(e.positionMs) }) + (remaining(e) ? ` · ${remaining(e)}` : "")}
                        </div>
                      )}
                      {v.overview && <div className="episode-overview">{v.overview}</div>}
                    </div>
                    </button>
                    <button
                      className={`watched-toggle ${watchedKeys.includes(epKey) ? "on" : ""}`}
                      data-episode-watched={`${v.season}:${v.episode}`}
                      title={t("Marcar como assistido")}
                      aria-label={t("Marcar como assistido")}
                      onClick={() => toggleWatched(m.contentType, m.id, v.season ?? null, v.episode ?? null, v.title)}
                    >✓</button>
                  </div>
                );
              })}
            </div>
          </section>
        )}

        {m.cast.length > 0 && sectionOn("cast") && (
          <section className="details-section">
            <h2>{t("Elenco")}</h2>
            <div className="cast">
              {m.cast.slice(0, 30).map((p) => (
                <div key={`${p.name}-${p.role ?? ""}`} className="person">
                  <div className="person-photo">{p.photo ? <img src={p.photo} alt="" loading="lazy" /> : p.name.slice(0, 1)}</div>
                  <div className="small">{p.name}</div>
                  {p.role && <div className="muted small">{p.role}</div>}
                </div>
              ))}
            </div>
          </section>
        )}

        {moreLikeThis.length > 0 && sectionOn("moreLikeThis") && (
          <section className="details-section more-like-this" data-testid="more-like-this">
            <ResultRow title={t("Mais como este")} addonName="" items={moreLikeThis} onOpen={onOpen} />
            <div className="muted small tmdb-credit">{t("Powered by TMDB")}</div>
          </section>
        )}
        </div>

        </div>
      </div>

      {selected && (
        <div className="sources-overlay" onClick={() => setSelected(null)}>
          <aside
            className="sources-panel"
            ref={sources}
            tabIndex={-1}
            role="dialog"
            aria-modal="true"
            aria-label={t("Fontes")}
            data-testid="sources-panel"
            onClick={(event) => event.stopPropagation()}
          >
            <header className="sources-header">
              <div>
                <h2>{t("Fontes")}</h2>
                <div className="muted small">
                  {selected === "movie" ? m.name : `${episodeLabel(selected)} · ${selected.title}`}
                  {resumeAt !== null && <span data-testid="resume-note"> · {t("continua de {time}", { time: clock(resumeAt) })}</span>}
                </div>
              </div>
              <button className="icon-btn" aria-label={t("Fechar fontes")} onClick={() => setSelected(null)}>✕</button>
            </header>
            <div className="sources-body">
              <StreamList contentType={m.contentType} id={selected === "movie" ? m.id : selected.id} onPlay={play} />
            </div>
          </aside>
        </div>
      )}
      {torrent && (
        <TorrentStart
          stream={torrent}
          onCancel={() => setTorrent(null)}
          onReady={() => {
            const s = torrent;
            setTorrent(null);
            play(s, true);
          }}
        />
      )}
    </div>
  );
}

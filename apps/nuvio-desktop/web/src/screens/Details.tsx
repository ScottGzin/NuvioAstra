// Details of a movie or series: artwork, info, genres, cast, seasons/episodes and the sources of
// the chosen title or episode (asked to the addons only when the user opens them).
// Settings → Fontes: the automatic choice of a source, trying the next sources when one fails to
// start, and playing the last link of a movie/episode again (no addon asked).

import { useEffect, useMemo, useRef, useState } from "react";
import { api, errorText } from "../api";
import { FavoriteButton } from "../components/FavoriteButton";
import { StreamList } from "../components/StreamList";
import { fallbackAfter, startDownload, viaTorbox, type ListedStream, type TorboxStates } from "../sources";
import { showToast } from "../toast";
import { TorrentStart } from "../components/TorrentStart";
import { clock, episodeCode, libraryItemFromDetails, playbackItem, remaining, useLibraryVersion } from "../library";
import { usePlayer } from "../player/PlayerContext";
import { hasAired } from "../player/episodes";
import { useScreens } from "../screens";
import type { Details as DetailsData, HistoryEntry, MetaPreview, MetaRef, MetaVideo, SourceSettings, StreamEntry, TitleProgress, TmdbCompany, TmdbEntityKind } from "../types";
import { PosterCard } from "../components/PosterCard";
import { ExternalRatings } from "../components/ExternalRatings";
import { addonLabel, locale, t } from "../i18n";

interface Props {
  /** Type and id exactly as the catalog item gave them (validated by `metaRef`); `resume` when
   *  opened from Continue watching / history (selects that movie/episode and resumes). */
  item: MetaRef;
  onBack(): void;
  /** Opens another title (More like this, the movie's collection). */
  onOpen?(item: MetaPreview): void;
  /** Opens a person's, studio's or network's page (TMDB). */
  onEntity?(kind: TmdbEntityKind, id: number, name: string): void;
}

/** A horizontal row of titles (More like this, the collection). */
function TitleRow({ title, items, onOpen, testId }: { title: string; items: MetaPreview[]; onOpen?(item: MetaPreview): void; testId: string }) {
  if (items.length === 0 || !onOpen) return null;
  return (
    <section className="details-section row-section" data-testid={testId}>
      <header className="row-header"><h2>{title}</h2></header>
      <div className="row-track">
        {items.map((item) => <PosterCard key={`${item.contentType}/${item.id}`} item={item} onOpen={onOpen} />)}
      </div>
    </section>
  );
}

function Companies({ label, list, kind, onEntity }: { label: string; list: TmdbCompany[]; kind: TmdbEntityKind; onEntity?: Props["onEntity"] }) {
  if (list.length === 0) return null;
  return (
    <div className="fact tmdb-companies" data-testid={`tmdb-${kind}`}>
      <dt>{label}</dt>
      <dd>
        {list.map((c) =>
          c.id != null && onEntity ? (
            <button key={`${c.id}-${c.name}`} className="company-chip" onClick={() => onEntity(kind, c.id!, c.name)} title={c.name}>
              {c.logo ? <img src={c.logo} alt={c.name} /> : c.name}
            </button>
          ) : (
            <span key={c.name} className="company-chip">{c.logo ? <img src={c.logo} alt={c.name} /> : c.name}</span>
          ),
        )}
      </dd>
    </div>
  );
}

/** TMDB's status words. */
const STATUS: Record<string, string> = {
  Released: "Lançado", Ended: "Finalizada", "Returning Series": "Em exibição", Canceled: "Cancelada", "In Production": "Em produção",
  Planned: "Planejado", "Post Production": "Pós-produção", Rumored: "Rumor", Pilot: "Piloto",
};

/** "BR, US" (TMDB codes) as country names in the interface language; other texts as they are. */
function regionNames(value: string | null): string | null {
  if (!value || !/^[A-Z]{2}(, ?[A-Z]{2})*$/.test(value.trim())) return value;
  try {
    const names = new Intl.DisplayNames([locale()], { type: "region" });
    return value.split(",").map((c) => names.of(c.trim()) ?? c.trim()).join(", ");
  } catch {
    return value;
  }
}

/** "pt" (an ISO code) as the language name; other texts as they are. */
function languageName(value: string | null): string | null {
  if (!value || !/^[a-z]{2,3}$/.test(value.trim())) return value;
  try {
    const name = new Intl.DisplayNames([locale()], { type: "language" }).of(value.trim());
    return name ? name.charAt(0).toLocaleUpperCase(locale()) + name.slice(1) : value;
  } catch {
    return value;
  }
}

function episodeLabel(v: MetaVideo): string {
  return v.season != null && v.episode != null ? t("T{season} E{episode}", { season: v.season, episode: v.episode }) : "";
}

export function Details({ item, onBack, onOpen, onEntity }: Props) {
  const { contentType, id } = item;
  const player = usePlayer();
  const [data, setData] = useState<DetailsData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [season, setSeason] = useState<number | null>(null);
  const [selected, setSelected] = useState<MetaVideo | "movie" | null>(null);
  // "Recarregar" of the sources panel: counts clicks for the title/episode shown (0 for another one).
  const [reload, setReload] = useState({ at: "", n: 0 });
  const [reloading, setReloading] = useState(false);
  const [fromStart, setFromStart] = useState(false);
  /** Torrent source being started by the P2P engine (before the player opens). */
  const [torrent, setTorrent] = useState<StreamEntry | null>(null);
  const [trailerBusy, setTrailerBusy] = useState(false);
  const [progress, setProgress] = useState<TitleProgress | null>(null);
  const version = useLibraryVersion();
  const sources = useRef<HTMLElement>(null);
  const [sourceSettings, setSourceSettings] = useState<SourceSettings | null>(null);
  const screens = useScreens();
  const [trailerLanguage, setTrailerLanguage] = useState("original");
  const localizedTrailer = trailerLanguage !== "original";
  /** The sources panel's list in order (for "Tentar outra fonte se falhar"). */
  const listed = useRef<{ list: ListedStream[]; torbox: TorboxStates }>({ list: [], torbox: {} });
  /** A source that is starting while the next ones wait in case it fails. */
  const attempt = useRef<{ key: string; next: StreamEntry[]; loaded: boolean; timer: number } | null>(null);

  useEffect(() => {
    api.settingsGet().then((v) => {
      setSourceSettings(v.settings.sources);
      setTrailerLanguage(v.settings.trailer?.language ?? "original");
    }, () => {});
  }, []);

  // Saved progress of this title (NextGen data only; refreshed after each save).
  useEffect(() => {
    let cancelled = false;
    api.libraryTitleProgress(id).then((p) => !cancelled && setProgress(p), () => {});
    return () => {
      cancelled = true;
    };
  }, [id, version]);

  useEffect(() => {
    let cancelled = false;
    setData(null);
    setError(null);
    setSelected(null);
    api.details(contentType, id).then(
      (d) => {
        if (cancelled) return;
        setData(d);
        // Then completed from TMDB (when it is set up); the addon's details stay if it fails.
        if (!d.enriched) api.detailsEnriched(contentType, id).then((e) => !cancelled && setData(e), () => {});
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
    // The panel is sticky on the right: keep it in view (series: chosen far down the episode list).
    if (selected) sources.current?.scrollIntoView({ behavior: "smooth", block: "nearest" });
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
    void reuseLastLink(target, start);
  };

  /** "Marcar como assistido" (`markWatched` / `unmarkWatched`): a movie, an episode or a season. */
  const setWatched = async (targets: (MetaVideo | "movie")[], watched: boolean) => {
    if (!data) return;
    const m = data.meta;
    await api.librarySetWatched(
      { contentType: m.contentType, id: m.id, name: m.name, poster: m.poster, releaseInfo: m.releaseInfo },
      targets.map((v) => (v === "movie" ? { season: null, episode: null, videoId: m.id } : { season: v.season, episode: v.episode, videoId: v.id })),
      watched,
    ).catch((e) => window.alert(errorText(e)));
  };

  /** "Episódio aleatório" (`RandomEpisodePicker` of the original): an aired episode of a regular
   *  season, unwatched ones first, not repeating until every candidate was picked (this session). */
  const randomEpisode = () => {
    if (!data) return;
    const aired = data.meta.videos.filter((v) => v.season != null && v.season > 0 && v.episode != null && hasAired(v.released));
    if (!aired.length) return;
    const watched = new Set((progress?.entries ?? []).filter((e) => e.completed).map((e) => `${e.season}:${e.episode}`));
    const key = `nuvio.shuffle.${data.meta.id}`;
    let used: string[] = [];
    try {
      used = JSON.parse(sessionStorage.getItem(key) ?? "[]");
    } catch {
      used = [];
    }
    const current = selected && selected !== "movie" ? selected.id : null;
    const unwatched = aired.filter((v) => !watched.has(`${v.season}:${v.episode}`));
    const base = unwatched.length ? unwatched : aired;
    let pool = base.filter((v) => !used.includes(v.id) && v.id !== current);
    if (!pool.length) {
      used = [];
      pool = base.filter((v) => v.id !== current);
      if (!pool.length) pool = base;
    }
    const pick = pool[Math.floor(Math.random() * pool.length)];
    try {
      sessionStorage.setItem(key, JSON.stringify([...used, pick.id]));
    } catch {
      /* not remembered */
    }
    setSeason(pick.season);
    choose(pick);
  };

  /** "Reutilizar último link": plays the link this movie/episode last played from, if fresh. */
  const reuseLastLink = async (target: MetaVideo | "movie", fromBeginning: boolean) => {
    if (!data || !sourceSettings?.reuseLastLink) return;
    const video = target === "movie" ? null : target;
    const item = playbackItem(data.meta, video);
    if (!(await api.lastLinkHas(item).catch(() => false))) return;
    const entry = entryFor(target);
    const at = !fromBeginning && entry?.inProgress ? entry.positionMs / 1000 : null;
    void player.play(() => api.lastLinkPlay(at, item), {
      title: data.meta.name,
      subtitle: video ? `${episodeLabel(video)} · ${video.title}` : data.meta.releaseInfo ?? undefined,
      view: "full",
      episode: video ? { meta: data.meta, video } : undefined,
      movie: video ? undefined : data.meta,
    });
  };

  /** Trailer in the app's player; closing it returns here. Not saved to history or progress.
   *  With "Idioma dos trailers" set, YouTube is searched for that language first; without a fitting
   *  video the addon's trailer plays. */
  const playTrailer = async () => {
    if (!data || (!data.trailer && !localizedTrailer)) return;
    setTrailerBusy(true);
    const addonTrailer = data.trailer;
    const start = async () => {
      if (localizedTrailer) {
        try {
          return await api.trailerPlayLocalized(data.meta.name, data.meta.releaseInfo ?? null);
        } catch (e) {
          if (!addonTrailer) throw new Error(t("Nenhum trailer nesse idioma foi encontrado no YouTube."));
          console.info("trailer: sem versão no idioma escolhido", errorText(e));
        }
      }
      return api.trailerPlay(addonTrailer!.key);
    };
    await player.play(start, { title: data.meta.name, subtitle: addonTrailer && !localizedTrailer ? t("Trailer · {name}", { name: addonTrailer.name }) : t("Trailer"), view: "full" });
    setTrailerBusy(false);
  };

  const downloadRequest = (stream: StreamEntry) => {
    const video = selected && selected !== "movie" ? selected : null;
    return {
      key: stream.key, title: data!.meta.name, subtitle: video ? `${episodeLabel(video)} · ${video.title}` : null,
      poster: data!.meta.poster, season: video?.season ?? null, episode: video?.episode ?? null, playback: playbackItem(data!.meta, video),
    };
  };

  const [preparing, setPreparing] = useState<string | null>(null);

  const play = async (stream: StreamEntry, prepared = false, torbox: TorboxStates = {}, chain: StreamEntry[] | null = null) => {
    if (!data) return;
    const video = selected && selected !== "movie" ? selected : null;
    if (stream.kind === "external") {
      api.streamOpenExternal(stream.key).catch((e) => window.alert(errorText(e)));
      return;
    }
    // Sources tried next if this one does not start (Settings → Fontes).
    const s = sourceSettings;
    const next = s?.fallbackOnFailure ? chain ?? fallbackAfter(listed.current.list, stream.key, listed.current.torbox, s.fallbackMaxSources - 1) : [];
    window.clearTimeout(attempt.current?.timer);
    attempt.current = null;
    let direct = stream.kind === "direct";
    if (!prepared && stream.kind !== "direct") {
      setPreparing(stream.key);
      try {
        direct = await viaTorbox(stream, torbox, { season: video?.season ?? null, episode: video?.episode ?? null });
      } catch (e) {
        setPreparing(null);
        window.alert(errorText(e));
        return;
      }
      setPreparing(null);
    }
    if (stream.kind === "torrent" && !prepared && !direct) {
      setTorrent(stream);
      return;
    }
    const subtitle = video ? `${episodeLabel(video)} · ${video.title}` : data.meta.releaseInfo ?? undefined;
    const start = resumeAt !== null ? resumeAt / 1000 : null;
    const begin = stream.kind === "youTube"
      ? () => api.streamPlayYoutube(stream.key, start, playbackItem(data.meta, video))
      : () => api.playStream(stream.key, start, playbackItem(data.meta, video));
    if (next.length) {
      const timer = window.setTimeout(() => {
        if (attempt.current?.key === stream.key && !attempt.current.loaded) failed(stream.key);
      }, (s?.fallbackStartupTimeoutSeconds ?? 15) * 1000);
      attempt.current = { key: stream.key, next, loaded: false, timer };
    }
    void player
      .play(begin, {
        title: data.meta.name,
        subtitle,
        view: "full",
        episode: video ? { meta: data.meta, video } : undefined,
        movie: video ? undefined : data.meta,
        torrent: stream.kind === "torrent" && !direct,
        download: downloadRequest(stream),
      })
      .then((ok) => {
        // A torrent whose player could not open: its P2P session ends too.
        if (!ok && stream.kind === "torrent" && !direct) void api.torrentStop();
        if (!ok) failed(stream.key);
      });
  };

  /** The source did not start: the next one of the chain, if any. */
  const failed = (key: string) => {
    const current = attempt.current;
    if (!current || current.key !== key) return;
    window.clearTimeout(current.timer);
    attempt.current = null;
    const [next, ...rest] = current.next;
    if (!next) return;
    showToast(t("A fonte não abriu. Tentando a próxima: {name}", { name: next.name || t("Fonte") }));
    void play(next, false, listed.current.torbox, rest);
  };

  // Started (loaded) or failed (error) while waiting with a fallback chain.
  useEffect(() => {
    const current = attempt.current;
    if (!current) return;
    if (player.snapshot?.error) failed(current.key);
    else if (player.snapshot?.loaded) {
      window.clearTimeout(current.timer);
      attempt.current = null;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [player.snapshot?.error, player.snapshot?.loaded]);
  useEffect(() => {
    // The user closed the player: nothing more is tried.
    if (!player.session && attempt.current) {
      window.clearTimeout(attempt.current.timer);
      attempt.current = null;
    }
  }, [player.session]);

  if (error) {
    return (
      <div className="screen-message">
        <button className="back" onClick={onBack}>←</button> {t("Não foi possível abrir os detalhes:")} {error}
      </div>
    );
  }
  if (!data) return <div className="screen-message">{t("Carregando detalhes…")}</div>;
  const m = data.meta;
  const shownId = selected === "movie" ? m.id : selected?.id ?? "";
  const movieEntry = entryFor("movie");
  const resumeVideo = progress?.resume ? m.videos.find((v) => v.season === progress.resume!.season && v.episode === progress.resume!.episode) : undefined;

  const seasonPoster = episodes.find((v) => v.seasonPoster)?.seasonPoster ?? null;
  const seasonAired = episodes.filter((v) => hasAired(v.released));
  const seasonWatched = seasonAired.length > 0 && seasonAired.every((v) => entryFor(v)?.completed);

  return (
    <div className={`details details-bg-${screens.detailsBackground}`} data-id={m.id} data-content-type={m.contentType}>
      {screens.detailsBackground !== "none" && <div className="details-backdrop" style={m.background ? { backgroundImage: `url("${m.background}")` } : undefined} />}
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
            <ExternalRatings contentType={m.contentType} id={m.id} />
            {m.genres.length > 0 && <div className="chips">{m.genres.map((g) => <span key={g} className="chip">{g}</span>)}</div>}
            {m.description && <p className="details-desc">{m.description}</p>}
            {m.director.length > 0 && <div className="muted">{t("Direção:")} {m.director.join(", ")}</div>}
            <div className="details-actions">
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
              {!isSeries && (
                <button className={`chip ${movieEntry?.completed ? "active" : ""}`} data-testid="mark-watched" onClick={() => void setWatched(["movie"], !movieEntry?.completed)}>
                  {movieEntry?.completed ? t("✓ Assistido") : t("Marcar como assistido")}
                </button>
              )}
              {isSeries && (
                <button className="chip" data-testid="random-episode" title={t("Escolhe um episódio já lançado, de preferência um que você ainda não viu, sem repetir até passar por todos.")} onClick={randomEpisode}>
                  {t("🔀 Episódio aleatório")}
                </button>
              )}
              {(data.trailer || localizedTrailer) && (
                <button className="chip" data-testid="watch-trailer" disabled={trailerBusy} onClick={() => void playTrailer()}>
                  {trailerBusy ? t("Carregando trailer…") : t("▶ Assistir trailer")}
                </button>
              )}
              <FavoriteButton item={libraryItemFromDetails(m)} variant="pill" />
            </div>
            {!isSeries && movieEntry && (
              <div className="muted small" data-testid="movie-progress">
                {movieEntry.completed ? t("✓ Assistido") : [t("{time} assistidos", { time: clock(movieEntry.positionMs) }), remaining(movieEntry)].filter(Boolean).join(" · ")}
              </div>
            )}
            <div className="muted small">{t("Detalhes por")} {addonLabel(data.addonName)}</div>
            {screens.detailsShowInfo && <dl className="facts">
              {(
                [
                  [t("Classificação"), m.ageRating],
                  [t("Situação"), m.status && t(STATUS[m.status] ?? m.status)],
                  [t("Roteiro"), m.writer.join(", ")],
                  [t("País"), regionNames(m.country)],
                  [t("Idioma"), languageName(m.language)],
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

        {isSeries && (
          <section className={`details-section ${seasonPoster ? "with-season-poster" : ""}`}>
            {seasonPoster && <img className="season-poster" src={seasonPoster} alt={t("Pôster da temporada")} data-testid="season-poster" />}
            <div className="season-tabs" role="tablist">
              {data.seasons.map((s) => (
                <button key={s} role="tab" aria-selected={season === s} className={`chip ${season === s ? "active" : ""}`} onClick={() => setSeason(s)}>
                  {s === 0 ? t("Especiais") : t("Temporada {season}", { season: s })}
                </button>
              ))}
              {seasonAired.length > 0 && (
                <button className="chip season-watched" data-testid="mark-season" onClick={() => void setWatched(seasonAired, !seasonWatched)}>
                  {seasonWatched ? t("Desmarcar temporada") : t("✓ Marcar temporada como assistida")}
                </button>
              )}
            </div>
            <div className={`episodes episodes-${screens.detailsEpisodeStyle}`}>
              {episodes.map((v) => {
                const e = entryFor(v);
                const spoiler = screens.detailsBlurUnwatched && !e?.completed && !e?.inProgress;
                return (
                  <div key={v.id} className={`episode-wrap ${spoiler ? "spoiler" : ""}`}>
                  <button
                    className={`episode-watched ${e?.completed ? "on" : ""}`}
                    aria-label={e?.completed ? t("Desmarcar como assistido") : t("Marcar como assistido")}
                    title={e?.completed ? t("Desmarcar como assistido") : t("Marcar como assistido")}
                    disabled={!hasAired(v.released)}
                    onClick={() => void setWatched([v], !e?.completed)}
                    data-testid="episode-watched"
                  >
                    ✓
                  </button>
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
                  </div>
                );
              })}
            </div>
          </section>
        )}

        {screens.detailsShowCast && m.cast.length > 0 && (
          <section className="details-section">
            <h2>{t("Elenco")}</h2>
            <div className="cast">
              {m.cast.slice(0, 30).map((p) => {
                const body = (
                  <>
                    <div className="person-photo">{p.photo ? <img src={p.photo} alt="" loading="lazy" /> : p.name.slice(0, 1)}</div>
                    <div className="small">{p.name}</div>
                    {p.role && <div className="muted small">{t(p.role)}</div>}
                  </>
                );
                return p.tmdbId != null && onEntity ? (
                  <button key={`${p.name}-${p.role ?? ""}`} className="person person-link" onClick={() => onEntity("person", p.tmdbId!, p.name)} data-person={p.tmdbId}>{body}</button>
                ) : (
                  <div key={`${p.name}-${p.role ?? ""}`} className="person">{body}</div>
                );
              })}
            </div>
          </section>
        )}

        <TitleRow title={data.extras?.collectionName ?? t("Coleção")} items={data.extras?.collection ?? []} onOpen={onOpen} testId="tmdb-collection" />
        <TitleRow title={t("Mais como este")} items={data.extras?.moreLikeThis ?? []} onOpen={onOpen} testId="more-like-this" />
        {((data.extras?.companies.length ?? 0) > 0 || (data.extras?.networks.length ?? 0) > 0) && (
          <section className="details-section production" data-testid="production">
            <h2>{t("Produção")}</h2>
            <dl className="facts production-facts">
              <Companies label={t("Estúdios")} list={data.extras?.companies ?? []} kind="company" onEntity={onEntity} />
              <Companies label={t("Canais")} list={data.extras?.networks ?? []} kind="network" onEntity={onEntity} />
            </dl>
          </section>
        )}
        </div>

        {selected && (
          <aside className="sources-panel" ref={sources} aria-label={t("Fontes")} data-testid="sources-panel">
            <header className="sources-header">
              <div>
                <h2>{t("Fontes")}</h2>
                <div className="muted small">
                  {selected === "movie" ? m.name : `${episodeLabel(selected)} · ${selected.title}`}
                  {resumeAt !== null && <span data-testid="resume-note"> · {t("continua de {time}", { time: clock(resumeAt) })}</span>}
                </div>
              </div>
              <div className="sources-actions">
                <button
                  className={`icon-btn sources-reload ${reloading ? "busy" : ""}`}
                  aria-label={t("Recarregar fontes")}
                  title={t("Recarregar fontes")}
                  onClick={() => setReload((r) => ({ at: shownId, n: (r.at === shownId ? r.n : 0) + 1 }))}
                  data-testid="sources-reload"
                >
                  <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M20 11a8 8 0 1 0-2.3 5.7" />
                    <path d="M20 4v7h-7" />
                  </svg>
                </button>
                <button className="icon-btn" aria-label={t("Fechar fontes")} onClick={() => setSelected(null)}>✕</button>
              </div>
            </header>
            <div className="sources-body">
              {preparing && <div className="muted small torbox-preparing" role="status"><span className="spinner" aria-hidden /> {t("Abrindo pelo serviço debrid…")}</div>}
              <StreamList
                contentType={m.contentType}
                id={selected === "movie" ? m.id : selected.id}
                onPlay={(stream, torbox) => void play(stream, false, torbox)}
                onDownload={(stream) => void startDownload(downloadRequest(stream))}
                reload={reload.at === shownId ? reload.n : 0}
                onLoading={setReloading}
                onList={(list, torbox) => { listed.current = { list, torbox }; }}
                showBadges={sourceSettings?.showBadges ?? true}
                auto={sourceSettings && sourceSettings.autoPlayMode !== "MANUAL" && !player.session
                  ? { mode: sourceSettings.autoPlayMode, regex: sourceSettings.autoPlayRegex, addons: sourceSettings.autoPlaySelectedAddons, timeoutSeconds: sourceSettings.autoPlayTimeoutSeconds }
                  : null}
                onAuto={(stream, torbox) => {
                  showToast(t("Fonte escolhida automaticamente: {name}", { name: stream.name || t("Fonte") }));
                  void play(stream, false, torbox);
                }}
              />
            </div>
          </aside>
        )}
        </div>
      </div>
      {torrent && (
        <TorrentStart
          stream={torrent}
          onCancel={() => setTorrent(null)}
          onReady={() => {
            const s = torrent;
            setTorrent(null);
            void play(s, true);
          }}
        />
      )}
    </div>
  );
}

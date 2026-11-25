// Descobrir (port of the original's Discover, in its own tab): browse any catalog of the installed
// addons by type, catalog and genre, as a poster grid that loads more as it scrolls.
// - Catalogs: the original's `supportsDiscover` rule (no required search; a required genre needs
//   its options), movies/series/anime first.
// - Genre: "Todos" unless the catalog requires one (then its first option).
// - Three groups only: Filmes, Séries and Anime. Anime = catalogs of type "anime" plus movie/series
//   catalogs named after anime (e.g. "Anime - Popular"); other types (TV, collections…) are left out.
// - The chosen catalog and genre are remembered (`DiscoverSelectionStorage`).
// - Paging stops when the addon has no more pages or sends the same page again twice in a row (the
//   original's duplicate-page guard). Pages come from the shared catalog page cache.

import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorText } from "../api";
import { PosterCard } from "../components/PosterCard";
import { useWhenVisible } from "../components/useInView";
import type { DiscoverCatalog, MetaPreview } from "../types";
import { localizeCatalogTitle, t } from "../i18n";

interface Props {
  onOpen(item: MetaPreview): void;
}

const SELECTION_KEY = "nuvio.discover.selection";

type Group = "movie" | "series" | "anime" | "all";
const GROUPS: { id: Group; label: string }[] = [
  { id: "movie", label: "Filmes" },
  { id: "series", label: "Séries" },
  { id: "anime", label: "Anime" },
];
function groupOf(c: DiscoverCatalog): Group | null {
  const type = c.contentType.toLowerCase();
  if (type === "anime" || /anime/i.test(c.name)) return "anime";
  if (type === "movie") return "movie";
  if (type === "series") return "series";
  return null;
}
const MAX_DUPLICATE_PAGES = 2;

interface Feed {
  items: MetaPreview[];
  next: number | null;
  loading: boolean;
  error: string | null;
  duplicates: number;
}
const FRESH: Feed = { items: [], next: null, loading: true, error: null, duplicates: 0 };

function loadSelection(): { key?: string; genre?: string | null } {
  try {
    const v = JSON.parse(localStorage.getItem(SELECTION_KEY) ?? "{}");
    return v && typeof v === "object" ? v : {};
  } catch {
    return {};
  }
}
function saveSelection(key: string, genre: string | null) {
  try {
    localStorage.setItem(SELECTION_KEY, JSON.stringify({ key, genre }));
  } catch {
    /* this session only */
  }
}

/** `resolveGenreSelection`: a valid chosen genre, else the first one when required, else all. */
function genreFor(catalog: DiscoverCatalog, wanted: string | null | undefined): string | null {
  if (!catalog.genres.length) return null;
  if (wanted && catalog.genres.includes(wanted)) return wanted;
  return catalog.genreRequired ? catalog.genres[0] : null;
}

export function Discover({ onOpen }: Props) {
  const [catalogs, setCatalogs] = useState<DiscoverCatalog[] | null>(null);
  const [group, setGroup] = useState<Group>("movie");
  const [failed, setFailed] = useState<string | null>(null);
  const [key, setKey] = useState<string | null>(null);
  const [genre, setGenre] = useState<string | null>(null);
  const [feed, setFeed] = useState<Feed>(FRESH);
  // The selection a page belongs to: an answer for an older selection is dropped.
  const current = useRef("");

  useEffect(() => {
    let alive = true;
    api.discoverCatalogs().then(
      (list) => {
        if (!alive) return;
        const usable = list.filter((c) => groupOf(c) !== null);
        setCatalogs(usable);
        const saved = loadSelection();
        const first = usable.find((c) => c.key === saved.key) ?? usable.find((c) => groupOf(c) === "movie") ?? usable[0];
        if (first) {
          setGroup(groupOf(first)!);
          setKey(first.key);
          setGenre(genreFor(first, saved.genre));
        }
      },
      (e) => alive && setFailed(errorText(e)),
    );
    return () => {
      alive = false;
    };
  }, []);

  const catalog = catalogs?.find((c) => c.key === key) ?? null;
  const ofGroup = group === "all" ? catalogs ?? [] : catalogs?.filter((c) => groupOf(c) === group) ?? [];
  const shown = catalog && (group === "all" || groupOf(catalog) === group) ? catalog : null;

  const load = useCallback(async (cat: DiscoverCatalog, g: string | null, skip: number | null) => {
    const id = `${cat.key}#${g ?? ""}`;
    setFeed((f) => ({ ...(skip === null ? FRESH : f), loading: true, error: null }));
    try {
      const page = await api.catalogPage(cat.key, skip, g);
      if (current.current !== id) return;
      setFeed((f) => {
        const base = skip === null ? [] : f.items;
        const seen = new Set(base.map((i) => `${i.contentType}:${i.id}`));
        const added = page.items.filter((i) => !seen.has(`${i.contentType}:${i.id}`));
        const duplicates = skip !== null && added.length === 0 ? f.duplicates + 1 : 0;
        const more = (cat.supportsPagination || page.rawItemCount >= 100) && page.items.length > 0 && duplicates < MAX_DUPLICATE_PAGES;
        return { items: [...base, ...added], next: more ? page.nextSkip : null, loading: false, error: null, duplicates };
      });
    } catch (e) {
      if (current.current !== id) return;
      setFeed((f) => ({ ...f, loading: false, next: null, error: errorText(e) }));
    }
  }, []);

  useEffect(() => {
    if (!catalog || groupOf(catalog) !== group) return;
    current.current = `${catalog.key}#${genre ?? ""}`;
    saveSelection(catalog.key, genre);
    void load(catalog, genre, null);
  }, [catalog, genre, group, load]);

  const loadMore = () => {
    if (shown && !feed.loading && feed.next !== null && !feed.error) void load(shown, genre, feed.next);
  };
  const sentinel = useWhenVisible<HTMLDivElement>(loadMore, [feed.items.length, feed.loading, key, genre], "700px");

  const pick = (next: DiscoverCatalog | undefined) => {
    if (!next) return;
    setKey(next.key);
    setGenre(genreFor(next, genre));
  };
  const chooseGroup = (g: Group) => {
    if (g === group) return;
    setGroup(g);
    pick(g === "all" ? catalogs?.[0] : catalogs?.find((c) => groupOf(c) === g));
  };

  return (
    <div className="discover" data-testid="discover">
      <header className="discover-header">
        <h1>{t("Descobrir")}</h1>
        {catalogs && catalogs.length > 0 && (
          <div className="discover-filters">
            <div className="discover-types" role="tablist" aria-label={t("Tipo")}>
              {GROUPS.map((g) => (
                <button
                  key={g.id}
                  role="tab"
                  aria-selected={g.id === group}
                  className={`chip ${g.id === group ? "active" : ""}`}
                  data-type={g.id}
                  onClick={() => chooseGroup(g.id)}
                >
                  {t(g.label)}
                </button>
              ))}
            </div>
            <label className="discover-field">
              <span>{t("Tipo")}</span>
              <select
                className="select pill-select"
                data-testid="discover-type"
                value={group === "anime" ? "all" : group}
                onChange={(e) => chooseGroup(e.target.value as Group)}
              >
                <option value="all">{t("Todos")}</option>
                <option value="movie">{t("Filmes")}</option>
                <option value="series">{t("Séries")}</option>
              </select>
            </label>
            {shown && (
            <label className="discover-field">
              <span>{t("Catálogo")}</span>
              <select className="select pill-select" value={shown.key} onChange={(e) => pick(catalogs.find((c) => c.key === e.target.value))} data-testid="discover-catalog">
                {ofGroup.map((c) => (
                  <option key={c.key} value={c.key}>
                    {localizeCatalogTitle(c.name)}
                  </option>
                ))}
              </select>
            </label>
            )}
            {shown && shown.genres.length > 0 && (
              <label className="discover-field">
                <span>{t("Gênero")}</span>
                <select className="select pill-select" value={genre ?? ""} onChange={(e) => setGenre(e.target.value || null)} data-testid="discover-genre">
                  {!shown.genreRequired && <option value="">{t("Todos")}</option>}
                  {shown.genres.map((g) => (
                    <option key={g} value={g}>
                      {g}
                    </option>
                  ))}
                </select>
              </label>
            )}
          </div>
        )}
        {shown && <div className="muted small discover-context">{shown.addonName} · {t(shown.typeLabel)}</div>}
      </header>

      {failed && <div className="screen-message">{t("Não foi possível carregar os catálogos ({error}).", { error: failed })}</div>}
      {!failed && catalogs === null && (
        <div className="grid discover-grid">{Array.from({ length: 12 }, (_, i) => <div key={i} className="card skeleton" />)}</div>
      )}
      {catalogs && catalogs.length === 0 && (
        <div className="screen-message">{t("Nenhum addon instalado tem catálogos para descobrir.")}</div>
      )}

      {catalogs && catalogs.length > 0 && ofGroup.length === 0 && (
        <div className="screen-message" data-testid="discover-empty-group">
          {t(group === "anime" ? "Nenhum addon instalado tem catálogos de anime." : group === "series" ? "Nenhum addon instalado tem catálogos de séries." : "Nenhum addon instalado tem catálogos de filmes.")}
        </div>
      )}

      {shown && (
        <>
          <div className="grid discover-grid">
            {feed.items.map((item) => <PosterCard key={`${item.contentType}/${item.id}`} item={item} onOpen={onOpen} />)}
            {feed.loading && feed.items.length === 0 && Array.from({ length: 12 }, (_, i) => <div key={i} className="card skeleton" />)}
          </div>
          {!feed.loading && feed.items.length === 0 && (
            <div className="screen-message">
              {feed.error ? t("Não foi possível carregar este catálogo ({error}).", { error: feed.error }) : t("Nenhum título neste catálogo.")}
            </div>
          )}
          {feed.loading && feed.items.length > 0 && <div className="discover-more"><span className="spinner" aria-label={t("Carregando")} /></div>}
          <div ref={sentinel} className="grid-sentinel" />
        </>
      )}
    </div>
  );
}

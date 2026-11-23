// Search: results shown as Home-style horizontal rows, one per catalog (as the original).
//
// Fast as you type:
// - every catalog's results appear as soon as that addon answers (`search_stream` events), the
//   slowest addon no longer holds the others back;
// - while a new query is on its way, the results already loaded for a shorter query are filtered
//   on the spot (typing "vinga" after "ving" shows matches at once);
// - answers of this session are kept in memory: going back to a query is instant;
// - typing is debounced lightly (220 ms); an outdated answer never replaces a newer one.
// Recent searches are remembered (last 12) and shown when the field is empty.
//
// Discover (empty field) is the original's type / catalog / genre browser: the listable catalogs
// of the enabled addons (`supportsDiscover`), the genre options of each catalog's `genre` extra,
// the catalog remembered per profile, endless paging with the original's duplicate-page guard, and
// the Home "hide unreleased" filter (applied by Rust). See `services::discover`.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, dataEpoch, errorText, onDataEpoch, onRemoteSettingsChanged } from "../api";
import { PosterCard } from "../components/PosterCard";
import { ResultRow } from "../components/ResultRow";
import { useWhenVisible } from "../components/useInView";
import { invoke, listen } from "../tauri";
import type { DiscoverSource, DiscoverView, MetaPreview, SearchSection } from "../types";
import { addonLabel, localizeCatalogTitle, mediaTypeLabel, t } from "../i18n";

interface Props {
  onOpen(item: MetaPreview): void;
}

interface Part { requestId: number; order: number; section: SearchSection }
interface Result { sections: { order: number; section: SearchSection }[]; done: boolean }

const DEBOUNCE_MS = 220;
const HISTORY_KEY = "nuvio.search.history";
const HISTORY_MAX = 12;
const CACHE_MAX = 40;

// ---- session state (kept while switching tabs; dropped on a profile/addon change) ----
let lastQuery = "";
const cache = new Map<string, Result>();
onDataEpoch(() => {
  lastQuery = "";
  cache.clear();
});

const norm = (s: string) => s.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase().trim();

function remember(q: string, r: Result) {
  cache.delete(q);
  cache.set(q, r);
  while (cache.size > CACHE_MAX) cache.delete(cache.keys().next().value!);
}

/** Results already loaded for a shorter query, filtered by the new one (shown while it loads). */
function provisional(q: string): { order: number; section: SearchSection }[] | null {
  const nq = norm(q);
  let best: string | null = null;
  for (const k of cache.keys()) if (nq.startsWith(norm(k)) && (!best || k.length > best.length)) best = k;
  if (!best) return null;
  return cache
    .get(best)!
    .sections.map((s) => ({ ...s, section: { ...s.section, items: s.section.items.filter((i) => norm(i.name).includes(nq)) } }))
    .filter((s) => s.section.items.length > 0);
}

function loadHistory(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(HISTORY_KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((x) => typeof x === "string").slice(0, HISTORY_MAX) : [];
  } catch {
    return [];
  }
}
function saveHistory(list: string[]) {
  try {
    localStorage.setItem(HISTORY_KEY, JSON.stringify(list));
  } catch {
    /* kept for this session only */
  }
}

const itemKey = (item: MetaPreview) => `${item.contentType}:${item.id}`;

/** Appends `incoming` to `existing`, dropping items already shown (same `type:id`). */
function mergeDiscover(existing: MetaPreview[], incoming: MetaPreview[]): MetaPreview[] {
  const seen = new Set(existing.map(itemKey));
  return [...existing, ...incoming.filter((item) => !seen.has(itemKey(item)))];
}

/** Genre the picker should use with this catalog (`resolveGenreSelection`): the requested option
 *  when the catalog offers it, else the required genre's first option, else all genres. */
function resolveGenre(source: DiscoverSource, requested: string): string {
  if (!source.genreOptions.length) return "";
  if (requested && source.genreOptions.includes(requested)) return requested;
  return source.defaultGenre ?? "";
}

export function Search({ onOpen }: Props) {
  const [query, setQuery] = useState(lastQuery);
  const [result, setResult] = useState<Result | null>(() => cache.get(lastQuery.trim()) ?? null);
  const [preview, setPreview] = useState<{ order: number; section: SearchSection }[] | null>(null);
  const [history, setHistory] = useState<string[]>(loadHistory);
  const requestId = useRef(0);
  const pending = useRef<{ id: number; q: string; epoch: number; parts: Part[] } | null>(null);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => input.current?.focus(), []);

  const addHistory = (q: string) => {
    const t = q.trim();
    if (t.length < 2) return;
    setHistory((h) => {
      const next = [t, ...h.filter((x) => norm(x) !== norm(t))].slice(0, HISTORY_MAX);
      saveHistory(next);
      return next;
    });
  };
  const removeHistory = (q: string) =>
    setHistory((h) => {
      const next = h.filter((x) => x !== q);
      saveHistory(next);
      return next;
    });
  const clearHistory = () => {
    setHistory([]);
    saveHistory([]);
  };

  // Streamed answers: kept only for the latest request.
  useEffect(() => {
    const offPart = listen<Part>("search-part", (p) => {
      const cur = pending.current;
      if (!cur || p.requestId !== cur.id || cur.epoch !== dataEpoch()) return;
      cur.parts.push(p);
      const sections = [...cur.parts].sort((a, b) => a.order - b.order).map(({ order, section }) => ({ order, section }));
      setResult({ sections, done: false });
      setPreview(null);
    });
    const offDone = listen<{ requestId: number }>("search-done", (d) => {
      const cur = pending.current;
      if (!cur || d.requestId !== cur.id) return;
      const sections = [...cur.parts].sort((a, b) => a.order - b.order).map(({ order, section }) => ({ order, section }));
      const r = { sections, done: true };
      if (cur.epoch === dataEpoch()) remember(cur.q, r);
      pending.current = null;
      setResult(r);
      setPreview(null);
    });
    return () => {
      offPart();
      offDone();
    };
  }, []);

  useEffect(() => {
    const q = query.trim();
    lastQuery = query;
    if (q.length < 2) {
      pending.current = null;
      setResult(null);
      setPreview(null);
      return;
    }
    const hit = cache.get(q);
    if (hit) {
      pending.current = null;
      setResult(hit);
      setPreview(null);
      return;
    }
    // Instant: filter what is already loaded; the real answer follows.
    setPreview(provisional(q));
    setResult(null);
    const timer = window.setTimeout(() => {
      const id = ++requestId.current;
      pending.current = { id, q, epoch: dataEpoch(), parts: [] };
      invoke("search_stream", { query: q, requestId: id }).catch(() => {
        if (pending.current?.id === id) {
          pending.current = null;
          setResult({ sections: [], done: true });
        }
      });
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [query]);

  // A search left on screen for a moment with results goes to the history.
  useEffect(() => {
    if (!result?.done || !result.sections.length) return;
    const q = query;
    const t = window.setTimeout(() => addHistory(q), 2500);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [result]);

  const shown = result?.sections.length ? result.sections : preview ?? [];
  const loading = query.trim().length >= 2 && !result?.done;
  const open = (item: MetaPreview) => {
    addHistory(query);
    onOpen(item);
  };
  const empty = useMemo(() => query.trim().length < 2, [query]);

  // ---- Discover: the original's "Movies / TMDB Popular / All Genres" filters, backed by every
  // listable catalog of the enabled addons (a catalog's `genre` extra provides its genres). ----
  const [discover, setDiscover] = useState<DiscoverView | null>(null);
  const [discoverType, setDiscoverType] = useState("");
  const [discoverSourceKey, setDiscoverSourceKey] = useState("");
  const [discoverGenre, setDiscoverGenre] = useState("");
  const [discoverItems, setDiscoverItems] = useState<MetaPreview[]>([]);
  const [discoverNext, setDiscoverNext] = useState<number | null>(null);
  const [discoverDuplicates, setDiscoverDuplicates] = useState(0);
  const [discoverLoading, setDiscoverLoading] = useState(false);
  const [discoverStarted, setDiscoverStarted] = useState(false);
  const [discoverError, setDiscoverError] = useState<string | null>(null);
  const [discoverReload, setDiscoverReload] = useState(0);
  const discoverRequest = useRef(0);
  const discoverBusy = useRef(false);
  const discoverItemsRef = useRef<MetaPreview[]>([]);
  discoverItemsRef.current = discoverItems;
  const discoverNextRef = useRef<number | null>(null);
  discoverNextRef.current = discoverNext;
  const discoverDuplicatesRef = useRef(0);
  discoverDuplicatesRef.current = discoverDuplicates;

  const discoverSource = discover?.sources.find((s) => s.key === discoverSourceKey) ?? null;
  const catalogOptions = discover?.sources.filter((s) => s.contentType === discoverType) ?? [];
  const discoverSourceRef = useRef<DiscoverSource | null>(null);
  discoverSourceRef.current = discoverSource;
  const discoverGenreRef = useRef("");
  discoverGenreRef.current = discoverGenre;

  // Catalogs of the enabled addons + the catalog remembered for this profile.
  useEffect(() => {
    let cancelled = false;
    api.discoverSources().then(
      (view) => {
        if (cancelled) return;
        setDiscover(view);
        // `resolveDiscoverCatalog`: the saved catalog when it still exists, else the first.
        const first = view.sources.find((s) => s.key === view.selectedKey) ?? view.sources[0];
        if (first) {
          setDiscoverType(first.contentType);
          setDiscoverSourceKey(first.key);
          setDiscoverGenre(first.defaultGenre ?? "");
        }
      },
      (e) => {
        if (!cancelled) setDiscoverError(errorText(e));
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  // Home settings changed (this app or the account): refetch, keeping a genre the catalog still
  // offers (`hideUnreleasedContent` changes what the page shows).
  useEffect(() => {
    const refresh = () => {
      const source = discoverSourceRef.current;
      if (source) {
        const genre = resolveGenre(source, discoverGenreRef.current);
        if (genre !== discoverGenreRef.current) setDiscoverGenre(genre);
      }
      setDiscoverReload((n) => n + 1);
    };
    const offHome = api.onHomeSettingsChanged(refresh);
    const offRemote = onRemoteSettingsChanged(refresh);
    return () => {
      offHome();
      offRemote();
    };
  }, []);

  const loadDiscover = useCallback(
    async (reset: boolean) => {
      const source = discover?.sources.find((s) => s.key === discoverSourceKey);
      if (!source) return;
      const skip = reset ? 0 : discoverNextRef.current;
      if (skip === null) return;
      // A new catalog/genre supersedes a page in flight; another "load more" waits for it.
      if (!reset && discoverBusy.current) return;
      discoverBusy.current = true;
      const id = ++discoverRequest.current;
      setDiscoverLoading(true);
      setDiscoverError(null);
      if (reset) {
        setDiscoverItems([]);
        setDiscoverNext(null);
        setDiscoverDuplicates(0);
      }
      try {
        const existing = reset ? [] : discoverItemsRef.current.map(itemKey);
        const page = await api.discoverPage(
          source.key,
          skip,
          reset,
          existing,
          reset ? 0 : discoverDuplicatesRef.current,
          discoverGenre || null,
        );
        if (id !== discoverRequest.current) return;
        setDiscoverItems((prev) => (reset ? page.items : mergeDiscover(prev, page.items)));
        setDiscoverNext(page.nextSkip);
        setDiscoverDuplicates(page.duplicatePages);
      } catch (e) {
        if (id !== discoverRequest.current) return;
        setDiscoverError(errorText(e));
        setDiscoverNext(null);
      } finally {
        if (id === discoverRequest.current) {
          setDiscoverLoading(false);
          setDiscoverStarted(true);
          discoverBusy.current = false;
        }
      }
    },
    [discover, discoverSourceKey, discoverGenre],
  );

  // A new catalog/genre loads its first page; a Home settings change reloads it.
  useEffect(() => {
    if (!discoverSource) return;
    void loadDiscover(true);
  }, [discoverSource, discoverReload, loadDiscover]);

  const discoverSentinel = useWhenVisible<HTMLDivElement>(() => {
    if (discoverNextRef.current !== null && !discoverBusy.current) void loadDiscover(false);
  }, [discoverNext, discoverLoading, discoverItems.length, discoverSourceKey, discoverGenre]);

  const selectDiscoverSource = (source: DiscoverSource) => {
    setDiscoverSourceKey(source.key);
    setDiscoverGenre(resolveGenre(source, ""));
    void api.discoverSelect(source.key).catch(() => {});
  };
  const selectDiscoverType = (type: string) => {
    setDiscoverType(type);
    const first = discover?.sources.find((s) => s.contentType === type);
    if (first) selectDiscoverSource(first);
  };

  return (
    <div className="search">
      <div className="search-bar">
        <input
          ref={input}
          type="search"
          placeholder={t("Buscar filmes, séries…")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && addHistory(query)}
          aria-label={t("Buscar")}
          data-testid="search-input"
        />
        {loading && <span className="spinner" aria-label={t("Buscando")} />}
      </div>

      {empty && (
        <div className="discover">
          <h1 className="discover-title">{t("Discover")}</h1>
          {discover && discover.sources.length > 0 && (
            <>
              <div className="discover-filters">
                <select aria-label={t("Tipo")} value={discoverType} onChange={(e) => selectDiscoverType(e.target.value)}>
                  {discover.typeOptions.map((type) => (
                    <option key={type} value={type}>{mediaTypeLabel(type)}</option>
                  ))}
                </select>
                <select aria-label={t("Fontes")} value={discoverSourceKey} onChange={(e) => {
                  const source = discover.sources.find((s) => s.key === e.target.value);
                  if (source) selectDiscoverSource(source);
                }}>
                  {catalogOptions.map((s) => (
                    <option key={s.key} value={s.key}>{localizeCatalogTitle(s.catalogName)}</option>
                  ))}
                </select>
                <select
                  aria-label={t("Gêneros")}
                  value={discoverGenre}
                  disabled={!discoverSource || (discoverSource.genreOptions.length === 0 && !discoverSource.genreRequired)}
                  onChange={(e) => setDiscoverGenre(e.target.value)}
                >
                  {(!discoverSource || !discoverSource.genreRequired) && <option value="">{t("All Genres")}</option>}
                  {(discoverSource?.genreOptions ?? []).map((genre) => (
                    <option key={genre} value={genre}>{genre}</option>
                  ))}
                </select>
              </div>
              {discoverSource && (
                <div className="discover-context muted small">
                  {addonLabel(discoverSource.addonName)} · {localizeCatalogTitle(discoverSource.catalogName)}
                  {discoverGenre ? ` · ${discoverGenre}` : ""}
                </div>
              )}
              {discoverError && (
                <div className="discover-error" role="alert">
                  {t("Não foi possível carregar este catálogo ({error}).", { error: discoverError })}
                </div>
              )}
              {discoverItems.length > 0 && (
                <div className="grid discover-grid">
                  {discoverItems.map((item) => (
                    <PosterCard key={`${item.contentType}/${item.id}`} item={item} onOpen={open} />
                  ))}
                </div>
              )}
              {discoverLoading && discoverItems.length === 0 && !discoverError && (
                <div className="grid discover-grid">
                  {Array.from({ length: 12 }, (_, i) => <div key={i} className="card skeleton" />)}
                </div>
              )}
              {!discoverLoading && !discoverError && discoverStarted && discoverItems.length === 0 && (
                <div className="discover-empty muted">{t("Nenhum título neste catálogo.")}</div>
              )}
              {discoverLoading && discoverItems.length > 0 && <div className="discover-more muted small">{t("Carregando…")}</div>}
              <div ref={discoverSentinel} className="grid-sentinel" />
            </>
          )}
          {discover && discover.sources.length === 0 && (
            <div className="screen-message">{t("Nenhum addon instalado tem catálogo para descobrir.")}</div>
          )}
          {!discover && !discoverError && <div className="discover-empty muted">{t("Carregando…")}</div>}
          {!discover && discoverError && <div className="discover-error" role="alert">{discoverError}</div>}
        </div>
      )}

      {empty && (
        history.length ? (
          <div className="search-history" data-testid="search-history">
            <div className="search-history-head">
              <span>{t("Pesquisas recentes")}</span>
              <button className="link" onClick={clearHistory}>{t("Limpar histórico")}</button>
            </div>
            <div className="search-history-list">
              {history.map((h) => (
                <span key={h} className="history-chip">
                  <button className="history-term" onClick={() => setQuery(h)}>
                    <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></svg>
                    {h}
                  </button>
                  <button className="history-remove" aria-label={t("Remover “{term}” do histórico", { term: h })} onClick={() => removeHistory(h)}>×</button>
                </span>
              ))}
            </div>
          </div>
        ) : (
          <div className="screen-message muted">{t("Digite ao menos 2 letras.")}</div>
        )
      )}

      {!empty && result?.done && !result.sections.length && <div className="screen-message">{t("Nada encontrado para “{query}”.", { query: query.trim() })}</div>}

      {!empty &&
        shown.map(({ section }) => (
          <ResultRow key={section.key} title={section.title} addonName={section.addonName} items={section.items} onOpen={open} />
        ))}
    </div>
  );
}

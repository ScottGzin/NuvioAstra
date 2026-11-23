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

import { useEffect, useMemo, useRef, useState } from "react";
import { api, dataEpoch, onDataEpoch } from "../api";
import { ResultRow } from "../components/ResultRow";
import { invoke, listen } from "../tauri";
import type { CatalogChoice, MetaPreview, SearchSection } from "../types";
import { t } from "../i18n";

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

  // Discover: the original's "Movies / TMDB Popular / All Genres" filters, backed by the addons'
  // listable catalogs (catalog_page already accepts an optional genre).
  const [catalogs, setCatalogs] = useState<CatalogChoice[]>([]);
  const [discoverType, setDiscoverType] = useState("movie");
  const [discoverCatalog, setDiscoverCatalog] = useState("");
  const [discoverGenre, setDiscoverGenre] = useState("");
  const [discoverItems, setDiscoverItems] = useState<MetaPreview[] | null>(null);

  useEffect(() => {
    if (!empty) return;
    let cancelled = false;
    api.settingsGet().then(
      (v) => { if (!cancelled) setCatalogs(v.catalogs); },
      () => {},
    );
    return () => { cancelled = true; };
  }, [empty]);

  const typeCatalogs = catalogs.filter((c) => c.contentType === discoverType);
  useEffect(() => {
    if (typeCatalogs.length && !typeCatalogs.some((c) => c.key === discoverCatalog)) {
      const preferred = typeCatalogs.find((c) => /popular|popular/i.test(c.defaultTitle)) ?? typeCatalogs[0];
      setDiscoverCatalog(preferred.key);
    }
  }, [catalogs, discoverType]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!empty || !discoverCatalog) { setDiscoverItems(null); return; }
    let cancelled = false;
    api.catalogPage(discoverCatalog, 0, discoverGenre || null).then(
      (page) => { if (!cancelled) setDiscoverItems(page.items); },
      () => { if (!cancelled) setDiscoverItems([]); },
    );
    return () => { cancelled = true; };
  }, [empty, discoverCatalog, discoverGenre]);

  const selectedCatalog = catalogs.find((c) => c.key === discoverCatalog);
  const GENRES = ["Action", "Adventure", "Animation", "Comedy", "Crime", "Documentary", "Drama", "Family",
    "Fantasy", "History", "Horror", "Music", "Mystery", "Romance", "Science Fiction", "Thriller", "War", "Western"];

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
          <div className="discover-filters">
            <select aria-label={t("Discover")} value={discoverType} onChange={(e) => { setDiscoverType(e.target.value); setDiscoverCatalog(""); setDiscoverGenre(""); }}>
              <option value="movie">{t("Filme")}</option>
              <option value="series">{t("Série")}</option>
            </select>
            <select aria-label={t("Fontes")} value={discoverCatalog} onChange={(e) => setDiscoverCatalog(e.target.value)}>
              {typeCatalogs.map((c) => <option key={c.key} value={c.key}>{c.defaultTitle}</option>)}
            </select>
            <select aria-label={t("Gêneros")} value={discoverGenre} onChange={(e) => setDiscoverGenre(e.target.value)}>
              <option value="">{t("All Genres")}</option>
              {GENRES.map((g) => <option key={g} value={g}>{t(g)}</option>)}
            </select>
          </div>
          {selectedCatalog && (
            <div className="discover-context muted small">{selectedCatalog.addonName} · {selectedCatalog.defaultTitle}{discoverGenre ? ` · ${t(discoverGenre)}` : ""}</div>
          )}
          {discoverItems && discoverItems.length > 0 && (
            <ResultRow
              title={discoverGenre ? t(discoverGenre) : selectedCatalog?.defaultTitle ?? t("Discover")}
              addonName={selectedCatalog?.addonName ?? ""}
              items={discoverItems}
              onOpen={open}
            />
          )}
          {discoverItems && discoverItems.length === 0 && <div className="screen-message muted">{t("Nada encontrado para “{query}”.", { query: selectedCatalog?.defaultTitle ?? t("Discover") })}</div>}
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
                  <button className="history-remove" aria-label={`Remover “${h}” do histórico`} onClick={() => removeHistory(h)}>×</button>
                </span>
              ))}
            </div>
          </div>
        ) : (
          <div className="screen-message muted">{t("Digite ao menos 2 letras.")}</div>
        )
      )}

      {!empty && result?.done && !result.sections.length && <div className="screen-message">Nada encontrado para “{query.trim()}”.</div>}

      {!empty &&
        shown.map(({ section }) => (
          <ResultRow key={section.key} title={section.title} addonName={section.addonName} items={section.items} onOpen={open} />
        ))}
    </div>
  );
}

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
// "Pesquisar em" (Astra): each addon with search catalogs can be left out of the search, e.g. to
// search only the metadata addon. Kept per profile on this computer.

import { useEffect, useMemo, useRef, useState } from "react";
import { Discover } from "./Discover";
import { api, dataEpoch, onDataEpoch } from "../api";
import { ResultRow } from "../components/ResultRow";
import { invoke, listen } from "../tauri";
import type { MetaPreview, SearchAddon, SearchSection } from "../types";
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

/** "Pesquisar em": button with the count of addons searched and a list to turn each on or off. */
function SearchAddons({ addons, onToggle }: { addons: SearchAddon[]; onToggle(key: string | null, enabled: boolean): void }) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent ? e.key === "Escape" : !box.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", close, true);
    window.addEventListener("keydown", close, true);
    return () => {
      window.removeEventListener("pointerdown", close, true);
      window.removeEventListener("keydown", close, true);
    };
  }, [open]);
  if (addons.length < 2) return null;
  const on = addons.filter((a) => a.enabled).length;
  return (
    <div className="search-addons" ref={box}>
      <button
        className={`search-addons-btn ${on < addons.length ? "filtered" : ""}`}
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        aria-haspopup="dialog"
        data-testid="search-addons"
        title={t("Escolher em quais addons pesquisar")}
      >
        <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M4 5h16l-6 7.5V19l-4 1.5v-8z" /></svg>
        <span>{on === addons.length ? t("Todos os addons") : t("{on} de {total} addons", { on, total: addons.length })}</span>
      </button>
      {open && (
        <div className="search-addons-pop" role="dialog" aria-label={t("Pesquisar em")} data-testid="search-addons-pop">
          <div className="search-addons-head">
            <b>{t("Pesquisar em")}</b>
            {on < addons.length && (
              <button className="link" onClick={() => onToggle(null, true)}>{t("Marcar todos")}</button>
            )}
          </div>
          {addons.map((a) => (
            <label key={a.key} className="search-addon" data-addon={a.name}>
              <input type="checkbox" checked={a.enabled} onChange={(e) => onToggle(a.key, e.target.checked)} />
              {a.logo ? <img src={a.logo} alt="" /> : <span className="search-addon-logo" aria-hidden>{a.name.slice(0, 1)}</span>}
              <span className="search-addon-text">
                <span>{a.name}</span>
                <small className="muted">{a.catalogs.map((c) => t(c)).join(" · ")}</small>
              </span>
            </label>
          ))}
          <p className="muted small">{t("Só os addons marcados são consultados na pesquisa deste perfil.")}</p>
        </div>
      )}
    </div>
  );
}

export function Search({ onOpen }: Props) {
  const [query, setQuery] = useState(lastQuery);
  const [result, setResult] = useState<Result | null>(() => cache.get(lastQuery.trim()) ?? null);
  const [preview, setPreview] = useState<{ order: number; section: SearchSection }[] | null>(null);
  const [history, setHistory] = useState<string[]>(loadHistory);
  const requestId = useRef(0);
  const pending = useRef<{ id: number; q: string; epoch: number; parts: Part[] } | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const [addons, setAddons] = useState<SearchAddon[]>([]);
  // Changes when the searched addons change: the same query is asked again.
  const [scope, setScope] = useState(0);

  useEffect(() => input.current?.focus(), []);
  useEffect(() => {
    let alive = true;
    api.searchAddons().then((list) => alive && setAddons(list), () => {});
    return () => {
      alive = false;
    };
  }, []);
  /** One addon on/off; `key` null: every addon on ("Marcar todos"), saved one after the other. */
  const toggleAddon = async (key: string | null, enabled: boolean) => {
    const keys = key === null ? addons.filter((a) => !a.enabled).map((a) => a.key) : [key];
    setAddons((list) => list.map((a) => (keys.includes(a.key) ? { ...a, enabled } : a)));
    try {
      let list: SearchAddon[] = [];
      for (const k of keys) list = await api.searchAddonSet(k, enabled);
      setAddons(list);
    } catch {
      api.searchAddons().then(setAddons, () => {});
    }
    cache.clear();
    setScope((n) => n + 1);
  };
  const noneSearched = addons.length > 0 && addons.every((a) => !a.enabled);

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
    if (q.length < 2 || noneSearched) {
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, scope, noneSearched]);

  // A search left on screen for a moment with results goes to the history.
  useEffect(() => {
    if (!result?.done || !result.sections.length) return;
    const q = query;
    const t = window.setTimeout(() => addHistory(q), 2500);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [result]);

  const shown = result?.sections.length ? result.sections : preview ?? [];
  const loading = query.trim().length >= 2 && !result?.done && !noneSearched;
  const open = (item: MetaPreview) => {
    addHistory(query);
    onOpen(item);
  };
  const empty = useMemo(() => query.trim().length < 2, [query]);


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
        <SearchAddons addons={addons} onToggle={(key, enabled) => void toggleAddon(key, enabled)} />
      </div>

      {empty && history.length > 0 && (
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
      )}

      {/* Nothing typed: Descobrir, like the original's Search page. */}
      {empty && <Discover onOpen={onOpen} />}

      {!empty && noneSearched && <div className="screen-message">{t("Nenhum addon marcado em “Pesquisar em”.")}</div>}

      {!empty && result?.done && !result.sections.length && <div className="screen-message">{t("Nada encontrado para “{query}”.", { query: query.trim() })}</div>}

      {!empty &&
        shown.map(({ section }) => (
          <ResultRow key={section.key} title={section.title} addonName={section.addonName} items={section.items} onOpen={open} />
        ))}
    </div>
  );
}

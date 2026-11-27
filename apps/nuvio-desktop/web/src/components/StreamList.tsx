// Sources of one title/episode, asked to the addons only when shown. Each addon's answer appears
// as soon as it arrives (`streams-part`); the complete list keeps the addons' order.
// - Direct and torrent (P2P) streams play in the app's player; YouTube sources too; external
//   links open in the browser.
// - With a debrid service, torrents it has are marked "TB/PM Instant", torrents a service will be
//   asked for show its code (RD, AD, DL), and the addons' own debrid sources play too.
// - `auto`: the automatic choice of Settings → Fontes (first source or regex), made once the
//   addons answered or after `timeoutSeconds`.
// - Top bar: which addon's sources to show (all, or one) and filters (quality, type, extras);
//   the filters are remembered on this computer.
// - Right click: play or download. Badges come from the texts (Settings → Fontes can hide them).
// `reload` (the sources panel's "Recarregar"): each new value asks every addon again.

import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { api, errorText } from "../api";
import { listen } from "../tauri";
import { autoCandidates, badgesOf, canDownload, passesFilters, SOURCE_FILTERS, type AutoChoice, type ListedStream, type TorboxStates } from "../sources";
import type { StreamEntry, StreamGroup, StreamKind } from "../types";
import { t } from "../i18n";
import { message } from "../rustErrors";

const PENDING: Partial<Record<StreamKind, string>> = {
  debrid: "Debrid — conecte o serviço desta fonte em Configurações → Debrid para abrir",
};

let lastRequest = 0;

const FILTERS_KEY = "nuvio.sources.filters";
function loadFilters(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(FILTERS_KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((x) => SOURCE_FILTERS.some((f) => f.id === x)) : [];
  } catch {
    return [];
  }
}
function saveFilters(v: string[]) {
  try {
    localStorage.setItem(FILTERS_KEY, JSON.stringify(v));
  } catch {
    /* this session only */
  }
}

const GROUP_TITLES: Record<string, string> = { quality: "Qualidade", type: "Tipo", extra: "Extras" };

interface Props {
  contentType: string;
  id: string;
  onPlay(stream: StreamEntry, torbox: TorboxStates): void;
  onDownload?(stream: StreamEntry, torbox: TorboxStates): void;
  reload?: number;
  /** Whether the addons are being asked now (the reload button spins meanwhile). */
  onLoading?(loading: boolean): void;
  /** The sources in list order and what the debrid services can do (fallback, automatic choice). */
  onList?(list: ListedStream[], torbox: TorboxStates): void;
  auto?: (AutoChoice & { timeoutSeconds: number }) | null;
  /** The automatic choice (called at most once per list). */
  onAuto?(stream: StreamEntry, torbox: TorboxStates): void;
  showBadges?: boolean;
}

function flatten(groups: StreamGroup[]): ListedStream[] {
  return groups.flatMap((g) => g.streams.map((stream) => ({ stream, addonName: g.addonName })));
}

export function StreamList({ contentType, id, onPlay, onDownload, reload = 0, onLoading, onList, auto = null, onAuto, showBadges = true }: Props) {
  const [groups, setGroups] = useState<StreamGroup[] | null>(null);
  const [complete, setComplete] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [torbox, setTorbox] = useState<TorboxStates>({});
  const [codes, setCodes] = useState<Record<string, string>>({});
  const [checked, setChecked] = useState(false);
  const [menu, setMenu] = useState<{ stream: StreamEntry; x: number; y: number } | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const autoDone = useRef(false);
  const [timedOut, setTimedOut] = useState(false);
  const autoRef = useRef(auto);
  autoRef.current = auto;
  /** Addon whose sources are shown (`null`: all). */
  const [addon, setAddon] = useState<string | null>(null);
  const [filters, setFiltersState] = useState<string[]>(loadFilters);
  const [filtersOpen, setFiltersOpen] = useState<{ left: number; top: number } | null>(null);
  const filtersRef = useRef<HTMLDivElement>(null);
  const popRef = useRef<HTMLDivElement>(null);
  const setFilters = (v: string[]) => {
    setFiltersState(v);
    saveFilters(v);
  };
  const toggleFilter = (id: string) => setFilters(filters.includes(id) ? filters.filter((f) => f !== id) : [...filters, id]);

  useEffect(() => {
    if (!filtersOpen) return;
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key !== "Escape") return;
      if (e instanceof PointerEvent && (filtersRef.current?.contains(e.target as Node) || popRef.current?.contains(e.target as Node))) return;
      setFiltersOpen(null);
    };
    window.addEventListener("pointerdown", close, true);
    window.addEventListener("keydown", close, true);
    return () => {
      window.removeEventListener("pointerdown", close, true);
      window.removeEventListener("keydown", close, true);
    };
  }, [filtersOpen]);

  useEffect(() => {
    let cancelled = false;
    let finished = false;
    const request = ++lastRequest;
    const parts = new Map<number, StreamGroup>();
    setGroups(null);
    setComplete(false);
    setError(null);
    setTorbox({});
    setCodes({});
    setChecked(false);
    setTimedOut(false);
    setAddon(null);
    autoDone.current = false;
    onLoading?.(true);
    const timeout = autoRef.current && autoRef.current.mode !== "MANUAL" ? autoRef.current.timeoutSeconds : 0;
    const timer = timeout > 0 ? window.setTimeout(() => !cancelled && setTimedOut(true), timeout * 1000) : 0;
    const stop = listen<{ request: number; index: number; group: StreamGroup }>("streams-part", (p) => {
      if (cancelled || finished || p.request !== request) return;
      parts.set(p.index, p.group);
      setGroups([...parts.entries()].sort((a, b) => a[0] - b[0]).map(([, g]) => g));
    });
    api.streams(contentType, id, reload > 0, request).then(
      (g) => {
        if (cancelled) return;
        finished = true;
        onLoading?.(false);
        setGroups(g);
        setComplete(true);
        const keys = g.flatMap((group) => group.streams).filter((s) => s.kind === "torrent" || s.kind === "debrid").map((s) => s.key);
        if (!keys.length) {
          setChecked(true);
          return;
        }
        api.torboxCheck(keys).then(
          (check) => {
            if (cancelled) return;
            setTorbox(check.states);
            setCodes(check.codes);
            setChecked(true);
          },
          () => !cancelled && setChecked(true),
        );
      },
      (e) => {
        if (cancelled) return;
        onLoading?.(false);
        setError(errorText(e));
      },
    );
    return () => {
      cancelled = true;
      stop();
      window.clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [contentType, id, reload]);

  // The list for the screen (fallback) and the automatic choice.
  useEffect(() => {
    if (!groups) return;
    const list = flatten(groups);
    onList?.(list, torbox);
    const choice = autoRef.current;
    if (!choice || choice.mode === "MANUAL" || autoDone.current || reload > 0) return;
    // Every addon answered and the debrid services were asked, or the waiting time is over.
    if (!(complete && checked) && !timedOut) return;
    const pick = autoCandidates(list, choice, torbox)[0];
    if (!pick && !(complete && checked)) return; // nothing ready yet: keep waiting for the others
    autoDone.current = true;
    if (pick) onAuto?.(pick, torbox);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [groups, torbox, complete, checked, timedOut]);

  useEffect(() => {
    if (!menu) return;
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key !== "Escape") return;
      if (e instanceof PointerEvent && menuRef.current?.contains(e.target as Node)) return;
      setMenu(null);
    };
    window.addEventListener("pointerdown", close, true);
    window.addEventListener("keydown", close, true);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("pointerdown", close, true);
      window.removeEventListener("keydown", close, true);
      window.removeEventListener("blur", close);
    };
  }, [menu]);

  if (error) return <div className="row-error">{t("Falha ao buscar fontes: {error}", { error })}</div>;
  if (!groups || (!groups.length && !complete)) return <div className="muted">{t("Buscando fontes nos addons…")}</div>;
  if (!groups.length) return <div className="muted">{t("Nenhum addon instalado fornece fontes para este título.")}</div>;

  const playable = (s: StreamEntry) =>
    s.kind === "direct" || s.kind === "torrent" || s.kind === "youTube" || s.kind === "external" || (s.kind === "debrid" && torbox[s.key] === "cached");

  const total = groups.reduce((n, g) => n + g.streams.length, 0);
  const chosenGroup = addon !== null && groups.some((g) => g.addonName === addon) ? addon : null;
  const visible = groups
    .filter((g) => chosenGroup === null || g.addonName === chosenGroup)
    .map((g) => ({ ...g, streams: g.streams.filter((s) => passesFilters(s, torbox, filters)) }))
    .filter((g) => g.streams.length > 0 || (!filters.length && (chosenGroup !== null || g.error)));
  const shown = visible.reduce((n, g) => n + g.streams.length, 0);

  return (
    <div className="streams">
      <div className="streams-toolbar" data-testid="streams-toolbar">
        <select
          className="select pill-select streams-addon"
          aria-label={t("Addon das fontes")}
          value={chosenGroup ?? ""}
          onChange={(e) => setAddon(e.target.value || null)}
          data-testid="streams-addon"
        >
          <option value="">{t("Todos os addons ({count})", { count: total })}</option>
          {groups.map((g) => (
            <option key={g.addonName} value={g.addonName}>{`${g.addonName} (${g.streams.length})`}</option>
          ))}
        </select>
        <div className="streams-filter-wrap" ref={filtersRef}>
          <button
            className={`chip streams-filter-btn ${filters.length ? "active" : ""}`}
            aria-expanded={!!filtersOpen}
            onClick={(e) => {
              if (filtersOpen) return setFiltersOpen(null);
              // On the page itself: the sources panel would clip it.
              const r = e.currentTarget.getBoundingClientRect();
              setFiltersOpen({ left: Math.max(8, Math.min(r.left, window.innerWidth - 336)), top: r.bottom + 6 });
            }}
            data-testid="streams-filters"
          >
            <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round"><path d="M3 5h18l-7 8v6l-4 2v-8z" /></svg>
            {filters.length ? t("Filtros ({count})", { count: filters.length }) : t("Filtros")}
          </button>
          {filtersOpen && createPortal(
            <div ref={popRef} className="streams-filter-pop" style={{ left: filtersOpen.left, top: filtersOpen.top }} role="dialog" aria-label={t("Filtrar fontes")} data-testid="streams-filter-pop">
              {(["quality", "type", "extra"] as const).map((group) => (
                <div key={group} className="streams-filter-group">
                  <div className="streams-filter-title">{t(GROUP_TITLES[group])}</div>
                  <div className="chips">
                    {SOURCE_FILTERS.filter((f) => f.group === group).map((f) => (
                      <button key={f.id} className={`chip ${filters.includes(f.id) ? "active" : ""}`} aria-pressed={filters.includes(f.id)} onClick={() => toggleFilter(f.id)} data-filter={f.id}>
                        {t(f.label)}
                      </button>
                    ))}
                  </div>
                </div>
              ))}
              <div className="streams-filter-foot">
                <span className="muted small">{t("Mostrando {shown} de {total}", { shown, total })}</span>
                {filters.length > 0 && <button className="link" onClick={() => setFilters([])}>{t("Limpar filtros")}</button>}
              </div>
            </div>,
            document.body,
          )}
        </div>
        {(filters.length > 0 || chosenGroup) && <span className="muted small streams-count" data-testid="streams-count">{t("{shown} de {total}", { shown, total })}</span>}
      </div>
      {!complete && <div className="muted small streams-more" role="status" data-testid="streams-more"><span className="spinner" aria-hidden /> {t("Buscando nos outros addons…")}</div>}
      {visible.length === 0 && complete && (
        <div className="muted streams-none" data-testid="streams-none">
          {t("Nenhuma fonte com esses filtros.")} <button className="link" onClick={() => setFilters([])}>{t("Limpar filtros")}</button>
        </div>
      )}
      {visible.map((group) => (
        <div key={group.addonName} className="stream-group">
          <div className="stream-group-title">
            {group.addonName}
            <span className="muted small"> · {group.error ? t("erro: {error}", { error: message(group.error) }) : t("{count} fonte(s)", { count: group.streams.length })}</span>
          </div>
          {group.streams.map((s) => {
            const ok = playable(s);
            const instant = torbox[s.key] === "cached";
            const trying = torbox[s.key] === "try";
            const code = codes[s.key] ?? "TB";
            const badges = showBadges ? badgesOf(s) : [];
            return (
              <button
                key={s.key}
                className="stream"
                data-kind={s.kind}
                data-key={s.key}
                data-instant={instant || undefined}
                disabled={!ok}
                onClick={() => ok && onPlay(s, torbox)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  if (ok && s.kind !== "external") setMenu({ stream: s, x: e.clientX, y: e.clientY });
                }}
              >
                <span className="stream-name">
                  {s.name || t("Fonte")}
                  {instant && <span className="stream-tag instant" title={t("Já está no cache do serviço debrid: abre na hora")}>{code} Instant</span>}
                  {trying && <span className="stream-tag try" title={t("Abre pelo serviço debrid se ele tiver este torrent; senão, por P2P")}>{code}</span>}
                  {s.kind === "torrent" && !instant && <span className="stream-tag">P2P</span>}
                  {s.kind === "youTube" && <span className="stream-tag">YouTube</span>}
                  {s.kind === "external" && <span className="stream-tag" title={t("Abre no navegador")}>{t("Link externo")}</span>}
                </span>
                <span className="stream-desc">{s.description}</span>
                {badges.length > 0 && (
                  <span className="stream-badges" aria-label={t("Detalhes da fonte")}>
                    {badges.map((b) => <span key={b} className="stream-badge">{b}</span>)}
                  </span>
                )}
                {!ok && PENDING[s.kind] && <span className="stream-pending">{t(PENDING[s.kind]!)}</span>}
              </button>
            );
          })}
        </div>
      ))}
      {/* On the page itself: the sources panel's blur would clip and move a fixed menu. */}
      {menu && createPortal(
        <div ref={menuRef} className="context-menu" role="menu" style={{ left: Math.min(menu.x, window.innerWidth - 210), top: Math.min(menu.y, window.innerHeight - 100) }} data-testid="stream-menu">
          <button role="menuitem" onClick={() => { const s = menu.stream; setMenu(null); onPlay(s, torbox); }}>{t("▶ Reproduzir")}</button>
          {onDownload && canDownload(menu.stream, torbox) && (
            <button role="menuitem" data-testid="stream-download" onClick={() => { const s = menu.stream; setMenu(null); onDownload(s, torbox); }}>{t("⤓ Baixar")}</button>
          )}
        </div>,
        document.body,
      )}
    </div>
  );
}

// "See all" of a Home catalog: a full grid with the catalog's genre filter and paging. Uses the
// Home row key directly (the Rust `catalog_page` / `catalog_genres` commands resolve it with the
// same `resolveCatalog` as the row), so every Home catalog can be browsed, not just Discover ones.

import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorText } from "../api";
import type { CatalogGenres, HomeRow, MetaPreview } from "../types";
import { addonLabel, localizeCatalogTitle, t } from "../i18n";
import { PosterCard } from "./PosterCard";
import { useWhenVisible } from "./useInView";

interface Props {
  row: HomeRow;
  onOpen(item: MetaPreview): void;
  onClose(): void;
}

const itemKey = (item: MetaPreview) => `${item.contentType}:${item.id}`;

function merge(existing: MetaPreview[], incoming: MetaPreview[]): MetaPreview[] {
  const seen = new Set(existing.map(itemKey));
  return [...existing, ...incoming.filter((item) => !seen.has(itemKey(item)))];
}

/** Genre selection with the same precedence as Discover (`resolveGenreSelection`). */
function resolveGenre(genres: CatalogGenres, requested: string): string {
  if (!genres.options.length) return "";
  if (requested && genres.options.includes(requested)) return requested;
  return genres.required ? (genres.options[0] ?? "") : "";
}

export function SeeAll({ row, onOpen, onClose }: Props) {
  const [genres, setGenres] = useState<CatalogGenres | null>(null);
  const [genre, setGenre] = useState("");
  const [items, setItems] = useState<MetaPreview[]>([]);
  const [next, setNext] = useState<number | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const request = useRef(0);
  const busy = useRef(false);
  const nextRef = useRef<number | null>(null);
  nextRef.current = next;

  useEffect(() => {
    let cancelled = false;
    api.catalogGenres(row.key).then(
      (g) => !cancelled && setGenres(g),
      () => !cancelled && setGenres({ options: [], required: false }),
    );
    return () => { cancelled = true; };
  }, [row.key]);

  const selectedGenre = genres ? resolveGenre(genres, genre) : "";

  const load = useCallback(
    async (reset: boolean) => {
      const skip = reset ? 0 : nextRef.current;
      if (skip === null) return;
      if (!reset && busy.current) return;
      busy.current = true;
      const id = ++request.current;
      setLoading(true);
      setError(null);
      if (reset) {
        setItems([]);
        setNext(null);
      }
      try {
        const page = await api.catalogPage(row.key, skip, selectedGenre || null);
        if (id !== request.current) return;
        setItems((prev) => (reset ? page.items : merge(prev, page.items)));
        setNext(page.nextSkip);
      } catch (e) {
        if (id !== request.current) return;
        setError(errorText(e));
        setNext(null);
      } finally {
        if (id === request.current) {
          setLoading(false);
          busy.current = false;
        }
      }
    },
    [row.key, selectedGenre],
  );

  useEffect(() => { void load(true); }, [load]);

  const sentinel = useWhenVisible<HTMLDivElement>(() => {
    if (nextRef.current !== null && !busy.current) void load(false);
  }, [next, loading, items.length, selectedGenre]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const open = (item: MetaPreview) => { onClose(); onOpen(item); };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal see-all" role="dialog" aria-modal="true" aria-label={localizeCatalogTitle(row.title)} data-testid="see-all">
        <header className="modal-header">
          <div>
            <h2>{localizeCatalogTitle(row.title)}</h2>
            {row.addonName && <span className="muted small">{addonLabel(row.addonName)}</span>}
          </div>
          <div className="modal-actions">
            {genres && genres.options.length > 0 && (
              <select aria-label={t("Gênero")} value={genre} onChange={(e) => setGenre(e.target.value)}>
                {!genres.required && <option value="">{t("Todos os gêneros")}</option>}
                {genres.options.map((option) => <option key={option} value={option}>{option}</option>)}
              </select>
            )}
            <button className="icon-btn" aria-label={t("Fechar")} onClick={onClose}>✕</button>
          </div>
        </header>
        {error && <div className="row-error" role="alert">{error}</div>}
        <div className="see-all-body">
          {items.length > 0 && (
            <div className="grid discover-grid">
              {items.map((item) => <PosterCard key={`${item.contentType}/${item.id}`} item={item} onOpen={open} />)}
            </div>
          )}
          {loading && items.length === 0 && !error && (
            <div className="grid discover-grid">{Array.from({ length: 12 }, (_, i) => <div key={i} className="card skeleton" />)}</div>
          )}
          {!loading && !error && items.length === 0 && <div className="discover-empty muted">{t("Nenhum título neste catálogo.")}</div>}
          {loading && items.length > 0 && <div className="discover-more muted small">{t("Carregando…")}</div>}
          <div ref={sentinel} className="grid-sentinel" />
        </div>
      </div>
    </div>
  );
}

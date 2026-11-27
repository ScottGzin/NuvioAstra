// A person's, studio's or network's page from TMDB (`TmdbEntityBrowseScreen` of the original):
// the header (photo or logo, name, biography) and their titles. A person's titles come in one
// request; a studio's or network's are paged, with movies and series apart.

import { useEffect, useRef, useState } from "react";
import { api, errorText } from "../api";
import { PosterCard } from "../components/PosterCard";
import type { MetaPreview, TmdbEntityKind, TmdbEntityPage } from "../types";
import { t } from "../i18n";

interface Props {
  kind: TmdbEntityKind;
  id: number;
  name: string;
  onBack(): void;
  onOpen(item: MetaPreview): void;
}

const KIND_LABEL: Record<TmdbEntityKind, string> = { person: "Pessoa", company: "Estúdio", network: "Canal" };

export function TmdbEntity({ kind, id, name, onBack, onOpen }: Props) {
  const [page, setPage] = useState<TmdbEntityPage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [media, setMedia] = useState<string>(kind === "network" ? "series" : "movie");
  const [items, setItems] = useState<MetaPreview[]>([]);
  const [next, setNext] = useState<number | null>(null);
  const [loading, setLoading] = useState(false);
  const [more, setMore] = useState(false);
  const request = useRef(0);

  useEffect(() => {
    let cancelled = false;
    api.tmdbEntity(kind, id).then(
      (p) => {
        if (cancelled) return;
        setPage(p);
        if (kind === "person") setMedia(p.header.media.length === 1 ? p.header.media[0] : "all");
      },
      (e) => !cancelled && setError(errorText(e)),
    );
    return () => {
      cancelled = true;
    };
  }, [kind, id]);

  // Studios and networks: the first page of the chosen media type, then "Mostrar mais".
  const load = async (pageNumber: number, replace: boolean) => {
    const mine = ++request.current;
    setLoading(true);
    try {
      const result = await api.tmdbEntityTitles(kind, id, media, pageNumber);
      if (mine !== request.current) return;
      setItems((before) => (replace ? result.items : [...before, ...result.items.filter((i) => !before.some((b) => b.id === i.id))]));
      setNext(result.nextSkip ?? null);
    } catch (e) {
      if (mine === request.current) setError(errorText(e));
    } finally {
      if (mine === request.current) setLoading(false);
    }
  };

  useEffect(() => {
    if (kind === "person" || !page) return;
    setItems([]);
    setNext(null);
    void load(1, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind, id, media, page]);

  const header = page?.header;
  const titles = kind === "person" ? (page?.titles ?? []).filter((i) => media === "all" || i.contentType === media) : items;
  const tabs = kind === "person" ? (header && header.media.length > 1 ? ["all", ...header.media] : []) : header?.media.length && header.media.length > 1 ? header.media : [];
  const tabLabel = (m: string) => (m === "all" ? t("Tudo") : m === "series" ? t("Séries") : t("Filmes"));

  return (
    <div className="entity" data-testid="tmdb-entity" data-kind={kind}>
      <button className="back" onClick={onBack} aria-label={t("Voltar")}>←</button>
      {error && !header && <div className="screen-message">{t("Não foi possível abrir esta página:")} {t(error)}</div>}
      {!header && !error && <div className="screen-message">{t("Carregando…")}</div>}
      {header && (
        <>
          <header className={`entity-head entity-${kind}`}>
            {header.image ? <img className="entity-image" src={header.image} alt="" /> : <div className="entity-image entity-initial">{header.name.slice(0, 1)}</div>}
            <div className="entity-text">
              <div className="muted small">{t(KIND_LABEL[kind])}{header.subtitle ? ` · ${t(header.subtitle)}` : ""}</div>
              <h1>{header.name || name}</h1>
              {header.facts.map((f) => <div key={f} className="muted small">{f}</div>)}
              {header.description && (
                <p className={`entity-bio ${more ? "open" : ""}`}>{header.description}</p>
              )}
              {header.description && header.description.length > 420 && (
                <button className="link" onClick={() => setMore(!more)}>{more ? t("Mostrar menos") : t("Mostrar mais")}</button>
              )}
            </div>
          </header>
          {tabs.length > 0 && (
            <div className="chips entity-tabs" role="tablist">
              {tabs.map((m) => (
                <button key={m} role="tab" aria-selected={media === m} className={`chip ${media === m ? "active" : ""}`} onClick={() => setMedia(m)} data-media={m}>{tabLabel(m)}</button>
              ))}
            </div>
          )}
          <div className="grid entity-grid">
            {titles.map((item) => <PosterCard key={`${item.contentType}/${item.id}`} item={item} onOpen={onOpen} />)}
            {loading && titles.length === 0 && Array.from({ length: 12 }, (_, i) => <div key={i} className="card skeleton" />)}
          </div>
          {!loading && titles.length === 0 && <div className="screen-message muted">{t("Nenhum título encontrado.")}</div>}
          {kind !== "person" && next !== null && (
            <div className="entity-more">
              <button onClick={() => void load(next, false)} disabled={loading} data-testid="entity-more">{loading ? t("Carregando…") : t("Mostrar mais")}</button>
            </div>
          )}
        </>
      )}
    </div>
  );
}

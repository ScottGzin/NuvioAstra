// Home: hero + catalog and collection rows, in the user's saved order (Rust applies the original's
// home settings). Catalog rows load lazily as they approach the viewport; collection rows show
// their folders at once.
//
// Hero (`HomeRepository`): the first page of the hero-source catalogs chosen by Rust (at most 2,
// as the original; the same requests the rows use, so nothing is asked twice), titles with
// artwork, shuffled, at most 8. It is built once per Home load, so coming back to Home keeps the
// same featured titles.

import { useEffect, useState } from "react";
import { api, dataEpoch, errorText, onDataEpoch } from "../api";
import { CatalogRow } from "../components/CatalogRow";
import { CollectionRow } from "../components/CollectionRow";
import { ContinueWatchingRow } from "../components/ContinueWatching";
import { HeroCarousel } from "../components/HeroCarousel";
import type { CollectionView, FolderView, HistoryEntry, HomeRow, HomeView, MetaPreview } from "../types";
import { addonLabel, localizeCatalogTitle, t } from "../i18n";

interface Props {
  onOpen(item: MetaPreview): void;
  onResume(entry: HistoryEntry): void;
  /** Continue watching set to open the details page. */
  onOpenEntry?(entry: HistoryEntry): void;
  onOpenFolder(collection: CollectionView, folder: FolderView): void;
}

const HERO_ITEM_LIMIT = 8;
const HERO_WAIT_MS = 3500;

let viewCache: HomeView | null = null;
let heroCache: MetaPreview[] | null = null;
// Rows depend on the profile's addons and Home settings.
onDataEpoch(() => {
  viewCache = null;
  heroCache = null;
});

function shuffle<T>(list: T[]): T[] {
  const out = [...list];
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [out[i], out[j]] = [out[j], out[i]];
  }
  return out;
}

/** Titles per Home row; the "+" at the end opens the whole catalog. */
const ROW_LIMIT = 20;

export function Home({ onOpen, onResume, onOpenEntry, onOpenFolder }: Props) {
  const [view, setView] = useState<HomeView | null>(viewCache);
  const [error, setError] = useState<string | null>(null);
  const [hero, setHero] = useState<MetaPreview[] | null>(heroCache);
  const [generation, setGeneration] = useState(0);
  const rows = view?.rows ?? null;

  // Settings → Início changes (order, visibility, titles, hero) and collections received from the
  // account reload the rows.
  useEffect(
    () =>
      api.onHomeSettingsChanged(() => {
        viewCache = null;
        heroCache = null;
        setHero(null);
        setGeneration((g) => g + 1);
      }),
    [],
  );

  useEffect(() => {
    if (viewCache) return;
    const epoch = dataEpoch();
    let replaced = false;
    api.homeRows().then(
      (v) => {
        // Answer for the previous profile/addons, or for the layout before a newer change (a sync
        // often ends while the first answer is still waiting for the addons).
        if (replaced || epoch !== dataEpoch()) return;
        viewCache = v;
        setView(v);
      },
      (e) => !replaced && setError(errorText(e)),
    );
    return () => {
      replaced = true;
    };
  }, [generation]);

  useEffect(() => {
    if (!view || !view.heroEnabled || heroCache) return;
    const epoch = dataEpoch();
    const sources = view.heroSources;
    const pages: MetaPreview[][] = sources.map(() => []);
    let done = false;
    const finish = () => {
      if (done || epoch !== dataEpoch()) return;
      done = true;
      const seen = new Set<string>();
      const candidates = pages.flat().filter((i) => {
        const key = `${i.contentType}:${i.id}`;
        if (seen.has(key) || !(i.landscapePoster || i.banner)) return false;
        seen.add(key);
        return true;
      });
      heroCache = shuffle(candidates).slice(0, HERO_ITEM_LIMIT);
      setHero(heroCache);
    };
    const timer = window.setTimeout(finish, HERO_WAIT_MS);
    Promise.allSettled(
      sources.map((key, i) => api.catalogPage(key, null).then((p) => { pages[i] = p.items; })),
    ).then(finish);
    return () => {
      done = true;
      window.clearTimeout(timer);
    };
  }, [view]);

  // "Ver todos": the catalog opens in the folder screen as a one-catalog grid (pages load as it scrolls).
  const seeAll = (row: HomeRow) => {
    const title = localizeCatalogTitle(row.title);
    onOpenFolder(
      { id: `catalog_${row.key}`, title: addonLabel(row.addonName), backdrop: null, pinToTop: false, viewMode: "TABBED_GRID", showAllTab: false, folders: [] },
      {
        id: row.key, title, image: null, stillImage: null, emoji: null, shape: "poster", hideTitle: false, backdrop: null, logo: null,
        sources: [{ key: row.key, genre: null, label: title, addonName: row.addonName, contentType: row.contentType, supportsPagination: row.supportsPagination }],
        skippedSources: 0,
      },
    );
  };

  if (error) return <div className="screen-message">{t("Não foi possível ler os addons:")} {error}</div>;
  if (!rows) return <div className="screen-message">{t("Carregando catálogos…")}</div>;
  if (!rows.length) return <div className="screen-message">{t("Nenhum catálogo disponível. Ative ou adicione um addon em Configurações → Addons para ver catálogos na tela inicial.")}</div>;

  return (
    <div className="home">
      {view?.heroEnabled && view.heroSources.length > 0 && (hero ? <HeroCarousel items={hero} onOpen={onOpen} /> : <div className="hero hero-placeholder" aria-hidden />)}
      <ContinueWatchingRow onResume={onResume} onOpenEntry={onOpenEntry} />
      {rows.map((row) =>
        row.collection ? (
          <CollectionRow key={`${generation}:${row.key}`} title={row.title} collection={row.collection} onOpenFolder={onOpenFolder} />
        ) : (
          <CatalogRow key={`${generation}:${row.key}`} row={row} onOpen={onOpen} limit={ROW_LIMIT} onSeeAll={seeAll} />
        ),
      )}
    </div>
  );
}

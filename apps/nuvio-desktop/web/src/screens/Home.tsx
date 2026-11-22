// Home: hero + catalog rows from the enabled addons, in the user's saved order (Rust applies the
// original's home settings). Rows load lazily as they approach the viewport.

import { useCallback, useEffect, useState } from "react";
import { api, dataEpoch, errorText, onDataEpoch } from "../api";
import { useI18n } from "../i18n";
import { CatalogRow } from "../components/CatalogRow";
import { ContinueWatchingRow } from "../components/ContinueWatching";
import type { HistoryEntry, HomeRow, HomeView, MetaPreview } from "../types";

interface Props {
  onOpen(item: MetaPreview): void;
  onResume(entry: HistoryEntry): void;
}

let viewCache: HomeView | null = null;
// Rows depend on the profile's addons and Home settings.
onDataEpoch(() => {
  viewCache = null;
});

export function Home({ onOpen, onResume }: Props) {
  const { t } = useI18n();
  const [view, setView] = useState<HomeView | null>(viewCache);
  const [error, setError] = useState<string | null>(null);
  const [hero, setHero] = useState<MetaPreview | null>(null);
  const [generation, setGeneration] = useState(0);
  const rows = view?.rows ?? null;

  // Settings → Início changes (order, visibility, titles, hero, unreleased) reload the rows.
  useEffect(
    () =>
      api.onHomeSettingsChanged(() => {
        viewCache = null;
        setHero(null);
        setGeneration((g) => g + 1);
      }),
    [],
  );

  useEffect(() => {
    if (viewCache) return;
    const epoch = dataEpoch();
    api.homeRows().then(
      (v) => {
        if (epoch !== dataEpoch()) return; // answer for the previous profile/addons
        viewCache = v;
        setView(v);
      },
      (e) => setError(errorText(e)),
    );
  }, [generation]);

  // The hero is the first item with artwork of the first row that loads.
  const onLoaded = useCallback((_: HomeRow, items: MetaPreview[]) => {
    setHero((current) => current ?? items.find((i) => i.landscapePoster || i.banner) ?? null);
  }, []);

  if (error) return <div className="screen-message">{t("home.error", { error })}</div>;
  if (!rows) return <div className="screen-message">{t("home.loading")}</div>;
  if (!rows.length) return <div className="screen-message">{t("home.empty")}</div>;

  return (
    <div className="home">
      {view?.heroEnabled && hero && (
        <section className="hero" style={{ backgroundImage: `url("${hero.landscapePoster ?? hero.banner}")` }}>
          <div className="hero-shade">
            {hero.logo ? <img className="hero-logo" src={hero.logo} alt={hero.name} /> : <h1>{hero.name}</h1>}
            <div className="meta-line">
              {[hero.releaseInfo, hero.imdbRating && `★ ${hero.imdbRating}`, hero.genres.slice(0, 3).join(" · ")].filter(Boolean).join("   ")}
            </div>
            {hero.description && <p className="hero-desc">{hero.description}</p>}
            <button className="primary" onClick={() => onOpen(hero)}>{t("home.details")}</button>
          </div>
        </section>
      )}
      <ContinueWatchingRow onResume={onResume} />
      {rows.map((row) => (
        <CatalogRow key={`${generation}:${row.key}`} row={row} onOpen={onOpen} onLoaded={onLoaded} />
      ))}
    </div>
  );
}

// Ratings of a title from MDBList on the details page (`MetaExternalRating` rows of the original).
// Nothing is shown when MDBList is off, has no key or does not know the title.

import { useEffect, useState } from "react";
import { api } from "../api";
import type { ExternalRating } from "../types";
import { RATING_LABELS } from "./MdbListSettings";
import { decimal, t } from "../i18n";

/** IMDb and MyAnimeList out of 10, Letterboxd out of 5, the others as a percentage. */
function shown(r: ExternalRating): string {
  if (r.max === 10) return decimal(r.value, 1);
  if (r.max === 5) return `${decimal(r.value, 1)}/5`;
  return r.source === "metacritic" ? String(Math.round(r.value)) : `${Math.round(r.value)}%`;
}

export function ExternalRatings({ contentType, id }: { contentType: string; id: string }) {
  const [list, setList] = useState<ExternalRating[]>([]);

  useEffect(() => {
    let cancelled = false;
    setList([]);
    api.mdblistRatings(contentType, id).then((r) => !cancelled && setList(r), () => {});
    return () => {
      cancelled = true;
    };
  }, [contentType, id]);

  if (list.length === 0) return null;
  return (
    <div className="external-ratings" data-testid="external-ratings">
      {list.map((r) => (
        <span key={r.source} className={`ext-rating ext-${r.source} ${r.certified ? "certified" : ""}`} title={`${t(RATING_LABELS[r.source] ?? r.source)}${r.certified ? ` — ${t("certificado")}` : ""}`} data-source={r.source}>
          <span className="ext-name">{t(RATING_LABELS[r.source] ?? r.source)}</span>
          <span className="ext-value">{shown(r)}</span>
        </span>
      ))}
    </div>
  );
}

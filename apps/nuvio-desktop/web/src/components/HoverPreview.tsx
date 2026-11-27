// Poster hover preview (the original's poster card hover preview): after the mouse rests on a card
// for the chosen time, a larger card beside it shows the backdrop, logo or title, year, runtime,
// rating, genres and synopsis the catalog already sent. Nothing is fetched for it.

import { useEffect, useState } from "react";
import type { MetaPreview } from "../types";
import { t } from "../i18n";

type Shown = { item: MetaPreview; rect: DOMRect } | null;
const listeners = new Set<(s: Shown) => void>();
let shown: Shown = null;

function emit(next: Shown) {
  shown = next;
  for (const l of listeners) l(next);
}

export function showHoverPreview(item: MetaPreview, rect: DOMRect) {
  emit({ item, rect });
}

export function hideHoverPreview(id?: string) {
  if (!shown || (id && shown.item.id !== id)) return;
  emit(null);
}

const WIDTH = 340;

export function HoverPreview() {
  const [value, setValue] = useState<Shown>(shown);
  useEffect(() => {
    listeners.add(setValue);
    const hide = () => emit(null);
    // The mouse is over something else than that card (a missed mouseleave never leaves it open).
    const moved = (e: MouseEvent) => {
      if (!shown) return;
      const card = (e.target as Element | null)?.closest?.(".card[data-id]") as HTMLElement | null;
      if (card?.dataset.id !== shown.item.id) emit(null);
    };
    window.addEventListener("scroll", hide, true);
    window.addEventListener("blur", hide);
    window.addEventListener("mousemove", moved, true);
    document.documentElement.addEventListener("mouseleave", hide);
    return () => {
      listeners.delete(setValue);
      window.removeEventListener("scroll", hide, true);
      window.removeEventListener("blur", hide);
      window.removeEventListener("mousemove", moved, true);
      document.documentElement.removeEventListener("mouseleave", hide);
    };
  }, []);
  if (!value) return null;
  const { item, rect } = value;
  const art = item.landscapePoster ?? item.banner ?? item.poster;
  const right = rect.right + 12 + WIDTH < window.innerWidth;
  const left = right ? rect.right + 12 : Math.max(8, rect.left - 12 - WIDTH);
  const top = Math.max(8, Math.min(rect.top - 10, window.innerHeight - 380));
  const facts = [item.releaseInfo, item.runtime, item.imdbRating && `★ ${item.imdbRating}`].filter(Boolean).join("  ·  ");
  return (
    <div className="hover-preview" style={{ left, top, width: WIDTH }} role="tooltip" data-testid="hover-preview">
      <div className="hover-preview-art">
        {art && <img src={art} alt="" draggable={false} />}
        <div className="hover-preview-shade" />
        {item.logo ? <img className="hover-preview-logo" src={item.logo} alt={item.name} /> : <div className="hover-preview-name">{item.name}</div>}
      </div>
      <div className="hover-preview-body">
        {facts && <div className="hover-preview-facts">{facts}</div>}
        {item.genres.length > 0 && <div className="hover-preview-genres">{item.genres.slice(0, 4).join(" · ")}</div>}
        {item.description ? <p>{item.description}</p> : <p className="muted">{t("Sem sinopse.")}</p>}
      </div>
    </div>
  );
}

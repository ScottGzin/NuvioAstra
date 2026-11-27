import { memo, useRef, useState } from "react";
import type { MetaPreview } from "../types";
import { FavoriteButton } from "./FavoriteButton";
import { useCardStyle } from "../cardStyle";
import { customPoster, useScreens } from "../screens";
import { hideHoverPreview, showHoverPreview } from "./HoverPreview";
import { t } from "../i18n";

interface Props {
  item: MetaPreview;
  onOpen(item: MetaPreview): void;
  /** Shows the type label on the art (Library grid). */
  showType?: boolean;
}

export function typeLabel(contentType: string): string {
  const kind = contentType.toLowerCase();
  if (kind === "movie") return t("Filme");
  if (kind === "series" || kind === "tv" || kind === "show") return t("Série");
  if (kind === "anime") return "Anime";
  return contentType;
}

/** Poster with native lazy loading/decoding: off-screen images are not downloaded. The favorite
 *  heart is a sibling of the card button (no nested buttons).
 *  - Settings → Pôsteres: a custom poster address (falls back to the addon's art if it fails) and
 *    a preview of the title after the mouse rests on the card. */
export const PosterCard = memo(function PosterCard({ item, onOpen, showType = false }: Props) {
  const cards = useCardStyle();
  const screens = useScreens();
  const [customFailed, setCustomFailed] = useState(false);
  const timer = useRef(0);
  const wide = cards.landscape && item.posterShape !== "Square" && !!(item.landscapePoster ?? item.banner);
  const shape = item.posterShape === "Landscape" || wide ? "landscape" : item.posterShape === "Square" ? "square" : "poster";
  const original = shape === "landscape" ? item.landscapePoster ?? (wide ? item.banner : null) ?? item.poster : item.poster;
  const custom = !customFailed && screens.posterPattern ? customPoster(screens.posterPattern, item, shape) : null;
  const image = custom ?? original;
  const enter = (e: React.MouseEvent<HTMLButtonElement>) => {
    if (!screens.hoverPreview) return;
    const target = e.currentTarget;
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => showHoverPreview(item, target.getBoundingClientRect()), screens.hoverDelayMs);
  };
  const leave = () => {
    window.clearTimeout(timer.current);
    hideHoverPreview(item.id);
  };
  return (
    <div className={`card-wrap ${shape}`}>
      <button
        className={`card ${shape}`}
        onClick={() => {
          leave();
          onOpen(item);
        }}
        onMouseEnter={enter}
        onMouseLeave={leave}
        title={screens.hoverPreview ? undefined : item.name}
        data-id={item.id}
        data-content-type={item.contentType}
      >
        <div className="card-art">
          {image ? (
            <img src={image} alt="" loading="lazy" decoding="async" draggable={false} onError={custom ? () => setCustomFailed(true) : undefined} data-custom={custom ? "" : undefined} />
          ) : (
            <span className="card-fallback">{item.name}</span>
          )}
          {showType && <span className="card-type">{typeLabel(item.contentType)}</span>}
        </div>
        <div className="card-title">{item.name}</div>
        {item.releaseInfo && <div className="card-sub">{item.releaseInfo}</div>}
      </button>
      <FavoriteButton item={item} />
    </div>
  );
});

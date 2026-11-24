import { memo, useEffect, useRef, useState } from "react";
import type { MetaPreview } from "../types";
import { FavoriteButton } from "./FavoriteButton";
import { HoverPreview } from "./HoverPreview";
import { ensurePosterPreferences, posterPreferences } from "../posterPreferences";
import { t } from "../i18n";

interface Props {
  item: MetaPreview;
  onOpen(item: MetaPreview): void;
  /** Shows the type label on the art (Library grid). */
  showType?: boolean;
  eager?: boolean;
}

export function typeLabel(contentType: string): string {
  const kind = contentType.toLowerCase();
  if (kind === "movie") return t("Filme");
  if (kind === "series" || kind === "tv" || kind === "show") return t("Série");
  if (kind === "anime") return "Anime";
  return contentType;
}

/** Poster with native lazy loading/decoding: off-screen images are not downloaded. The favorite
 *  heart is a sibling of the card button (no nested buttons). */
export const PosterCard = memo(function PosterCard({ item, onOpen, showType = false, eager = false }: Props) {
  const shape = item.posterShape === "Landscape" ? "landscape" : item.posterShape === "Square" ? "square" : "poster";
  const primary = shape === "landscape" ? item.landscapePoster ?? item.poster : item.poster;
  const fallbackUrl = shape === "landscape" ? item.rawLandscapePosterUrl ?? item.rawPosterUrl : item.rawPosterUrl;
  const [imageStep, setImageStep] = useState(0);
  const image = imageStep === 0 ? primary : imageStep === 1 ? fallbackUrl : null;
  const [preview, setPreview] = useState(false);
  const [previewHovered, setPreviewHovered] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  const openTimer = useRef(0);
  const closeTimer = useRef(0);
  const dismissed = useRef(false);

  useEffect(() => {
    void ensurePosterPreferences();
    setImageStep(0);
    return () => { window.clearTimeout(openTimer.current); window.clearTimeout(closeTimer.current); };
  }, [primary, fallbackUrl]);

  const scheduleOpen = () => {
    if (!posterPreferences().hoverPreviewEnabled) return;
    dismissed.current = false;
    window.clearTimeout(closeTimer.current);
    window.clearTimeout(openTimer.current);
    openTimer.current = window.setTimeout(() => setPreview(true), posterPreferences().hoverPreviewDelayMs);
  };
  const scheduleClose = () => {
    window.clearTimeout(openTimer.current);
    window.clearTimeout(closeTimer.current);
    closeTimer.current = window.setTimeout(() => {
      if (!previewHovered) setPreview(false);
    }, 120);
  };
  const previewHover = (hovered: boolean) => {
    setPreviewHovered(hovered);
    if (hovered) window.clearTimeout(closeTimer.current);
    else scheduleClose();
  };
  const dismiss = () => {
    dismissed.current = true;
    setPreview(false);
  };

  return (
    <div className={`card-wrap ${shape}`}>
      <button
        ref={anchor}
        className={`card ${shape}`}
        onClick={() => onOpen(item)}
        onPointerEnter={scheduleOpen}
        onPointerLeave={() => { if (!previewHovered) scheduleClose(); }}
        onFocus={(event) => { if (event.currentTarget.matches(":focus-visible")) scheduleOpen(); }}
        onBlur={() => { if (!previewHovered) scheduleClose(); }}
        title={item.name}
        data-id={item.id}
        data-content-type={item.contentType}
      >
        <div className="card-art">
          {image ? <img src={image} alt="" loading="eager" decoding="async" fetchPriority={eager ? "high" : "auto"} draggable={false} onError={() => setImageStep((step) => Math.min(2, step + 1))} /> : <span className="card-fallback">{item.name}</span>}
          {showType && <span className="card-type">{typeLabel(item.contentType)}</span>}
        </div>
        <div className="card-title">{item.name}</div>
        {item.releaseInfo && <div className="card-sub">{item.releaseInfo}</div>}
      </button>
      <FavoriteButton item={item} />
      {preview && (
        <HoverPreview
          item={item}
          anchor={anchor.current}
          onHover={previewHover}
          onDismiss={dismiss}
        />
      )}
    </div>
  );
});

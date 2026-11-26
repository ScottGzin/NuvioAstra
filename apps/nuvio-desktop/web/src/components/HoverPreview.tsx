// Desktop hover preview of a poster card: artwork + metadata, with an optional muted trailer that
// is resolved only after the popup has been visible for a second. Port of the original's
// `HomePosterHoverPreview` / `HomePosterHoverTrailer`; never opens the real player.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { api } from "../api";
import type { MetaPreview } from "../types";
import { posterPreferences } from "../posterPreferences";
import { t } from "../i18n";

const POPUP_WIDTH = 420;
const EDGE = 16;
const TRAILER_DEBOUNCE_MS = 1_000;

interface Props {
  item: MetaPreview;
  anchor: HTMLElement | null;
  onHover(hovered: boolean): void;
  onDismiss(): void;
}

export function HoverPreview({ item, anchor, onHover, onDismiss }: Props) {
  const [style, setStyle] = useState<React.CSSProperties>({ visibility: "hidden" });
  const [video, setVideo] = useState<string | null>(null);
  const [audio, setAudio] = useState<string | null>(null);
  const mounted = useRef(true);
  const audioRef = useRef<HTMLAudioElement>(null);

  useLayoutEffect(() => {
    mounted.current = true;
    if (!anchor) return;
    const rect = anchor.getBoundingClientRect();
    const width = Math.min(POPUP_WIDTH, window.innerWidth - EDGE * 2);
    const left = Math.max(EDGE, Math.min(rect.left + rect.width / 2 - width / 2, window.innerWidth - width - EDGE));
    const top = Math.max(EDGE, rect.top - 24);
    setStyle({ left, top, width });
    return () => { mounted.current = false; };
  }, [anchor]);

  useEffect(() => {
    const prefs = posterPreferences();
    if (!prefs.hoverPreviewTrailerEnabled) return;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void api.details(item.contentType, item.id)
        .then((details) => details.trailer ? api.trailerResolve(details.trailer.key) : null)
        .then((resolved) => {
          if (!cancelled && mounted.current && resolved) {
            setVideo(resolved.videoUrl);
            setAudio(resolved.audioUrl);
          }
        })
        .catch(() => {});
    }, TRAILER_DEBOUNCE_MS);
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [item.contentType, item.id]);

  useEffect(() => {
    const dismiss = () => onDismiss();
    window.addEventListener("scroll", dismiss, { passive: true, capture: true });
    return () => window.removeEventListener("scroll", dismiss, { capture: true });
  }, [onDismiss]);

  const prefs = posterPreferences();
  const artwork = item.banner || item.landscapePoster || item.poster;
  const meta = [item.contentType === "series" ? t("Série") : t("Filme"), item.releaseInfo, item.imdbRating ? `IMDb ${item.imdbRating}` : null]
    .filter(Boolean).join(" · ");

  return (
    <div
      className="hover-preview"
      style={style}
      aria-hidden="true"
      data-testid="hover-preview"
      onPointerEnter={() => onHover(true)}
      onPointerLeave={() => onHover(false)}
    >
      {video && (
        <video
          className="hover-preview-video"
          src={video}
          autoPlay
          loop
          muted={!prefs.hoverPreviewTrailerSound}
          playsInline
          onLoadedMetadata={(event) => {
            event.currentTarget.currentTime = prefs.hoverPreviewTrailerStart;
            const sound = audioRef.current;
            if (sound) {
              sound.currentTime = prefs.hoverPreviewTrailerStart;
              if (!prefs.hoverPreviewTrailerSound) sound.pause();
              else void sound.play().catch(() => {});
            }
          }}
          onPlay={() => {
            const sound = audioRef.current;
            if (sound && prefs.hoverPreviewTrailerSound) void sound.play().catch(() => {});
          }}
          onPause={() => audioRef.current?.pause()}
        />
      )}
      {audio && <audio ref={audioRef} src={audio} muted={!prefs.hoverPreviewTrailerSound} />}
      {!video && artwork && <img className="hover-preview-art" src={artwork} alt="" draggable={false} />}
      <div className="hover-preview-body">
        <div className="hover-preview-title">{item.logo ? <img src={item.logo} alt="" /> : item.name}</div>
        {meta && <div className="hover-preview-meta muted small">{meta}</div>}
        {item.description && <p className="hover-preview-desc">{item.description}</p>}
      </div>
    </div>
  );
}

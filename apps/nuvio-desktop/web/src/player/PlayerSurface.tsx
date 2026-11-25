// Reserves the video area: a transparent "hole" through which mpv's window (below the webview)
// shows. Its rectangle is sent to Rust whenever the layout changes, so mpv draws exactly here.
// Everything around the hole paints its own opaque background (see styles.css `.hole`).

import { useEffect, useRef, type ReactNode } from "react";
import { api } from "../api";

interface Props {
  /** Sends the rect only while true (the player is shown in this surface). */
  active: boolean;
  /** The playback shown here: a new one (another channel) gets a new mpv, which starts with the
   *  whole window as its video area, so the rect is sent again for it. */
  playback?: unknown;
  className?: string;
  children?: ReactNode;
}

export function PlayerSurface({ active, playback, className, children }: Props) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el || !active) return;
    let frame = 0;
    const sync = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const r = el.getBoundingClientRect();
        void api.playerRect({ x: r.left, y: r.top, width: r.width, height: r.height });
      });
    };
    sync();
    const observer = new ResizeObserver(sync);
    observer.observe(el);
    window.addEventListener("resize", sync);
    el.addEventListener("transitionend", sync);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      window.removeEventListener("resize", sync);
      el.removeEventListener("transitionend", sync);
    };
  }, [active, playback]);

  return (
    <div ref={ref} className={`hole ${active ? "" : "idle"} ${className ?? ""}`}>
      {children}
    </div>
  );
}

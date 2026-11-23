// UI side of the automated player tests (`--selftest`, `--selftest-addon`). Rust drives the
// player; this view keeps the contract of the validated test UI: events `selftest-source` (badge)
// and `selftest-rect` (physical-pixel rect of the video, `null` = whole window), a transparent
// hole at that rect, opaque page around it and the real controls overlay at the bottom.

import { useEffect, useState } from "react";
import { listen } from "../tauri";
import type { PlayerSnapshot } from "../types";
import { PlayerControls } from "./PlayerControls";
import { t } from "../i18n";

type Rect = [number, number, number, number] | null;

export function SelftestView() {
  const [rect, setRect] = useState<Rect>(null);
  const [badge, setBadge] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<PlayerSnapshot | null>(null);

  useEffect(() => {
    document.body.classList.add("playing", "selftest");
    const offs = [
      listen<Rect>("selftest-rect", setRect),
      listen<string>("selftest-source", setBadge),
      listen<PlayerSnapshot>("player-state", setSnapshot),
    ];
    return () => offs.forEach((off) => off());
  }, []);

  const dpr = window.devicePixelRatio;
  const style = rect
    ? { left: rect[0] / dpr, top: rect[1] / dpr, width: rect[2] / dpr, height: rect[3] / dpr, borderRadius: 14 }
    : { left: 0, top: 0, width: "100vw", height: "100vh", borderRadius: 0 };

  return (
    <div className="selftest-page">
      {rect && (
        <aside className="selftest-side">
          <div className="brand">Nuvio Astra</div>
          <p className="muted">{t("Autoteste do player (libmpv + WebView2)")}</p>
        </aside>
      )}
      <div className="hole selftest-hole" style={style}>
        {badge && <div className="badge">Overlay WebView2 · {badge}</div>}
        <PlayerControls snapshot={snapshot} fullscreen={!rect} onToggleFullscreen={() => {}} onClose={() => {}} />
      </div>
    </div>
  );
}

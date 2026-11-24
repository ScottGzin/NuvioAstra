import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { api } from "./api";
import { invoke } from "./tauri";
import { App } from "./App";
import { PipView } from "./player/PipView";
import { PlayerProvider } from "./player/PlayerContext";
import { SelftestView } from "./player/SelftestView";
import { runUiTest } from "./uitest/driver";
import { installDesktopFeel } from "./desktopFeel";
import { useLang } from "./i18n";
import "./styles.css";

// The automated player tests get only the video overlay: no catalogs are loaded, so their
// measurements are not mixed with addon traffic.
function Root() {
  const [mode, setMode] = useState<"app" | "selftest" | "uitest" | null>(null);
  useEffect(() => {
    api.appMode().then(setMode, () => setMode("app"));
  }, []);
  useEffect(() => {
    if (mode === "uitest") void runUiTest();
    // Interface mounted: the (hidden) window is shown maximized now. A timer, not an animation
    // frame: frames may not run while the window is still hidden.
    if (mode !== null) window.setTimeout(() => void invoke("window_ready").catch(() => {}), 30);
  }, [mode]);
  if (mode === null) return null;
  if (mode === "selftest") return <SelftestView />;
  return (
    <PlayerProvider>
      <App />
    </PlayerProvider>
  );
}

installDesktopFeel();
// The picture-in-picture window loads this same page and shows only its controls (`pip.rs`).
const pip = (window as { __NUVIO_PIP__?: boolean }).__NUVIO_PIP__ === true;

// Re-mount the UI when the language changes so every t() call is re-evaluated.
function LangRoot() {
  const lang = useLang();
  return pip ? <PipView /> : <Root key={lang} />;
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <LangRoot />
  </StrictMode>,
);

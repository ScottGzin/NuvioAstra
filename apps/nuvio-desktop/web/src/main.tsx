import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { api } from "./api";
import { invoke } from "./tauri";
import { App } from "./App";
import { I18nProvider } from "./i18n";
import { PlayerProvider } from "./player/PlayerContext";
import { SelftestView } from "./player/SelftestView";
import { runUiTest } from "./uitest/driver";
import { installDesktopFeel } from "./desktopFeel";
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
  return (
    <I18nProvider>
      {mode === "selftest" ? (
        <SelftestView />
      ) : (
        <PlayerProvider>
          <App />
        </PlayerProvider>
      )}
    </I18nProvider>
  );
}

installDesktopFeel();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Root />
  </StrictMode>,
);

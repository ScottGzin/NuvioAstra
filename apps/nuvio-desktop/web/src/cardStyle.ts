// Poster card format of the profile (Settings → Início), applied as CSS variables to the whole app.

import { useSyncExternalStore } from "react";
import { api, onRemoteSettingsChanged } from "./api";
import type { CardStyle } from "./types";

/** The original's sizes are in dp for a 126 dp card; here 126 means the 150 px card Astra had. */
const PX_PER_DP = 150 / 126;
const DEFAULT: CardStyle = {
  width: 126, radius: 12, landscape: false, hideLabels: false,
  hoverPreviewEnabled: true, hoverPreviewDelayMs: 2000,
  hoverPreviewTrailerEnabled: false, hoverPreviewTrailerSound: false, hoverPreviewTrailerStart: 0,
  customPosterUrlPattern: "", customPosterScreens: ["home", "continue_watching", "collections", "library", "search", "details"],
};

let current: CardStyle = DEFAULT;
const listeners = new Set<() => void>();

export function applyCardStyle(style: CardStyle) {
  current = style;
  const root = document.documentElement;
  root.style.setProperty("--card-w", `${Math.round(style.width * PX_PER_DP)}px`);
  root.style.setProperty("--card-radius", `${Math.round(style.radius * PX_PER_DP)}px`);
  root.classList.toggle("cards-no-labels", style.hideLabels);
  listeners.forEach((l) => l());
}

export function loadCardStyle() {
  api.settingsGet().then((view) => applyCardStyle(view.settings.player.cards ?? DEFAULT), () => {});
}

let started = false;
export function startCardStyle() {
  if (started) return;
  started = true;
  loadCardStyle();
  onRemoteSettingsChanged(loadCardStyle);
}

export function useCardStyle(): CardStyle {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => current,
  );
}

export const CARD_WIDTHS: [number, string][] = [[104, "Compacto"], [112, "Denso"], [120, "Padrão menor"], [126, "Equilibrado (padrão)"], [134, "Confortável"], [140, "Grande"]];
export const CARD_RADII: [number, string][] = [[0, "Retos"], [4, "Sutis"], [8, "Clássicos"], [12, "Arredondados (padrão)"], [16, "Pílula"]];

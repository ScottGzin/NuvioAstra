// Cached poster/hover preferences. Loaded once (and refreshed after settings are saved) so every
// PosterCard does not call `settingsGet` on its own.

import { api } from "./api";

export interface PosterPreferences {
  hoverPreviewEnabled: boolean;
  hoverPreviewDelayMs: number;
  hoverPreviewTrailerEnabled: boolean;
  hoverPreviewTrailerSound: boolean;
  hoverPreviewTrailerStart: number;
}

const DEFAULTS: PosterPreferences = {
  hoverPreviewEnabled: true,
  hoverPreviewDelayMs: 2_000,
  hoverPreviewTrailerEnabled: false,
  hoverPreviewTrailerSound: false,
  hoverPreviewTrailerStart: 0,
};

let current: PosterPreferences = DEFAULTS;
let inFlight: Promise<void> | null = null;

export function posterPreferences(): PosterPreferences {
  return current;
}

export function ensurePosterPreferences(): Promise<void> {
  if (inFlight) return inFlight;
  inFlight = api.settingsGet().then((view) => {
    const cards = view.settings.player.cards;
    current = {
      hoverPreviewEnabled: cards?.hoverPreviewEnabled ?? DEFAULTS.hoverPreviewEnabled,
      hoverPreviewDelayMs: cards?.hoverPreviewDelayMs ?? DEFAULTS.hoverPreviewDelayMs,
      hoverPreviewTrailerEnabled: cards?.hoverPreviewTrailerEnabled ?? DEFAULTS.hoverPreviewTrailerEnabled,
      hoverPreviewTrailerSound: cards?.hoverPreviewTrailerSound ?? DEFAULTS.hoverPreviewTrailerSound,
      hoverPreviewTrailerStart: cards?.hoverPreviewTrailerStart ?? DEFAULTS.hoverPreviewTrailerStart,
    };
  }, () => {}).finally(() => { inFlight = null; });
  return inFlight;
}

export async function reloadPosterPreferences(): Promise<void> {
  inFlight = null;
  await ensurePosterPreferences();
}

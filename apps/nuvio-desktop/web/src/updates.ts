// App updates (official Tauri updater, driven by Rust): one shared state for the banner at the
// top and Settings → Atualizações. Automatic check right after start and every 6 h (normal app mode
// only); t("Depois") hides the banner for that version until the app restarts.

import { useEffect, useSyncExternalStore } from "react";
import { api, errorText } from "./api";
import { listen } from "./tauri";
import type { UpdateInfo, UpdateProgress, UpdateStatus } from "./types";

export interface UpdatesState {
  status: UpdateStatus | null;
  checking: boolean;
  lastCheck: number | null;
  available: UpdateInfo | null;
  /** `null` when up to date after a check. */
  checkError: string | null;
  installing: UpdateProgress | null;
  dismissed: string | null;
}

let state: UpdatesState = { status: null, checking: false, lastCheck: null, available: null, checkError: null, installing: null, dismissed: null };
let started = false;
const listeners = new Set<() => void>();
const set = (patch: Partial<UpdatesState>) => {
  state = { ...state, ...patch };
  listeners.forEach((l) => l());
};

export async function checkForUpdates(): Promise<void> {
  if (state.checking || state.installing) return;
  set({ checking: true, checkError: null });
  try {
    const found = await api.updateCheck();
    set({ available: found, checking: false, lastCheck: Date.now() });
  } catch (e) {
    set({ checking: false, checkError: errorText(e), lastCheck: Date.now() });
  }
}

export async function installUpdate(): Promise<void> {
  if (!state.available || state.installing) return;
  set({ installing: { phase: "downloading", downloaded: 0, total: null, error: null } });
  try {
    await api.updateInstall();
    // On success the app closes and the installer reopens it.
  } catch (e) {
    set({ installing: { phase: "error", downloaded: 0, total: null, error: errorText(e) } });
  }
}

export function dismissUpdate() {
  set({ dismissed: state.available?.version ?? null });
}

export function clearInstallError() {
  set({ installing: null });
}

/** Starts the automatic checks (normal app mode). */
export function startAutomaticChecks() {
  if (started) return;
  started = true;
  // Right after the window opens (a short delay keeps the first screen fluid).
  window.setTimeout(() => void checkForUpdates(), 800);
  window.setInterval(() => void checkForUpdates(), 6 * 60 * 60 * 1000);
}

function init() {
  if (state.status) return;
  api.updateStatus().then((s) => set({ status: s }), () => {});
  listen<UpdateProgress>("update-progress", (p) => set({ installing: p }));
}

export function useUpdates(): UpdatesState {
  useEffect(init, []);
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => state,
  );
}

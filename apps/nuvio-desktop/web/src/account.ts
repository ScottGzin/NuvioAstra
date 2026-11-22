// Shared account state for every screen: the status Rust sends (`sync-status` event) and the avatar
// catalog of the account (loaded once). What the account changed on this computer
// (`sync-applied`) reloads the affected data through the data epoch, like a local change.

import { useEffect, useSyncExternalStore } from "react";
import { api, notifyRemoteChange } from "./api";
import { listen } from "./tauri";
import type { AccountStatus, AvatarItem } from "./types";

let status: AccountStatus | null = null;
let avatars: AvatarItem[] = [];
let started = false;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

function start() {
  if (started) return;
  started = true;
  api.accountStatus().then(set, () => {});
  listen<AccountStatus>("sync-status", set);
  listen<{ profile: number; changed: string[] }>("sync-applied", (e) => notifyRemoteChange(e.profile, e.changed));
}

export function set(next: AccountStatus) {
  const wasConfigured = status?.configured;
  status = next;
  emit();
  if (next.configured && !wasConfigured) loadAvatars();
}

export function loadAvatars() {
  api.avatarsCatalog().then(
    (list) => {
      avatars = list;
      emit();
    },
    () => {},
  );
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => listeners.delete(l);
}

export function useAccount(): AccountStatus | null {
  useEffect(start, []);
  return useSyncExternalStore(subscribe, () => status);
}

export function useAvatars(): AvatarItem[] {
  useEffect(start, []);
  return useSyncExternalStore(subscribe, () => avatars);
}

export function syncLabel(s: AccountStatus): { text: string; tone: "ok" | "busy" | "warn" | "off" | "error" } {
  switch (s.state) {
    case "syncing": return { text: "Syncing…", tone: "busy" };
    case "pending": return { text: `${s.pending} change(s) waiting to be sent`, tone: "warn" };
    case "offline": return { text: "No connection — changes stored to send later", tone: "off" };
    case "error": return { text: "Sync error", tone: "error" };
    case "needsReview": return { text: "Review the first sync", tone: "warn" };
    case "expired": return { text: "Session expired", tone: "error" };
    case "signedOut": return { text: "Signed out", tone: "off" };
    case "notConfigured": return { text: "Account unavailable in this version", tone: "off" };
    default: return { text: "Synced", tone: "ok" };
  }
}

export function when(epochMs: number): string {
  if (!epochMs) return "never";
  const d = new Date(epochMs);
  return d.toLocaleString("en-US", { day: "2-digit", month: "2-digit", year: "numeric", hour: "2-digit", minute: "2-digit" });
}

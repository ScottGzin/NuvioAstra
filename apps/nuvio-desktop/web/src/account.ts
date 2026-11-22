// Shared account state for every screen: the status Rust sends (`sync-status` event) and the avatar
// catalog of the account (loaded once). What the account changed on this computer
// (`sync-applied`) reloads the affected data through the data epoch, like a local change.

import { useEffect, useSyncExternalStore } from "react";
import { api, notifyRemoteChange } from "./api";
import { locale, translate } from "./i18n";
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
    case "syncing": return { text: translate("account.statusSyncing"), tone: "busy" };
    case "pending": return { text: translate("account.statusPending", { count: s.pending }), tone: "warn" };
    case "offline": return { text: translate("account.statusOffline"), tone: "off" };
    case "error": return { text: translate("account.statusError"), tone: "error" };
    case "needsReview": return { text: translate("account.statusNeedsReview"), tone: "warn" };
    case "expired": return { text: translate("account.statusExpired"), tone: "error" };
    case "signedOut": return { text: translate("account.statusSignedOut"), tone: "off" };
    case "notConfigured": return { text: translate("account.statusNotConfigured"), tone: "off" };
    default: return { text: translate("account.statusSynced"), tone: "ok" };
  }
}

export function when(epochMs: number): string {
  if (!epochMs) return translate("account.never");
  const d = new Date(epochMs);
  return d.toLocaleString(locale(), { day: "2-digit", month: "2-digit", year: "numeric", hour: "2-digit", minute: "2-digit" });
}

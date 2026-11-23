// Shared account state for every screen: the status Rust sends (`sync-status` event) and the avatar
// catalog of the account (loaded once). What the account changed on this computer
// (`sync-applied`) reloads the affected data through the data epoch, like a local change.

import { useEffect, useSyncExternalStore } from "react";
import { api, notifyRemoteChange } from "./api";
import { listen } from "./tauri";
import type { AccountStatus, AvatarItem } from "./types";
import { t } from "./i18n";

let status: AccountStatus | null = null;
let avatars: AvatarItem[] = [];
let started = false;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

export function start() {
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
    case "syncing": return { text: t("Sincronizando…"), tone: "busy" };
    case "pending": return { text: `${s.pending} alteração(ões) aguardando envio`, tone: "warn" };
    case "offline": return { text: t("Sem conexão — alterações guardadas para enviar depois"), tone: "off" };
    case "error": return { text: t("Erro de sincronização"), tone: "error" };
    case "needsReview": return { text: t("Revise a primeira sincronização"), tone: "warn" };
    case "expired": return { text: t("Sessão expirada"), tone: "error" };
    case "signedOut": return { text: t("Desconectado"), tone: "off" };
    case "notConfigured": return { text: t("Conta indisponível nesta versão"), tone: "off" };
    default: return { text: t("Sincronizado"), tone: "ok" };
  }
}

export function when(epochMs: number): string {
  if (!epochMs) return t("nunca");
  const d = new Date(epochMs);
  return d.toLocaleString("pt-BR", { day: "2-digit", month: "2-digit", year: "numeric", hour: "2-digit", minute: "2-digit" });
}

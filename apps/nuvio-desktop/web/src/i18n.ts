// UI language: Portuguese (original), English and Arabic, with a persisted switcher and RTL.
// The dictionary lives in i18n.strings.ts; keys are the exact Portuguese source strings.
// Strings missing from the dictionary fall back to Portuguese, so this layer is safe to roll out
// incrementally.

import { useSyncExternalStore } from "react";
import { STRINGS } from "./i18n.strings";

export type Lang = "pt" | "en" | "ar";

const STORAGE_KEY = "nuvio.ui.language";

function initialLang(): Lang {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === "pt" || saved === "en" || saved === "ar") return saved;
  } catch { /* storage may be blocked */ }
  const nav = (navigator.language || "").toLowerCase();
  if (nav.startsWith("ar")) return "ar";
  if (nav.startsWith("en")) return "en";
  return "pt";
}

let current: Lang = initialLang();
const listeners = new Set<() => void>();

function applyDocumentLang() {
  const root = document.documentElement;
  root.lang = current;
  root.dir = current === "ar" ? "rtl" : "ltr";
}

applyDocumentLang();

export function getLang(): Lang {
  return current;
}

export function subscribeLang(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function setLang(lang: Lang) {
  if (lang === current) return;
  current = lang;
  try { localStorage.setItem(STORAGE_KEY, lang); } catch { /* ignore */ }
  applyDocumentLang();
  listeners.forEach((listener) => listener());
}

/** React hook for the switcher and for re-mounting the tree when the language changes. */
export function useLang(): Lang {
  return useSyncExternalStore(subscribeLang, getLang, getLang);
}

/** Translates a Portuguese source string into the active UI language, with `{name}` interpolation. */
export function t(source: string, vars?: Record<string, string | number>): string {
  let out = current === "pt" ? source : (STRINGS[source]?.[current] || source);
  if (vars) {
    for (const [key, value] of Object.entries(vars)) {
      out = out.replaceAll(`{${key}}`, String(value));
    }
  }
  return out;
}

/** BCP-47 locale for date/time formatting in the active UI language. */
export function locale(): string {
  return current === "pt" ? "pt-BR" : current === "ar" ? "ar" : "en-US";
}

/** Translates the suffix addons append to catalog names ("- Filmes" → "- Movies"). */
export function localizeCatalogTitle(title: string): string {
  if (current === "pt") return title;
  const suffixes: Record<string, string> = current === "ar"
    ? { "filmes": "أفلام", "séries": "مسلسلات", "series": "مسلسلات", "canais": "قنوات", "channels": "قنوات" }
    : { "filmes": "Movies", "séries": "Series", "series": "Series", "canais": "Channels", "channels": "Channels" };
  return title.replace(/\s-\s([^-\s].*)$/, (match, rest: string) => {
    const key = rest.trim().toLowerCase();
    return suffixes[key] ? ` - ${suffixes[key]}` : match;
  });
}

export const LANGUAGES_UI: [Lang, string][] = [
  ["pt", "Português"],
  ["en", "English"],
  ["ar", "العربية"],
];

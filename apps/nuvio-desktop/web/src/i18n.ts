// UI language: Portuguese (original) plus the 28 languages the original Nuvio offers, with a
// persisted switcher and RTL. The dictionary lives in i18n.strings.ts; keys are the exact
// Portuguese source strings and every entry carries one translation per language.
// Strings missing from the dictionary fall back to English and then to Portuguese, so this layer
// is safe to roll out incrementally.

import { useSyncExternalStore } from "react";
import { STRINGS } from "./i18n.strings";
import {
  LANGS,
  LANG_LOCALES,
  LANGUAGES_UI,
  RTL_LANGS,
  isLang,
  langFromTag,
  type Lang,
} from "./i18nLangs";

export { LANGS, LANGUAGES_UI, isLang, langFromTag };
export type { Lang };

const STORAGE_KEY = "nuvio.ui.language";

function initialLang(): Lang {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (isLang(saved)) return saved;
  } catch { /* storage may be blocked */ }
  return langFromTag(navigator.language) ?? "pt";
}

let current: Lang = initialLang();
const listeners = new Set<() => void>();

function applyDocumentLang() {
  const root = document.documentElement;
  root.lang = current;
  root.dir = RTL_LANGS.has(current) ? "rtl" : "ltr";
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
  const entry = STRINGS[source];
  let out = current === "pt" ? source : (entry?.[current] || entry?.en || source);
  if (vars) {
    for (const [key, value] of Object.entries(vars)) {
      out = out.replaceAll(`{${key}}`, String(value));
    }
  }
  return out;
}

/** Number with fixed decimals in the active locale (comma in Portuguese, dot in English). */
export function decimal(value: number, digits = 1): string {
  try {
    return new Intl.NumberFormat(locale(), { minimumFractionDigits: digits, maximumFractionDigits: digits }).format(value);
  } catch {
    return value.toFixed(digits);
  }
}

/** BCP-47 locale for date/time formatting in the active UI language. */
export function locale(): string {
  return LANG_LOCALES[current];
}

/** Addon names come from the addons themselves; the only one the app writes is the collection marker. */
export function addonLabel(name: string): string {
  return name === "Coleção" ? t("Coleção") : name;
}

/** Translates the suffix addons append to catalog names ("- Filmes" → "- Movies"). */
export function localizeCatalogTitle(title: string): string {
  if (current === "pt") return title;
  const suffixes: Record<string, string> = {
    filmes: t("Filmes"),
    séries: t("Séries"),
    series: t("Séries"),
    canais: t("Canais"),
    channels: t("Canais"),
  };
  return title.replace(/\s-\s([^-\s].*)$/, (match, rest: string) => {
    const key = rest.trim().toLowerCase();
    return suffixes[key] ? ` - ${suffixes[key]}` : match;
  });
}

/** Content type as the original's `localizedMediaTypeLabel` shows it in the Discover picker. */
export function mediaTypeLabel(contentType: string): string {
  switch (contentType.trim().toLowerCase()) {
    case "movie": return t("Filmes");
    case "series": return t("Séries");
    case "anime": return t("Anime");
    case "channel": return t("Canais");
    case "tv": return t("TV");
    default: {
      const type = contentType.trim();
      return type ? type[0].toUpperCase() + type.slice(1) : "";
    }
  }
}

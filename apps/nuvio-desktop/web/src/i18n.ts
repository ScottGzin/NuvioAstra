// UI language: the 29 languages of Nuvio Desktop (`AppLanguage`), with a persisted switcher and RTL.
// The dictionary lives in i18n.strings.ts; keys are the exact Portuguese (Brazil) source strings.
// A string missing in a language falls back to English (Portugal: to the Portuguese key), so new
// texts never show up empty.
//
// Which language opens: the one chosen on this computer; else the language of the system. The
// Nuvio account's language (`appLanguage.ts`) replaces it after the first sync.

import { useSyncExternalStore } from "react";
import { STRINGS } from "./i18n.strings";

/** `pt` is Portuguese (Brazil), the source language. */
export type Lang =
  | "pt" | "pt-PT" | "en" | "ar" | "bn" | "bg" | "hr" | "cs" | "nl" | "fr" | "de" | "el" | "he" | "hu" | "id"
  | "it" | "pl" | "ro" | "ru" | "sk" | "es" | "ta" | "tr" | "uk" | "ur" | "nb" | "ja" | "vi" | "zh";

/** Each language in its own name (the original's `lang_*` labels), in the original's order. */
export const LANGUAGES_UI: [Lang, string][] = [
  ["ar", "العربية"],
  ["bn", "বাংলা"],
  ["bg", "Български"],
  ["hr", "Hrvatski"],
  ["cs", "Čeština"],
  ["nl", "Nederlands"],
  ["en", "English"],
  ["fr", "Français"],
  ["de", "Deutsch"],
  ["el", "Ελληνικά"],
  ["he", "עברית"],
  ["hu", "Magyar"],
  ["id", "Bahasa Indonesia"],
  ["it", "Italiano"],
  ["pl", "Polski"],
  ["pt", "Português (Brasil)"],
  ["pt-PT", "Português (Portugal)"],
  ["ro", "Română"],
  ["ru", "Русский"],
  ["sk", "Slovenčina"],
  ["es", "Español"],
  ["ta", "தமிழ்"],
  ["tr", "Türkçe"],
  ["uk", "Українська"],
  ["ur", "اردو"],
  ["nb", "Norsk"],
  ["ja", "日本語"],
  ["vi", "Tiếng Việt"],
  ["zh", "中文"],
];

const LANGS = new Set<string>(LANGUAGES_UI.map(([code]) => code));
const RTL = new Set<Lang>(["ar", "he", "ur"]);

export function isLang(value: unknown): value is Lang {
  return typeof value === "string" && LANGS.has(value);
}

/** A BCP-47 tag (system or account) as one of the UI languages; null when none fits. */
export function langFromTag(tag: string | null | undefined): Lang | null {
  const raw = (tag ?? "").trim().replace("_", "-").toLowerCase();
  if (!raw) return null;
  const [base, region] = raw.split("-");
  if (base === "pt") return region === "pt" ? "pt-PT" : "pt";
  if (base === "no" || base === "nn" || base === "nb") return "nb";
  if (base === "in") return "id";
  if (base === "iw") return "he";
  return isLang(base) ? base : null;
}

/** The system's language (Windows display language through WebView2), English when not supported. */
export function systemLang(): Lang {
  const tags = navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const tag of tags) {
    const lang = langFromTag(tag);
    if (lang) return lang;
  }
  return "en";
}

const STORAGE_KEY = "nuvio.ui.language";

function initialLang(): Lang {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (isLang(saved)) return saved;
  } catch { /* storage may be blocked */ }
  return systemLang();
}

let current: Lang = initialLang();
const listeners = new Set<() => void>();

function applyDocumentLang() {
  const root = document.documentElement;
  root.lang = locale();
  root.dir = RTL.has(current) ? "rtl" : "ltr";
}

applyDocumentLang();

export function getLang(): Lang {
  return current;
}

export function subscribeLang(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** `remember`: kept as this computer's choice (false when it only follows the system). */
export function setLang(lang: Lang, remember = true) {
  try {
    if (remember) localStorage.setItem(STORAGE_KEY, lang);
    else localStorage.removeItem(STORAGE_KEY);
  } catch { /* ignore */ }
  if (lang === current) return;
  current = lang;
  applyDocumentLang();
  listeners.forEach((listener) => listener());
}

/** React hook for the switcher and for re-mounting the tree when the language changes. */
export function useLang(): Lang {
  return useSyncExternalStore(subscribeLang, getLang, getLang);
}

/** Translates a Portuguese source string into the active UI language, with `{name}` interpolation. */
export function t(source: string, vars?: Record<string, string | number>): string {
  let out = source;
  if (current !== "pt") {
    const entry = STRINGS[source];
    out = entry?.[current] || (current === "pt-PT" ? source : entry?.en) || source;
  }
  if (vars) {
    for (const [key, value] of Object.entries(vars)) {
      out = out.replaceAll(`{${key}}`, String(value));
    }
  }
  return out;
}

/** Number with fixed decimals in the active locale (comma in Portuguese, dot in English…). */
export function decimal(value: number, digits = 1): string {
  try {
    return new Intl.NumberFormat(locale(), { minimumFractionDigits: digits, maximumFractionDigits: digits }).format(value);
  } catch {
    return value.toFixed(digits);
  }
}

/** BCP-47 locale for dates, numbers and language names in the active UI language. */
export function locale(): string {
  switch (current) {
    case "pt":
      return "pt-BR";
    case "en":
      return "en-US";
    default:
      return current;
  }
}

/** Addon names come from the addons themselves; the only one the app writes is the collection marker. */
export function addonLabel(name: string): string {
  return name === "Coleção" ? t("Coleção") : name;
}

/** Translates the suffix addons append to catalog names ("- Filmes" → "- Movies"). */
export function localizeCatalogTitle(title: string): string {
  if (current === "pt") return title;
  const suffixes: Record<string, string> = { filmes: "Filmes", "séries": "Séries", series: "Séries" };
  return title.replace(/\s-\s([^-\s].*)$/, (match, rest: string) => {
    const source = suffixes[rest.trim().toLowerCase()];
    return source ? ` - ${t(source)}` : match;
  });
}

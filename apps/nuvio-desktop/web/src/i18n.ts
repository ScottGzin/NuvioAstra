// UI language. English is the source language: `t("English text")` returns the text itself, and
// other languages map that exact text to a translation (see locales/). Missing entries fall back
// to English. The choice lives in localStorage (UI only; Rust messages stay English).
import { useSyncExternalStore } from "react";
import { ptBR } from "./locales/pt-BR";

export type Lang = "en" | "pt-BR";
/** Endonyms: each language is always shown in its own language. */
export const LANGUAGES: { code: Lang; label: string }[] = [
  { code: "en", label: "English" },
  { code: "pt-BR", label: "Português (Brasil)" },
];
const TABLES: Record<Lang, Record<string, string>> = { en: {}, "pt-BR": ptBR };
const KEY = "nuvio.lang";

function initial(): Lang {
  try {
    const saved = localStorage.getItem(KEY);
    if (saved === "en" || saved === "pt-BR") return saved;
  } catch {
    // storage unavailable: use the system language
  }
  return navigator.language?.toLowerCase().startsWith("pt") ? "pt-BR" : "en";
}

let current: Lang = initial();
const listeners = new Set<() => void>();
document.documentElement.lang = current;

export const getLang = (): Lang => current;
/** Locale for Intl/toLocale* calls. */
export const locale = (): string => (current === "pt-BR" ? "pt-BR" : "en-US");

export function setLang(lang: Lang): void {
  if (lang === current) return;
  current = lang;
  document.documentElement.lang = lang;
  try {
    localStorage.setItem(KEY, lang);
  } catch {
    // not persisted; applies for this session
  }
  listeners.forEach((l) => l());
}

/**
 * Translate `text`; `{name}` placeholders are filled from `vars`. When one English word needs
 * two translations, prefix a context: t("singular|Series") shows "Series" in English.
 */
export function t(text: string, vars?: Record<string, string | number>): string {
  const s = TABLES[current][text] ?? text.replace(/^[a-z]+\|/, "");
  return vars ? s.replace(/\{(\w+)\}/g, (m, k: string) => (k in vars ? String(vars[k]) : m)) : s;
}

/** Re-renders the caller (and its children) when the language changes. */
export function useLang(): Lang {
  return useSyncExternalStore((cb) => (listeners.add(cb), () => listeners.delete(cb)), getLang);
}

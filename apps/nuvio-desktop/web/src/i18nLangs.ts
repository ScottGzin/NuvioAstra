// Languages offered by the interface, matching the `AppLanguage` list of the original Nuvio
// (`features/settings/AppLanguage.kt`). The source language is Portuguese (Brazil); every other
// entry exists in `i18n.strings.ts`, which is keyed by the Portuguese source text.

export const LANGS = [
  "pt",
  "pt-PT",
  "ar",
  "bn",
  "bg",
  "hr",
  "cs",
  "nl",
  "en",
  "fr",
  "de",
  "el",
  "he",
  "hu",
  "id",
  "it",
  "pl",
  "ro",
  "ru",
  "sk",
  "es",
  "ta",
  "tr",
  "uk",
  "ur",
  "nb",
  "ja",
  "vi",
  "zh",
] as const;

export type Lang = (typeof LANGS)[number];

/** Written right to left; `applyDocumentLang` sets `dir="rtl"` for these. */
export const RTL_LANGS: ReadonlySet<Lang> = new Set<Lang>(["ar", "he", "ur"]);

/** BCP-47 locale used by `Intl` for dates, numbers and language names. */
export const LANG_LOCALES: Record<Lang, string> = {
  pt: "pt-BR",
  "pt-PT": "pt-PT",
  ar: "ar",
  bn: "bn-BD",
  bg: "bg-BG",
  hr: "hr-HR",
  cs: "cs-CZ",
  nl: "nl-NL",
  en: "en-US",
  fr: "fr-FR",
  de: "de-DE",
  el: "el-GR",
  he: "he-IL",
  hu: "hu-HU",
  id: "id-ID",
  it: "it-IT",
  pl: "pl-PL",
  ro: "ro-RO",
  ru: "ru-RU",
  sk: "sk-SK",
  es: "es-ES",
  ta: "ta-IN",
  tr: "tr-TR",
  uk: "uk-UA",
  ur: "ur-PK",
  nb: "nb-NO",
  ja: "ja-JP",
  vi: "vi-VN",
  zh: "zh-CN",
};

/** Native names for the Settings switcher, alphabetical by English name. */
export const LANGUAGES_UI: [Lang, string][] = [
  ["ar", "العربية"],
  ["bn", "বাংলা"],
  ["bg", "Български"],
  ["zh", "中文"],
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
  ["ja", "日本語"],
  ["nb", "Norsk bokmål"],
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
  ["vi", "Tiếng Việt"],
];

export function isLang(value: string | null | undefined): value is Lang {
  return !!value && (LANGS as readonly string[]).includes(value);
}

/** Best match for a BCP-47 tag (`pt-BR`, `en-GB`, `zh-Hans`), exact tag first. */
export function langFromTag(tag: string | null | undefined): Lang | null {
  const value = (tag ?? "").toLowerCase();
  if (!value) return null;
  if (isLang(value)) return value;
  const base = value.split("-")[0];
  return isLang(base) ? base : null;
}

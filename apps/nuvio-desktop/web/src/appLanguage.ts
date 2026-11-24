// The interface language saved with the profile and the Nuvio account (`selected_app_language`),
// so another computer signed in to the same account opens in the same language.

import { api, onDataEpoch, onRemoteSettingsChanged } from "./api";
import { getLang, setLang, type Lang } from "./i18n";
import { langFromTag } from "./i18nLangs";

/** Codes of the original's `AppLanguage`; the source language is Brazilian Portuguese. */
const ACCOUNT_CODE: Record<Lang, string> = {
  pt: "pt-BR",
  "pt-PT": "pt",
  ar: "ar",
  bn: "bn",
  bg: "bg",
  hr: "hr",
  cs: "cs",
  nl: "nl",
  en: "en",
  fr: "fr",
  de: "de",
  el: "el",
  he: "he",
  hu: "hu",
  id: "id",
  it: "it",
  pl: "pl",
  ro: "ro",
  ru: "ru",
  sk: "sk",
  es: "es",
  ta: "ta",
  tr: "tr",
  uk: "uk",
  ur: "ur",
  nb: "nb",
  ja: "ja",
  vi: "vi",
  zh: "zh",
};

function fromAccount(code: string | null | undefined): Lang | null {
  const value = (code ?? "").trim().toLowerCase();
  if (!value) return null;
  // The source language is Brazilian Portuguese; the original also offers Portugal's.
  if (value === "pt-br") return "pt";
  if (value === "pt") return "pt-PT";
  return langFromTag(value);
}

// Changing the language re-mounts the interface; not while a video is on screen.
let waiting = 0;
function apply(lang: Lang) {
  window.clearTimeout(waiting);
  if (lang === getLang()) return;
  if (document.querySelector(".full-player")) {
    waiting = window.setTimeout(() => apply(lang), 5000);
    return;
  }
  setLang(lang);
}

function load() {
  api.settingsGet().then((view) => {
    const lang = fromAccount(view.settings.player.appLanguage);
    if (lang) apply(lang);
  }, () => {});
}

let started = false;
export function startAppLanguage() {
  if (started) return;
  started = true;
  load();
  onRemoteSettingsChanged(load);
  onDataEpoch((reason) => reason === "profile" && load());
}

/** Chosen in Settings: saved in the profile (and so sent to the account), then applied. */
export async function chooseAppLanguage(lang: Lang): Promise<void> {
  try {
    const view = await api.settingsGet();
    const before = view.settings;
    const edited = { ...before, player: { ...before.player, appLanguage: ACCOUNT_CODE[lang] } };
    await api.settingsSet(edited, before, view.profile.index);
  } catch {
    /* the profile may be unavailable; the choice still applies to this session */
  } finally {
    window.clearTimeout(waiting);
    setLang(lang);
  }
}

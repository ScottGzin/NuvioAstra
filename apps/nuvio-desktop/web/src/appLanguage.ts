// The interface language saved with the profile and the Nuvio account (`selected_app_language`),
// so another computer signed in to the same account opens in the same language.
//
// Codes are the original's `AppLanguage` ones: `pt-BR`, `pt` (Portugal), `en`, `de`… and `device`,
// which follows the system. Until the account says otherwise (or with `device`), the app opens in
// the language of the system.

import { api, onDataEpoch, onRemoteSettingsChanged } from "./api";
import { getLang, langFromTag, setLang, subscribeLang, systemLang, type Lang } from "./i18n";

/** `AppLanguage.code` of the original for a UI language. */
function accountCode(lang: Lang): string {
  return lang === "pt" ? "pt-BR" : lang === "pt-PT" ? "pt" : lang;
}

/** The UI language of an account code; `device` (or nothing) is the system's. */
function fromAccount(code: string | null | undefined): { lang: Lang; remember: boolean } | null {
  if (!code) return null;
  if (code.toLowerCase() === "device") return { lang: systemLang(), remember: false };
  const lang = code.toLowerCase() === "pt" ? "pt-PT" : langFromTag(code);
  return lang ? { lang, remember: true } : null;
}

// Changing the language re-mounts the interface; not while a video is on screen.
let waiting = 0;
function apply(lang: Lang, remember: boolean) {
  window.clearTimeout(waiting);
  if (document.querySelector(".full-player") && lang !== getLang()) {
    waiting = window.setTimeout(() => apply(lang, remember), 5000);
    return;
  }
  setLang(lang, remember);
}

function load() {
  api.settingsGet().then((view) => {
    const chosen = fromAccount(view.settings.player.appLanguage);
    if (chosen) apply(chosen.lang, chosen.remember);
  }, () => {});
}

let started = false;
export function startAppLanguage() {
  if (started) return;
  started = true;
  // TMDB follows the interface language unless Settings → TMDB picks another.
  void api.uiLanguageSet(getLang());
  subscribeLang(() => void api.uiLanguageSet(getLang()));
  load();
  onRemoteSettingsChanged(load);
  onDataEpoch((reason) => reason === "profile" && load());
}

/** Chosen in Settings (`device` = the system's): saved in the profile (and so sent to the
 *  account), then applied. */
export async function chooseAppLanguage(choice: Lang | "device"): Promise<void> {
  const lang = choice === "device" ? systemLang() : choice;
  try {
    const view = await api.settingsGet();
    const before = view.settings;
    const edited = { ...before, player: { ...before.player, appLanguage: choice === "device" ? "device" : accountCode(choice) } };
    await api.settingsSet(edited, before, view.profile.index);
  } finally {
    window.clearTimeout(waiting);
    setLang(lang, choice !== "device");
  }
}

/** What the Settings switcher shows: the account's choice, `device` when it follows the system. */
export function chosenLanguage(appLanguage: string | null | undefined): Lang | "device" {
  const chosen = fromAccount(appLanguage);
  if (!chosen || !chosen.remember) return "device";
  return chosen.lang;
}

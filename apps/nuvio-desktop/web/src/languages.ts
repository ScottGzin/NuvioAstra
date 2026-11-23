// Generated from `AvailableLanguageOptions` of the original (PlayerLanguagePreferences.kt) with its
// pt-BR names (composeResources/values-pt*).
export const LANGUAGES: [code: string, name: string][] = [
  ["af", "Afrikaans"],
  ["sq", "Albanês"],
  ["am", "Amárico"],
  ["ar", "Árabe"],
  ["hy", "Arménio"],
  ["az", "Azerbaijano"],
  ["eu", "Basco"],
  ["be", "Bielorrusso"],
  ["bn", "Bengali"],
  ["bs", "Bósnio"],
  ["bg", "Búlgaro"],
  ["my", "Birmanês"],
  ["ca", "Catalão"],
  ["zh", "Chinês"],
  ["zh-CN", "Chinês (Simplificado)"],
  ["zh-TW", "Chinês (Tradicional)"],
  ["hr", "Croata"],
  ["cs", "Checo"],
  ["da", "Dinamarquês"],
  ["nl", "Neerlandês"],
  ["en", "Inglês"],
  ["et", "Estónio"],
  ["tl", "Filipino"],
  ["fi", "Finlandês"],
  ["fr", "Francês"],
  ["gl", "Galego"],
  ["ka", "Georgiano"],
  ["de", "Alemão"],
  ["el", "Grego"],
  ["gu", "Gujarati"],
  ["he", "Hebraico"],
  ["hi", "Hindi"],
  ["hu", "Húngaro"],
  ["is", "Islandês"],
  ["id", "Indonésio"],
  ["ga", "Irlandês"],
  ["it", "Italiano"],
  ["ja", "Japonês"],
  ["kn", "Canarim"],
  ["kk", "Cazaque"],
  ["km", "Khmer"],
  ["ko", "Coreano"],
  ["lo", "Laosiano"],
  ["lv", "Letão"],
  ["lt", "Lituano"],
  ["mk", "Macedónio"],
  ["ms", "Malaio"],
  ["ml", "Malayalam"],
  ["mt", "Maltês"],
  ["mr", "Marata"],
  ["mn", "Mongol"],
  ["ne", "Nepali"],
  ["no", "Norueguês"],
  ["pa", "Panjabi"],
  ["fa", "Persa"],
  ["pl", "Polaco"],
  ["pt", "Português (Portugal)"],
  ["pt-BR", "Português (Brasil)"],
  ["ro", "Romeno"],
  ["ru", "Russo"],
  ["sr", "Sérvio"],
  ["si", "Cingalês"],
  ["sk", "Eslovaco"],
  ["sl", "Esloveno"],
  ["es", "Espanhol"],
  ["es-419", "Espanhol (América Latina)"],
  ["sw", "Suaíli"],
  ["sv", "Sueco"],
  ["ta", "Tâmil"],
  ["te", "Telugu"],
  ["th", "Tailandês"],
  ["tr", "Turco"],
  ["uk", "Ucraniano"],
  ["ur", "Urdu"],
  ["uz", "Usbeque"],
  ["vi", "Vietnamita"],
  ["cy", "Galês"],
  ["zu", "Zulu"],
];

/**
 * Name of a track language in the active UI language. The original list above has pt-BR names;
 * Intl.DisplayNames gives proper English/Arabic names natively (no extra dictionary), with the
 * Portuguese list as a fallback for old WebKit builds.
 */
export function languageLabel(code: string, uiLang: "pt" | "en" | "ar"): string {
  const portuguese = LANGUAGES.find(([c]) => c.toLowerCase() === code.toLowerCase())?.[1];
  if (uiLang === "pt" || !code) return portuguese ?? code;
  try {
    const names = new Intl.DisplayNames([uiLang], { type: "language" });
    return names.of(code) ?? portuguese ?? code;
  } catch {
    return portuguese ?? code;
  }
}

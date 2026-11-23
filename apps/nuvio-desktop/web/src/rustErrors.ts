// Messages the Rust side builds with `format!` (and so cannot be looked up as exact dictionary
// keys) are matched here by shape. The values Rust interpolated — HTTP codes, engine output —
// pass through untouched. Fixed messages live in i18n.strings.ts; `message()` tries these rules
// first and falls back to the dictionary, which is also what `errorText()` uses.

import { getLang, t } from "./i18n";

interface Rule {
  pattern: RegExp;
  en: string;
  ar: string;
}

const RULES: Rule[] = [
  {
    pattern: /^Campo ausente no manifesto: "(.*)"$/,
    en: "Missing field in the manifest: “$1”",
    ar: "حقل مفقود في البيان: «$1»",
  },
  {
    pattern: /^O endereço respondeu com erro HTTP (\d+)\.$/,
    en: "The address returned HTTP error $1.",
    ar: "أعاد العنوان خطأ HTTP $1.",
  },
  {
    pattern: /^Sem conexão com o servidor de atualizações \((.*)\)\.$/,
    en: "No connection to the update server ($1).",
    ar: "لا اتصال بخادم التحديثات ($1).",
  },
  {
    pattern: /^Não foi possível atualizar: (.*)$/,
    en: "Could not update: $1",
    ar: "تعذّر التحديث: $1",
  },
  {
    pattern: /^Não foi possível guardar a fonte: (.*)$/,
    en: "Could not save the font: $1",
    ar: "تعذّر حفظ الخط: $1",
  },
  {
    pattern: /^Não foi possível iniciar o motor P2P: (.*)$/,
    en: "Could not start the P2P engine: $1",
    ar: "تعذّر تشغيل محرّك P2P: $1",
  },
  {
    pattern: /^Não sincronizado: (.*)$/,
    en: "Not synced: $1",
    ar: "لم تتم المزامنة: $1",
  },
  {
    pattern: /^Bloqueado\. Tente novamente em (\d+)s$/,
    en: "Locked. Try again in $1s",
    ar: "مقفل. حاول مجددًا بعد $1 ث",
  },
  {
    pattern: /^HTTP (\d+)$/,
    en: "HTTP $1",
    ar: "HTTP $1",
  },
];

/** Translates a message built by Rust; null when no rule matches. */
export function rustError(raw: string): string | null {
  const lang = getLang();
  if (lang === "pt") return null;
  for (const rule of RULES) {
    const match = raw.match(rule.pattern);
    if (!match) continue;
    return rule[lang].replace(/\$(\d)/g, (_, index: string) => match[Number(index)] ?? "");
  }
  return null;
}

/** Any text that may come from Rust: shaped messages first, then the exact dictionary. */
export function message(raw: string): string {
  return rustError(raw) ?? t(raw);
}

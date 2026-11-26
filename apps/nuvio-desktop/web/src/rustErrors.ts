// Messages the Rust side builds with `format!` (and so cannot be looked up as exact dictionary
// keys) are matched here by shape. The values Rust interpolated — HTTP codes, engine output —
// pass through untouched. Templates live in i18n.strings.ts (as English source keys), so they are
// translated into every UI language; `message()` tries these rules first and falls back to the
// dictionary, which is also what `errorText()` uses.

import { getLang, t } from "./i18n";

interface Rule {
  pattern: RegExp;
  message: string;
}

const RULES: Rule[] = [
  { pattern: /^Campo ausente no manifesto: "(.*)"$/, message: "Missing field in the manifest: “{1}”" },
  { pattern: /^O endereço respondeu com erro HTTP (\d+)\.$/, message: "The address returned HTTP error {1}." },
  { pattern: /^Sem conexão com o servidor de atualizações \((.*)\)\.$/, message: "No connection to the update server ({1})." },
  { pattern: /^Não foi possível atualizar: (.*)$/, message: "Could not update: {1}" },
  { pattern: /^Não foi possível guardar a fonte: (.*)$/, message: "Could not save the font: {1}" },
  { pattern: /^Não foi possível iniciar o motor P2P: (.*)$/, message: "Could not start the P2P engine: {1}" },
  { pattern: /^Não sincronizado: (.*)$/, message: "Not synced: {1}" },
  { pattern: /^Bloqueado\. Tente novamente em (\d+)s$/, message: "Locked. Try again in {1}s" },
  { pattern: /^HTTP (\d+)$/, message: "HTTP {1}" },
];

/** Translates a message built by Rust; null when no rule matches. */
export function rustError(raw: string): string | null {
  if (getLang() === "pt") return null;
  for (const rule of RULES) {
    const match = raw.match(rule.pattern);
    if (!match) continue;
    const vars: Record<string, string> = {};
    for (let i = 1; i < match.length; i += 1) vars[String(i)] = match[i] ?? "";
    return t(rule.message, vars);
  }
  return null;
}

/** Any text that may come from Rust: shaped messages first, then the exact dictionary. */
export function message(raw: string): string {
  return rustError(raw) ?? t(raw);
}

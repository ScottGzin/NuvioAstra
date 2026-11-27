// Release notes as written in the commit description (emojis included), with the small part of
// Markdown that Discord-style messages use: "# / ## / ###" titles, "- " items, "> " quotes, "---",
// **bold**, *italic* and `code`. Built as React elements: nothing is read as HTML.
//
// Bilingual notes: a title line "## 🇧🇷 Português" (or "## Português", "## PT") starts the Portuguese
// part and "## 🇺🇸 English" (or "## English", "## EN") the English part. The app shows only one: the
// Portuguese part to Portuguese (Brazil or Portugal) users, the English part to everyone else, and the
// other part when the chosen one is empty. Lines before any of these titles are shown to everyone.
// Notes without these titles are shown whole, as before.

import type { ReactNode } from "react";
import { getLang } from "../i18n";

function inline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  const pattern = /\*\*(.+?)\*\*|`([^`]+)`|\*([^*\s][^*]*?)\*|_([^_\s][^_]*?)_/g;
  let last = 0;
  let key = 0;
  for (let m = pattern.exec(text); m; m = pattern.exec(text)) {
    if (m.index > last) out.push(text.slice(last, m.index));
    if (m[1] !== undefined) out.push(<b key={key++}>{inline(m[1])}</b>);
    else if (m[2] !== undefined) out.push(<code key={key++}>{m[2]}</code>);
    else out.push(<i key={key++}>{m[3] ?? m[4]}</i>);
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

type Block = { kind: "h" | "p" | "quote" | "hr" } & { text: string } | { kind: "list"; items: string[] };

type NotesLang = "pt" | "en";
const PT_TITLE = /^(?:[-*•]\s+)?#{1,3}\s+[^\p{L}]*(portugu[eê]s|portuguese|pt(?:-br)?)\s*$/iu;
const EN_TITLE = /^(?:[-*•]\s+)?#{1,3}\s+[^\p{L}]*(english|ingl[eê]s|en)\s*$/iu;

/** Keeps the lines meant for `lang` (see the header). */
export function notesFor(notes: string, lang: NotesLang): string {
  const common: string[] = [];
  const parts: Record<NotesLang, string[]> = { pt: [], en: [] };
  let current: NotesLang | null = null;
  let marked = false;
  for (const raw of notes.replace(/\r/g, "").split("\n")) {
    const line = raw.trim();
    if (PT_TITLE.test(line)) { current = "pt"; marked = true; continue; }
    if (EN_TITLE.test(line)) { current = "en"; marked = true; continue; }
    (current ? parts[current] : common).push(raw);
  }
  if (!marked) return notes;
  const other: NotesLang = lang === "pt" ? "en" : "pt";
  const chosen = parts[lang].some((l) => l.trim()) ? parts[lang] : parts[other];
  // A "---" left at the end of the shared part only separated it from the language titles.
  while (common.length && /^\s*(-{3,})?\s*$/.test(common[common.length - 1])) common.pop();
  return [...common, ...(common.length ? ["---"] : []), ...chosen].join("\n");
}

export function parseNotes(notes: string): Block[] {
  const blocks: Block[] = [];
  for (const raw of notes.replace(/\r/g, "").split("\n")) {
    let line = raw.trim();
    if (!line) continue;
    // The release workflow prefixes every line with "- "; titles, quotes and rules keep their meaning.
    const unprefixed = line.replace(/^[-*•]\s+(?=#{1,3}\s|>\s?|-{3,}$)/, "");
    line = unprefixed;
    if (/^-{3,}$/.test(line)) blocks.push({ kind: "hr", text: "" });
    else if (/^#{1,3}\s/.test(line)) blocks.push({ kind: "h", text: line.replace(/^#{1,3}\s+/, "") });
    else if (line.startsWith(">")) blocks.push({ kind: "quote", text: line.replace(/^>\s?/, "") });
    else if (/^[-*•]\s+/.test(line)) {
      const item = line.replace(/^[-*•]\s+/, "");
      const previous = blocks[blocks.length - 1];
      if (previous?.kind === "list") previous.items.push(item);
      else blocks.push({ kind: "list", items: [item] });
    } else blocks.push({ kind: "p", text: line });
  }
  return blocks;
}

export function ReleaseNotes({ notes }: { notes: string }) {
  return (
    <div className="release-notes" data-testid="release-notes">
      {parseNotes(notesFor(notes, getLang() === "pt" || getLang() === "pt-PT" ? "pt" : "en")).map((block, i) => {
        switch (block.kind) {
          case "h": return <h3 key={i}>{inline(block.text)}</h3>;
          case "quote": return <blockquote key={i}>{inline(block.text)}</blockquote>;
          case "hr": return <hr key={i} />;
          case "list": return <ul key={i}>{block.items.map((item, j) => <li key={j}>{inline(item)}</li>)}</ul>;
          default: return <p key={i}>{inline(block.text)}</p>;
        }
      })}
    </div>
  );
}

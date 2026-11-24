// Release notes as written in the commit description (emojis included), with the small part of
// Markdown that Discord-style messages use: "# / ## / ###" titles, "- " items, "> " quotes, "---",
// **bold**, *italic* and `code`. Built as React elements: nothing is read as HTML.

import type { ReactNode } from "react";

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
      {parseNotes(notes).map((block, i) => {
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

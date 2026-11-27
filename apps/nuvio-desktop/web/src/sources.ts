// What a source becomes when it is chosen: TorBox link (cached torrents and the addons' own TorBox
// sources), P2P (other torrents) or the addon's link. Also the badges read from its texts and the
// download request built from what the screen knows.

import { api, errorText } from "./api";
import { decimal, t } from "./i18n";
import { showToast } from "./toast";
import type { DebridState, DownloadRequest, StreamEntry } from "./types";

export type TorboxStates = Record<string, DebridState>;

/** The episode a source belongs to (picks the file inside a season pack). */
export interface SourceContext {
  season: number | null;
  episode: number | null;
}

/** Plays through a debrid service when one has the source (or may have it: `try`). `true`: ready
 *  to play as a link. A torrent no service has still plays through P2P. */
export async function viaTorbox(stream: StreamEntry, states: TorboxStates, ctx: SourceContext): Promise<boolean> {
  if (!stream.torbox && states[stream.key] !== "cached" && states[stream.key] !== "try") return false;
  try {
    await api.torboxPrepare(stream.key, ctx.season, ctx.episode);
    return true;
  } catch (e) {
    // A torrent TorBox could not open still plays through P2P.
    if (stream.kind === "torrent") return false;
    throw e;
  }
}

export function canDownload(stream: StreamEntry, states: TorboxStates): boolean {
  return stream.kind === "direct" || stream.kind === "torrent" || (stream.kind === "debrid" && states[stream.key] === "cached");
}

export async function startDownload(request: DownloadRequest): Promise<void> {
  try {
    const item = await api.downloadStart(request);
    showToast(
      item.kind === "torbox"
        ? t("Download iniciado pelo TorBox. Acompanhe em Biblioteca → Downloads.")
        : item.kind === "p2p"
          ? t("Download iniciado por P2P. Acompanhe em Biblioteca → Downloads.")
          : t("Download iniciado. Acompanhe em Biblioteca → Downloads."),
    );
  } catch (e) {
    const text = errorText(e);
    showToast(text.includes("P2P_CONSENT_REQUIRED") ? t("Ative o Streaming P2P em Configurações → Torrent / P2P para baixar torrents.") : t("Não foi possível baixar: {error}", { error: text }), true);
  }
}

export function sizeText(bytes: number): string {
  if (bytes >= 1073741824) return `${decimal(bytes / 1073741824, 2)} GB`;
  if (bytes >= 1048576) return `${Math.round(bytes / 1048576)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

/** Quality, HDR, codec, audio and size read from the addon's texts (what Fusion-style addons show). */
export function badgesOf(stream: StreamEntry): string[] {
  const text = `${stream.name ?? ""} ${stream.description ?? ""} ${stream.filename ?? ""}`;
  const has = (pattern: RegExp) => pattern.test(text);
  const out: string[] = [];
  if (has(/\b(2160p|4k|uhd)\b/i)) out.push("4K");
  else if (has(/\b1440p\b/i)) out.push("1440p");
  else if (has(/\b1080p\b/i)) out.push("1080p");
  else if (has(/\b720p\b/i)) out.push("720p");
  else if (has(/\b(480p|576p|sd)\b/i)) out.push("SD");
  if (has(/\b(dv|dovi|dolby[ .-]?vision)\b/i)) out.push("Dolby Vision");
  if (has(/\bhdr10\+/i)) out.push("HDR10+");
  else if (has(/\bhdr(10)?\b/i)) out.push("HDR");
  if (has(/\b(hevc|x265|h\.?265)\b/i)) out.push("HEVC");
  else if (has(/\bav1\b/i)) out.push("AV1");
  if (has(/\batmos\b/i)) out.push("Atmos");
  else if (has(/\b(ddp?|eac3|dd\+)\s?5\.1\b|\b5\.1\b/i)) out.push("5.1");
  if (has(/\bremux\b/i)) out.push("REMUX");
  if (stream.size && stream.size > 0) out.push(sizeText(stream.size));
  else {
    const size = /(\d+(?:[.,]\d+)?)\s?(GB|GiB|MB|MiB)\b/i.exec(text);
    if (size) out.push(`${size[1].replace(".", ",")} ${size[2].toUpperCase().replace("IB", "B")}`);
  }
  const seeds = /(?:👤|seeds?:?|seeders:?)\s?(\d+)/i.exec(text);
  if (seeds) out.push(`${seeds[1]} seeds`);
  return out;
}

// ---------------------------------------------------------------- automatic choice and fallback
// Port of the original's `StreamAutoPlaySelector.evaluateAutoPlayStream` for the sources the UI
// sees (names and texts, never URLs): FIRST_STREAM takes the first ready source, REGEX_MATCH the
// first whose texts match the pattern (case-insensitive); words inside `(?!...(a|b))` also exclude
// sources that contain them anywhere, as in the original.

export interface AutoChoice {
  mode: "MANUAL" | "FIRST_STREAM" | "REGEX_MATCH";
  regex: string;
  /** Allowed addons (empty = all). */
  addons: string[];
}

export interface ListedStream {
  stream: StreamEntry;
  addonName: string;
}

/** Can start without asking the user (`isAutoPlayable`): a link, a YouTube video, a debrid
 *  source a service has (or will be asked for). Plain P2P torrents are left to the user. */
export function autoPlayable(s: StreamEntry, states: TorboxStates): boolean {
  return (
    s.kind === "direct" ||
    s.kind === "youTube" ||
    (s.kind === "debrid" && states[s.key] === "cached") ||
    (s.kind === "torrent" && (states[s.key] === "cached" || states[s.key] === "try"))
  );
}

function matcher(pattern: string): ((text: string) => boolean) | null {
  const p = pattern.trim();
  if (!p || !/[\p{L}\p{N}]/u.test(p)) return null;
  let user: RegExp;
  try {
    user = new RegExp(p, "i");
  } catch {
    return null;
  }
  const words = [...p.matchAll(/\(\?![^)]*?\(([^)]+)\)/g)].flatMap((m) => m[1].split("|")).map((w) => w.trim()).filter(Boolean);
  const escape = (w: string) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const exclude = words.length ? new RegExp(`\\b(${words.map(escape).join("|")})\\b`, "i") : null;
  return (text) => user.test(text) && !(exclude && exclude.test(text));
}

/** The ready sources the automatic choice may use, in list order (the first one is the choice). */
export function autoCandidates(list: ListedStream[], choice: AutoChoice, states: TorboxStates): StreamEntry[] {
  if (choice.mode === "MANUAL") return [];
  const allowed = list.filter((x) => !choice.addons.length || choice.addons.includes(x.addonName));
  const match = choice.mode === "REGEX_MATCH" ? matcher(choice.regex) : () => true;
  if (!match) return [];
  return allowed
    .filter((x) => autoPlayable(x.stream, states))
    .filter((x) => match(`${x.addonName} ${x.stream.name ?? ""} ${x.stream.description ?? ""} ${x.stream.filename ?? ""}`))
    .map((x) => x.stream);
}

/** Sources to try after `failed` when "Tentar outra fonte se falhar" is on: the next ready ones. */
export function fallbackAfter(list: ListedStream[], failed: string, states: TorboxStates, max: number): StreamEntry[] {
  const at = list.findIndex((x) => x.stream.key === failed);
  return list
    .slice(at + 1)
    .map((x) => x.stream)
    .filter((s) => autoPlayable(s, states))
    .slice(0, Math.max(0, max));
}

// ---------------------------------------------------------------- filters of the sources panel
// Quality, type and extras read like the badges. Inside one group a source needs any of the
// chosen options; across groups it needs all of them.

export interface SourceFilter {
  id: string;
  label: string;
  group: "quality" | "type" | "extra";
  test(s: StreamEntry, states: TorboxStates): boolean;
}

const quality = (s: StreamEntry) => badgesOf(s)[0] ?? "";
const has = (s: StreamEntry, badge: string) => badgesOf(s).includes(badge);

export const SOURCE_FILTERS: SourceFilter[] = [
  { id: "4k", label: "4K", group: "quality", test: (s) => quality(s) === "4K" },
  { id: "1080p", label: "1080p", group: "quality", test: (s) => quality(s) === "1080p" || quality(s) === "1440p" },
  { id: "720p", label: "720p", group: "quality", test: (s) => quality(s) === "720p" },
  { id: "480p", label: "480p / SD", group: "quality", test: (s) => quality(s) === "SD" },
  { id: "unknown", label: "Sem qualidade informada", group: "quality", test: (s) => !["4K", "1440p", "1080p", "720p", "SD"].includes(quality(s)) },
  { id: "direct", label: "Link direto", group: "type", test: (s) => s.kind === "direct" || s.kind === "youTube" },
  { id: "debrid", label: "Debrid (Instant)", group: "type", test: (s, st) => st[s.key] === "cached" },
  { id: "p2p", label: "P2P", group: "type", test: (s, st) => s.kind === "torrent" && st[s.key] !== "cached" },
  { id: "hdr", label: "HDR", group: "extra", test: (s) => has(s, "HDR") || has(s, "HDR10+") },
  { id: "dv", label: "Dolby Vision", group: "extra", test: (s) => has(s, "Dolby Vision") },
  { id: "hevc", label: "HEVC", group: "extra", test: (s) => has(s, "HEVC") },
  { id: "remux", label: "REMUX", group: "extra", test: (s) => has(s, "REMUX") },
  { id: "atmos", label: "Atmos / 5.1", group: "extra", test: (s) => has(s, "Atmos") || has(s, "5.1") },
];

export function passesFilters(s: StreamEntry, states: TorboxStates, chosen: string[]): boolean {
  if (!chosen.length) return true;
  const active = SOURCE_FILTERS.filter((f) => chosen.includes(f.id));
  return (["quality", "type", "extra"] as const).every((group) => {
    const inGroup = active.filter((f) => f.group === group);
    return !inGroup.length || inGroup.some((f) => f.test(s, states));
  });
}

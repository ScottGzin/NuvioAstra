// What a source becomes when it is chosen: TorBox link (cached torrents and the addons' own TorBox
// sources), P2P (other torrents) or the addon's link. Also the badges read from its texts and the
// download request built from what the screen knows.

import { api, errorText } from "./api";
import { decimal, t } from "./i18n";
import { showToast } from "./toast";
import type { DownloadRequest, StreamEntry } from "./types";

export type TorboxStates = Record<string, "cached" | "notCached">;

/** The episode a source belongs to (picks the file inside a season pack). */
export interface SourceContext {
  season: number | null;
  episode: number | null;
}

/** Plays through TorBox when it has the source. `true`: ready to play as a link. */
export async function viaTorbox(stream: StreamEntry, states: TorboxStates, ctx: SourceContext): Promise<boolean> {
  if (!stream.torbox && states[stream.key] !== "cached") return false;
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
  return stream.kind === "direct" || stream.kind === "torrent" || (stream.kind === "debrid" && (stream.torbox || states[stream.key] === "cached"));
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

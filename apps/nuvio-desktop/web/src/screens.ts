// Screen preferences (Settings → Conteúdo): read once, kept in memory and refreshed when Settings
// saves them, so screens that stay mounted (Home) follow the change at once. Also the custom
// poster address of the original (`CustomPosterUrlResolver`).

import { useEffect, useState } from "react";
import { api, onDataEpoch } from "./api";
import type { MetaPreview, ScreenSettings } from "./types";

export const SCREEN_DEFAULTS: ScreenSettings = {
  detailsShowCast: true, detailsShowInfo: true, detailsBlurUnwatched: false, detailsEpisodeStyle: "list", detailsBackground: "normal",
  cwVisible: true, cwStyle: "card", cwEpisodeThumbnails: true, cwShowUnaired: true, cwBlurNextUp: false, cwResumePrompt: true,
  cwSort: "default", cwClick: "stream", hoverPreview: true, hoverDelayMs: 2000, posterPattern: "",
};

let current: ScreenSettings | null = null;
let loading: Promise<ScreenSettings> | null = null;
const listeners = new Set<(s: ScreenSettings) => void>();

function load(): Promise<ScreenSettings> {
  loading ??= api.settingsGet().then(
    (v) => (current = { ...SCREEN_DEFAULTS, ...v.settings.screens }),
    () => (current = SCREEN_DEFAULTS),
  );
  return loading;
}

/** Settings saved new screen preferences. */
export function screensChanged(next: ScreenSettings) {
  current = next;
  for (const l of listeners) l(next);
}

// Another profile: its own preferences.
onDataEpoch((reason) => {
  if (reason !== "profile") return;
  current = null;
  loading = null;
  void load().then((s) => listeners.forEach((l) => l(s)));
});

export function useScreens(): ScreenSettings {
  const [value, setValue] = useState<ScreenSettings>(current ?? SCREEN_DEFAULTS);
  useEffect(() => {
    listeners.add(setValue);
    if (current) setValue(current);
    else void load().then(setValue);
    return () => {
      listeners.delete(setValue);
    };
  }, []);
  return value;
}

export function screensNow(): ScreenSettings {
  return current ?? SCREEN_DEFAULTS;
}

// ---------------------------------------------------------------- custom poster address
// Port of `CustomPosterUrlResolver`: `{imdb_id}`, `{tmdb_id}`, `{tvdb_id}`, `{kitsu_id}`,
// `{anilist_id}`, `{mal_id}`, `{anidb_id}`, `{id}`, `{id_type}`, `{typed_id}`, `{type}`, `{shape}`;
// `{x?}` is optional, `{a|b}` takes the first available; a required placeholder without a value
// keeps the addon's poster. RPDB-family addresses fall back imdb → tmdb → tvdb.

interface Ids { id: string; imdb?: string; tmdb?: string; tvdb?: string; kitsu?: string; anilist?: string; mal?: string; anidb?: string }

function idsOf(metaId: string): Ids {
  const ids: Ids = { id: metaId };
  const [prefix, rest] = metaId.includes(":") ? [metaId.slice(0, metaId.indexOf(":")), metaId.slice(metaId.indexOf(":") + 1)] : ["", metaId];
  if (metaId.startsWith("tt")) ids.imdb = metaId.split(":")[0];
  else if (prefix === "tmdb") ids.tmdb = rest.split(":")[0];
  else if (prefix === "tvdb") ids.tvdb = rest.split(":")[0];
  else if (prefix === "kitsu") ids.kitsu = rest.split(":")[0];
  else if (prefix === "anilist") ids.anilist = rest.split(":")[0];
  else if (prefix === "mal") ids.mal = rest.split(":")[0];
  else if (prefix === "anidb") ids.anidb = rest.split(":")[0];
  return ids;
}

const RPDB = ["ratingposterdb.com", "aioratings.com", "top-posters.com", "btttr.cc"];

function fill(pattern: string, ids: Ids, type: string, shape: string): string | null {
  const [rawId, idType] = ids.id.startsWith("tt") ? [ids.id, "imdb"] : ids.id.includes(":") ? [ids.id.slice(ids.id.indexOf(":") + 1), ids.id.slice(0, ids.id.indexOf(":"))] : [ids.id, "unknown"];
  const typedId = idType === "imdb" ? rawId : idType === "tmdb" || idType === "tvdb" ? `${type === "movie" ? "movie" : "series"}-${rawId}` : rawId;
  const lookup = (key: string): string | undefined => {
    switch (key) {
      case "id": return ids.id;
      case "id_type": return idType;
      case "typed_id": return typedId;
      case "type": return type;
      case "shape": return shape;
      case "imdb_id": return ids.imdb ?? "";
      case "tmdb_id": return ids.tmdb ?? "";
      case "tvdb_id": return ids.tvdb ?? "";
      case "kitsu_id": return ids.kitsu ?? "";
      case "anilist_id": return ids.anilist ?? "";
      case "mal_id": return ids.mal ?? "";
      case "anidb_id": return ids.anidb ?? "";
      default: return undefined;
    }
  };
  let failed = false;
  const out = pattern.replace(/\{([a-z_|]+)(\?)?\}/g, (whole, body: string, optional?: string) => {
    if (failed) return whole;
    const keys = body.split("|");
    if (keys.some((k) => !k)) return whole;
    if (keys.length > 1) {
      const found = keys.map(lookup).find((v) => v);
      if (!found) failed = true;
      return found ?? "";
    }
    const value = lookup(keys[0]);
    if (value === undefined) return whole;
    if (!value && !optional) failed = true;
    return value;
  });
  return failed ? null : out;
}

/** The custom poster of an item, or `null` to keep the addon's. */
export function customPoster(pattern: string, item: Pick<MetaPreview, "id" | "contentType">, shape = "poster"): string | null {
  const raw = pattern.trim();
  if (!raw || !/^https?:\/\//i.test(raw)) return null;
  const decoded = raw.replace(/%7B/gi, "{").replace(/%7D/gi, "}").replace(/%7C/gi, "|");
  const ids = idsOf(item.id);
  const type = item.contentType;
  const direct = fill(decoded, ids, type, shape);
  if (direct !== null || !RPDB.some((d) => decoded.includes(d))) return direct;
  const prefix = type === "movie" ? "movie" : "series";
  const fallbacks: string[] = [];
  if (decoded.includes("{imdb_id}")) {
    if (ids.tmdb) fallbacks.push(decoded.replace("/imdb/", "/tmdb/").replace("{imdb_id}", `${prefix}-${ids.tmdb}`));
    if (ids.tvdb) fallbacks.push(decoded.replace("/imdb/", "/tvdb/").replace("{imdb_id}", `${prefix}-${ids.tvdb}`));
  } else if (decoded.includes("{tmdb_id}")) {
    if (ids.imdb) fallbacks.push(decoded.replace("/tmdb/", "/imdb/").replace(`${prefix}-{tmdb_id}`, ids.imdb).replace("{tmdb_id}", ids.imdb));
    if (ids.tvdb) fallbacks.push(decoded.replace("/tmdb/", "/tvdb/").replace("{tmdb_id}", ids.tvdb));
  } else if (decoded.includes("{tvdb_id}")) {
    if (ids.imdb) fallbacks.push(decoded.replace("/tvdb/", "/imdb/").replace(`${prefix}-{tvdb_id}`, ids.imdb).replace("{tvdb_id}", ids.imdb));
    if (ids.tmdb) fallbacks.push(decoded.replace("/tvdb/", "/tmdb/").replace("{tvdb_id}", ids.tmdb));
  }
  for (const fb of fallbacks) {
    const r = fill(fb, ids, type, shape);
    if (r !== null) return r;
  }
  return null;
}

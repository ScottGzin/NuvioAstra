// Sources of one title/episode. Addons are chips at the top (All, then one per addon, with a
// refresh button), and every source is a card with the addon's own text plus the badges of the
// rules imported in the original app (Settings → Streams → badges): the same regex filters and
// chips, read from its store. Direct and torrent (P2P) streams play in the app's player; the other
// YouTube sources are resolved by the in-app extractor and external links open in the browser.

import { useCallback, useEffect, useMemo, useState } from "react";
import type { CSSProperties } from "react";
import { api, errorText } from "../api";
import type { PlaybackItem, StreamBadgeFilter, StreamBadgeRules, StreamEntry, StreamGroup } from "../types";
import { t } from "../i18n";
import { message } from "../rustErrors";

interface Props {
  contentType: string;
  id: string;
  onPlay(stream: StreamEntry): void;
  /** When given, direct sources get a download button (`PlaybackItem` stays opaque). */
  downloadProgress?: PlaybackItem;
}

/** Text the badge rules are matched against, as `badgeMatchCandidates` in the original. */
function candidates(stream: StreamEntry): string[] {
  return [stream.filename, stream.name, stream.title, stream.description]
    .map((value) => value?.replace(/\s+/g, " ").trim())
    .filter((value): value is string => !!value);
}

/** Java/Kotlin patterns start with inline flags (`(?i)…`), which JavaScript does not accept;
 *  the leading flags become the RegExp flags. A pattern the browser still rejects is skipped. */
function compilePattern(pattern: string): RegExp | null {
  let source = pattern;
  let flags = "";
  const leading = source.match(/^\(\?([ims]+)\)/);
  if (leading) {
    flags = [...new Set(leading[1])].join("");
    source = source.slice(leading[0].length);
  }
  try {
    return new RegExp(source, flags);
  } catch {
    return null;
  }
}

/** `StreamBadgeMatcher`: the source's active filters that match one of the candidates, in order. */
function matchedBadges(stream: StreamEntry, filters: StreamBadgeFilter[]): StreamBadgeFilter[] {
  if (!filters.length) return [];
  const texts = candidates(stream);
  if (!texts.length) return [];
  const seen = new Set<string>();
  const matched: StreamBadgeFilter[] = [];
  for (const filter of filters) {
    const regex = compilePattern(filter.pattern);
    if (!regex) continue;
    if (!texts.some((text) => regex.test(text))) continue;
    const key = (filter.imageUrl || filter.name).toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    matched.push(filter);
  }
  return matched;
}

/** `#RRGGBB` or `#AARRGGBB` (the original stores ARGB) → a CSS color. */
function cssColor(value: string): string | undefined {
  const hex = value.trim().replace(/^#/, "");
  if (/^[0-9a-f]{6}$/i.test(hex)) return `#${hex}`;
  if (/^[0-9a-f]{8}$/i.test(hex)) {
    const [a, r, g, b] = [0, 2, 4, 6].map((i) => parseInt(hex.slice(i, i + 2), 16));
    return `rgba(${r}, ${g}, ${b}, ${(a / 255).toFixed(3)})`;
  }
  return undefined;
}

/** `StreamFileSizeBadge`: GiB with one decimal, or MiB below 1 GiB. */
function sizeLabel(bytes: number): string {
  const gib = bytes / (1024 * 1024 * 1024);
  if (gib >= 1) return `${Math.round(gib * 10) / 10} GB`;
  return `${Math.round(bytes / (1024 * 1024))} MB`;
}

function Badge({ filter }: { filter: StreamBadgeFilter }) {
  const [imageFailed, setImageFailed] = useState(false);
  const style: CSSProperties = {};
  if (filter.tagStyle === "filled" && filter.tagColor) style.background = cssColor(filter.tagColor);
  if (filter.borderColor) style.borderColor = cssColor(filter.borderColor);
  if (filter.textColor) style.color = cssColor(filter.textColor);
  const showImage = !!filter.imageUrl && !imageFailed;
  return (
    <span className="badge" style={style} title={filter.name} role="img" aria-label={filter.name}>
      {showImage ? (
        <img src={filter.imageUrl} alt="" loading="lazy" decoding="async" onError={() => setImageFailed(true)} />
      ) : (
        filter.name
      )}
    </span>
  );
}

export function StreamList({ contentType, id, onPlay, downloadProgress }: Props) {
  const [groups, setGroups] = useState<StreamGroup[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [rules, setRules] = useState<StreamBadgeRules | null>(null);

  const load = useCallback(
    async (refresh: boolean) => {
      if (refresh) setRefreshing(true);
      try {
        const next = await api.streams(contentType, id, refresh);
        setGroups(next);
        setError(null);
      } catch (e) {
        setError(errorText(e));
      } finally {
        setRefreshing(false);
      }
    },
    [contentType, id],
  );

  useEffect(() => {
    setGroups(null);
    setSelected(null);
    setError(null);
    void load(false);
  }, [load]);

  // Badge rules imported in the original app; empty rules simply mean no chips.
  useEffect(() => {
    let cancelled = false;
    api.streamBadges().then(
      (value) => !cancelled && setRules(value),
      () => {},
    );
    return () => {
      cancelled = true;
    };
  }, []);

  const addonGroups = useMemo(() => (groups ?? []).filter((group) => group.streams.length > 0), [groups]);
  // A chip of an addon that answered nothing in a refresh falls back to All.
  const effective = selected && addonGroups.some((g) => g.addonName === selected) ? selected : null;
  const shown = effective ? addonGroups.filter((g) => g.addonName === effective) : addonGroups;
  const failed = (groups ?? []).filter((group) => group.error && group.streams.length === 0);

  if (error) return <div className="row-error">{t("Falha ao buscar fontes: {error}", { error })}</div>;
  if (!groups) return <div className="muted">{t("Buscando fontes nos addons…")}</div>;
  if (!addonGroups.length && !failed.length) return <div className="muted">{t("Nenhum addon instalado fornece fontes para este título.")}</div>;

  return (
    <div className="streams">
      <div className="source-filters" role="group" aria-label={t("Addons com fontes")}>
        <button
          className={`chip refresh ${refreshing ? "spinning" : ""}`}
          aria-label={t("Atualizar fontes")}
          title={t("Atualizar fontes")}
          onClick={() => void load(true)}
          disabled={refreshing}
        >
          ⟳
        </button>
        <button className={`chip ${effective === null ? "active" : ""}`} onClick={() => setSelected(null)}>
          {t("Todos")}
        </button>
        {addonGroups.map((group) => (
          <button
            key={group.addonName}
            className={`chip ${effective === group.addonName ? "active" : ""}`}
            title={group.addonName}
            onClick={() => setSelected(group.addonName)}
          >
            {group.addonName}
          </button>
        ))}
      </div>

      {failed.map((group) => (
        <div key={group.addonName} className="stream-addon-error muted small">
          {group.addonName} · {t("erro: {error}", { error: message(group.error ?? "") })}
        </div>
      ))}

      {shown.map((group) =>
        group.streams.map((stream) => {
          const playable = stream.kind === "direct" || stream.kind === "torrent" || stream.kind === "debrid" || stream.kind === "youTube";
          const external = stream.kind === "external" && !!stream.externalUrl;
          const badges = rules ? matchedBadges(stream, rules.filters) : [];
          const showSize = !!rules?.showFileSizeBadges && !!stream.videoSize;
          return (
            <div key={stream.key} className="stream" data-kind={stream.kind}>
              <button
                className="stream-main"
                disabled={!playable}
                onClick={() => playable && onPlay(stream)}
              >
                <span className="stream-name">
                  {stream.name || t("Fonte")}
                  {stream.kind === "torrent" && !stream.debridProvider && <span className="stream-tag">P2P</span>}
                  {stream.debridProvider && <span className="stream-tag debrid-tag" title={t("Resolvido por debrid")}>{stream.debridProvider}</span>}
                  {stream.kind === "youTube" && <span className="stream-tag">YouTube</span>}
                  {stream.kind === "external" && <span className="stream-tag">↗</span>}
                </span>
                {stream.description && <span className="stream-desc">{stream.description}</span>}
                {(badges.length > 0 || showSize) && (
                  <span className="stream-badges">
                    {showSize && <span className="badge badge-size">{t("SIZE {size}", { size: sizeLabel(stream.videoSize!) })}</span>}
                    {badges.map((filter) => (
                      <Badge key={`${filter.imageUrl || filter.name}`} filter={filter} />
                    ))}
                  </span>
                )}
              </button>
              {downloadProgress && stream.kind === "direct" && (
                <button
                  className="stream-action"
                  title={t("Baixar fonte")}
                  aria-label={t("Baixar fonte")}
                  data-download={stream.key}
                  onClick={() => void api.downloadFromStream(stream.key, downloadProgress).catch(() => {})}
                >↓</button>
              )}
              {external && (
                <button
                  className="stream-action"
                  title={t("Abrir link externo")}
                  aria-label={t("Abrir link externo")}
                  onClick={() => void api.streamOpenExternal(stream.key).catch(() => {})}
                >↗</button>
              )}
              {stream.kind === "external" && !external && <span className="stream-pending">{t("Link externo inválido")}</span>}
            </div>
          );
        }),
      )}
    </div>
  );
}

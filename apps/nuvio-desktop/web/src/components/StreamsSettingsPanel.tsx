// Streams settings: imported badge rules from the original plus Astra-side overrides.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { Row, Toggle } from "./SettingControls";
import type { StreamsSettings } from "../types";
import { t } from "../i18n";

export function StreamsSettingsPanel() {
  const [view, setView] = useState<{ settings: StreamsSettings; importedFilters: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [url, setUrl] = useState("");

  const reload = () => api.streamsSettings().then(setView, (e) => setError(errorText(e)));
  useEffect(() => { reload(); }, []);

  const save = async (change: Partial<StreamsSettings>) => {
    if (!view) return;
    setError(null);
    const next = { ...view.settings, ...change };
    setView({ ...view, settings: next });
    try { setView(await api.streamsSettingsSet(next)); }
    catch (e) { setError(errorText(e)); }
  };

  if (!view) return error ? <div className="row-error" role="alert">{error}</div> : <div className="muted">{t("Carregando…")}</div>;
  const s = view.settings;
  return (
    <div className="settings-group">
      <Row title={t("Size badges")} description={t("Show file size badges in stream results and player source panels.")}>
        <Toggle label={t("Size badges")} checked={s.showSizeBadges ?? true} onChange={(v) => save({ showSizeBadges: v })} />
      </Row>
      <Row title={t("Badge position")}>
        <select aria-label={t("Badge position")} value={s.badgePlacement} onChange={(e) => save({ badgePlacement: e.target.value })}>
          <option value="bottom">{t("Bottom")}</option>
          <option value="top">{t("Top")}</option>
        </select>
      </Row>
      <Row title={t("Fusion badge URLs")} description={t("{count} active Fusion badges", { count: view.importedFilters })}>
        <span className="tmdb-key">
          <input aria-label={t("Fusion badge URLs")} placeholder="https://…/badges.json" value={url} onChange={(e) => setUrl(e.target.value)} />
          <button className="ghost" disabled={!url.trim() || s.badgeUrls.length >= 3} onClick={() => {
            void save({ badgeUrls: [...s.badgeUrls, url.trim()] });
            setUrl("");
          }}>{t("Add")}</button>
        </span>
      </Row>
      {s.badgeUrls.map((item, i) => (
        <Row key={i} title={item} description={`${i + 1}/3`}>
          <button className="ghost" onClick={() => save({ badgeUrls: s.badgeUrls.filter((_, n) => n !== i) })}>{t("Remove")}</button>
        </Row>
      ))}
      <Row title={t("Show in player")} description={t("Show badges for the video and audio that is playing under the player title.")}>
        <Toggle label={t("Show in player")} checked={s.showInPlayer} onChange={(v) => save({ showInPlayer: v })} />
      </Row>
      <Row title={t("Player badges")}>
        <input aria-label={t("Player badges")} value={s.playerBadgeSource} placeholder="https://…/badges-icon-light.json" onChange={(e) => save({ playerBadgeSource: e.target.value })} />
      </Row>
      <div className="settings-subhead">{t("Display")}</div>
      <Row title={t("Background")}>
        <select aria-label={t("Background")} value={s.backgroundMode} onChange={(e) => save({ backgroundMode: e.target.value })}>
          <option value="normal">{t("Normal")}</option>
          <option value="cinematic">{t("Cinematic")}</option>
        </select>
      </Row>
      <Row title={t("Addon logo")} description={t("Show addon logo and name next to stream sources.")}>
        <Toggle label={t("Addon logo")} checked={s.showAddonLogo} onChange={(v) => save({ showAddonLogo: v })} />
      </Row>
      {error && <div className="row-error" role="alert">{error}</div>}
    </div>
  );
}

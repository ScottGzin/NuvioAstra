// Poster Card Style — the original's PosterCardStyleRepository presets. Applied to every card
// through CSS variables, so the live preview matches the real grid.

import { useEffect } from "react";
import { Row, Toggle } from "./SettingControls";
import type { CardStyle, PlayerSettings } from "../types";
import { reloadPosterPreferences } from "../posterPreferences";
import { t } from "../i18n";

const WIDTHS: [number, string][] = [
  [104, "Compact"], [112, "Dense"], [120, "Standard"], [126, "Balanced"], [134, "Comfort"], [140, "Large"],
];
const RADII: [number, string][] = [
  [0, "Sharp"], [4, "Subtle"], [8, "Classic"], [12, "Rounded"], [16, "Pill"],
];

function apply(cards: CardStyle) {
  const root = document.documentElement.style;
  root.setProperty("--card-width", `${cards.width}px`);
  root.setProperty("--card-radius", `${cards.radius}px`);
}

const DEFAULT_CARDS: CardStyle = {
  width: 126, radius: 12, landscape: false, hideLabels: false,
  hoverPreviewEnabled: true, hoverPreviewDelayMs: 2000,
  hoverPreviewTrailerEnabled: false, hoverPreviewTrailerSound: false, hoverPreviewTrailerStart: 0,
  customPosterUrlPattern: "", customPosterScreens: ["home", "continue_watching", "collections", "library", "search", "details"],
};
const SCREENS = ["home", "continue_watching", "collections", "library", "search", "details"];

export function PosterCardStyle({ value, onChange }: { value: PlayerSettings; onChange(change: (p: PlayerSettings) => PlayerSettings): void }) {
  const cards: CardStyle = { ...DEFAULT_CARDS, ...(value.cards ?? {}) };
  useEffect(() => {
    const timer = window.setTimeout(() => void reloadPosterPreferences(), 450);
    return () => window.clearTimeout(timer);
  }, [cards.hoverPreviewEnabled, cards.hoverPreviewDelayMs, cards.hoverPreviewTrailerEnabled, cards.hoverPreviewTrailerSound, cards.hoverPreviewTrailerStart]);
  const patch = (change: Partial<CardStyle>) => {
    const next = { ...cards, ...change };
    apply(next);
    onChange((p) => ({ ...p, cards: next }));
  };
  return (
    <div className="settings-group">
      <div className="setting-title">{t("Poster Card Style")}</div>
      <div className="settings-subhead">{t("Live Preview")}</div>
      <div className="card-style-preview">
        <div className="card-art" style={{ width: cards.width, borderRadius: cards.radius }} />
        <div className="muted small">
          {t("Width: {width}dp", { width: cards.width })} · {t("Corner radius: {radius}dp", { radius: cards.radius })} · {t("Height: {height}dp", { height: Math.round(cards.width * 1.5) })}
        </div>
      </div>
      <Row title={t("Width (Balanced)")}>
        <div className="chip-select">
          {WIDTHS.map(([width, label]) => (
            <button key={width} className={`chip ${cards.width === width ? "active" : ""}`} onClick={() => patch({ width })}>{t(label)}</button>
          ))}
        </div>
      </Row>
      <Row title={t("Corner Radius (Rounded)")}>
        <div className="chip-select">
          {RADII.map(([radius, label]) => (
            <button key={radius} className={`chip ${cards.radius === radius ? "active" : ""}`} onClick={() => patch({ radius })}>{t(label)}</button>
          ))}
        </div>
      </Row>
      <Row title={t("Landscape Posters")}>
        <Toggle label={t("Landscape Posters")} checked={cards.landscape} onChange={(v) => patch({ landscape: v })} />
      </Row>
      <Row title={t("Hide labels")}>
        <Toggle label={t("Hide labels")} checked={cards.hideLabels} onChange={(v) => patch({ hideLabels: v })} />
      </Row>
      <div className="settings-subhead">{t("Prévia ao passar o mouse")}</div>
      <Row title={t("Mostrar prévia ao passar o mouse")}>
        <Toggle label={t("Prévia ao passar o mouse")} checked={cards.hoverPreviewEnabled} onChange={(v) => patch({ hoverPreviewEnabled: v })} />
      </Row>
      {cards.hoverPreviewEnabled && (
        <>
          <Row title={t("Atraso da prévia")}>
            <input type="range" min={500} max={5000} step={500} value={cards.hoverPreviewDelayMs} aria-label={t("Atraso da prévia")} onChange={(e) => patch({ hoverPreviewDelayMs: Number(e.target.value) })} />
            <output>{cards.hoverPreviewDelayMs} ms</output>
          </Row>
          <Row title={t("Prévia com trailer")} description={t("O trailer é resolvido apenas depois de 1 segundo com a prévia aberta e começa sem som.")}>
            <Toggle label={t("Prévia com trailer")} checked={cards.hoverPreviewTrailerEnabled} onChange={(v) => patch({ hoverPreviewTrailerEnabled: v })} />
          </Row>
          {cards.hoverPreviewTrailerEnabled && (
            <>
              <Row title={t("Som do trailer")}>
                <Toggle label={t("Som do trailer")} checked={cards.hoverPreviewTrailerSound} onChange={(v) => patch({ hoverPreviewTrailerSound: v })} />
              </Row>
              <Row title={t("Começar em")}>
                <input type="range" min={0} max={5} step={1} value={cards.hoverPreviewTrailerStart} aria-label={t("Começar em")} onChange={(e) => patch({ hoverPreviewTrailerStart: Number(e.target.value) })} />
                <output>{cards.hoverPreviewTrailerStart}s</output>
              </Row>
            </>
          )}
        </>
      )}
      <div className="settings-subhead">{t("Fonte de pôster personalizada")}</div>
      <Row title={t("Padrão de URL")} description={t("Use {id}, {id_type}, {typed_id}, {type}, {shape} ou {imdb_id}, {tmdb_id}, {tvdb_id}, {mal_id}, {kitsu_id}, {anilist_id}, {anidb_id}. Um {token?} ausente vira texto vazio.")}>
        <textarea rows={3} value={cards.customPosterUrlPattern} aria-label={t("Padrão de URL")} onChange={(e) => patch({ customPosterUrlPattern: e.target.value })} />
      </Row>
      <Row title={t("Aplicar em")}>
        <div className="chip-select">
          {SCREENS.map((screen) => {
            const active = cards.customPosterScreens.includes(screen);
            return (
              <button key={screen} className={`chip ${active ? "active" : ""}`} onClick={() => patch({ customPosterScreens: active ? cards.customPosterScreens.filter((s) => s !== screen) : [...cards.customPosterScreens, screen] })}>
                {t(screen)}
              </button>
            );
          })}
        </div>
      </Row>
      <button className="ghost" onClick={() => patch({ width: 126, radius: 12, landscape: false, hideLabels: false, hoverPreviewEnabled: true, hoverPreviewDelayMs: 2000, hoverPreviewTrailerEnabled: false, hoverPreviewTrailerSound: false, hoverPreviewTrailerStart: 0, customPosterUrlPattern: "", customPosterScreens: [...SCREENS] })}>{t("Reset to Default")}</button>
    </div>
  );
}

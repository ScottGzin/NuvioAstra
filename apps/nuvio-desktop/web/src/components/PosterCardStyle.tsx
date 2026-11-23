// Poster Card Style — the original's PosterCardStyleRepository presets. Applied to every card
// through CSS variables, so the live preview matches the real grid.

import { Row, Toggle } from "./SettingControls";
import type { CardStyle, PlayerSettings } from "../types";
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

export function PosterCardStyle({ value, onChange }: { value: PlayerSettings; onChange(change: (p: PlayerSettings) => PlayerSettings): void }) {
  const cards: CardStyle = value.cards ?? { width: 126, radius: 12, landscape: false, hideLabels: false };
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
      <button className="ghost" onClick={() => patch({ width: 126, radius: 12, landscape: false, hideLabels: false })}>{t("Reset to Default")}</button>
    </div>
  );
}

// Settings → Reprodução: when something counts as watched, skip buttons and the next-episode card
// (the original's "Pular intro" and t("Próximo episódio") settings, plus the watched percentage).

import type { PlayerSettings } from "../types";
import { Row, Toggle } from "./SettingControls";
import { decimal, t } from "../i18n";

const AUTO_KINDS: [string, string][] = [
  ["intro", "Introdução"],
  ["recap", "Recapitulação"],
  ["outro", "Encerramento"],
  ["movie-credits", "Créditos de filmes"],
];

function Range({ label, value, min, max, step, unit, onChange }: { label: string; value: number; min: number; max: number; step: number; unit: string; onChange(v: number): void }) {
  return (
    <span className="slider">
      <input type="range" min={min} max={max} step={step} value={value} aria-label={label} onChange={(e) => onChange(Number(e.target.value))} />
      <output>{decimal(value)}{unit}</output>
    </span>
  );
}

export function WatchingSettings({ value: p, onChange }: { value: PlayerSettings; onChange(change: (p: PlayerSettings) => PlayerSettings): void }) {
  const percentMode = p.nextEpisodeThresholdMode !== "MINUTES_BEFORE_END";
  return (
    <>
      <h3 className="settings-subhead">{t("Assistido e próximo episódio")}</h3>
      <Row title={t("Considerar assistido a partir de")} description={t("Antes disso o episódio ou filme fica em Continuar assistindo. Depois, a série segue com o próximo episódio. Vale só neste computador: os outros apps Nuvio usam sempre 90%.")}>
        <Range label={t("Considerar assistido a partir de")} value={p.watchedPercent} min={90} max={100} step={1} unit="%" onChange={(v) => onChange((x) => ({ ...x, watchedPercent: v }))} />
      </Row>
      <Row title={t("Mostrar o próximo episódio")} description={t("Quando o cartão aparece no player. Com o tempo do encerramento conhecido, ele aparece junto com os créditos finais.")}>
        <select aria-label={t("Mostrar o próximo episódio")} value={p.nextEpisodeThresholdMode} onChange={(e) => onChange((x) => ({ ...x, nextEpisodeThresholdMode: e.target.value as PlayerSettings["nextEpisodeThresholdMode"] }))}>
          <option value="PERCENTAGE">{t("Por porcentagem")}</option>
          <option value="MINUTES_BEFORE_END">{t("Por minutos antes do fim")}</option>
        </select>
      </Row>
      {percentMode ? (
        <Row title={t("Porcentagem do episódio")}>
          <Range label={t("Porcentagem do episódio")} value={p.nextEpisodeThresholdPercent} min={97} max={100} step={0.5} unit="%" onChange={(v) => onChange((x) => ({ ...x, nextEpisodeThresholdPercent: v }))} />
        </Row>
      ) : (
        <Row title={t("Minutos antes do fim")}>
          <Range label={t("Minutos antes do fim")} value={p.nextEpisodeThresholdMinutes} min={0} max={3.5} step={0.5} unit=" min" onChange={(v) => onChange((x) => ({ ...x, nextEpisodeThresholdMinutes: v }))} />
        </Row>
      )}

      <h3 className="settings-subhead">{t("Pular introdução e créditos")}</h3>
      <Row title={t("Botões de pular")} description={t("Os tempos vêm dos capítulos do vídeo, do IntroDB e, para animes, do AniSkip. Para isso o app envia a esses serviços o código público do título e o número do episódio.")}>
        <Toggle label={t("Botões de pular")} checked={p.skipSegmentsEnabled} onChange={(v) => onChange((x) => ({ ...x, skipSegmentsEnabled: v }))} />
      </Row>
      {p.skipSegmentsEnabled &&
        AUTO_KINDS.map(([kind, name]) => (
          <Row key={kind} title={t("Pular automaticamente: {name}", { name: t(name) })}>
            <Toggle
              label={t("Pular automaticamente: {name}", { name: t(name) })}
              checked={p.autoSkipSegments.includes(kind)}
              onChange={(v) => onChange((x) => ({ ...x, autoSkipSegments: [...x.autoSkipSegments.filter((k) => k !== kind), ...(v ? [kind] : [])].sort() }))}
            />
          </Row>
        ))}
    </>
  );
}

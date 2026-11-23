// Audio preferences from the original's PlayerSettingsRepository / SubtitleAudioModels. The delay
// is applied in the player (SetAudioDelay); gain and downmix are stored here and applied on the
// next playback session, like the original.

import { Row, Toggle } from "./SettingControls";
import type { PlayerSettings } from "../types";
import { decimal, t } from "../i18n";

const CHANNELS = ["2.0", "2.1", "3.0", "3.1", "4.0", "4.1", "5.0", "5.1", "7.0", "7.1"];

function Range({ label, value, min, max, step, unit, onChange }: { label: string; value: number; min: number; max: number; step: number; unit: string; onChange(v: number): void }) {
  return (
    <span className="slider">
      <input type="range" min={min} max={max} step={step} value={value} aria-label={label} onChange={(e) => onChange(Number(e.target.value))} />
      <output>{decimal(value)}{unit}</output>
    </span>
  );
}

export function AudioSettings({ value: p, onChange }: { value: PlayerSettings; onChange(change: (p: PlayerSettings) => PlayerSettings): void }) {
  const patch = (change: Partial<PlayerSettings>) => onChange((x) => ({ ...x, ...change }));
  return (
    <>
      <h3 className="settings-subhead">{t("Áudio")}</h3>
      <Row title={t("Amplificação de áudio")} description={t("Ganho em dB aplicado ao áudio; 0 mantém o volume original.")}>
        <Range label={t("Amplificação de áudio")} value={p.audioAmplificationDb} min={0} max={10} step={1} unit=" dB" onChange={(v) => patch({ audioAmplificationDb: v })} />
      </Row>
      <Row title={t("Manter amplificação entre sessões")} description={t("Por padrão cada reprodução começa sem ganho extra.")}>
        <Toggle label={t("Manter amplificação entre sessões")} checked={p.persistAudioAmplification} onChange={(v) => patch({ persistAudioAmplification: v })} />
      </Row>
      <Row title={t("Memorizar atraso por dispositivo")} description={t("Guarda o atraso de áudio ajustado no player para cada saída.")}>
        <Toggle label={t("Memorizar atraso por dispositivo")} checked={p.rememberAudioDelayPerDevice} onChange={(v) => patch({ rememberAudioDelayPerDevice: v })} />
      </Row>
      <Row title={t("Downmix de áudio")} description={t("Reduz os canais para o número escolhido; requer decodificação por software.")}>
        <Toggle label={t("Downmix de áudio")} checked={p.downmixEnabled} onChange={(v) => patch({ downmixEnabled: v })} />
      </Row>
      {p.downmixEnabled && (
        <>
          <Row title={t("Canais de saída")}>
            <select aria-label={t("Canais de saída")} value={p.audioOutputChannels} onChange={(e) => patch({ audioOutputChannels: e.target.value })}>
              {CHANNELS.map((channels) => <option key={channels} value={channels}>{channels}</option>)}
            </select>
          </Row>
          <Row title={t("Nível do canal central")}>
            <Range label={t("Nível do canal central")} value={p.centerMixLevelDb} min={-10} max={30} step={1} unit=" dB" onChange={(v) => patch({ centerMixLevelDb: v })} />
          </Row>
          <Row title={t("Manter áudio original no downmix")}>
            <Toggle label={t("Manter áudio original no downmix")} checked={p.maintainOriginalAudioOnDownmix} onChange={(v) => patch({ maintainOriginalAudioOnDownmix: v })} />
          </Row>
        </>
      )}
    </>
  );
}

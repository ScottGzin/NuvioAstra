// Buffer and disk-cache settings from the original's PlaybackBufferSettings / VodCacheSizing.

import { Row, Toggle } from "./SettingControls";
import type { PlayerSettings } from "../types";
import { t } from "../i18n";

interface Props {
  value: PlayerSettings;
  onChange(change: (p: PlayerSettings) => PlayerSettings): void;
}

export function BufferSettings({ value: p, onChange }: Props) {
  const patch = (change: Partial<PlayerSettings>) => onChange((x) => ({ ...x, ...change }));
  const seconds = (key: keyof PlayerSettings, label: string, min: number, max: number) => (
    <Row title={t(label)}>
      <input
        type="number"
        aria-label={t(label)}
        min={min}
        max={max}
        value={Math.round((p[key] as number) / 1000)}
        onChange={(e) => patch({ [key]: Math.max(min, Math.min(max, Number(e.target.value))) * 1000 } as Partial<PlayerSettings>)}
      />
    </Row>
  );
  return (
    <>
      <h3 className="settings-subhead">{t("Buffer e cache")}</h3>
      <Row title={t("Ajustes de buffer")} description={t("Valores personalizados para streams HTTP.")}>
        <Toggle label={t("Ajustes de buffer")} checked={p.bufferEnabled} onChange={(v) => patch({ bufferEnabled: v })} />
      </Row>
      {p.bufferEnabled && (
        <>
          {seconds("minBufferMs", "Buffer mínimo (s)", 5, 120)}
          {seconds("maxBufferMs", "Buffer máximo (s)", 5, 1200)}
          {seconds("bufferForPlaybackMs", "Iniciar após (s)", 1, 60)}
          {seconds("bufferForPlaybackAfterRebufferMs", "Retomar após (s)", 1, 120)}
          {seconds("backBufferDurationMs", "Buffer retroativo (s)", 0, 120)}
          <Row title={t("Tamanho alvo do buffer (MB)")}>
            <input type="number" aria-label={t("Tamanho alvo do buffer (MB)")} min={50} max={65536} value={p.targetBufferSizeMb} onChange={(e) => patch({ targetBufferSizeMb: Number(e.target.value) })} />
          </Row>
        </>
      )}
      <Row title={t("Cache em disco")} description={t("Guarda parte do vídeo em disco durante a reprodução.")}>
        <Toggle label={t("Cache em disco")} checked={p.vodCacheEnabled} onChange={(v) => patch({ vodCacheEnabled: v })} />
      </Row>
      {p.vodCacheEnabled && (
        <>
          <Row title={t("Tamanho da cache")}>
            <select aria-label={t("Tamanho da cache")} value={p.vodCacheSizeMode} onChange={(e) => patch({ vodCacheSizeMode: e.target.value as PlayerSettings["vodCacheSizeMode"] })}>
              <option value="auto">{t("Automático")}</option>
              <option value="manual">{t("Manual")}</option>
            </select>
          </Row>
          {p.vodCacheSizeMode === "manual" && (
            <Row title={t("Tamanho da cache (MB)")}>
              <input type="number" aria-label={t("Tamanho da cache (MB)")} min={100} max={65536} value={p.vodCacheSizeMb} onChange={(e) => patch({ vodCacheSizeMb: Number(e.target.value) })} />
            </Row>
          )}
        </>
      )}
    </>
  );
}

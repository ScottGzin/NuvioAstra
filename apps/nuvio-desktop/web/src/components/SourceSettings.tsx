// Sources: automatic playback selection, regex, fallback and reusing the last link. The fields and
// defaults are the original's PlayerSettingsUiState; only the storage is Astra's profile settings.

import { Row, Toggle } from "./SettingControls";
import type { PlayerSettings } from "../types";
import { t } from "../i18n";

const REUSE_HOURS = [1, 2, 3, 6, 12, 24, 48, 72, 168];
const TIMEOUTS = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 15, 20, 25, 30];

interface Props {
  value: PlayerSettings;
  addons: string[];
  onChange(change: (p: PlayerSettings) => PlayerSettings): void;
}

export function SourceSettings({ value: p, addons, onChange }: Props) {
  const patch = (change: Partial<PlayerSettings>) => onChange((x) => ({ ...x, ...change }));
  const toggleAddon = (name: string) => {
    const current = p.autoPlaySelectedAddons;
    patch({ autoPlaySelectedAddons: current.includes(name) ? current.filter((a) => a !== name) : [...current, name] });
  };
  return (
    <>
      <h3 className="settings-subhead">{t("Seleção automática de fonte")}</h3>
      <Row title={t("Modo de reprodução automática")} description={t("Escolhe uma fonte ao abrir um título, sem precisar selecionar.")}>
        <select aria-label={t("Modo de reprodução automática")} value={p.autoPlayMode} onChange={(e) => patch({ autoPlayMode: e.target.value as PlayerSettings["autoPlayMode"] })}>
          <option value="manual">{t("Manual")}</option>
          <option value="firstStream">{t("Primeira fonte")}</option>
          <option value="regexMatch">{t("Correspondência por regex")}</option>
        </select>
      </Row>
      {p.autoPlayMode !== "manual" && (
        <>
          <Row title={t("Fontes permitidas")} description={t("Limita a seleção automática a addons instalados ou a plugins ativados.")}>
            <select aria-label={t("Fontes permitidas")} value={p.autoPlaySource} onChange={(e) => patch({ autoPlaySource: e.target.value as PlayerSettings["autoPlaySource"] })}>
              <option value="allSources">{t("Todas as fontes")}</option>
              <option value="installedAddonsOnly">{t("Somente addons instalados")}</option>
              <option value="enabledPluginsOnly">{t("Somente plugins ativados")}</option>
            </select>
          </Row>
          {addons.length > 0 && (
            <Row title={t("Addons permitidos")} description={t("Nenhum selecionado = todos.")}>
              <div className="chip-select">
                {addons.map((name) => (
                  <button key={name} className={`chip ${p.autoPlaySelectedAddons.includes(name) ? "active" : ""}`} onClick={() => toggleAddon(name)}>{name}</button>
                ))}
              </div>
            </Row>
          )}
          {p.autoPlayMode === "regexMatch" && (
            <>
              <Row title={t("Expressão regular")} description={t("Corresponde ao nome do addon, fonte, descrição e URL. Um trecho (?!palavra|outra) exclui resultados.")}>
                <input aria-label={t("Expressão regular")} className="regex-input" value={p.autoPlayRegex} placeholder="1080p (?!CAM)" onChange={(e) => patch({ autoPlayRegex: e.target.value })} />
              </Row>
              <Row title={t("Tempo limite de seleção")} description={t("Segundos esperando uma fonte pronta; 0 significa sem espera.")}>
                <select aria-label={t("Tempo limite de seleção")} value={p.autoPlayTimeoutSeconds} onChange={(e) => patch({ autoPlayTimeoutSeconds: Number(e.target.value) })}>
                  {TIMEOUTS.map((seconds) => <option key={seconds} value={seconds}>{seconds === 0 ? t("Imediato") : `${seconds} s`}</option>)}
                </select>
              </Row>
            </>
          )}
        </>
      )}

      <h3 className="settings-subhead">{t("Se fallback e reutilização")}</h3>
      <Row title={t("Tentar outra fonte se falhar")} description={t("Depois de uma falha na inicialização, tenta as próximas fontes elegíveis.")}>
        <Toggle label={t("Tentar outra fonte se falhar")} checked={p.autoPlayFallbackOnFailure} onChange={(v) => patch({ autoPlayFallbackOnFailure: v })} />
      </Row>
      {p.autoPlayFallbackOnFailure && (
        <>
          <Row title={t("Máximo de fontes")}>
            <select aria-label={t("Máximo de fontes")} value={p.autoPlayFallbackMaxSources} onChange={(e) => patch({ autoPlayFallbackMaxSources: Number(e.target.value) })}>
              {[1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map((n) => <option key={n} value={n}>{n}</option>)}
            </select>
          </Row>
          <Row title={t("Tempo limite por fonte")} description={t("Segundos até considerar que a fonte falhou.")}>
            <select aria-label={t("Tempo limite por fonte")} value={p.autoPlayFallbackStartupTimeoutSeconds} onChange={(e) => patch({ autoPlayFallbackStartupTimeoutSeconds: Number(e.target.value) })}>
              {[5, 10, 15, 20, 25, 30, 35, 40, 45].map((n) => <option key={n} value={n}>{n} s</option>)}
            </select>
          </Row>
        </>
      )}
      <Row title={t("Reutilizar último link")} description={t("Usa novamente o link assistido por último enquanto ele estiver dentro do prazo.")}>
        <Toggle label={t("Reutilizar último link")} checked={p.reuseLastLinkEnabled} onChange={(v) => patch({ reuseLastLinkEnabled: v })} />
      </Row>
      {p.reuseLastLinkEnabled && (
        <Row title={t("Validade do link")}>
          <select aria-label={t("Validade do link")} value={p.reuseLastLinkCacheHours} onChange={(e) => patch({ reuseLastLinkCacheHours: Number(e.target.value) })}>
            {REUSE_HOURS.map((hours) => <option key={hours} value={hours}>{hours} h</option>)}
          </select>
        </Row>
      )}
    </>
  );
}

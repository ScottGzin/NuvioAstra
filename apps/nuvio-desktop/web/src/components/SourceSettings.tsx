// Settings → Fontes: the original's automatic source selection (first source or regex, allowed
// addons, waiting time), trying the next sources when one fails, reusing the last link, and the
// badges on the source cards. Kept on this computer (`ProfileSettings.sources`).

import { useEffect, useState } from "react";
import { api } from "../api";
import { Row, Toggle } from "./SettingControls";
import type { SourceSettings } from "../types";
import { t } from "../i18n";

const TIMEOUTS = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 15, 20, 25, 30];
const REUSE_HOURS = [1, 2, 3, 6, 12, 24, 48, 72, 168];

interface Props {
  value: SourceSettings;
  onChange(change: (s: SourceSettings) => SourceSettings): void;
}

export function SourcesSection({ value: s, onChange }: Props) {
  const [addons, setAddons] = useState<string[]>([]);
  useEffect(() => {
    api.addonsList().then((v) => setAddons(v.addons.filter((a) => a.enabled && a.resources.includes("stream")).map((a) => a.name)), () => {});
  }, []);
  const patch = (change: Partial<SourceSettings>) => onChange((x) => ({ ...x, ...change }));
  const toggleAddon = (name: string) =>
    onChange((x) => ({ ...x, autoPlaySelectedAddons: x.autoPlaySelectedAddons.includes(name) ? x.autoPlaySelectedAddons.filter((a) => a !== name) : [...x.autoPlaySelectedAddons, name] }));
  let regexError = false;
  if (s.autoPlayMode === "REGEX_MATCH" && s.autoPlayRegex.trim()) {
    try {
      new RegExp(s.autoPlayRegex, "i");
    } catch {
      regexError = true;
    }
  }

  return (
    <div className="settings-group" data-testid="sources-settings">
      <h3 className="settings-subhead">{t("Seleção automática")}</h3>
      <Row title={t("Escolher a fonte sozinho")} description={t("Ao abrir um filme ou episódio, a fonte é escolhida e começa a tocar sem você clicar. A lista continua aberta para trocar.")}>
        <select aria-label={t("Escolher a fonte sozinho")} value={s.autoPlayMode} onChange={(e) => patch({ autoPlayMode: e.target.value as SourceSettings["autoPlayMode"] })} data-setting="autoPlayMode">
          <option value="MANUAL">{t("Não (manual)")}</option>
          <option value="FIRST_STREAM">{t("Primeira fonte pronta")}</option>
          <option value="REGEX_MATCH">{t("Primeira que combina com uma regex")}</option>
        </select>
      </Row>
      {s.autoPlayMode === "REGEX_MATCH" && (
        <Row
          title={t("Expressão regular")}
          description={regexError ? t("Expressão inválida: nenhuma fonte será escolhida.") : t("Procura no nome do addon, da fonte, na descrição e no arquivo, sem diferenciar maiúsculas. Ex.: 1080p.*(?!.*(CAM|TS)) evita CAM e TS.")}
        >
          <input className={`regex-input ${regexError ? "invalid" : ""}`} aria-label={t("Expressão regular")} spellCheck={false} placeholder="2160p|1080p" value={s.autoPlayRegex} onChange={(e) => patch({ autoPlayRegex: e.target.value })} data-setting="autoPlayRegex" />
        </Row>
      )}
      {s.autoPlayMode !== "MANUAL" && (
        <>
          <Row title={t("Tempo de espera")} description={t("Quanto esperar pelos addons antes de escolher entre os que já responderam. 0 espera todos.")}>
            <select aria-label={t("Tempo de espera")} value={s.autoPlayTimeoutSeconds} onChange={(e) => patch({ autoPlayTimeoutSeconds: Number(e.target.value) })}>
              {TIMEOUTS.map((n) => <option key={n} value={n}>{n === 0 ? t("Esperar todos") : `${n} s`}</option>)}
            </select>
          </Row>
          {addons.length > 1 && (
            <Row title={t("Addons usados na escolha")} description={t("Nenhum marcado = todos.")}>
              <div className="chips" data-testid="auto-addons">
                {addons.map((name) => (
                  <button key={name} className={`chip ${s.autoPlaySelectedAddons.includes(name) ? "active" : ""}`} onClick={() => toggleAddon(name)}>{name}</button>
                ))}
              </div>
            </Row>
          )}
          <p className="muted small pad-top">{t("A escolha usa fontes que abrem direto: links, YouTube e torrents de um serviço debrid. Torrents só por P2P ficam para você escolher.")}</p>
        </>
      )}

      <h3 className="settings-subhead">{t("Se a fonte falhar")}</h3>
      <Row title={t("Tentar outra fonte se falhar")} description={t("Se a fonte não começar a tocar, as próximas da lista são tentadas sozinhas.")}>
        <Toggle label={t("Tentar outra fonte se falhar")} checked={s.fallbackOnFailure} onChange={(v) => patch({ fallbackOnFailure: v })} />
      </Row>
      {s.fallbackOnFailure && (
        <>
          <Row title={t("Máximo de fontes tentadas")}>
            <select aria-label={t("Máximo de fontes tentadas")} value={s.fallbackMaxSources} onChange={(e) => patch({ fallbackMaxSources: Number(e.target.value) })}>
              {[1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map((n) => <option key={n} value={n}>{n}</option>)}
            </select>
          </Row>
          <Row title={t("Tempo para começar")} description={t("Sem começar a tocar nesse tempo, a fonte conta como falha.")}>
            <select aria-label={t("Tempo para começar")} value={s.fallbackStartupTimeoutSeconds} onChange={(e) => patch({ fallbackStartupTimeoutSeconds: Number(e.target.value) })}>
              {[5, 10, 15, 20, 25, 30, 35, 40, 45].map((n) => <option key={n} value={n}>{n} s</option>)}
            </select>
          </Row>
        </>
      )}

      <h3 className="settings-subhead">{t("Último link")}</h3>
      <Row title={t("Reutilizar o último link")} description={t("Abre o filme ou episódio direto pelo link usado da última vez, sem perguntar aos addons. Fontes P2P não são guardadas.")}>
        <Toggle label={t("Reutilizar o último link")} checked={s.reuseLastLink} onChange={(v) => patch({ reuseLastLink: v })} />
      </Row>
      {s.reuseLastLink && (
        <Row title={t("Validade do link")}>
          <select aria-label={t("Validade do link")} value={s.reuseLastLinkHours} onChange={(e) => patch({ reuseLastLinkHours: Number(e.target.value) })}>
            {REUSE_HOURS.map((h) => <option key={h} value={h}>{h < 24 ? `${h} h` : h === 168 ? t("7 dias") : t("{n} dias", { n: h / 24 })}</option>)}
          </select>
        </Row>
      )}

      <h3 className="settings-subhead">{t("Aparência")}</h3>
      <Row title={t("Emblemas nas fontes")} description={t("Qualidade, HDR, codec, áudio, tamanho e seeds lidos do texto de cada fonte.")}>
        <Toggle label={t("Emblemas nas fontes")} checked={s.showBadges} onChange={(v) => patch({ showBadges: v })} />
      </Row>
    </div>
  );
}

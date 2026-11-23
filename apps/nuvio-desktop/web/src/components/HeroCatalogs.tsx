// Settings → Início: the two catalogs of the hero ("Catálogos do destaque" of Nuvio).

import type { CatalogChoice, HomeCatalogSettings } from "../types";
import { Row } from "./SettingControls";
import { t } from "../i18n";

const LIMIT = 2;

interface Props {
  home: HomeCatalogSettings;
  /** In the profile's order. */
  catalogs: CatalogChoice[];
  onChange(chosen: string[]): void;
}

/** The rule of Rust's `hero_sources`. */
function inUse(home: HomeCatalogSettings, catalogs: CatalogChoice[]): string[] {
  if (home.heroCatalogs) {
    const installed = home.heroCatalogs.filter((key) => catalogs.some((c) => c.key === key)).slice(0, LIMIT);
    if (!home.heroCatalogs.length || installed.length) return installed;
  }
  return catalogs
    .filter((c) => home.items.find((i) => i.key === c.key)?.heroSourceEnabled ?? true)
    .slice(0, LIMIT)
    .map((c) => c.key);
}

export function HeroCatalogs({ home, catalogs, onChange }: Props) {
  const choices = catalogs.filter((c) => c.contentType !== "collection");
  const chosen = inUse(home, choices);
  const name = (c: CatalogChoice) => {
    const custom = home.items.find((i) => i.key === c.key)?.customTitle ?? c.customTitle;
    return `${custom.trim() || c.defaultTitle} · ${c.addonName}`;
  };
  const choose = (slot: number, key: string) => {
    const next = [chosen[0] ?? "", chosen[1] ?? ""];
    next[slot] = key;
    onChange(next.filter(Boolean));
  };

  return (
    <>
      {[0, 1].map((slot) => (
        <Row
          key={slot}
          title={t("Catálogo {index} do destaque", { index: slot + 1 })}
          description={
            slot === 0
              ? t("Os títulos destes dois catálogos passam no topo do Início, mesmo que a fileira esteja oculta.")
              : t("O Nuvio não guarda esta escolha na conta. Neste computador, se você mudar no Nuvio, o Astra acompanha.")
          }
        >
          <select className="hero-catalog" aria-label={t("Catálogo {index} do destaque", { index: slot + 1 })} data-hero-slot={slot} value={chosen[slot] ?? ""} disabled={slot === 1 && !chosen.length} onChange={(e) => choose(slot, e.target.value)}>
            <option value="">{t("Nenhum")}</option>
            {choices
              .filter((c) => c.key === chosen[slot] || !chosen.includes(c.key))
              .map((c) => <option key={c.key} value={c.key}>{name(c)}</option>)}
          </select>
        </Row>
      ))}
    </>
  );
}

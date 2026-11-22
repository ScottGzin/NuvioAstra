// Read-only comparison of this computer with the Nuvio account, per profile. Before the first sync it
// is the preview the user reviews before starting it; afterwards it is the check that both sides hold
// the same records ("Comparar com a conta"). Built by Rust from the account's read functions only.

import { useI18n, type TranslationKey } from "../i18n";
import type { SyncCompare, SyncPreview as Preview } from "../types";
import { when } from "../account";

const KINDS: { key: "addons" | "favorites" | "progress" | "watched"; labelKey: TranslationKey }[] = [
  { key: "addons", labelKey: "sync.kindAddons" },
  { key: "favorites", labelKey: "sync.kindFavorites" },
  { key: "progress", labelKey: "sync.kindProgress" },
  { key: "watched", labelKey: "sync.kindWatched" },
];

function Row({ label, c, note }: { label: string; c: SyncCompare; note?: string }) {
  const { t } = useI18n();
  const dup = c.localDuplicates + c.remoteDuplicates;
  return (
    <tr>
      <th scope="row">{label}{note && <span className="muted small"> · {note}</span>}</th>
      <td>{c.local}</td>
      <td>{c.remote}</td>
      <td>{c.both}</td>
      <td className={c.onlyLocal ? "hl-send" : ""}>{c.onlyLocal}</td>
      <td className={c.onlyRemote ? "hl-receive" : ""}>{c.onlyRemote}</td>
      <td className={c.differing ? "hl-conflict" : ""}>
        {c.differing}
        {c.localNewer > 0 && <span className="muted small"> {t("sync.newerHere", { count: c.localNewer })}</span>}
        {dup > 0 && <span className="muted small"> {t("sync.duplicates", { count: dup })}</span>}
      </td>
    </tr>
  );
}

export function SyncPreview({ preview }: { preview: Preview }) {
  const { t } = useI18n();
  const first = preview.firstSync;
  return (
    <div className="sync-preview" data-testid="sync-preview" data-first={first ? "1" : "0"}>
      <div className="setting-title">{first ? t("sync.firstTitle") : t("sync.compareTitle")}</div>
      <p className="muted small">
        {t("sync.readOnly", { when: when(preview.generatedAtEpochMs) })}
        {preview.otherAccount && ` ${t("sync.otherAccount")}`}
      </p>
      <dl className="facts account-facts">
        <div className="fact"><dt>{t("sync.profiles")}</dt><dd>{t("sync.profilesValues", { local: preview.profiles.local, remote: preview.profiles.remote })}</dd></div>
        <div className="fact"><dt>{first ? t("sync.willSend") : t("sync.onlyHere")}</dt><dd data-testid="preview-send">{preview.toSend}</dd></div>
        <div className="fact"><dt>{first ? t("sync.willReceive") : t("sync.onlyAccount")}</dt><dd data-testid="preview-receive">{preview.toReceive}</dd></div>
        <div className="fact"><dt>{t("sync.conflicts")}</dt><dd data-testid="preview-conflicts">{preview.conflicts}</dd></div>
        <div className="fact"><dt>{t("sync.pending")}</dt><dd>{preview.pending}</dd></div>
      </dl>
      {preview.entries.map((e) => (
        <div key={e.index} className="sync-preview-profile" data-profile={e.index}>
          <div className="setting-title">
            {t("sync.profileLine", { index: e.index, local: e.localName ?? "—", remote: e.remoteName ?? "—" })}
            {!e.localName && <span className="muted small"> {t("sync.willCreateHere")}</span>}
            {!e.remoteName && <span className="muted small"> {t("sync.willCreateAccount")}</span>}
          </div>
          {e.profileDiffers && <div className="muted small hl-conflict">{t("sync.profileDiffers")}</div>}
          <table className="sync-table">
            <thead>
              <tr><th /><th>{t("sync.computer")}</th><th>{t("sync.account")}</th><th>{t("sync.both")}</th><th>{t("sync.onlyHereCol")}</th><th>{t("sync.onlyAccountCol")}</th><th>{t("sync.different")}</th></tr>
            </thead>
            <tbody>
              {KINDS.map((k) =>
                k.key === "addons" && e.sharesPrimaryAddons ? (
                  <tr key={k.key}><th scope="row">{t("sync.kindAddons")}</th><td colSpan={6} className="muted small">{t("sync.usesPrimary")}</td></tr>
                ) : (
                  <Row key={k.key} label={t(k.labelKey)} c={e[k.key]} note={k.key === "addons" && e.addonOrderDiffers ? t("sync.orderDiffers") : undefined} />
                ),
              )}
            </tbody>
          </table>
        </div>
      ))}
      {first && (
        <ul className="muted small sync-rules">
          <li>{t("sync.ruleSend")}</li>
          <li>{t("sync.ruleProgress")}</li>
          <li>{t("sync.ruleAddons")}</li>
          <li>{t("sync.ruleBackup")}</li>
        </ul>
      )}
    </div>
  );
}

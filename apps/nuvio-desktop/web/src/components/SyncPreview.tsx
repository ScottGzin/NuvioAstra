// Read-only comparison of this computer with the Nuvio account, per profile. Before the first sync it
// is the preview the user reviews before starting it; afterwards it is the check that both sides hold
// the same records ("Comparar com a conta"). Built by Rust from the account's read functions only.

import type { SyncCompare, SyncPreview as Preview } from "../types";
import { when } from "../account";
import { t } from "../i18n";

const KINDS: { key: "addons" | "favorites" | "progress" | "watched"; label: string }[] = [
  { key: "addons", label: "Addons" },
  { key: "favorites", label: "Favorites" },
  { key: "progress", label: "Progress" },
  { key: "watched", label: "Watched" },
];

function Row({ label, c, note }: { label: string; c: SyncCompare; note?: string }) {
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
        {c.localNewer > 0 && <span className="muted small"> {t("({n} newer here)", { n: c.localNewer })}</span>}
        {dup > 0 && <span className="muted small"> · {t("{n} duplicate(s)", { n: dup })}</span>}
      </td>
    </tr>
  );
}

export function SyncPreview({ preview }: { preview: Preview }) {
  const first = preview.firstSync;
  return (
    <div className="sync-preview" data-testid="sync-preview" data-first={first ? "1" : "0"}>
      <div className="setting-title">{first ? t("First sync preview") : t("Comparison with the account")}</div>
      <p className="muted small">
        {t("Queried {time} using account reads only: nothing was sent or changed.", { time: when(preview.generatedAtEpochMs) })}
        {preview.otherAccount && t(" The data on this computer came from a different Nuvio account.")}
      </p>
      <dl className="facts account-facts">
        <div className="fact"><dt>{t("Profiles")}</dt><dd>{t("{local} on this computer · {remote} in the account", { local: preview.profiles.local, remote: preview.profiles.remote })}</dd></div>
        <div className="fact"><dt>{first ? t("To be sent") : t("Only on this computer")}</dt><dd data-testid="preview-send">{preview.toSend}</dd></div>
        <div className="fact"><dt>{first ? t("To be received") : t("Only in the account / different")}</dt><dd data-testid="preview-receive">{preview.toReceive}</dd></div>
        <div className="fact"><dt>{t("Conflicts")}</dt><dd data-testid="preview-conflicts">{preview.conflicts}</dd></div>
        <div className="fact"><dt>{t("Pending local changes")}</dt><dd>{preview.pending}</dd></div>
      </dl>
      {preview.entries.map((e) => (
        <div key={e.index} className="sync-preview-profile" data-profile={e.index}>
          <div className="setting-title">
            {t("Profile {n}: {local} (computer) · {remote} (account)", { n: e.index, local: e.localName ?? "—", remote: e.remoteName ?? "—" })}
            {!e.localName && <span className="muted small">{t(" · will be created on this computer")}</span>}
            {!e.remoteName && <span className="muted small">{t(" · will be created in the account")}</span>}
          </div>
          {e.profileDiffers && <div className="muted small hl-conflict">{t("Different name, color, or avatar: the account's version wins.")}</div>}
          <table className="sync-table">
            <thead>
              <tr><th /><th>{t("Computer")}</th><th>{t("Account")}</th><th>{t("In both")}</th><th>{t("Only here")}</th><th>{t("Only in the account")}</th><th>{t("Different")}</th></tr>
            </thead>
            <tbody>
              {KINDS.map((k) =>
                k.key === "addons" && e.sharesPrimaryAddons ? (
                  <tr key={k.key}><th scope="row">{t("Addons")}</th><td colSpan={6} className="muted small">{t("uses profile 1's addons")}</td></tr>
                ) : (
                  <Row key={k.key} label={t(k.label)} c={e[k.key]} note={k.key === "addons" && e.addonOrderDiffers ? t("different order") : undefined} />
                ),
              )}
            </tbody>
          </table>
        </div>
      ))}
      {first && (
        <ul className="muted small sync-rules">
          <li>{t("Items only on this computer are sent; items only in the account are pulled. Nothing is deleted on either side.")}</li>
          <li>{t("Different progress: the most recent wins (this computer's is sent when it is the newest).")}</li>
          <li>{t("Addons: the account's order and activation, plus the addons that only exist here. Profiles with the same number: the account's name, color, and avatar.")}</li>
          <li>{t("Before writing, Nuvio Astra makes a verified copy of this computer's data.")}</li>
        </ul>
      )}
    </div>
  );
}

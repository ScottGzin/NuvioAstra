// Read-only comparison of this computer with the Nuvio account, per profile. Before the first sync it
// is the preview the user reviews before starting it; afterwards it is the check that both sides hold
// the same records ("Comparar com a conta"). Built by Rust from the account's read functions only.

import type { SyncCompare, SyncPreview as Preview } from "../types";
import { when } from "../account";

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
        {c.localNewer > 0 && <span className="muted small"> ({c.localNewer} newer here)</span>}
        {dup > 0 && <span className="muted small"> · {dup} duplicate(s)</span>}
      </td>
    </tr>
  );
}

export function SyncPreview({ preview }: { preview: Preview }) {
  const first = preview.firstSync;
  return (
    <div className="sync-preview" data-testid="sync-preview" data-first={first ? "1" : "0"}>
      <div className="setting-title">{first ? "First sync preview" : "Comparison with the account"}</div>
      <p className="muted small">
        Queried {when(preview.generatedAtEpochMs)} using account reads only: nothing was sent or changed.
        {preview.otherAccount && " The data on this computer came from a different Nuvio account."}
      </p>
      <dl className="facts account-facts">
        <div className="fact"><dt>Profiles</dt><dd>{preview.profiles.local} on this computer · {preview.profiles.remote} in the account</dd></div>
        <div className="fact"><dt>{first ? "To be sent" : "Only on this computer"}</dt><dd data-testid="preview-send">{preview.toSend}</dd></div>
        <div className="fact"><dt>{first ? "To be received" : "Only in the account / different"}</dt><dd data-testid="preview-receive">{preview.toReceive}</dd></div>
        <div className="fact"><dt>Conflicts</dt><dd data-testid="preview-conflicts">{preview.conflicts}</dd></div>
        <div className="fact"><dt>Pending local changes</dt><dd>{preview.pending}</dd></div>
      </dl>
      {preview.entries.map((e) => (
        <div key={e.index} className="sync-preview-profile" data-profile={e.index}>
          <div className="setting-title">
            Profile {e.index}: {e.localName ?? "—"} (computer) · {e.remoteName ?? "—"} (account)
            {!e.localName && <span className="muted small"> · will be created on this computer</span>}
            {!e.remoteName && <span className="muted small"> · will be created in the account</span>}
          </div>
          {e.profileDiffers && <div className="muted small hl-conflict">Different name, color, or avatar: the account's version wins.</div>}
          <table className="sync-table">
            <thead>
              <tr><th /><th>Computer</th><th>Account</th><th>In both</th><th>Only here</th><th>Only in the account</th><th>Different</th></tr>
            </thead>
            <tbody>
              {KINDS.map((k) =>
                k.key === "addons" && e.sharesPrimaryAddons ? (
                  <tr key={k.key}><th scope="row">Addons</th><td colSpan={6} className="muted small">uses profile 1's addons</td></tr>
                ) : (
                  <Row key={k.key} label={k.label} c={e[k.key]} note={k.key === "addons" && e.addonOrderDiffers ? "different order" : undefined} />
                ),
              )}
            </tbody>
          </table>
        </div>
      ))}
      {first && (
        <ul className="muted small sync-rules">
          <li>Items only on this computer are sent; items only in the account are pulled. Nothing is deleted on either side.</li>
          <li>Different progress: the most recent wins (this computer's is sent when it is the newest).</li>
          <li>Addons: the account's order and activation, plus the addons that only exist here. Profiles with the same number: the account's name, color, and avatar.</li>
          <li>Before writing, Nuvio Astra makes a verified copy of this computer's data.</li>
        </ul>
      )}
    </div>
  );
}

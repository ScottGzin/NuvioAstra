// Read-only comparison of this computer with the Nuvio account, per profile. Before the first sync it
// is the preview the user reviews before starting it; afterwards it is the check that both sides hold
// the same records (t("Comparar com a conta")). Built by Rust from the account's read functions only.

import type { SyncCompare, SyncPreview as Preview } from "../types";
import { when } from "../account";
import { t } from "../i18n";

const KINDS: { key: "addons" | "favorites" | "progress" | "watched"; label: string }[] = [
  { key: "addons", label: "Addons" },
  { key: "favorites", label: "Favoritos" },
  { key: "progress", label: "Progresso" },
  { key: "watched", label: "Assistidos" },
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
        {c.localNewer > 0 && <span className="muted small"> ({t("{count} mais recente(s) aqui", { count: c.localNewer })})</span>}
        {dup > 0 && <span className="muted small"> · {t("{count} duplicado(s)", { count: dup })}</span>}
      </td>
    </tr>
  );
}

export function SyncPreview({ preview }: { preview: Preview }) {
  const first = preview.firstSync;
  return (
    <div className="sync-preview" data-testid="sync-preview" data-first={first ? "1" : "0"}>
      <div className="setting-title">{first ? t("Prévia da primeira sincronização") : t("Comparação com a conta")}</div>
      <p className="muted small">
        {t("Consultado em {when} somente com leituras da conta: nada foi enviado nem alterado.", { when: when(preview.generatedAtEpochMs) })}
        {preview.otherAccount && ` ${t("Os dados deste computador vieram de outra conta Nuvio.")}`}
      </p>
      <dl className="facts account-facts">
        <div className="fact"><dt>{t("Perfis")}</dt><dd>{t("{local} neste computador · {remote} na conta", { local: preview.profiles.local, remote: preview.profiles.remote })}</dd></div>
        <div className="fact"><dt>{first ? t("Serão enviados") : t("Só neste computador")}</dt><dd data-testid="preview-send">{preview.toSend}</dd></div>
        <div className="fact"><dt>{first ? t("Serão recebidos") : t("Só na conta / diferentes")}</dt><dd data-testid="preview-receive">{preview.toReceive}</dd></div>
        <div className="fact"><dt>{t("Conflitos")}</dt><dd data-testid="preview-conflicts">{preview.conflicts}</dd></div>
        <div className="fact"><dt>{t("Alterações locais pendentes")}</dt><dd>{preview.pending}</dd></div>
      </dl>
      {preview.entries.map((e) => (
        <div key={e.index} className="sync-preview-profile" data-profile={e.index}>
          <div className="setting-title">
            {t("Perfil {index}: {local} (computador) · {remote} (conta)", { index: e.index, local: e.localName ?? "—", remote: e.remoteName ?? "—" })}
            {!e.localName && <span className="muted small"> {t("· será criado neste computador")}</span>}
            {!e.remoteName && <span className="muted small"> {t("· será criado na conta")}</span>}
          </div>
          {e.profileDiffers && <div className="muted small hl-conflict">{t("Nome, cor ou avatar diferentes: fica a versão da conta.")}</div>}
          <table className="sync-table">
            <thead>
              <tr><th /><th>{t("Computador")}</th><th>{t("Conta")}</th><th>{t("Em ambos")}</th><th>{t("Só aqui")}</th><th>{t("Só na conta")}</th><th>{t("Diferentes")}</th></tr>
            </thead>
            <tbody>
              {KINDS.map((k) =>
                k.key === "addons" && e.sharesPrimaryAddons ? (
                  <tr key={k.key}><th scope="row">{t("Addons")}</th><td colSpan={6} className="muted small">{t("usa os addons do perfil 1")}</td></tr>
                ) : (
                  <Row key={k.key} label={t(k.label)} c={e[k.key]} note={k.key === "addons" && e.addonOrderDiffers ? t("ordem diferente") : undefined} />
                ),
              )}
            </tbody>
          </table>
        </div>
      ))}
      {first && (
        <ul className="muted small sync-rules">
          <li>{t("Itens só deste computador são enviados; itens só da conta são trazidos. Nada é apagado em nenhum dos lados.")}</li>
          <li>{t("Progresso diferente: fica o mais recente (o deste computador é enviado quando for o mais novo).")}</li>
          <li>{t("Addons: ordem e ativação da conta, mais os addons que só existem aqui. Perfis com o mesmo número: nome, cor e avatar da conta.")}</li>
          <li>{t("Antes de gravar, o Nuvio Astra faz uma cópia verificada dos dados deste computador.")}</li>
        </ul>
      )}
    </div>
  );
}

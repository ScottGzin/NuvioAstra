// Settings → Coleções (the original's collection editor): create, rename, reorder and remove
// collections and their folders, and pick the addon catalogs (and genre) each folder shows. Saved
// in the profile and sent to the Nuvio account; TMDB/Trakt sources made elsewhere are kept.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import type { CollectionCatalogOption, EditCollection, EditFolder } from "../types";
import { Row, Toggle } from "./SettingControls";
import { t } from "../i18n";

const newId = (prefix: string) => `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;

function move<T>(list: T[], index: number, dir: -1 | 1): T[] {
  const next = [...list];
  const target = index + dir;
  if (target < 0 || target >= next.length) return list;
  [next[index], next[target]] = [next[target], next[index]];
  return next;
}

function FolderEditor({ folder, catalogs, onChange, onRemove, onMove }: {
  folder: EditFolder;
  catalogs: CollectionCatalogOption[];
  onChange(f: EditFolder): void;
  onRemove(): void;
  onMove(dir: -1 | 1): void;
}) {
  const [pick, setPick] = useState("");
  const [genre, setGenre] = useState("");
  const chosen = catalogs.find((c) => `${c.addonId}|${c.contentType}|${c.catalogId}` === pick);
  const add = () => {
    if (!chosen) return;
    onChange({
      ...folder,
      sources: [...folder.sources, { addonId: chosen.addonId, contentType: chosen.contentType, catalogId: chosen.catalogId, genre: genre || null, label: `${chosen.label} (${chosen.addonName})${genre ? ` · ${genre}` : ""}` }],
    });
    setPick("");
    setGenre("");
  };
  return (
    <div className="ce-folder" data-testid="ce-folder">
      <div className="ce-folder-head">
        <span className="ce-emoji">{folder.coverEmoji || "📁"}</span>
        <input className="ce-title" value={folder.title} placeholder={t("Nome da pasta")} aria-label={t("Nome da pasta")} onChange={(e) => onChange({ ...folder, title: e.target.value })} />
        <button className="icon-btn" aria-label={t("Subir")} onClick={() => onMove(-1)}>↑</button>
        <button className="icon-btn" aria-label={t("Descer")} onClick={() => onMove(1)}>↓</button>
        <button className="icon-btn danger" aria-label={t("Remover pasta")} onClick={onRemove}>✕</button>
      </div>
      <div className="ce-folder-fields">
        <label>
          <span>{t("Emoji")}</span>
          <input value={folder.coverEmoji ?? ""} maxLength={4} onChange={(e) => onChange({ ...folder, coverEmoji: e.target.value || null })} />
        </label>
        <label className="ce-wide">
          <span>{t("Imagem da capa (endereço)")}</span>
          <input value={folder.coverImage ?? ""} placeholder="https://…" spellCheck={false} onChange={(e) => onChange({ ...folder, coverImage: e.target.value || null })} />
        </label>
        <label>
          <span>{t("Formato")}</span>
          <select value={folder.tileShape} onChange={(e) => onChange({ ...folder, tileShape: e.target.value })}>
            <option value="poster">{t("Pôster")}</option>
            <option value="landscape">{t("Paisagem")}</option>
            <option value="square">{t("Quadrado")}</option>
          </select>
        </label>
        <label className="ce-check">
          <input type="checkbox" checked={folder.hideTitle} onChange={(e) => onChange({ ...folder, hideTitle: e.target.checked })} />
          {t("Ocultar nome")}
        </label>
      </div>
      <div className="ce-sources">
        {folder.sources.map((s, i) => (
          <div key={`${s.addonId}-${s.catalogId}-${s.genre}-${i}`} className="ce-source" data-testid="ce-source">
            <span>{s.label}</span>
            <button className="icon-btn" aria-label={t("Subir")} onClick={() => onChange({ ...folder, sources: move(folder.sources, i, -1) })}>↑</button>
            <button className="icon-btn danger" aria-label={t("Remover catálogo")} onClick={() => onChange({ ...folder, sources: folder.sources.filter((_, j) => j !== i) })}>✕</button>
          </div>
        ))}
        {folder.otherSources > 0 && <div className="muted small">{t("+ {n} catálogo(s) do TMDB/Trakt mantidos como estão.", { n: folder.otherSources })}</div>}
        <div className="ce-add-source">
          <select value={pick} onChange={(e) => { setPick(e.target.value); setGenre(""); }} aria-label={t("Catálogo")} data-testid="ce-catalog">
            <option value="">{t("Adicionar catálogo…")}</option>
            {catalogs.map((c) => (
              <option key={`${c.addonId}|${c.contentType}|${c.catalogId}`} value={`${c.addonId}|${c.contentType}|${c.catalogId}`}>{`${c.label} — ${c.addonName}`}</option>
            ))}
          </select>
          {chosen && chosen.genres.length > 0 && (
            <select value={genre} onChange={(e) => setGenre(e.target.value)} aria-label={t("Gênero")}>
              <option value="">{t("Todos os gêneros")}</option>
              {chosen.genres.map((g) => <option key={g} value={g}>{g}</option>)}
            </select>
          )}
          <button className="ghost" disabled={!chosen} onClick={add} data-testid="ce-add-source">{t("Adicionar")}</button>
        </div>
      </div>
    </div>
  );
}

export function CollectionsEditor() {
  const [list, setList] = useState<EditCollection[] | null>(null);
  const [catalogs, setCatalogs] = useState<CollectionCatalogOption[]>([]);
  const [open, setOpen] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const [status, setStatus] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    api.collectionsEdit().then((v) => {
      setList(v.collections);
      setCatalogs(v.catalogs);
    }, (e) => setStatus({ ok: false, text: errorText(e) }));
  }, []);

  if (!list) return <div className="muted">{status?.text ?? t("Carregando coleções…")}</div>;

  const change = (next: EditCollection[]) => {
    setList(next);
    setDirty(true);
    setStatus(null);
  };
  const edit = (id: string, patch: Partial<EditCollection>) => change(list.map((c) => (c.id === id ? { ...c, ...patch } : c)));
  const save = async () => {
    try {
      await api.collectionsSave(list);
      setDirty(false);
      setStatus({ ok: true, text: t("Coleções salvas. A tela Início é atualizada.") });
    } catch (e) {
      setStatus({ ok: false, text: errorText(e) });
    }
  };
  const create = () => {
    const c: EditCollection = { id: newId("astra"), title: t("Nova coleção"), backdrop: null, viewMode: "TABBED_GRID", showAllTab: true, pinToTop: false, folders: [] };
    change([...list, c]);
    setOpen(c.id);
  };

  return (
    <div className="settings-group collections-editor" data-testid="collections-editor">
      <p className="muted">{t("Coleções aparecem no Início como uma fileira de pastas. Cada pasta mostra um ou mais catálogos dos seus addons. Com uma conta Nuvio, elas sincronizam com os outros aparelhos.")}</p>
      {list.length === 0 && <div className="muted">{t("Nenhuma coleção ainda.")}</div>}
      {list.map((c, index) => (
        <div key={c.id} className={`ce-collection ${open === c.id ? "open" : ""}`} data-testid="ce-collection">
          <div className="ce-collection-head">
            <button className="ce-toggle" onClick={() => setOpen(open === c.id ? null : c.id)} aria-expanded={open === c.id}>
              <b>{c.title || t("Sem nome")}</b>
              <span className="muted small">{t("{n} pasta(s)", { n: c.folders.length })}</span>
            </button>
            <button className="icon-btn" aria-label={t("Subir")} onClick={() => change(move(list, index, -1))}>↑</button>
            <button className="icon-btn" aria-label={t("Descer")} onClick={() => change(move(list, index, 1))}>↓</button>
            <button className="icon-btn danger" aria-label={t("Excluir coleção")} onClick={() => window.confirm(t("Excluir a coleção “{name}”?", { name: c.title })) && change(list.filter((x) => x.id !== c.id))}>✕</button>
          </div>
          {open === c.id && (
            <div className="ce-collection-body">
              <Row title={t("Nome")}>
                <input value={c.title} onChange={(e) => edit(c.id, { title: e.target.value })} aria-label={t("Nome da coleção")} data-testid="ce-collection-title" />
              </Row>
              <Row title={t("Exibição da pasta")} description={t("Abas com os catálogos em grade, ou uma fileira por catálogo.")}>
                <select value={c.viewMode} onChange={(e) => edit(c.id, { viewMode: e.target.value })}>
                  <option value="TABBED_GRID">{t("Abas em grade")}</option>
                  <option value="ROWS">{t("Fileiras")}</option>
                </select>
              </Row>
              <Row title={t("Aba “Todos”")} description={t("Junta os catálogos da pasta numa única aba.")}>
                <Toggle label={t("Aba “Todos”")} checked={c.showAllTab} onChange={(v) => edit(c.id, { showAllTab: v })} />
              </Row>
              <Row title={t("Fixar no topo do Início")}>
                <Toggle label={t("Fixar no topo do Início")} checked={c.pinToTop} onChange={(v) => edit(c.id, { pinToTop: v })} />
              </Row>
              <Row title={t("Imagem de fundo (endereço)")}>
                <input value={c.backdrop ?? ""} placeholder="https://…" spellCheck={false} onChange={(e) => edit(c.id, { backdrop: e.target.value || null })} />
              </Row>
              <h3 className="settings-subhead">{t("Pastas")}</h3>
              {c.folders.map((f, fi) => (
                <FolderEditor
                  key={f.id}
                  folder={f}
                  catalogs={catalogs}
                  onChange={(nf) => edit(c.id, { folders: c.folders.map((x) => (x.id === f.id ? nf : x)) })}
                  onRemove={() => edit(c.id, { folders: c.folders.filter((x) => x.id !== f.id) })}
                  onMove={(dir) => edit(c.id, { folders: move(c.folders, fi, dir) })}
                />
              ))}
              <button
                className="ghost"
                onClick={() => edit(c.id, { folders: [...c.folders, { id: newId("folder"), title: t("Nova pasta"), coverImage: null, coverEmoji: "🎬", tileShape: "poster", hideTitle: false, sources: [], otherSources: 0 }] })}
                data-testid="ce-add-folder"
              >
                {t("+ Nova pasta")}
              </button>
            </div>
          )}
        </div>
      ))}
      <div className="ce-actions">
        <button className="ghost" onClick={create} data-testid="ce-new">{t("+ Nova coleção")}</button>
        <span className="spacer" />
        {status && <span className={`addon-feedback ${status.ok ? "ok" : "error"}`} role="status">{status.text}</span>}
        <button className="primary" disabled={!dirty} onClick={() => void save()} data-testid="ce-save">{t("Salvar coleções")}</button>
      </div>
    </div>
  );
}

// Visual collections editor: search, create, rename, add/remove folders and delete. The document is
// edited as raw JSON objects so unknown fields from the original/account are preserved on save.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { Row, Toggle } from "./SettingControls";
import { t } from "../i18n";

type Json = Record<string, unknown>;
type Folder = Json & { id: string; title: string; sources?: Json[]; catalogSources?: Json[] };
type Collection = Json & { id: string; title: string; folders?: Folder[] };

const newId = () => `col_${Date.now().toString(36)}`;

export function CollectionsManager() {
  const [items, setItems] = useState<Collection[]>([]);
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<Collection | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [advanced, setAdvanced] = useState(false);
  const [raw, setRaw] = useState("");

  const load = () => api.collectionsEditorLoad().then((value) => {
    const list = Array.isArray(value) ? (value as Collection[]) : [];
    setItems(list);
    setRaw(JSON.stringify(list, null, 2));
  }, (e) => setError(errorText(e)));
  useEffect(() => { load(); }, []);

  const save = async (next: Collection[]) => {
    setError(null);
    try {
      const saved = await api.collectionsEditorSave(next);
      const list = Array.isArray(saved) ? (saved as Collection[]) : next;
      setItems(list);
      setRaw(JSON.stringify(list, null, 2));
    } catch (e) { setError(errorText(e)); }
  };

  const create = () => {
    const draft: Collection = { id: newId(), title: t("Nova coleção"), pinToTop: false, viewMode: "TABBED_GRID", showAllTab: true, folders: [] };
    setEditing(draft);
  };

  const shown = items.filter((item) => !query || item.title.toLowerCase().includes(query.toLowerCase()));

  return (
    <div className="collections-manager">
      {error && <div className="row-error" role="alert">{error}</div>}
      {!editing && <>
        <input className="collections-search" aria-label={t("Buscar coleções")} placeholder={t("Buscar coleções")} value={query} onChange={(e) => setQuery(e.target.value)} />
        <button className="primary" data-testid="new-collection" onClick={create}>+ {t("Nova coleção")}</button>
        <div className="settings-group collections-list">
          {shown.length === 0 && <div className="muted">{t("Nenhuma coleção encontrada.")}</div>}
          {shown.map((item) => {
            const folders = Array.isArray(item.folders) ? item.folders : [];
            const sources = folders.reduce((sum, folder) => sum + (Array.isArray(folder.sources) ? folder.sources.length : 0) + (Array.isArray(folder.catalogSources) ? folder.catalogSources.length : 0), 0);
            return (
              <div key={item.id} className="collection-row" data-collection={item.id}>
                <div className="collection-info">
                  <div className="setting-title">{item.title}</div>
                  <div className="muted small">{folders.length} {t("pastas")} · {sources} {t("itens")}</div>
                </div>
                <button className="chip" aria-label={t("Editar")} onClick={() => setEditing(structuredClone(item))}>✎</button>
                <button className="chip" aria-label={t("Remover")} onClick={() => {
                  if (window.confirm(t("Remover esta coleção?"))) void save(items.filter((c) => c.id !== item.id));
                }}>🗑</button>
              </div>
            );
          })}
        </div>
        <button className="link" onClick={() => setAdvanced(!advanced)}>{advanced ? t("Ocultar JSON") : t("Avançado (JSON)")}</button>
        {advanced && <>
          <textarea className="collections-json" aria-label={t("Documento de coleções")} value={raw} onChange={(e) => setRaw(e.target.value)} spellCheck={false} rows={16} />
          <button className="ghost" onClick={() => {
            try { void save(JSON.parse(raw) as Collection[]); } catch (e) { setError(String(e)); }
          }}>{t("Salvar JSON")}</button>
        </>}
      </>}

      {editing && <CollectionEditor
        collection={editing}
        onCancel={() => setEditing(null)}
        onSave={(next) => {
          const exists = items.some((item) => item.id === next.id);
          void save(exists ? items.map((item) => item.id === next.id ? next : item) : [...items, next]);
          setEditing(null);
        }}
      />}
    </div>
  );
}

function CollectionEditor({ collection, onCancel, onSave }: { collection: Collection; onCancel(): void; onSave(c: Collection): void }) {
  const [draft, setDraft] = useState<Collection>(collection);
  const folders = Array.isArray(draft.folders) ? draft.folders : [];
  const patch = (change: Partial<Collection>) => setDraft((d) => ({ ...d, ...change }));
  const patchFolder = (id: string, change: Partial<Folder>) => patch({ folders: folders.map((f) => f.id === id ? { ...f, ...change } : f) });
  return (
    <div className="settings-group">
      <button className="ghost" onClick={onCancel}>← {t("Coleções")}</button>
      <Row title={t("Título")}>
        <input aria-label={t("Título")} value={draft.title} onChange={(e) => patch({ title: e.target.value })} />
      </Row>
      <Row title={t("Fixar no topo")}>
        <Toggle label={t("Fixar no topo")} checked={!!draft.pinToTop} onChange={(v) => patch({ pinToTop: v })} />
      </Row>
      <Row title={t("Mostrar aba Todos")}>
        <Toggle label={t("Mostrar aba Todos")} checked={draft.showAllTab !== false} onChange={(v) => patch({ showAllTab: v })} />
      </Row>
      <div className="settings-subhead">{t("Pastas")}</div>
      {folders.map((folder, index) => (
        <div key={folder.id} className="collection-folder-row">
          <span className="muted small">{index + 1}</span>
          <input aria-label={t("Título da pasta")} value={folder.title} onChange={(e) => patchFolder(folder.id, { title: e.target.value })} />
          <button className="chip" aria-label={t("Remover pasta")} onClick={() => patch({ folders: folders.filter((f) => f.id !== folder.id) })}>✕</button>
        </div>
      ))}
      <button className="ghost" data-testid="new-folder" onClick={() => patch({ folders: [...folders, { id: `fld_${Date.now().toString(36)}`, title: t("Nova pasta"), sources: [] }] })}>
        + {t("Nova pasta")}
      </button>
      <div className="modal-actions">
        <button className="primary" data-testid="save-collection" disabled={!draft.title.trim()} onClick={() => onSave(draft)}>{t("Salvar")}</button>
        <button className="ghost" onClick={onCancel}>{t("Cancelar")}</button>
      </div>
    </div>
  );
}

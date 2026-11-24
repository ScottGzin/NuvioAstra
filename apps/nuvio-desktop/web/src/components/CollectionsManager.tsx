// Visual collections editor: search, create, rename, add/remove folders and delete. The document is
// edited as raw JSON objects so unknown fields from the original/account are preserved on save.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import type { TraktList } from "../types";
import { Row, Toggle } from "./SettingControls";
import { t } from "../i18n";

type Json = Record<string, unknown>;
type Folder = Json & { id: string; title: string; sources?: Json[]; catalogSources?: Json[] };
type Collection = Json & { id: string; title: string; folders?: Folder[] };

const newId = () => `col_${Date.now().toString(36)}`;
const listCover = (list: TraktList | undefined): string | undefined => list?.cover ?? undefined;

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

  type Picker = {
    folderId: string; input: string; title: string; media: "MOVIE" | "TV" | "BOTH"; sortBy: string; sortHow: string;
    query: string; results: TraktList[]; featured: TraktList[]; mine: TraktList[]; kind: "trending" | "popular"; error: string | null; busy: boolean;
  };
  const [picker, setPicker] = useState<Picker | null>(null);
  const patchPicker = (change: Partial<Picker>) => setPicker((p) => p ? { ...p, ...change } : p);

  const openPicker = async (folderId: string) => {
    const base: Picker = {
      folderId, input: "", title: "", media: "MOVIE", sortBy: "rank", sortHow: "asc",
      query: "", results: [], featured: [], mine: [], kind: "trending", error: null, busy: false,
    };
    setPicker(base);
    try { patchPicker({ featured: await api.traktTrendingLists() }); } catch (e) { patchPicker({ error: errorText(e) }); }
    try { patchPicker({ mine: await api.traktMyLists() }); } catch { /* not connected: public lists only */ }
  };

  const loadFeatured = async (kind: "trending" | "popular") => {
    patchPicker({ kind, busy: true, error: null });
    try { patchPicker({ featured: kind === "trending" ? await api.traktTrendingLists() : await api.traktPopularLists(), busy: false }); }
    catch (e) { patchPicker({ error: errorText(e), busy: false }); }
  };

  const search = async () => {
    if (!picker?.query.trim()) return;
    patchPicker({ busy: true, error: null });
    try { patchPicker({ results: await api.traktSearchLists(picker.query), busy: false }); }
    catch (e) { patchPicker({ error: errorText(e), busy: false }); }
  };

  const addLists = (lists: TraktList[], fallbackTitle?: string) => {
    if (!picker) return;
    const folder = folders.find((f) => f.id === picker.folderId);
    if (!folder) return;
    const make = (list: TraktList, media: "MOVIE" | "TV", title: string) => ({
      id: `trakt_${list.id}_${media === "TV" ? "show" : "movie"}_${picker.sortBy}_${picker.sortHow}`,
      provider: "trakt",
      traktListId: list.id,
      mediaType: media,
      sortBy: picker.sortBy,
      sortHow: picker.sortHow,
      title,
      name: title,
    });
    const created: Json[] = [];
    for (const list of lists) {
      const label = picker.title.trim() || fallbackTitle || list.title;
      const medias: ("MOVIE" | "TV")[] = picker.media === "BOTH" ? ["MOVIE", "TV"] : [picker.media];
      for (const media of medias) {
        const title = medias.length === 2 ? `${label} · ${media === "TV" ? t("Séries") : t("Filmes")}` : label;
        created.push(make(list, media, title));
      }
    }
    const existing = Array.isArray(folder.sources) ? folder.sources : [];
    const cover = listCover(lists[0]);
    patchFolder(folder.id, {
      sources: [...existing, ...created],
      ...(cover && !folder.coverImageUrl ? { coverImageUrl: cover } : {}),
    });
    setPicker(null);
  };

  const addFromInput = async () => {
    if (!picker?.input.trim()) return;
    patchPicker({ busy: true, error: null });
    try {
      const resolved = await api.traktResolveListInput(picker.input);
      addLists([{ ...resolved, itemCount: 0, likes: 0 }], picker.title.trim() ? undefined : resolved.title);
    } catch (e) { patchPicker({ error: errorText(e), busy: false }); }
  };

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
          <button className="chip" data-testid="add-trakt-source" onClick={() => void openPicker(folder.id)}>Trakt</button>
          <button className="chip" aria-label={t("Remover pasta")} onClick={() => patch({ folders: folders.filter((f) => f.id !== folder.id) })}>✕</button>
        </div>
      ))}
      {folders.map((folder) => {
        const sources = Array.isArray(folder.sources) ? folder.sources : [];
        const trakt = sources.filter((s) => String(s.provider ?? "").toLowerCase() === "trakt");
        return trakt.length === 0 ? null : (
          <div key={`${folder.id}-trakt`} className="collection-folder-row">
            <span className="muted small">{folder.title}</span>
            {trakt.map((source, index) => (
              <button key={`${String(source.id)}-${index}`} className="chip" onClick={() => patchFolder(folder.id, { sources: sources.filter((_, i) => sources[i] !== source) })}>
                Trakt · {String(source.title ?? source.traktListId ?? "")} ✕
              </button>
            ))}
          </div>
        );
      })}
      <button className="ghost" data-testid="new-folder" onClick={() => patch({ folders: [...folders, { id: `fld_${Date.now().toString(36)}`, title: t("Nova pasta"), sources: [] }] })}>
        + {t("Nova pasta")}
      </button>
      {picker && (
        <div className="settings-group" data-testid="trakt-picker">
          <div className="setting-title">{t("Adicionar lista do Trakt")}</div>
          <Row title={t("ID, URL ou slug da lista")}>
            <input aria-label={t("ID, URL ou slug da lista")} value={picker.input} placeholder="https://trakt.tv/lists/123456" onChange={(e) => patchPicker({ input: e.target.value })} />
          </Row>
          <Row title={t("Título")}>
            <input aria-label={t("Título da fonte")} value={picker.title} onChange={(e) => patchPicker({ title: e.target.value })} />
          </Row>
          <Row title={t("Tipo de mídia")}>
            <select aria-label={t("Tipo de mídia")} value={picker.media} onChange={(e) => patchPicker({ media: e.target.value as Picker["media"] })}>
              <option value="MOVIE">{t("Filmes")}</option>
              <option value="TV">{t("Séries")}</option>
              <option value="BOTH">{t("Ambos")}</option>
            </select>
          </Row>
          <Row title={t("Ordenar por")}>
            <select aria-label={t("Ordenar por")} value={picker.sortBy} onChange={(e) => patchPicker({ sortBy: e.target.value })}>
              {["rank", "added", "title", "released", "runtime", "popularity", "percentage", "votes"].map((sort) => <option key={sort} value={sort}>{sort}</option>)}
            </select>
          </Row>
          <Row title={t("Direção")}>
            <select aria-label={t("Direção")} value={picker.sortHow} onChange={(e) => patchPicker({ sortHow: e.target.value })}>
              <option value="asc">{t("Crescente")}</option>
              <option value="desc">{t("Decrescente")}</option>
            </select>
          </Row>
          <div className="pc-motion">
            <button className="primary" data-testid="trakt-add-input" disabled={picker.busy || !picker.input.trim()} onClick={() => void addFromInput()}>{t("Adicionar")}</button>
            <button className="ghost" onClick={() => setPicker(null)}>{t("Cancelar")}</button>
          </div>
          <Row title={t("Buscar listas públicas")}>
            <input aria-label={t("Buscar listas públicas")} value={picker.query} onChange={(e) => patchPicker({ query: e.target.value })} />
            <button className="ghost" disabled={picker.busy || !picker.query.trim()} onClick={() => void search()}>{t("Buscar")}</button>
          </Row>
          {picker.mine.length > 0 && (
            <>
              <div className="setting-title">{t("Minhas listas do Trakt")}</div>
              <div className="chip-select">
                {picker.mine.map((list, index) => (
                  <button key={`mine-${list.id}-${index}`} className="chip" data-testid="trakt-pick-my-list" disabled={picker.busy} onClick={() => addLists([list])}>
                    {list.title} <small className="muted small">{list.itemCount}</small>
                  </button>
                ))}
              </div>
            </>
          )}
          <div className="pc-motion">
            <button className={`chip ${picker.kind === "trending" ? "active" : ""}`} onClick={() => void loadFeatured("trending")}>{t("Em alta")}</button>
            <button className={`chip ${picker.kind === "popular" ? "active" : ""}`} onClick={() => void loadFeatured("popular")}>{t("Populares")}</button>
          </div>
          <div className="chip-select">
            {[...picker.results, ...picker.featured].map((list, index) => (
              <button key={`${list.id}-${index}`} className="chip" data-testid="trakt-pick-list" disabled={picker.busy} onClick={() => addLists([list])}>
                {list.title} <small className="muted small">{list.itemCount}</small>
              </button>
            ))}
          </div>
          {picker.error && <p className="row-error" role="alert">{picker.error}</p>}
        </div>
      )}
      <div className="modal-actions">
        <button className="primary" data-testid="save-collection" disabled={!draft.title.trim()} onClick={() => onSave(draft)}>{t("Salvar")}</button>
        <button className="ghost" onClick={onCancel}>{t("Cancelar")}</button>
      </div>
    </div>
  );
}

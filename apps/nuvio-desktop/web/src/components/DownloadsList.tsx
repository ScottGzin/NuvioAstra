// Biblioteca → Downloads: progress, speed and connections of each download, pause/resume, watch,
// show in the folder and remove.

import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { t } from "../i18n";
import { usePlayer } from "../player/PlayerContext";
import { message } from "../rustErrors";
import { sizeText } from "../sources";
import { listen } from "../tauri";
import { showToast } from "../toast";
import type { DownloadItem } from "../types";

const STATUS: Record<string, string> = {
  queued: "Na fila",
  preparing: "Preparando…",
  downloading: "Baixando",
  paused: "Pausado",
  done: "Concluído",
  failed: "Falhou",
};

const KIND: Record<string, string> = { http: "Link direto", torbox: "TorBox", p2p: "Torrent (P2P)" };

function remaining(seconds: number): string {
  if (seconds >= 3600) return `${Math.floor(seconds / 3600)} h ${Math.round((seconds % 3600) / 60)} min`;
  return seconds >= 60 ? `${Math.round(seconds / 60)} min` : `${Math.round(seconds)} s`;
}

export function DownloadsList() {
  const [items, setItems] = useState<DownloadItem[] | null>(null);
  const [folder, setFolder] = useState<string | null>(null);
  const player = usePlayer();

  useEffect(() => {
    api.downloadsList().then(setItems, () => setItems([]));
    api.downloadsFolder().then((f) => setFolder(f.folder), () => {});
    return listen<DownloadItem[]>("downloads", setItems);
  }, []);

  const act = (action: () => Promise<unknown>) => void action().catch((e) => showToast(errorText(e), true));

  const watch = (item: DownloadItem) =>
    void player.play(() => api.downloadPlay(item.id, null), { title: item.title, subtitle: item.subtitle ?? undefined, view: "full" });

  if (!items) return <div className="screen-message">{t("Carregando…")}</div>;

  return (
    <div className="downloads library-pad" data-testid="downloads">
      <div className="downloads-folder muted small">
        {t("Pasta:")} <code className="path">{folder}</code>
        <span> · {t("muda em Configurações → Downloads")}</span>
      </div>
      {!items.length && (
        <p className="muted">{t("Nenhum download. Clique com o botão direito numa fonte e escolha Baixar, ou use o botão de baixar no player.")}</p>
      )}
      {items.map((item) => {
        const pct = item.total ? Math.min(100, (item.downloaded / item.total) * 100) : 0;
        const left = item.total && item.speed > 0 ? Math.max(0, (item.total - item.downloaded) / item.speed) : null;
        return (
          <div key={item.id} className={`download-row ${item.status}`} data-id={item.id} data-status={item.status}>
            {item.poster ? <img className="download-art" src={item.poster} alt="" loading="lazy" /> : <span className="download-art" />}
            <div className="download-main">
              <div className="download-title">{item.title}{item.subtitle ? <span className="muted"> · {item.subtitle}</span> : null}</div>
              <div className="muted small download-file">{item.fileName ?? t("definindo o nome do arquivo…")}</div>
              <div className="progress download-progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(pct)}>
                <div style={{ width: `${item.status === "done" ? 100 : pct}%` }} />
              </div>
              <div className="small download-status">
                <b>{STATUS[item.status] ? t(STATUS[item.status]) : item.status}</b>
                {item.total ? ` · ${t("{done} de {total}", { done: sizeText(item.downloaded), total: sizeText(item.total) })}` : item.downloaded ? ` · ${sizeText(item.downloaded)}` : ""}
                {item.status === "downloading" && item.speed > 0 && ` · ${sizeText(item.speed)}/s`}
                {item.status === "downloading" && item.connections > 1 && ` · ${t("{count} conexões", { count: item.connections })}`}
                {left !== null && item.status === "downloading" && ` · ${t("faltam {time}", { time: remaining(left) })}`}
                {` · ${KIND[item.kind] ? t(KIND[item.kind]) : item.kind} · ${item.sourceName}`}
              </div>
              {item.error && <div className="row-error small">{message(item.error)}</div>}
            </div>
            <div className="download-actions">
              {item.status === "done" && <button className="primary" onClick={() => watch(item)}>{t("▶ Assistir")}</button>}
              {["downloading", "preparing", "queued"].includes(item.status) && <button className="ghost" onClick={() => act(() => api.downloadPause(item.id))}>{t("Pausar")}</button>}
              {["paused", "failed"].includes(item.status) && <button className="ghost" onClick={() => act(() => api.downloadResume(item.id))}>{item.status === "failed" ? t("Tentar de novo") : t("Retomar")}</button>}
              <button className="ghost" onClick={() => act(() => api.downloadShow(item.id))}>{t("Mostrar na pasta")}</button>
              <button
                className="ghost"
                onClick={() => {
                  const done = item.status === "done";
                  const deleteFile = done
                    ? window.confirm(t("Apagar também o arquivo baixado?\n\n{file}\n\nOK apaga o arquivo; Cancelar só tira da lista.", { file: item.fileName ?? "" }))
                    : window.confirm(t("Cancelar este download? O que já foi baixado será apagado."));
                  if (!done && !deleteFile) return;
                  act(() => api.downloadRemove(item.id, deleteFile));
                }}
              >
                {item.status === "done" ? t("Remover") : t("Cancelar")}
              </button>
            </div>
          </div>
        );
      })}
    </div>
  );
}

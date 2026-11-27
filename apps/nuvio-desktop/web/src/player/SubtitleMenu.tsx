// Subtitles inside the player: always there, whatever the file has. t("Legendas") chooses among the
// file's, the stream's and the addons'; "Ajustes" changes delay and look without leaving the video.

import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { PlayerSnapshot, ProfileSettings, SubtitleSettings } from "../types";
import { useSubtitles } from "./subtitles";
import { decimal, t } from "../i18n";

const SAVE_MS = 350;
/** Values the hidden section is laid out with until the settings arrive. */
const PLACEHOLDER = { fontSizeSp: 20, bottomOffset: 0, bold: false, outlineEnabled: false } as SubtitleSettings;

const CcIcon = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    <rect x="3" y="5" width="18" height="14" rx="3" />
    <path d="M10 10.2a2.4 2.4 0 1 0 0 3.6M17.2 10.2a2.4 2.4 0 1 0 0 3.6" />
  </svg>
);

/** Delay, size, position: saved in the profile and applied to what is playing. */
function Adjust({ snapshot }: { snapshot: PlayerSnapshot }) {
  const [style, setStyle] = useState<SubtitleSettings | null>(null);
  const base = useRef<{ settings: ProfileSettings; profile: number } | null>(null);
  const unsaved = useRef<SubtitleSettings | null>(null);
  const timer = useRef(0);
  const saving = useRef<Promise<void>>(Promise.resolve());

  useEffect(() => {
    let cancelled = false;
    api.settingsGet().then((view) => {
      if (cancelled) return;
      base.current = { settings: view.settings, profile: view.profile.index };
      setStyle(view.settings.player.subtitle);
    }, () => {});
    return () => {
      cancelled = true;
    };
  }, []);

  const save = useCallback(() => {
    window.clearTimeout(timer.current);
    const next = unsaved.current;
    unsaved.current = null;
    if (!next) return;
    saving.current = saving.current
      .then(async () => {
        const before = base.current;
        if (!before) return;
        const edited = { ...before.settings, player: { ...before.settings.player, subtitle: next } };
        const saved = await api.settingsSet(edited, before.settings, before.profile);
        base.current = { settings: saved, profile: before.profile };
        await api.playerSubtitleStyle();
      })
      .catch(() => {});
  }, []);
  useEffect(() => save, [save]);

  const change = (patch: Partial<SubtitleSettings>) => {
    if (!style) return;
    const next = { ...style, ...patch };
    setStyle(next);
    unsaved.current = next;
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(save, SAVE_MS);
  };

  const delay = (ms: number) => void api.playerCommand({ type: "setSubtitleDelay", ms: Math.max(-60000, Math.min(60000, ms)) });
  const [sync, setSync] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);
  const autoSync = async () => {
    setSyncing(true);
    setSync(null);
    try {
      const r = await api.playerSubtitleAutosync(snapshot.session);
      setSync(
        r.status === "applied"
          ? t("Sincronizada: atraso de {value} s.", { value: decimal((r.offsetMs ?? 0) / 1000, 2) })
          : r.status === "no-reference"
            ? t("O vídeo não tem legenda embutida para comparar. Ajuste o atraso acima.")
            : r.status === "no-match"
              ? t("Não foi possível achar o atraso com segurança. Ajuste acima.")
              : r.status === "not-addon"
                ? t("Esta legenda é do próprio vídeo e já está sincronizada.")
                : t("Escolha uma legenda primeiro."),
      );
    } catch (e) {
      setSync(String(e));
    } finally {
      setSyncing(false);
    }
  };

  return (
    <>
      <span className="pc-enhance-title">{t("Atraso da legenda")}</span>
      <div className="pc-delay">
        <button className="pc-level" aria-label={t("Adiantar legenda")} onClick={() => delay(snapshot.subDelayMs - 250)}>−{decimal(0.25, 2)} s</button>
        <output data-testid="subtitle-delay">{decimal(snapshot.subDelayMs / 1000, 2)} s</output>
        <button className="pc-level" aria-label={t("Atrasar legenda")} onClick={() => delay(snapshot.subDelayMs + 250)}>+{decimal(0.25, 2)} s</button>
        <button className="pc-level" disabled={!snapshot.subDelayMs} onClick={() => delay(0)}>{t("Zerar")}</button>
      </div>
      <button className="pc-level pc-autosync" disabled={syncing} onClick={() => void autoSync()} data-testid="subtitle-autosync">
        {syncing ? t("Sincronizando…") : t("Sincronizar automaticamente")}
      </button>
      {sync && <p className="pc-enhance-note" data-testid="subtitle-autosync-result">{sync}</p>}
      {/* Before the settings arrive the section keeps its place (hidden), so the menu doesn't grow
          upward under the pointer. */}
      {(() => {
        const shown = style ?? PLACEHOLDER;
        return (
        <div className="pc-sub-look" style={style ? undefined : { visibility: "hidden" }} aria-hidden={!style || undefined}>
          <span className="pc-enhance-title">{t("Aparência")}</span>
          <label className="pc-sub-slider">
            <span>{t("Tamanho")}</span>
            <input type="range" min={6} max={40} value={shown.fontSizeSp} aria-label={t("Tamanho da legenda")} onChange={(e) => change({ fontSizeSp: Number(e.target.value) })} />
            <output>{shown.fontSizeSp}</output>
          </label>
          <label className="pc-sub-slider">
            <span>{t("Altura")}</span>
            <input type="range" min={0} max={200} value={shown.bottomOffset} aria-label={t("Altura da legenda")} onChange={(e) => change({ bottomOffset: Number(e.target.value) })} />
            <output>{shown.bottomOffset}</output>
          </label>
          <div className="pc-motion">
            <button className={`pc-level ${shown.bold ? "active" : ""}`} aria-pressed={shown.bold} onClick={() => change({ bold: !shown.bold })}>{t("Negrito")}</button>
            <button className={`pc-level ${shown.outlineEnabled ? "active" : ""}`} aria-pressed={shown.outlineEnabled} onClick={() => change({ outlineEnabled: !shown.outlineEnabled })}>{t("Contorno")}</button>
          </div>
          <p className="pc-enhance-note">{t("Fonte, cores e fundo ficam em Configurações → Legendas.")}</p>
        </div>
        );
      })()}
    </>
  );
}

export function SubtitleMenu({ snapshot }: { snapshot: PlayerSnapshot }) {
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<"list" | "adjust">("list");
  const root = useRef<HTMLDivElement>(null);
  const { list, groups, searching, loading, error, choose } = useSubtitles(snapshot, open);
  const off = !snapshot.sid || snapshot.sid === "no";

  useEffect(() => {
    if (!open) return;
    const outside = (e: PointerEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    const escape = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation(); // closes the menu, not the player
      setOpen(false);
    };
    window.addEventListener("pointerdown", outside, true);
    window.addEventListener("keydown", escape, true);
    return () => {
      window.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("keydown", escape, true);
    };
  }, [open]);

  let empty: string | null = null;
  if (!snapshot.loaded) empty = t("Aguardando o vídeo carregar…");
  else if (!searching && !error && !groups.length) {
    empty = list?.failed
      ? t("Os addons de legenda não responderam. Abra de novo para tentar outra vez.")
      : list?.addons
        ? t("Nenhuma legenda encontrada para este vídeo.")
        : t("Este vídeo não traz legendas. Para buscar outras, instale um addon de legendas em Configurações → Addons.");
  }

  return (
    <div className="pc-subtitles" ref={root}>
      <button className={`pc-icon ${off ? "" : "pc-on"}`} aria-label={t("Legendas")} title={t("Legendas")} aria-expanded={open} data-testid="subtitle-button" onClick={() => setOpen((v) => !v)}>
        {CcIcon}
      </button>
      {open && (
        <div className="pc-enhance-menu" data-testid="subtitle-menu">
          <div className="pc-tabs" role="tablist">
            <button role="tab" aria-selected={tab === "list"} className={`pc-tab ${tab === "list" ? "active" : ""}`} onClick={() => setTab("list")}>{t("Legendas")}</button>
            <button role="tab" aria-selected={tab === "adjust"} className={`pc-tab ${tab === "adjust" ? "active" : ""}`} data-testid="subtitle-adjust" onClick={() => setTab("adjust")}>{t("Ajustes")}</button>
          </div>
          <div className="pc-tab-body">
            {tab === "adjust" ? (
              <Adjust snapshot={snapshot} />
            ) : (
              <>
                <button className={`pc-level ${off && !loading ? "active" : ""}`} aria-pressed={off} data-subtitle="off" onClick={() => void choose(null)}>
                  <b>{t("Desativadas")}</b>
                </button>
                {groups.map((group) => (
                  <div key={group.language} className="pc-sub-group" data-language={group.language}>
                    <span className="pc-enhance-title">{group.name}</span>
                    {group.choices.map((choice) => {
                      const active = choice.track !== undefined && String(choice.track.id) === snapshot.sid;
                      return (
                        <button key={choice.id} className={`pc-level ${active ? "active" : ""}`} aria-pressed={active} disabled={loading !== null} data-subtitle={choice.id} onClick={() => void choose(choice)}>
                          <b>{choice.label}</b>
                          <small>{loading === choice.id ? t("Carregando…") : choice.detail}</small>
                        </button>
                      );
                    })}
                  </div>
                ))}
                {searching && snapshot.loaded && <p className="pc-enhance-note">{t("Buscando legendas nos addons…")}</p>}
                {empty && <p className="pc-enhance-note">{empty}</p>}
                {error && <p className="pc-enhance-note pc-sub-error" role="alert">{error}</p>}
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

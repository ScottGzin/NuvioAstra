// Settings → Legendas, appearance (`SubtitleAppearanceSettings` + `SubtitleFontPicker` of the
// original, with more options): styles ready to use, every font of Windows and imported ones,
// colors, outline, box, shadow, spacing, position, and the style applied to anime (ASS) subtitles.

import { useEffect, useMemo, useState, type CSSProperties } from "react";
import { api, errorText } from "../api";
import type { SubtitleFontList, SubtitleSettings } from "../types";
import { ColorField, Row, Slider, Toggle } from "./SettingControls";
import { t } from "../i18n";

/** Shown first when installed. Arial has Helvetica's metrics; Helvetica itself is not in Windows. */
const FEATURED = [
  "Helvetica", "Helvetica Neue", "Arial", "Segoe UI", "Verdana", "Tahoma", "Trebuchet MS", "Calibri", "Candara", "Corbel",
  "Franklin Gothic Medium", "Bahnschrift", "Gadugi", "Roboto", "Inter", "Open Sans", "Noto Sans", "Georgia", "Cambria",
  "Times New Roman", "Consolas", "Comic Sans MS",
];
const BUILT_IN: [string, string][] = [["default", "Padrão"], ["sans", "Sem serifa"], ["serif", "Com serifa"], ["mono", "Monoespaçada"]];

const LOOK: Partial<SubtitleSettings> = {
  fontId: "default", textColor: "#FFFFFFFF", backgroundColor: "#00000000", outlineColor: "#FF000000", outlineEnabled: true, outlineWidth: 2,
  bold: false, italic: false, shadowEnabled: false, shadowOpacity: 75, shadowSize: 2, spacing: 0, blur: 0,
};
const PRESETS: { label: string; look: Partial<SubtitleSettings> }[] = [
  { label: "Padrão", look: LOOK },
  { label: "Streaming", look: { ...LOOK, fontId: "system:Arial", bold: true, outlineWidth: 1, shadowEnabled: true } },
  { label: "Anime", look: { ...LOOK, fontId: "system:Trebuchet MS", bold: true, outlineWidth: 3, shadowEnabled: true, shadowOpacity: 60, shadowSize: 1 } },
  { label: "Amarela clássica", look: { ...LOOK, fontId: "system:Arial", textColor: "#FFFFE14D", bold: true } },
  { label: "Caixa escura", look: { ...LOOK, fontId: "system:Segoe UI", backgroundColor: "#B3000000", outlineEnabled: false } },
  { label: "Cinema", look: { ...LOOK, fontId: "system:Georgia", textColor: "#FFF4ECD8", outlineWidth: 1, shadowEnabled: true, shadowOpacity: 85 } },
];

const rgba = (argb: string) => `rgba(${parseInt(argb.slice(3, 5), 16)},${parseInt(argb.slice(5, 7), 16)},${parseInt(argb.slice(7, 9), 16)},${parseInt(argb.slice(1, 3), 16) / 255})`;

function cssFamily(id: string): string {
  if (id.startsWith("system:")) return `"${id.slice(7).replace(/["\\]/g, "")}", Arial, sans-serif`;
  if (id.startsWith("custom-")) return `"${id}", Arial, sans-serif`;
  return id === "serif" ? '"Times New Roman", serif' : id === "mono" ? '"Courier New", monospace' : "Arial, sans-serif";
}

/** An imported file becomes a page font, for the preview only. */
function usePreviewFont(id: string) {
  const [, setLoaded] = useState("");
  useEffect(() => {
    if (!id.startsWith("custom-")) return;
    let face: FontFace | null = null;
    let cancelled = false;
    api.subtitleFontData(id).then(async (bytes) => {
      const next = new FontFace(id, bytes);
      await next.load();
      if (cancelled) return;
      document.fonts.add(next);
      face = next;
      setLoaded(id);
    }).catch(() => {});
    return () => {
      cancelled = true;
      if (face) document.fonts.delete(face);
    };
  }, [id]);
}

interface Props {
  value: SubtitleSettings;
  onChange(change: (s: SubtitleSettings) => SubtitleSettings): void;
}

export function SubtitleAppearance({ value: sub, onChange }: Props) {
  const [fonts, setFonts] = useState<SubtitleFontList | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  usePreviewFont(sub.fontId);

  useEffect(() => {
    api.subtitleFonts().then(setFonts, (e) => setError(errorText(e)));
  }, []);

  const featured = useMemo(() => FEATURED.filter((f) => fonts?.system.includes(f)), [fonts]);
  const others = useMemo(() => fonts?.system.filter((f) => !FEATURED.includes(f)) ?? [], [fonts]);
  const imported = fonts?.imported.find((f) => f.id === sub.fontId);
  const missing =
    fonts &&
    ((sub.fontId.startsWith("custom-") && !imported) || (sub.fontId.startsWith("system:") && !fonts.system.includes(sub.fontId.slice(7))));
  const boxed = !sub.backgroundColor.startsWith("#00");
  const set = (patch: Partial<SubtitleSettings>) => onChange((s) => ({ ...s, ...patch }));

  const importFont = async () => {
    setBusy(true);
    setError(null);
    try {
      const font = await api.subtitleFontImport();
      if (font) {
        setFonts(await api.subtitleFonts());
        set({ fontId: font.id });
      }
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const removeFont = async () => {
    if (!imported || !window.confirm(t("Remover “{name}” das fontes do Nuvio Astra? O arquivo original não é apagado.", { name: imported.name }))) return;
    try {
      await api.subtitleFontRemove(imported.id);
      set({ fontId: "default" });
      setFonts(await api.subtitleFonts());
    } catch (e) {
      setError(errorText(e));
    }
  };

  const style: CSSProperties = {
    fontFamily: cssFamily(sub.fontId),
    color: rgba(sub.textColor),
    fontSize: `${Math.max(11, sub.fontSizeSp * 1.3)}px`,
    fontWeight: sub.bold ? 700 : 400,
    fontStyle: sub.italic ? "italic" : "normal",
    letterSpacing: `${sub.spacing / 2}px`,
    background: rgba(sub.backgroundColor),
    WebkitTextStroke: !boxed && sub.outlineEnabled ? `${sub.outlineWidth * 1.4}px ${rgba(sub.outlineColor)}` : undefined,
    paintOrder: "stroke fill",
    textShadow: !boxed && sub.shadowEnabled ? `${sub.shadowSize}px ${sub.shadowSize}px ${sub.blur / 2}px rgba(0,0,0,${sub.shadowOpacity / 100})` : undefined,
    bottom: `${8 + sub.bottomOffset / 4}px`,
    textAlign: sub.align,
    ...(sub.align === "left" ? { left: 16, transform: "none" } : sub.align === "right" ? { left: "auto", right: 16, transform: "none" } : {}),
  };

  return (
    <>
      <div className="subtitle-preview subtitle-preview-large" aria-hidden>
        <span style={style} data-testid="subtitle-preview">{t("Exemplo de legenda")}<br />{t("ação, coração, amanhã")}</span>
      </div>
      <div className="subtitle-presets" aria-label={t("Estilos prontos")}>
        {PRESETS.map((preset) => (
          <button key={preset.label} className="chip" data-preset={preset.label} onClick={() => set(preset.look)}>{t(preset.label)}</button>
        ))}
      </div>

      <Row title={t("Fonte")} description={t("Escolha entre as fontes do Windows ou importe um arquivo TTF ou OTF. A Arial é a mais parecida com a Helvetica.")}>
        <span className="font-field">
          <select aria-label={t("Fonte da legenda")} data-setting="subtitleFont" value={sub.fontId} style={{ fontFamily: cssFamily(sub.fontId) }} onChange={(e) => set({ fontId: e.target.value })}>
            <optgroup label={t("Básicas")}>
              {BUILT_IN.map(([id, label]) => <option key={id} value={id}>{t(label)}</option>)}
            </optgroup>
            {!!fonts?.imported.length && (
              <optgroup label={t("Importadas")}>
                {fonts.imported.map((f) => <option key={f.id} value={f.id}>{f.name}</option>)}
              </optgroup>
            )}
            {featured.length > 0 && (
              <optgroup label={t("Recomendadas")}>
                {featured.map((f) => <option key={f} value={`system:${f}`} style={{ fontFamily: cssFamily(`system:${f}`) }}>{f}</option>)}
              </optgroup>
            )}
            {others.length > 0 && (
              <optgroup label={t("Todas do Windows")}>
                {others.map((f) => <option key={f} value={`system:${f}`}>{f}</option>)}
              </optgroup>
            )}
            {(missing || !fonts) && !BUILT_IN.some(([id]) => id === sub.fontId) && <option value={sub.fontId}>{sub.fontId.startsWith("system:") ? sub.fontId.slice(7) : t("Fonte importada")}</option>}
          </select>
          <button className="ghost" data-testid="font-import" disabled={busy} onClick={() => void importFont()}>{busy ? "Importando…" : "Importar fonte"}</button>
          {imported && <button className="ghost" data-testid="font-remove" onClick={() => void removeFont()}>{t("Remover")}</button>}
        </span>
      </Row>
      {missing && <p className="muted small">{t("Esta fonte não está neste computador: a legenda usa a padrão.")}</p>}
      {error && <div className="row-error">{error}</div>}

      <Row title={t("Aplicar meu estilo em legendas de anime (ASS)")} description={t("Legendas ASS trazem fonte e cores próprias e ignoram estes ajustes. Ligado, usam o seu estilo, mas placas e músicas legendadas podem perder o visual original.")}>
        <Toggle label={t("Aplicar meu estilo em legendas ASS")} checked={sub.overrideAss} onChange={(v) => set({ overrideAss: v })} />
      </Row>
      <Row title={t("Tamanho")}><Slider label={t("Tamanho da legenda")} value={sub.fontSizeSp} min={6} max={40} onChange={(v) => set({ fontSizeSp: v })} /></Row>
      <Row title={t("Distância da borda inferior")}><Slider label={t("Distância da borda inferior")} value={sub.bottomOffset} min={0} max={200} onChange={(v) => set({ bottomOffset: v })} /></Row>
      <Row title={t("Alinhamento")}>
        <select aria-label={t("Alinhamento da legenda")} value={sub.align} onChange={(e) => set({ align: e.target.value as SubtitleSettings["align"] })}>
          <option value="left">{t("Esquerda")}</option>
          <option value="center">{t("Centro")}</option>
          <option value="right">{t("Direita")}</option>
        </select>
      </Row>
      <Row title={t("Cor do texto")}><ColorField label={t("Cor do texto")} withOpacity value={sub.textColor} onChange={(v) => set({ textColor: v })} /></Row>
      <Row title={t("Negrito")}><Toggle label={t("Negrito")} checked={sub.bold} onChange={(v) => set({ bold: v })} /></Row>
      <Row title={t("Itálico")}><Toggle label={t("Itálico")} checked={sub.italic} onChange={(v) => set({ italic: v })} /></Row>
      <Row title={t("Espaço entre letras")}><Slider label={t("Espaço entre letras")} value={sub.spacing} min={0} max={10} onChange={(v) => set({ spacing: v })} /></Row>

      <Row title={t("Fundo em caixa")} description={t("Com a caixa ligada, ela substitui o contorno e a sombra.")}>
        <Toggle label={t("Fundo em caixa")} checked={boxed} onChange={(v) => set({ backgroundColor: `#${v ? "B3" : "00"}${sub.backgroundColor.slice(3)}` })} />
      </Row>
      {boxed ? (
        <Row title={t("Cor do fundo")}><ColorField label={t("Cor de fundo")} withOpacity value={sub.backgroundColor} onChange={(v) => set({ backgroundColor: v })} /></Row>
      ) : (
        <>
          <Row title={t("Contorno")}><Toggle label={t("Contorno")} checked={sub.outlineEnabled} onChange={(v) => set({ outlineEnabled: v })} /></Row>
          {sub.outlineEnabled && (
            <>
              <Row title={t("Cor do contorno")}><ColorField label={t("Cor do contorno")} withOpacity value={sub.outlineColor} onChange={(v) => set({ outlineColor: v })} /></Row>
              <Row title={t("Espessura do contorno")}><Slider label={t("Espessura do contorno")} value={sub.outlineWidth} min={1} max={5} onChange={(v) => set({ outlineWidth: v })} /></Row>
            </>
          )}
          <Row title={t("Sombra")}><Toggle label={t("Sombra")} checked={sub.shadowEnabled} onChange={(v) => set({ shadowEnabled: v })} /></Row>
          {sub.shadowEnabled && (
            <>
              <Row title={t("Opacidade da sombra")}><Slider label={t("Opacidade da sombra")} value={sub.shadowOpacity} min={0} max={100} unit="%" onChange={(v) => set({ shadowOpacity: v })} /></Row>
              <Row title={t("Tamanho da sombra")}><Slider label={t("Tamanho da sombra")} value={sub.shadowSize} min={1} max={8} onChange={(v) => set({ shadowSize: v })} /></Row>
            </>
          )}
          <Row title={t("Suavizar borda")}><Slider label={t("Suavizar borda")} value={sub.blur} min={0} max={10} onChange={(v) => set({ blur: v })} /></Row>
        </>
      )}
    </>
  );
}

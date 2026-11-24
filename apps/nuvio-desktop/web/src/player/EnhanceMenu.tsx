// "Melhorar imagem" during playback: level and Anime4K mode, effects, colors, motion. Changes apply
// at once and are saved in the profile by Rust; the watchdog may lower them (`autoChanges`), which
// is told on screen. "Segurar para comparar" shows the original picture while held.

import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { AnimeModeName, EnhancementName, MotionName, PictureSettings, PlayerSnapshot } from "../types";
import { t } from "../i18n";
import { FpsMeter } from "./FpsMeter";

export const ENHANCEMENTS: { value: EnhancementName; label: string; hint: string }[] = [
  { value: "off", label: "Desligado", hint: "Imagem original" },
  { value: "movies", label: "Filmes e séries", hint: "Tira faixas dos degradês, melhora HDR em tela comum" },
  { value: "animeFast", label: "Anime rápido", hint: "Anime4K com redes leves" },
  { value: "animeQuality", label: "Anime qualidade", hint: "Anime4K com redes grandes, pede placa de vídeo" },
];

export const MOTIONS: { value: MotionName; label: string; hint: string }[] = [
  { value: "off", label: "Original", hint: "Como o vídeo veio (normalmente 24 quadros)" },
  { value: "steady", label: "Sem tremidas", hint: "Acerta o ritmo no seu monitor, mantém a cara do anime" },
  { value: "fluid", label: "Fluido", hint: "Mistura quadros até a taxa do monitor (60, 120, 165 Hz); pode deixar rastros" },
];

export const ANIME_MODES: { value: AnimeModeName; label: string; hint: string }[] = [
  { value: "auto", label: "Automático", hint: "Escolhe pela resolução: A em 1080p, B em 720p, A ou A+A abaixo." },
  { value: "a", label: "Modo A", hint: "Anime 1080p, borrado ou muito comprimido: reconstrói os traços." },
  { value: "b", label: "Modo B", hint: "Anime 720p ou com serrilhado e halos: restauração mais suave." },
  { value: "c", label: "Modo C", hint: "Anime limpo em baixa resolução: amplia e tira ruído." },
  { value: "aa", label: "Modo A+A", hint: "A com mais força, para SD. Em 1080p costuma exagerar." },
  { value: "bb", label: "Modo B+B", hint: "B com mais força, para vídeo com metade da tela ou menos." },
  { value: "ca", label: "Modo C+A", hint: "C com traços reconstruídos, para baixa resolução." },
];

type Effect = "denoise" | "deblur" | "darken" | "thin" | "deband";
const EFFECTS: { key: Effect; label: string; hint: string }[] = [
  { key: "deblur", label: "Nitidez", hint: "Tira o borrado sem criar halo" },
  { key: "darken", label: "Escurecer traços", hint: "Contornos mais marcados" },
  { key: "thin", label: "Afinar traços", hint: "Linhas mais finas" },
  { key: "denoise", label: "Reduzir ruído", hint: "Limpa granulado e compressão" },
  { key: "deband", label: "Suavizar degradês", hint: "Tira faixas de cor no céu e nas sombras" },
];
const STEPS = ["Não", "Leve", "Média", "Forte"];

type Color = "brightness" | "contrast" | "saturation" | "gamma" | "hue";
const COLORS: { key: Color; label: string }[] = [
  { key: "brightness", label: "Brilho" },
  { key: "contrast", label: "Contraste" },
  { key: "saturation", label: "Saturação" },
  { key: "gamma", label: "Gama" },
  { key: "hue", label: "Matiz" },
];

const COLOR_PRESETS: { label: string; values: Record<Color, number> }[] = [
  { label: "Natural", values: { brightness: 0, contrast: 0, saturation: 0, gamma: 0, hue: 0 } },
  { label: "Vívido", values: { brightness: 0, contrast: 6, saturation: 16, gamma: 0, hue: 0 } },
  { label: "Cinema", values: { brightness: -2, contrast: 8, saturation: -4, gamma: -3, hue: 0 } },
  { label: "Cenas escuras", values: { brightness: 4, contrast: 2, saturation: 2, gamma: 10, hue: 0 } },
];

export const PLAIN_PICTURE: PictureSettings = {
  mode: "auto", denoise: 0, deblur: 0, thin: 0, darken: 0, deband: 0, brightness: 0, contrast: 0, saturation: 0, gamma: 0, hue: 0,
};

const TABS = [["quality", "Qualidade"], ["effects", "Efeitos"], ["color", "Cor"], ["motion", "Fluidez"]] as const;
type TabId = (typeof TABS)[number][0];

const samePicture = (a: PictureSettings, b: PictureSettings) => (Object.keys(PLAIN_PICTURE) as (keyof PictureSettings)[]).every((k) => a[k] === b[k]);
/** Dragging a slider sends at most this often. */
const SEND_MS = 120;

export function EnhanceMenu({ snapshot }: { snapshot: PlayerSnapshot }) {
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<TabId>("quality");
  const [chosenMotion, setMotion] = useState<MotionName | null>(null);
  const [chosenLevel, setLevel] = useState<EnhancementName | null>(null);
  const [chosenPicture, setPicture] = useState<PictureSettings | null>(null);
  const [comparing, setComparing] = useState(false);
  const unsent = useRef<PictureSettings | null>(null);
  const timer = useRef(0);
  const comparingNow = useRef(false);

  // What the player reports replaces the local choice (also when the watchdog lowered it). The
  // picture keeps it longer: confirmations of a slider arrive behind the hand.
  useEffect(() => setMotion(null), [snapshot.motion, snapshot.autoChanges]);
  useEffect(() => setLevel(null), [snapshot.enhancement, snapshot.autoChanges]);
  useEffect(() => setPicture(null), [snapshot.autoChanges]);

  const send = useCallback(() => {
    window.clearTimeout(timer.current);
    timer.current = 0;
    if (unsent.current) void api.playerCommand({ type: "setPicture", picture: unsent.current });
    unsent.current = null;
  }, []);

  const compare = useCallback((active: boolean) => {
    if (comparingNow.current === active) return;
    comparingNow.current = active;
    setComparing(active);
    void api.playerCommand({ type: "setCompare", active });
  }, []);

  useEffect(() => {
    if (!comparing) return;
    const release = () => compare(false);
    window.addEventListener("pointerup", release);
    window.addEventListener("blur", release);
    return () => {
      window.removeEventListener("pointerup", release);
      window.removeEventListener("blur", release);
    };
  }, [comparing, compare]);

  useEffect(
    () => () => {
      send();
      compare(false);
    },
    [send, compare],
  );

  const motion = chosenMotion ?? snapshot.motion;
  const level = chosenLevel ?? snapshot.enhancement;
  const picture = chosenPicture ?? snapshot.picture;
  const anime = level === "animeFast" || level === "animeQuality";
  const changed = level !== "off" || !samePicture(picture, { ...PLAIN_PICTURE, mode: picture.mode });
  const native = snapshot.height >= window.innerHeight * window.devicePixelRatio * 0.9;

  const change = (next: PictureSettings) => {
    setPicture(next);
    unsent.current = next;
    if (!timer.current) timer.current = window.setTimeout(send, SEND_MS);
  };

  return (
    <div className="pc-enhance">
      <button
        className={`pc-icon ${motion !== "off" || changed ? "pc-on" : ""}`}
        aria-label={t("Melhorar imagem")}
        title={t("Melhorar imagem")}
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
          <path d="M4 7h10M18 7h2M4 17h2M10 17h10" />
          <circle cx="16" cy="7" r="2" />
          <circle cx="8" cy="17" r="2" />
        </svg>
      </button>
      {open && (
        <div className="pc-enhance-menu">
          <div className="pc-tabs" role="tablist">
            {TABS.map(([id, label]) => (
              <button key={id} role="tab" aria-selected={tab === id} className={`pc-tab ${tab === id ? "active" : ""}`} data-tab-id={id} onClick={() => setTab(id)}>{t(label)}</button>
            ))}
          </div>
          <div className="pc-tab-body">
            {tab === "quality" && (
              <>
                {ENHANCEMENTS.map((option) => (
                  <button
                    key={option.value}
                    className={`pc-level ${level === option.value ? "active" : ""}`}
                    data-level={option.value}
                    aria-pressed={level === option.value}
                    onClick={() => {
                      setLevel(option.value);
                      void api.playerCommand({ type: "setEnhancement", enhancement: option.value });
                    }}
                  >
                    <b>{t(option.label)}</b>
                    <small>{t(option.hint)}</small>
                  </button>
                ))}
                {anime && (
                  <>
                    <span className="pc-enhance-title">{t("Modo do Anime4K")}</span>
                    <div className="pc-modes">
                      {ANIME_MODES.map((option) => (
                        <button
                          key={option.value}
                          className={`pc-level ${picture.mode === option.value ? "active" : ""}`}
                          data-mode={option.value}
                          aria-pressed={picture.mode === option.value}
                          title={t(option.hint)}
                          onClick={() => change({ ...picture, mode: option.value })}
                        >
                          {option.value.toUpperCase()}
                        </button>
                      ))}
                    </div>
                    <p className="pc-enhance-note">{t(ANIME_MODES.find((m) => m.value === picture.mode)?.hint ?? "")}</p>
                    {snapshot.width > 2048 ? (
                      <p className="pc-enhance-note">{t("Vídeo acima de 1080p: o Anime4K e os efeitos de traço não são aplicados.")}</p>
                    ) : native ? (
                      <p className="pc-enhance-note">{t("Este vídeo já tem a resolução da tela: aqui a mudança é sutil, mais nos traços. Os Efeitos aparecem mais.")}</p>
                    ) : null}
                  </>
                )}
              </>
            )}

            {tab === "effects" && (
              <>
                {EFFECTS.map((effect) => (
                  <div className="pc-effect" key={effect.key}>
                    <span><b>{t(effect.label)}</b><small>{t(effect.hint)}</small></span>
                    <div className="pc-steps">
                      {STEPS.map((label, value) => (
                        <button key={label} className={`pc-level ${picture[effect.key] === value ? "active" : ""}`} data-effect={`${effect.key}:${value}`} aria-pressed={picture[effect.key] === value} onClick={() => change({ ...picture, [effect.key]: value })}>
                          {t(label)}
                        </button>
                      ))}
                    </div>
                  </div>
                ))}
                <button className="pc-reset" data-reset="effects" onClick={() => change({ ...picture, denoise: 0, deblur: 0, thin: 0, darken: 0, deband: 0 })}>{t("Zerar efeitos")}</button>
              </>
            )}

            {tab === "color" && (
              <>
                <div className="pc-modes pc-presets">
                  {COLOR_PRESETS.map((preset) => {
                    const active = COLORS.every((c) => picture[c.key] === preset.values[c.key]);
                    return (
                      <button key={preset.label} className={`pc-level ${active ? "active" : ""}`} data-preset={preset.label} aria-pressed={active} onClick={() => change({ ...picture, ...preset.values })}>
                        {t(preset.label)}
                      </button>
                    );
                  })}
                </div>
                {COLORS.map(({ key, label }) => (
                  <label className="pc-color" key={key}>
                    <span>{t(label)}</span>
                    <input type="range" min={-50} max={50} value={picture[key]} data-color={key} onChange={(e) => change({ ...picture, [key]: Number(e.target.value) })} onPointerUp={send} />
                    <output>{picture[key]}</output>
                  </label>
                ))}
              </>
            )}

            {tab === "motion" && MOTIONS.map((option) => (
              <button
                key={option.value}
                className={`pc-level ${motion === option.value ? "active" : ""}`}
                data-motion={option.value}
                aria-pressed={motion === option.value}
                onClick={() => {
                  setMotion(option.value);
                  void api.playerCommand({ type: "setMotion", motion: option.value });
                }}
              >
                <b>{t(option.label)}</b>
                <small>{t(option.hint)}</small>
              </button>
            ))}
            {tab === "motion" && <FpsMeter snapshot={snapshot} chosen={motion} />}
          </div>
          {changed && tab !== "motion" && (
            <button
              className={`pc-compare ${comparing ? "active" : ""}`}
              data-compare=""
              onPointerDown={() => {
                send();
                compare(true);
              }}
            >
              {comparing ? t("Mostrando o original") : t("Segure para ver o original")}
            </button>
          )}
        </div>
      )}
    </div>
  );
}

/** Shown over the video (also with the controls hidden) when the watchdog lowered something. */
export function EnhanceNotice({ snapshot }: { snapshot: PlayerSnapshot | null }) {
  const changes = snapshot?.autoChanges ?? 0;
  const [shown, setShown] = useState(false);
  useEffect(() => {
    if (!changes) {
      setShown(false);
      return;
    }
    setShown(true);
    const timer = window.setTimeout(() => setShown(false), 7000);
    return () => window.clearTimeout(timer);
  }, [changes]);

  if (!shown || !snapshot) return null;
  const level = ENHANCEMENTS.find((e) => e.value === snapshot.enhancement)?.label;
  return (
    <div className="enhance-notice" role="status">
      {snapshot.autoReason === "motion"
        ? t("O movimento não sincronizou com este monitor: voltei para Original.")
        : snapshot.autoReason === "effects"
          ? t("A placa de vídeo não aguentou: desliguei os efeitos de imagem.")
          : t("A placa de vídeo não aguentou: melhoria de imagem reduzida para {level}.", { level: t(level ?? "") })}
    </div>
  );
}

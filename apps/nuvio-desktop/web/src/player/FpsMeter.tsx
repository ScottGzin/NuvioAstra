// Live frame rate in "Melhorar imagem" → Fluidez, so the motion modes can be judged by numbers:
// frames in the video, new frames on screen, the monitor's refresh and frames lost per second.
// "Na tela": with display sync + interpolation ("Fluido") every refresh draws a new blended frame;
// otherwise the screen shows the video's own frames (repeated to fill the refreshes).

import { useEffect, useRef, useState } from "react";
import type { MotionName, PlayerSnapshot } from "../types";
import { decimal, t } from "../i18n";

const rate = (n: number) => (n > 0 ? decimal(Math.round(n * 10) / 10) : "—");

/** Frames dropped or shown late in the last ~2 seconds, per second. */
function useLostPerSecond(snapshot: PlayerSnapshot): number {
  const samples = useRef<{ at: number; lost: number }[]>([]);
  const [perSecond, setPerSecond] = useState(0);
  const lost = snapshot.droppedFrames + snapshot.delayedFrames;
  useEffect(() => {
    const now = performance.now();
    const list = samples.current.filter((s) => now - s.at < 2000 && s.lost <= lost);
    list.push({ at: now, lost });
    samples.current = list;
    const first = list[0];
    const span = (now - first.at) / 1000;
    setPerSecond(span >= 0.5 ? (lost - first.lost) / span : 0);
  }, [lost, snapshot.timePos]);
  return perSecond;
}

export function FpsMeter({ snapshot, chosen }: { snapshot: PlayerSnapshot; chosen: MotionName }) {
  const lostPerSecond = useLostPerSecond(snapshot);
  const video = snapshot.fps * (snapshot.speed || 1);
  // Measured refresh while the video follows the monitor, otherwise the rate Windows reports.
  const monitor = snapshot.displayFps > 0 ? snapshot.displayFps : snapshot.displayRate;
  const blending = snapshot.motion === "fluid" && snapshot.displaySync;
  const shown = snapshot.paused ? 0 : blending ? monitor : video;
  const struggling = !snapshot.paused && lostPerSecond >= 1;

  let note: string;
  if (snapshot.paused) note = t("Dê play para medir.");
  else if (chosen !== "off" && !snapshot.displaySync) note = t("O ritmo do monitor ainda não foi acertado; o modo escolhido não está agindo agora.");
  else if (blending) note = t("Cada atualização do monitor recebe um quadro novo, misturado entre os do vídeo.");
  else if (snapshot.motion === "steady") note = t("Os quadros do vídeo aparecem em ritmo igual, sem quadros a mais.");
  else note = t("Quadros do vídeo como vieram, sem mistura.");

  return (
    <div className={`fps-meter ${struggling ? "warn" : ""}`} data-testid="fps-meter">
      <div className="fps-stats">
        <span><b data-fps="video">{rate(video)}</b><small>{t("Vídeo")}</small></span>
        <span className="fps-main"><b data-fps="shown">{snapshot.paused ? "—" : rate(shown)}</b><small>{t("Na tela")}</small></span>
        <span><b data-fps="display">{monitor > 0 ? `${Math.round(monitor)} Hz` : "—"}</b><small>{t("Monitor")}</small></span>
      </div>
      <p className="fps-note">{note}</p>
      {struggling && (
        <p className="fps-note fps-warn" data-fps="lost">
          {t("Perdendo {n} quadros por segundo: a placa de vídeo não está dando conta.", { n: Math.round(lostPerSecond) })}
        </p>
      )}
    </div>
  );
}

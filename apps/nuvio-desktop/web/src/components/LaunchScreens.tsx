// What the app may show right after it opens (the original's launch flow):
// - WhoIsWatching: "Quem está assistindo?" in full screen when there is more than one profile
//   (Settings → Perfil e dados can turn it off; kept on this computer).
// - ResumePrompt: "Continuar de onde parou?" for the newest unfinished movie/episode (Settings →
//   Continuar assistindo → Perguntar ao abrir), once per launch.

import { useEffect, useState, type CSSProperties } from "react";
import { api } from "../api";
import type { HistoryEntry, ProfilesView } from "../types";
import { Avatar } from "./ProfileMenu";
import { useAvatars } from "../account";
import { clock, episodeCode } from "../library";
import { screensNow, useScreens } from "../screens";
import { t } from "../i18n";

const CHOOSER_KEY = "nuvio.profileChooserOnLaunch";

export function chooserOnLaunch(): boolean {
  try {
    return localStorage.getItem(CHOOSER_KEY) !== "off";
  } catch {
    return true;
  }
}

export function setChooserOnLaunch(on: boolean) {
  try {
    localStorage.setItem(CHOOSER_KEY, on ? "on" : "off");
  } catch {
    /* this session only */
  }
}

export function WhoIsWatching({ view, onPick, onManage }: { view: ProfilesView; onPick(index: number): void; onManage(): void }) {
  const avatars = useAvatars();
  const [hover, setHover] = useState<number | null>(null);
  // The profile's own color: its avatar's background, else its color.
  const colorOf = (index: number | null) => {
    const p = view.profiles.find((x) => x.index === index) ?? view.profiles.find((x) => x.index === view.active) ?? view.profiles[0];
    const image = p?.avatarId ? avatars.find((a) => a.id === p.avatarId) : undefined;
    return image?.bgColor || p?.avatarColorHex || "#3d2a7a";
  };
  return (
    <div className="who-watching" data-testid="who-watching" style={{ "--who-tint": colorOf(hover) } as CSSProperties}>
      <img className="who-logo" src="./nuvio-astra-128.png" alt="Nuvio Astra" draggable={false} />
      <h1>{t("Quem está assistindo?")}</h1>
      <div className="who-profiles">
        {view.profiles.map((p) => (
          <button
            key={p.index}
            className={`who-profile ${p.index === view.active ? "current" : ""}`}
            style={{ "--who-color": colorOf(p.index) } as CSSProperties}
            onClick={() => onPick(p.index)}
            onMouseEnter={() => setHover(p.index)}
            onFocus={() => setHover(p.index)}
            data-profile={p.index}
          >
            <span className="who-ring"><Avatar profile={p} size={124} /></span>
            <span>{p.name}</span>
            {p.pinEnabled && <span className="who-lock" title={t("Com PIN")} aria-label={t("Com PIN")}>🔒</span>}
          </button>
        ))}
      </div>
      <button className="ghost who-manage" onClick={onManage}>{t("Gerenciar perfis")}</button>
    </div>
  );
}

let promptShown = false;

export function ResumePrompt({ onResume }: { onResume(entry: HistoryEntry): void }) {
  const screens = useScreens();
  const [entry, setEntry] = useState<HistoryEntry | null>(null);
  useEffect(() => {
    if (promptShown) return;
    let cancelled = false;
    // Wait for the profile's own preferences before deciding.
    const timer = window.setTimeout(() => {
      if (!screensNow().cwResumePrompt) return;
      api.libraryContinueWatching().then((items) => {
        if (cancelled || promptShown) return;
        const last = items.find((e) => e.inProgress && !e.nextUp);
        // Only something watched in the last two weeks.
        if (last && Date.now() - last.lastWatchedEpochMs < 14 * 86_400_000) {
          promptShown = true;
          setEntry(last);
        }
      }, () => {});
    }, 1200);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, []);
  if (!entry || !screens.cwResumePrompt) return null;
  const code = episodeCode(entry);
  return (
    <div className="resume-prompt" role="dialog" aria-label={t("Continuar de onde parou?")} data-testid="resume-prompt">
      <img src={entry.episodeThumbnail ?? entry.background ?? entry.poster ?? ""} alt="" />
      <div className="resume-prompt-body">
        <div className="muted small">{t("Continuar de onde parou?")}</div>
        <b>{entry.title}</b>
        <span className="muted small">{[code, entry.episodeTitle, t("parou em {time}", { time: clock(entry.positionMs) })].filter(Boolean).join(" · ")}</span>
        <div className="resume-prompt-actions">
          <button className="primary" onClick={() => { setEntry(null); onResume(entry); }} data-testid="resume-prompt-go">{t("▶ Continuar")}</button>
          <button className="ghost" onClick={() => setEntry(null)}>{t("Agora não")}</button>
        </div>
      </div>
    </div>
  );
}

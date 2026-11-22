// Whole-window player: the video uses the full client area (rect `null`) and the controls fade
// out after 3 s without mouse movement. Keyboard: Space, ←/→, ↑/↓, F, M, Esc.

import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import { usePlayer } from "./PlayerContext";
import { P2pStats } from "./P2pStats";
import { PlayerControls } from "./PlayerControls";

export function FullPlayer() {
  const player = usePlayer();
  const { session, snapshot } = player;
  const [visible, setVisible] = useState(true);
  const [pip, setPip] = useState(false);
  const hideTimer = useRef<number>(0);
  const live = session?.live === true;

  useEffect(() => {
    void api.playerRect(null);
  }, []);

  const leave = useCallback(async () => {
    if (player.windowFullscreen) await player.setWindowFullscreen(false);
    if (session?.returnTo) player.setView(session.returnTo);
    else await player.close();
  }, [player, session]);

  const toggleFullscreen = useCallback(() => {
    if (session?.returnTo && player.windowFullscreen) void leave();
    else void player.setWindowFullscreen(!player.windowFullscreen);
  }, [leave, player, session]);

  const poke = useCallback(() => {
    setVisible(true);
    window.clearTimeout(hideTimer.current);
    hideTimer.current = window.setTimeout(() => setVisible(false), 3000);
  }, []);
  useEffect(() => {
    poke();
    return () => window.clearTimeout(hideTimer.current);
  }, [poke]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement).tagName === "INPUT" || (e.target as HTMLElement).tagName === "SELECT") return;
      poke();
      switch (e.key) {
        case " ": case "k": e.preventDefault(); void api.playerCommand({ type: "togglePause" }); break;
        case "ArrowLeft": if (!live) void api.playerCommand({ type: "seek", seconds: -10, relative: true }); break;
        case "ArrowRight": if (!live) void api.playerCommand({ type: "seek", seconds: 10, relative: true }); break;
        case "ArrowUp": void api.playerCommand({ type: "setVolume", volume: Math.min(200, (snapshot?.volume ?? 100) + 5) }); break;
        case "ArrowDown": void api.playerCommand({ type: "setVolume", volume: Math.max(0, (snapshot?.volume ?? 100) - 5) }); break;
        case "m": void api.playerCommand({ type: "setMute", mute: !snapshot?.mute }); break;
        case "f": toggleFullscreen(); break;
        case "Escape": void leave(); break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [leave, live, poke, snapshot, toggleFullscreen]);

  const togglePip = async () => {
    try {
      await api.pictureInPicture(!pip, session?.title ?? "Nuvio");
      setPip(!pip);
    } catch {
      /* the player reports its own error state */
    }
  };

  return (
    <div className={`full-player ${visible ? "" : "chrome-hidden"}`} onMouseMove={poke}>
      <div className="full-player-top">
        <button className="back" onClick={() => void leave()} aria-label="Back">←</button>
        <div>
          <div className="full-player-title">{session?.title}</div>
          {session?.subtitle && <div className="muted">{session.subtitle}</div>}
        </div>
        <P2pStats />
      </div>
      <PlayerControls
        snapshot={snapshot}
        live={live}
        fullscreen={player.windowFullscreen}
        onToggleFullscreen={toggleFullscreen}
        onPictureInPicture={() => void togglePip()}
      />
    </div>
  );
}

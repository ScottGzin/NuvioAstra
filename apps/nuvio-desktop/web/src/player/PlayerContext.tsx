// One player for the whole app (Rust enforces one session at a time). Screens ask it to play a
// stream already resolved by Rust; the video itself is drawn by mpv below the webview and the UI
// only reserves its area (`PlayerSurface`) and draws the controls over it.

import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api, errorText } from "../api";
import { listen } from "../tauri";
import type { DownloadRequest, PlayerCommand, PlayerInfo, PlayerSnapshot } from "../types";
import type { EpisodeContext } from "./episodes";
import { usePartySync } from "./usePartySync";
import type { PartyStatus } from "./party";

/** `embedded`: video inside a screen area (Live TV preview). `full`: the whole window. */
export type PlayerView = "embedded" | "full";

export interface PlayerSession {
  title: string;
  subtitle?: string;
  /** Live channel: no timeline, seek and speed controls. */
  live?: boolean;
  view: PlayerView;
  /** View to return to when leaving `full` (Live TV fullscreen → preview); `null` closes. */
  returnTo: PlayerView | null;
  /** An episode of a series: what "next episode" needs. */
  episode?: EpisodeContext;
  /** Played by the P2P engine. */
  torrent?: boolean;
  /** What "Baixar" in the player downloads. */
  download?: DownloadRequest;
}

interface PlayerApi {
  session: PlayerSession | null;
  snapshot: PlayerSnapshot | null;
  error: string | null;
  play(start: () => Promise<PlayerInfo>, session: Omit<PlayerSession, "returnTo"> & { returnTo?: PlayerView | null }): Promise<boolean>;
  setView(view: PlayerView): void;
  close(): Promise<void>;
  windowFullscreen: boolean;
  setWindowFullscreen(on: boolean): Promise<void>;
  /** Watch Party (Supabase Realtime broadcast): room state and actions for the player overlay. */
  party: {
    status: PartyStatus;
    room: string;
    isHost: boolean;
    lastError: string | null;
    configured: boolean;
    join(room: string): void;
    leave(): void;
    noteLocalCommand(): void;
  };
}

const Ctx = createContext<PlayerApi | null>(null);

export function usePlayer(): PlayerApi {
  const value = useContext(Ctx);
  if (!value) throw new Error("PlayerProvider missing");
  return value;
}

export function PlayerProvider({ children }: { children: ReactNode }) {
  const [session, setSession] = useState<PlayerSession | null>(null);
  const [snapshot, setSnapshot] = useState<PlayerSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [windowFullscreen, setFs] = useState(false);
  const sessionId = useRef(0);
  const [partyConfig, setPartyConfig] = useState<{ url: string; key: string }>({ url: "", key: "" });
  const [partyRoom, setPartyRoom] = useState("");
  const actorId = useRef(
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `p-${Math.random().toString(36).slice(2)}-${Date.now()}`,
  );

  useEffect(() => {
    let cancelled = false;
    api.partyConfig().then((config) => { if (!cancelled) setPartyConfig(config); }, () => {});
    return () => { cancelled = true; };
  }, []);

  const partySend = useCallback((command: PlayerCommand) => {
    void api.playerCommand(command);
  }, []);
  const party = usePartySync({
    enabled: session !== null && Boolean(partyConfig.url && partyConfig.key),
    room: "",
    actorId: actorId.current,
    supabaseUrl: partyConfig.url,
    supabaseKey: partyConfig.key,
    snapshot,
    sendCommand: partySend,
    title: session?.title,
  });

  useEffect(
    () =>
      listen<PlayerSnapshot>("player-state", (s) => {
        // Late events of a closed session are ignored.
        if (s.session >= sessionId.current) setSnapshot(s);
      }),
    [],
  );

  useEffect(() => {
    document.body.classList.toggle("playing", session !== null);
  }, [session]);

  const setWindowFullscreen = useCallback(async (on: boolean) => {
    try {
      const active = await api.setFullscreen(on);
      setFs(active);
      void api.playerFocus();
      // Windows finishes the WS_POPUP transition and React re-renders just after the command;
      // move focus into WebView2 again once that is done.
      window.setTimeout(() => {
        window.focus();
        void api.playerFocus();
      }, 60);
    } catch (e) {
      setFs(false);
      console.error("window_set_fullscreen", e);
    }
  }, []);

  const play = useCallback<PlayerApi["play"]>(async (start, next) => {
    setError(null);
    try {
      const info = await start();
      sessionId.current = info.session;
      setSnapshot(null);
      setSession({ returnTo: null, ...next });
      return true;
    } catch (e) {
      setError(errorText(e));
      return false;
    }
  }, []);

  // UI test (`--uitest`, phase `enhance`): plays a synthetic source through this same session.
  useEffect(() => {
    const onTest = (e: Event) => void play((e as CustomEvent<() => Promise<PlayerInfo>>).detail, { title: "Teste", view: "full" });
    window.addEventListener("uitest-play", onTest);
    return () => window.removeEventListener("uitest-play", onTest);
  }, [play]);

  const close = useCallback(async () => {
    await api.playerClose().catch(() => {});
    setSession(null);
    setSnapshot(null);
    if (windowFullscreen) await setWindowFullscreen(false);
  }, [windowFullscreen, setWindowFullscreen]);

  const setView = useCallback((view: PlayerView) => {
    setSession((s) => (s ? { ...s, view } : s));
  }, []);

  const partyApi = useMemo(() => ({
    status: party.status,
    room: partyRoom,
    isHost: party.isHost,
    lastError: party.lastError,
    configured: Boolean(partyConfig.url && partyConfig.key),
    join: (room: string) => { setPartyRoom(room); party.startRoom(room); },
    leave: () => { party.leaveRoom(); setPartyRoom(""); },
    noteLocalCommand: party.noteLocalCommand,
  }), [party.status, party.isHost, party.lastError, party.startRoom, party.leaveRoom, party.noteLocalCommand, partyRoom, partyConfig]);

  const value = useMemo(
    () => ({ session, snapshot, error, play, setView, close, windowFullscreen, setWindowFullscreen, party: partyApi }),
    [session, snapshot, error, play, setView, close, windowFullscreen, setWindowFullscreen, partyApi],
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

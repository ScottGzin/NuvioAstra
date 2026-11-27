// Tells Rust what is on screen for Discord Rich Presence (`AppPresenceState` of the original): the
// tab, the title being viewed or what is playing. Sent only when something Discord shows changes
// (a playing position that just follows the clock is not resent). Rust does nothing with it while
// the option is off.

import { useEffect, useRef } from "react";
import { api } from "../api";
import { usePlayer } from "../player/PlayerContext";
import type { DiscordPresence } from "../types";

const TAB_NAMES: Record<string, string> = { home: "Início", tv: "TV", search: "Busca", discover: "Descobrir", library: "Biblioteca", settings: "Configurações" };

export function DiscordPresenceSync({ tab, viewing }: { tab: string; viewing: string | null }) {
  const player = usePlayer();
  const last = useRef<{ p: DiscordPresence; at: number } | null>(null);
  const session = player.session;
  const snap = player.snapshot;

  useEffect(() => {
    let p: DiscordPresence;
    if (session) {
      const video = session.episode?.video;
      const episode = video && video.season != null && video.episode != null ? `S${video.season}E${video.episode}${video.title ? ` - ${video.title}` : ""}` : null;
      p = {
        kind: "player",
        title: session.episode?.meta.name ?? session.title,
        episode,
        poster: session.episode?.meta.poster ?? session.movie?.poster ?? null,
        playing: !!snap && snap.loaded && !snap.paused && !snap.ended,
        positionMs: Math.round((snap?.timePos ?? 0) * 1000),
        durationMs: session.live ? 0 : Math.round((snap?.duration ?? 0) * 1000),
      };
    } else if (viewing) {
      p = { kind: "details", title: viewing };
    } else {
      p = { kind: "tab", title: TAB_NAMES[tab] ?? tab };
    }
    const now = Date.now();
    const prev = last.current;
    if (prev) {
      const sameShown = prev.p.kind === p.kind && prev.p.title === p.title && prev.p.episode === p.episode && prev.p.playing === p.playing && prev.p.poster === p.poster && prev.p.durationMs === p.durationMs;
      // While playing, the position follows the clock: only a seek (a jump) is news.
      const expected = (prev.p.positionMs ?? 0) + (p.playing ? now - prev.at : 0);
      if (sameShown && Math.abs((p.positionMs ?? 0) - expected) < 4000) return;
    }
    last.current = { p, at: now };
    void api.discordPresence(p);
  }, [tab, viewing, session, snap]);

  return null;
}

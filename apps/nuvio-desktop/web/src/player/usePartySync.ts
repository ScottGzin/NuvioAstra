// Watch-party sync for the player. Self-contained: all external data arrives through props, so
// the parent can wire it into PlayerContext / FullPlayer when ready.

import { useCallback, useEffect, useRef, useState } from "react";
import type { PlayerCommand, PlayerSnapshot } from "../types";
import {
  clockExtrapolate,
  createPartyClient,
  driftDecision,
  formatRoomCode,
  type PartyCommand,
  type PartyRealtimeClient,
  type PartyState,
  type PartyStatus,
} from "./party";

export interface UsePartySyncArgs {
  enabled: boolean;
  room: string;
  actorId: string;
  supabaseUrl: string;
  supabaseKey: string;
  accessToken?: string;
  snapshot: PlayerSnapshot | null;
  sendCommand: (command: PlayerCommand) => void;
  /** Explicit host wins; otherwise the lowest actorId in the room hosts. */
  isHost?: boolean;
  title?: string;
  mediaKey?: string;
}

export interface UsePartySyncResult {
  status: PartyStatus;
  peers: string[];
  lastError: string | null;
  isHost: boolean;
  startRoom: (room: string) => void;
  leaveRoom: () => void;
  /** Call when the user drives playback locally, for a 3 s grace before following again. */
  noteLocalCommand: () => void;
}

const clampSpeed = (speed: number) => Math.max(0.1, Math.min(4, speed));

export function usePartySync(args: UsePartySyncArgs): UsePartySyncResult {
  const {
    enabled, room, actorId, supabaseUrl, supabaseKey, accessToken,
    snapshot, sendCommand, isHost: explicitHost,
  } = args;

  // Optional local overrides make the panel usable without threading state through the parent.
  const [roomOverride, setRoomOverride] = useState<string | null>(null);
  const [enabledOverride, setEnabledOverride] = useState<boolean | null>(null);
  const effectiveRoom = roomOverride ?? room;
  const active = enabledOverride ?? enabled;
  const roomKey = formatRoomCode(effectiveRoom);

  const [status, setStatus] = useState<PartyStatus>("idle");
  const [peers, setPeers] = useState<string[]>([]);
  const [lastError, setLastError] = useState<string | null>(null);

  // Refs keep the transport alive across frequent snapshot updates.
  const snapshotRef = useRef(snapshot);
  const sendRef = useRef(sendCommand);
  const titleRef = useRef(args.title ?? "");
  const mediaKeyRef = useRef(args.mediaKey);
  const clientRef = useRef<PartyRealtimeClient | null>(null);
  const remoteRef = useRef<PartyState | null>(null);
  const peersRef = useRef<string[]>([]);
  const seqRef = useRef(0);
  const lastCommandSeqRef = useRef(0);
  /** Last time we applied a remote seek/command: no broadcast for 500 ms. */
  const appliedAtRef = useRef(0);
  /** 3 s echo guard after a remote command or a local user command. */
  const guardUntilRef = useRef(0);
  const expectLocalChangeRef = useRef(false);
  const prevLocalRef = useRef<{ paused: boolean; speed: number } | null>(null);

  useEffect(() => { snapshotRef.current = snapshot; }, [snapshot]);
  useEffect(() => { sendRef.current = sendCommand; }, [sendCommand]);
  useEffect(() => { titleRef.current = args.title ?? ""; }, [args.title]);
  useEffect(() => { mediaKeyRef.current = args.mediaKey; }, [args.mediaKey]);
  useEffect(() => { setRoomOverride(null); }, [room]);

  // A user command that reaches the player outside this hook also earns a grace period.
  useEffect(() => {
    const current = snapshot;
    if (!current) { prevLocalRef.current = null; return; }
    const previous = prevLocalRef.current;
    if (previous && !expectLocalChangeRef.current &&
        (previous.paused !== current.paused || Math.abs(previous.speed - current.speed) > 0.001)) {
      guardUntilRef.current = Date.now() + 3000;
    }
    expectLocalChangeRef.current = false;
    prevLocalRef.current = { paused: current.paused, speed: current.speed };
  }, [snapshot]);

  const noteLocalCommand = useCallback(() => {
    guardUntilRef.current = Date.now() + 3000;
    expectLocalChangeRef.current = true;
  }, []);

  const startRoom = useCallback((next: string) => {
    setRoomOverride(formatRoomCode(next));
    setEnabledOverride(true);
  }, []);

  const leaveRoom = useCallback(() => {
    setEnabledOverride(false);
    setRoomOverride(null);
  }, []);

  useEffect(() => {
    if (!active || !roomKey || !supabaseUrl || !supabaseKey || !actorId) {
      setStatus("idle");
      setPeers([]);
      peersRef.current = [];
      remoteRef.current = null;
      return;
    }

    const client = createPartyClient({ supabaseUrl, supabaseKey, room: roomKey, actorId, accessToken });
    clientRef.current = client;
    setPeers([]);
    peersRef.current = [];
    remoteRef.current = null;
    guardUntilRef.current = 0;
    appliedAtRef.current = 0;
    seqRef.current = 0;
    lastCommandSeqRef.current = 0;

    const isHostNow = (): boolean => {
      if (explicitHost !== undefined) return explicitHost;
      const ids = [actorId, ...peersRef.current].sort();
      return ids[0] === actorId;
    };

    const rememberPeer = (id: string) => {
      if (!id || id === actorId || peersRef.current.includes(id)) return;
      peersRef.current = [...peersRef.current, id];
      setPeers(peersRef.current);
    };

    const applyRemoteCommand = (command: PartyCommand) => {
      const now = Date.now();
      appliedAtRef.current = now;
      guardUntilRef.current = now + 3000;
      expectLocalChangeRef.current = true;
      if (command.action === "play") sendRef.current({ type: "setPaused", paused: false });
      else if (command.action === "pause") sendRef.current({ type: "setPaused", paused: true });
      else sendRef.current({ type: "seek", seconds: Math.max(0, command.positionSeconds), relative: false });
    };

    const offStatus = client.onStatus((next, error) => {
      setStatus(next);
      if (error) setLastError(error);
      else if (next === "joined") setLastError(null);
    });

    const offState = client.onState((state) => {
      if (state.actorId === actorId) return; // never echo our own state
      rememberPeer(state.actorId);
      const current = remoteRef.current;
      if (current && state.seq !== 0 && state.seq <= current.seq) return; // out of order
      remoteRef.current = state;
    });

    const offCommand = client.onCommand((command) => {
      if (command.actorId === actorId) return; // never execute our own command
      if (command.seq !== 0 && command.seq <= lastCommandSeqRef.current) return;
      lastCommandSeqRef.current = command.seq;
      rememberPeer(command.actorId);
      applyRemoteCommand(command);
    });

    // Host: report an extrapolatable snapshot every 4 seconds (VyseTV's _partyTick cadence).
    const hostTimer = setInterval(() => {
      if (!isHostNow()) return;
      const snap = snapshotRef.current;
      if (!snap || !snap.loaded) return;
      if (Date.now() - appliedAtRef.current < 500) return;
      const selectedSub = snap.tracks.find((track) => track.selected && track.kind === "sub");
      const state: PartyState = {
        title: titleRef.current,
        mediaKey: mediaKeyRef.current,
        positionSeconds: Math.max(0, snap.timePos),
        playing: !snap.paused && !snap.ended,
        rate: snap.speed > 0 ? snap.speed : 1,
        // Local epoch seconds stand in for server time; peers only ever read the difference.
        updatedAtServerSeconds: Date.now() / 1000,
        actorId,
        seq: ++seqRef.current,
        subtitleLabel: selectedSub?.title ?? undefined,
        subDelayMs: snap.subDelayMs,
      };
      client.sendState(state);
    }, 4000);

    // Guest: follow once a second, with Syncplay's jump / rate thresholds.
    const guestTimer = setInterval(() => {
      if (isHostNow()) return;
      const remote = remoteRef.current;
      const snap = snapshotRef.current;
      if (!remote || !snap || !snap.loaded) return;
      const now = Date.now();
      if (now < guardUntilRef.current || now - appliedAtRef.current < 500) return;

      // Play/pause first: a mismatch is a command, not drift.
      if (remote.playing === snap.paused) {
        appliedAtRef.current = now;
        guardUntilRef.current = now + 3000;
        expectLocalChangeRef.current = true;
        sendRef.current({ type: "setPaused", paused: !snap.paused });
        return;
      }

      const remotePosition = clockExtrapolate(remote, now / 1000) + (remote.playing ? 0.1 : 0);
      const decision = driftDecision(snap.timePos, remotePosition, !remote.playing && snap.paused);
      if (decision.action === "jump") {
        appliedAtRef.current = now;
        guardUntilRef.current = now + 3000;
        expectLocalChangeRef.current = true;
        sendRef.current({ type: "seek", seconds: Math.max(0, decision.targetSeconds ?? 0), relative: false });
      } else if (decision.action === "rate") {
        const speed = clampSpeed(remote.rate * (decision.rate ?? 1));
        if (Math.abs(snap.speed - speed) > 0.001) {
          appliedAtRef.current = now;
          expectLocalChangeRef.current = true;
          sendRef.current({ type: "setSpeed", speed });
        }
      } else if (Math.abs(snap.speed - remote.rate) > 0.001) {
        // Back inside the tolerance band: stop nudging.
        appliedAtRef.current = now;
        expectLocalChangeRef.current = true;
        sendRef.current({ type: "setSpeed", speed: clampSpeed(remote.rate) });
      }
    }, 1000);

    client.connect();

    return () => {
      clearInterval(hostTimer);
      clearInterval(guestTimer);
      offStatus();
      offState();
      offCommand();
      client.disconnect();
      if (clientRef.current === client) clientRef.current = null;
      remoteRef.current = null;
      peersRef.current = [];
    };
  }, [active, roomKey, actorId, supabaseUrl, supabaseKey, accessToken, explicitHost]);

  const isHost = explicitHost ?? (peers.length === 0 || [actorId, ...peers].sort()[0] === actorId);
  return { status, peers, lastError, isHost, startRoom, leaveRoom, noteLocalCommand };
}

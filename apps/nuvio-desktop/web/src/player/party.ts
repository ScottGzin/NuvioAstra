// Watch-party transport and sync math. Pure TypeScript: no React, no external dependencies.
//
// The wire protocol is Supabase Realtime's Phoenix v1.0.0 WebSocket protocol. Credentials are
// supplied by the caller — nothing is hardcoded here.

export interface PartyState {
  title: string;
  mediaKey?: string;
  /** Host position at `updatedAtServerSeconds`. */
  positionSeconds: number;
  playing: boolean;
  rate: number;
  /** Epoch seconds (server clock) at which `positionSeconds` was true. */
  updatedAtServerSeconds: number;
  actorId: string;
  /** Monotonic counter; higher wins so late packets cannot roll the room back. */
  seq: number;
  subtitleLabel?: string;
  subDelayMs?: number;
}

export interface PartyCommand {
  action: "play" | "pause" | "seek";
  positionSeconds: number;
  actorId: string;
  seq: number;
}

export type PartyStatus = "idle" | "connecting" | "joined" | "reconnecting" | "error" | "closed";

export interface PartyClientConfig {
  supabaseUrl: string;
  supabaseKey: string;
  room: string;
  actorId: string;
  /** Optional JWT: sent as `access_token` in the join payload. */
  accessToken?: string;
}

export interface PartyRealtimeClient {
  connect(): void;
  disconnect(): void;
  sendState(state: PartyState): void;
  sendCommand(command: PartyCommand): void;
  onState(listener: (state: PartyState) => void): () => void;
  onCommand(listener: (command: PartyCommand) => void): () => void;
  onStatus(listener: (status: PartyStatus, error: string | null) => void): () => void;
}

const BROADCAST_STATE = "party_state";
const BROADCAST_COMMAND = "party_command";
const HEARTBEAT_MS = 25_000;
const MAX_BACKOFF_MS = 30_000;

function finiteNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function asState(value: unknown): PartyState | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  const position = finiteNumber(v.positionSeconds);
  const rate = finiteNumber(v.rate);
  const updated = finiteNumber(v.updatedAtServerSeconds);
  const seq = finiteNumber(v.seq);
  if (position === null || rate === null || updated === null || seq === null) return null;
  if (typeof v.playing !== "boolean" || typeof v.actorId !== "string") return null;
  return {
    title: typeof v.title === "string" ? v.title : "",
    mediaKey: typeof v.mediaKey === "string" ? v.mediaKey : undefined,
    positionSeconds: position,
    playing: v.playing,
    rate,
    updatedAtServerSeconds: updated,
    actorId: v.actorId,
    seq,
    subtitleLabel: typeof v.subtitleLabel === "string" ? v.subtitleLabel : undefined,
    subDelayMs: finiteNumber(v.subDelayMs) ?? undefined,
  };
}

function asCommand(value: unknown): PartyCommand | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  const position = finiteNumber(v.positionSeconds);
  const seq = finiteNumber(v.seq);
  if (v.action !== "play" && v.action !== "pause" && v.action !== "seek") return null;
  if (position === null || seq === null || typeof v.actorId !== "string") return null;
  return { action: v.action, positionSeconds: position, actorId: v.actorId, seq };
}

/** A tiny Phoenix channel client for one party room. */
export function createPartyClient(config: PartyClientConfig): PartyRealtimeClient {
  const { supabaseUrl, supabaseKey } = config;
  const room = formatRoomCode(config.room);
  const topic = `realtime:party:${room}`;

  const stateListeners = new Set<(state: PartyState) => void>();
  const commandListeners = new Set<(command: PartyCommand) => void>();
  const statusListeners = new Set<(status: PartyStatus, error: string | null) => void>();

  let socket: WebSocket | null = null;
  let heartbeat: ReturnType<typeof setInterval> | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let refSeq = 0;
  let joinRef: string | null = null;
  let joined = false;
  let manualClose = false;
  let attempt = 0;
  let status: PartyStatus = "idle";

  const nextRef = () => String(++refSeq);

  function emitStatus(next: PartyStatus, error: string | null = null) {
    status = next;
    for (const listener of statusListeners) {
      try { listener(next, error); } catch { /* listener error must not break the transport */ }
    }
  }

  function send(message: Record<string, unknown>) {
    if (!socket || socket.readyState !== WebSocket.OPEN) return;
    try { socket.send(JSON.stringify(message)); } catch { /* socket died between check and send */ }
  }

  function clearHeartbeat() {
    if (heartbeat !== null) { clearInterval(heartbeat); heartbeat = null; }
  }

  function clearReconnect() {
    if (reconnectTimer !== null) { clearTimeout(reconnectTimer); reconnectTimer = null; }
  }

  function onOpen() {
    joinRef = nextRef();
    const payload: Record<string, unknown> = {
      config: { broadcast: { self: false, ack: false }, presence: {} },
    };
    if (config.accessToken) payload.access_token = config.accessToken;
    send({ topic, event: "phx_join", payload, ref: joinRef, join_ref: joinRef });
    clearHeartbeat();
    heartbeat = setInterval(() => {
      send({ topic: "phoenix", event: "heartbeat", payload: {}, ref: nextRef(), join_ref: joinRef ?? "" });
    }, HEARTBEAT_MS);
  }

  function onMessage(raw: MessageEvent<string>) {
    let message: Record<string, unknown>;
    try {
      const parsed: unknown = JSON.parse(raw.data);
      if (!parsed || typeof parsed !== "object") return;
      message = parsed as Record<string, unknown>;
    } catch { return; }

    if (message.event === "phx_reply") {
      const payload = message.payload;
      if (!payload || typeof payload !== "object") return;
      const reply = payload as Record<string, unknown>;
      if (message.ref !== joinRef) return;
      if (reply.status === "ok") {
        joined = true;
        attempt = 0;
        emitStatus("joined");
      } else {
        joined = false;
        emitStatus("error", `Party channel join failed: ${JSON.stringify(reply.response ?? reply.status)}`);
      }
      return;
    }

    if (message.event === "broadcast") {
      const payload = message.payload;
      if (!payload || typeof payload !== "object") return;
      const outer = payload as Record<string, unknown>;
      if (outer.event === BROADCAST_STATE) {
        const state = asState(outer.payload);
        if (state) for (const listener of stateListeners) {
          try { listener(state); } catch { /* ignore listener errors */ }
        }
      } else if (outer.event === BROADCAST_COMMAND) {
        const command = asCommand(outer.payload);
        if (command) for (const listener of commandListeners) {
          try { listener(command); } catch { /* ignore listener errors */ }
        }
      }
    }
  }

  function onClose() {
    joined = false;
    joinRef = null;
    clearHeartbeat();
    if (manualClose) { emitStatus("closed"); return; }
    emitStatus("reconnecting");
    clearReconnect();
    const delay = Math.min(MAX_BACKOFF_MS, 1000 * 2 ** attempt);
    attempt++;
    reconnectTimer = setTimeout(() => { reconnectTimer = null; openSocket(); }, delay);
  }

  function openSocket() {
    if (manualClose) return;
    clearReconnect();
    emitStatus(status === "idle" || status === "closed" ? "connecting" : status);
    const base = supabaseUrl.replace(/\/+$/, "");
    const url = `${base.replace(/^http/, "ws")}/realtime/v1/websocket?apikey=${encodeURIComponent(supabaseKey)}&vsn=1.0.0`;
    try {
      socket = new WebSocket(url);
    } catch (error) {
      emitStatus("error", error instanceof Error ? error.message : String(error));
      onClose();
      return;
    }
    socket.onopen = onOpen;
    socket.onmessage = onMessage;
    socket.onerror = () => emitStatus("error", "Party WebSocket error");
    socket.onclose = onClose;
  }

  return {
    connect() {
      if (socket || reconnectTimer !== null) return;
      manualClose = false;
      attempt = 0;
      emitStatus("connecting");
      openSocket();
    },
    disconnect() {
      manualClose = true;
      joined = false;
      joinRef = null;
      clearHeartbeat();
      clearReconnect();
      const current = socket;
      socket = null;
      if (current) {
        current.onopen = null;
        current.onmessage = null;
        current.onerror = null;
        current.onclose = null;
        try { current.close(); } catch { /* already closed */ }
      }
      emitStatus("closed");
    },
    sendState(state) {
      if (!joined) return;
      send({ topic, event: "broadcast", payload: { event: BROADCAST_STATE, payload: state }, ref: nextRef(), join_ref: joinRef ?? "" });
    },
    sendCommand(command) {
      if (!joined) return;
      send({ topic, event: "broadcast", payload: { event: BROADCAST_COMMAND, payload: command }, ref: nextRef(), join_ref: joinRef ?? "" });
    },
    onState(listener) {
      stateListeners.add(listener);
      return () => { stateListeners.delete(listener); };
    },
    onCommand(listener) {
      commandListeners.add(listener);
      return () => { commandListeners.delete(listener); };
    },
    onStatus(listener) {
      statusListeners.add(listener);
      listener(status, null);
      return () => { statusListeners.delete(listener); };
    },
  };
}

/**
 * VyseTV's `WatchState.now`: extrapolate the host position to "now", but only trust a report
 * up to 600 seconds old (older reports are stale and would land in a different era of the film).
 */
export function clockExtrapolate(state: PartyState, serverNowSeconds: number): number {
  const ageMs = (serverNowSeconds - state.updatedAtServerSeconds) * 1000;
  const age = Math.max(0, Math.min(600_000, Number.isFinite(ageMs) ? ageMs : 0));
  const played = state.playing ? (age / 1000) * state.rate : 0;
  return Math.max(0, state.positionSeconds + played);
}

export interface DriftDecision {
  action: "none" | "jump" | "rate";
  targetSeconds?: number;
  /** Multiplier for the host's rate: 1.05 / 0.95, or 1.1 / 0.9 for a bigger gap. */
  rate?: number;
}

/**
 * Syncplay thresholds from VyseTV: jump when >5 s behind or >4 s ahead while playing, or more
 * than 1.5 s off while paused. Otherwise nudge the rate by ±5% (±10% beyond 2 s).
 * A gap of 0.3 s or less needs no correction.
 */
export function driftDecision(local: number, remote: number, paused: boolean): DriftDecision {
  const diff = remote - local; // > 0: local is behind the room
  if (!Number.isFinite(diff)) return { action: "none" };

  if (paused) {
    return Math.abs(diff) > 1.5 ? { action: "jump", targetSeconds: remote } : { action: "none" };
  }
  if (diff > 5 || diff < -4) return { action: "jump", targetSeconds: remote };
  if (Math.abs(diff) <= 0.3) return { action: "none" };
  const magnitude = Math.abs(diff) > 2 ? 0.1 : 0.05;
  return { action: "rate", rate: 1 + (diff > 0 ? magnitude : -magnitude) };
}

/** Invite codes are short, unambiguous and safe to put in a URL. */
export function formatRoomCode(code: string): string {
  return code.trim().toUpperCase().replace(/[^A-Z0-9]/g, "");
}

const ROOM_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

export function randomRoomCode(length = 6): string {
  const size = Math.max(1, Math.floor(length));
  const bytes = new Uint8Array(size);
  const crypto = globalThis.crypto;
  if (crypto && typeof crypto.getRandomValues === "function") crypto.getRandomValues(bytes);
  else for (let i = 0; i < size; i++) bytes[i] = Math.floor(Math.random() * 256);
  let out = "";
  for (let i = 0; i < size; i++) out += ROOM_ALPHABET[bytes[i] % ROOM_ALPHABET.length];
  return out;
}

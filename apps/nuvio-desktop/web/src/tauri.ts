// Thin typed access to the Tauri globals (`withGlobalTauri`): no extra npm package needed.
type Unlisten = () => void;
interface TauriGlobal {
  core: { invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> };
  event: { listen<T>(event: string, handler: (e: { payload: T }) => void): Promise<Unlisten> };
}
declare global {
  interface Window { __TAURI__?: TauriGlobal }
}

function tauri(): TauriGlobal {
  if (!window.__TAURI__) throw new Error("Tauri runtime not available");
  return window.__TAURI__;
}

export function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return tauri().core.invoke<T>(cmd, args);
}

/** Subscribes to a Rust event; the returned function unsubscribes (safe before registration ends). */
export function listen<T>(event: string, handler: (payload: T) => void): Unlisten {
  let unlisten: Unlisten | null = null;
  let cancelled = false;
  void tauri().event.listen<T>(event, (e) => handler(e.payload)).then((u) => {
    if (cancelled) u();
    else unlisten = u;
  });
  return () => {
    cancelled = true;
    unlisten?.();
  };
}

/** Subscribes and waits until the listener is registered, so no event of the request that follows
 *  can be missed (streamed answers start arriving before the command returns). */
export async function listenRegistered<T>(event: string, handler: (payload: T) => void): Promise<Unlisten> {
  return tauri().event.listen<T>(event, (e) => handler(e.payload));
}

// Short messages at the bottom of the window (download started, failures of a click).

import { useEffect, useState } from "react";

export interface Toast {
  id: number;
  text: string;
  error: boolean;
}

let next = 1;
const listeners = new Set<(toast: Toast) => void>();

export function showToast(text: string, error = false) {
  const toast = { id: next++, text, error };
  listeners.forEach((l) => l(toast));
}

export function useToasts(): Toast[] {
  const [toasts, setToasts] = useState<Toast[]>([]);
  useEffect(() => {
    const add = (toast: Toast) => {
      setToasts((all) => [...all.slice(-2), toast]);
      window.setTimeout(() => setToasts((all) => all.filter((t) => t.id !== toast.id)), toast.error ? 7000 : 4500);
    };
    listeners.add(add);
    return () => {
      listeners.delete(add);
    };
  }, []);
  return toasts;
}

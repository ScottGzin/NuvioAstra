// Spatial focus navigation shared by keyboard and standard gamepads (Xbox /DualShock both use the
// browser's standard mapping). Buttons stay real DOM buttons, so Enter/A activates them normally.

import { useEffect } from "react";

const SELECTOR = 'button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

function focusables(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>(SELECTOR)).filter((el) => {
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return rect.width > 0 && rect.height > 0 && style.visibility !== "hidden" && style.display !== "none" && el.getAttribute("aria-hidden") !== "true";
  });
}

function move(direction: "up" | "down" | "left" | "right") {
  const current = document.activeElement as HTMLElement | null;
  const items = focusables();
  if (items.length === 0) return;
  if (!current || !items.includes(current)) {
    items[0].focus();
    return;
  }
  const from = current.getBoundingClientRect();
  const fx = from.left + from.width / 2;
  const fy = from.top + from.height / 2;
  let best: HTMLElement | null = null;
  let bestScore = Infinity;
  for (const el of items) {
    if (el === current) continue;
    const rect = el.getBoundingClientRect();
    const x = rect.left + rect.width / 2;
    const y = rect.top + rect.height / 2;
    const dx = x - fx;
    const dy = y - fy;
    const along = direction === "right" ? dx : direction === "left" ? -dx : direction === "down" ? dy : -dy;
    if (along <= 4) continue;
    const across = direction === "left" || direction === "right" ? Math.abs(dy) : Math.abs(dx);
    const score = along + across * 2.5;
    if (score < bestScore) { bestScore = score; best = el; }
  }
  if (best) {
    best.focus();
    best.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
  }
}

function closeOverlay(): boolean {
  const overlay = document.querySelector<HTMLElement>(".modal-backdrop, .chapters-overlay, .sources-overlay, .next-panel-layer");
  if (!overlay) return false;
  const close = overlay.querySelector<HTMLElement>('[aria-label="Fechar"], [aria-label="Cancelar"], .icon-btn');
  (close ?? overlay).click();
  return true;
}

export function useInputNavigation() {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      const typing = !!target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable);
      if (event.key === "Escape") { closeOverlay(); return; }
      if (typing) return;
      if (document.querySelector(".player-full, .full-player")) return; // player keeps arrow keys for seek
      const directions: Record<string, "up" | "down" | "left" | "right"> = {
        ArrowUp: "up", ArrowDown: "down", ArrowLeft: "left", ArrowRight: "right",
      };
      const direction = directions[event.key];
      if (!direction) return;
      event.preventDefault();
      move(direction);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}

/** Polls standard gamepads; returns a cleanup function. */
export function useGamepad() {
  useEffect(() => {
    let raf = 0;
    const pressed = new Set<number>();
    const axisPressed = { x: false, y: false };
    let poll = 0;
    const tick = () => {
      raf = requestAnimationFrame(tick);
      if (++poll % 2 !== 0) return; // ~30 Hz
      const pads = navigator.getGamepads?.() ?? [];
      const pad = Array.from(pads).find((p) => p && p.connected);
      if (!pad) { pressed.clear(); return; }
      document.body.classList.add("gamepad-active");
      const down = (index: number) => {
        const isDown = !!pad.buttons[index]?.pressed;
        const was = pressed.has(index);
        if (isDown) pressed.add(index); else pressed.delete(index);
        return isDown && !was;
      };
      // D-pad (12–15) then analogue stick / dpad axes.
      if (down(12)) move("up");
      if (down(13)) move("down");
      if (down(14)) move("left");
      if (down(15)) move("right");
      const ax = pad.axes[0] ?? 0;
      const ay = pad.axes[1] ?? 0;
      if (ax < -0.6 && !axisPressed.x) { axisPressed.x = true; move("left"); }
      else if (ax > 0.6 && !axisPressed.x) { axisPressed.x = true; move("right"); }
      else if (Math.abs(ax) < 0.3) axisPressed.x = false;
      if (ay < -0.6 && !axisPressed.y) { axisPressed.y = true; move("up"); }
      else if (ay > 0.6 && !axisPressed.y) { axisPressed.y = true; move("down"); }
      else if (Math.abs(ay) < 0.3) axisPressed.y = false;

      if (down(0)) (document.activeElement as HTMLElement | null)?.click();
      if (down(1)) closeOverlay();
      if (down(4) || down(5)) {
        const tabs = document.querySelectorAll<HTMLElement>('[data-section], .season-tabs .chip');
        if (tabs.length) tabs[0].focus();
      }
      if (down(9)) {
        const settings = document.querySelector<HTMLElement>('.sidebar [title="Settings"], [data-section="settings"]');
        settings?.click();
      }
    };
    raf = requestAnimationFrame(tick);
    return () => { cancelAnimationFrame(raf); document.body.classList.remove("gamepad-active"); };
  }, []);
}

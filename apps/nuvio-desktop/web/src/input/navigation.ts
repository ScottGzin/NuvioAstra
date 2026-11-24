// Spatial focus navigation shared by keyboard and standard gamepads (Xbox /DualShock both use the
// browser's standard mapping). Buttons stay real DOM buttons, so Enter/A activates them normally.

import { useEffect } from "react";

const SELECTOR = 'button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';
/** Panels that keep the focus for themselves while open. */
const PANELS = ".modal-backdrop, .chapters-overlay, .sources-overlay, .next-panel-layer, .see-all-body";

let navHideTimer = 0;
/** Non-pointer input hides the mouse cursor until a real mouse movement (the original's HTPC behavior). */
function markNavInput() {
  document.body.classList.add("nav-input");
  window.clearTimeout(navHideTimer);
  navHideTimer = window.setTimeout(() => document.body.classList.remove("nav-input"), 8000);
}

/** After scrolling settles, keep the focus ring on an element actually inside the viewport.
 *  The replacement stays in the same screen (or open panel) and nearest to the element that was
 *  focused: picking the first focusable of the document would send focus back to the sidebar. */
function ensureVisibleFocus() {
  const current = document.activeElement as HTMLElement | null;
  if (!current || current === document.body) return;
  const rect = current.getBoundingClientRect();
  const fullyVisible = (r: DOMRect) => r.top >= 0 && r.bottom <= innerHeight && r.left >= 0 && r.right <= innerWidth;
  if (fullyVisible(rect)) return;
  const scope = regionOf(current);
  const cx = rect.left + rect.width / 2;
  const cy = rect.top + rect.height / 2;
  const visible = focusables()
    .filter((el) => scope.contains(el) && fullyVisible(el.getBoundingClientRect()))
    .sort((a, b) => {
      const ar = a.getBoundingClientRect();
      const br = b.getBoundingClientRect();
      return Math.hypot(ar.left + ar.width / 2 - cx, ar.top + ar.height / 2 - cy) -
        Math.hypot(br.left + br.width / 2 - cx, br.top + br.height / 2 - cy);
    });
  visible[0]?.focus();
}

function focusables(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>(SELECTOR)).filter((el) => {
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return rect.width > 0 && rect.height > 0 && style.visibility !== "hidden" && style.display !== "none" && el.getAttribute("aria-hidden") !== "true";
  });
}

function inViewport(el: HTMLElement): boolean {
  const r = el.getBoundingClientRect();
  return r.bottom > 0 && r.top < innerHeight && r.right > 0 && r.left < innerWidth;
}

/** Where keyboard navigation starts when nothing is focused (first key after the mouse, a screen
 *  change): the first item of the content that is actually on screen, never the page top. */
function firstVisibleContent(): HTMLElement | null {
  const all = focusables();
  const visible = all.filter((el) => el.closest("main") && inViewport(el));
  const pool = visible.length ? visible : all;
  if (pool.length === 0) return null;
  return pool.reduce((best, el) => {
    const a = el.getBoundingClientRect();
    const b = best.getBoundingClientRect();
    return a.top < b.top - 2 || (Math.abs(a.top - b.top) <= 2 && a.left < b.left) ? el : best;
  });
}

type Direction = "up" | "down" | "left" | "right";

/** Region that owns the current focus: the open panel, the main content, or the sidebar. */
function regionOf(element: HTMLElement): HTMLElement {
  return element.closest<HTMLElement>(PANELS) ??
    element.closest<HTMLElement>("main") ??
    element.closest<HTMLElement>(".sidebar") ??
    document.body;
}

/** Nearest focusable in `direction`, by center distance. Off-screen elements count: focusing one
 *  scrolls it into view, which is how the UI reaches the rows below the fold. */
function nearest(from: HTMLElement, items: HTMLElement[], direction: Direction): HTMLElement | null {
  const rect = from.getBoundingClientRect();
  const fx = rect.left + rect.width / 2;
  const fy = rect.top + rect.height / 2;
  const horizontal = direction === "left" || direction === "right";
  let best: HTMLElement | null = null;
  let bestScore = Infinity;
  for (const el of items) {
    if (el === from) continue;
    const r = el.getBoundingClientRect();
    const dx = r.left + r.width / 2 - fx;
    const dy = r.top + r.height / 2 - fy;
    const along = direction === "right" ? dx : direction === "left" ? -dx : direction === "down" ? dy : -dy;
    if (along <= 4) continue;
    const across = horizontal ? Math.abs(dy) : Math.abs(dx);
    const score = along + across * 2.5;
    if (score < bestScore) { bestScore = score; best = el; }
  }
  return best;
}

/** Next target for a direction: vertical moves stay in the current region (the main content must
 *  not hand focus to the sidebar when a row has ended), and an open panel keeps every direction.
 *  Horizontal moves may cross between the content and the sidebar, which is how it is reached
 *  without the Ctrl+B shortcut. */
function nextInDirection(current: HTMLElement, direction: Direction): HTMLElement | null {
  const all = focusables();
  const panel = current.closest<HTMLElement>(PANELS);
  if (panel) return nearest(current, all.filter((el) => panel.contains(el)), direction);
  if (direction === "up" || direction === "down") {
    const region = regionOf(current);
    const scoped = all.filter((el) => region.contains(el));
    if (scoped.length > 1) return nearest(current, scoped, direction);
  }
  return nearest(current, all, direction);
}

/** Scroll container that can still move on the direction's axis (the screen vertically, a poster
 *  row horizontally). */
function scrollableAncestor(element: HTMLElement | null, direction: Direction): HTMLElement | null {
  const vertical = direction === "up" || direction === "down";
  for (let node = element?.parentElement ?? null; node; node = node.parentElement) {
    const style = getComputedStyle(node);
    const overflow = vertical ? style.overflowY : style.overflowX;
    if (overflow !== "auto" && overflow !== "scroll") continue;
    const canScroll = vertical ? node.scrollHeight > node.clientHeight + 2 : node.scrollWidth > node.clientWidth + 2;
    if (canScroll) return node;
  }
  return null;
}

let retryTimer = 0;
function move(direction: Direction) {
  window.clearTimeout(retryTimer);
  const current = document.activeElement as HTMLElement | null;
  const all = focusables();
  if (all.length === 0) return;
  if (!current || current === document.body || !all.includes(current)) {
    firstVisibleContent()?.focus();
    return;
  }
  const best = nextInDirection(current, direction);
  if (best) {
    best.focus();
    best.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
    return;
  }
  // Nothing focusable further in that direction (a lazily loaded row, or the next page of a
  // catalog). Move the scroll container one step and try again once its content is there.
  const scroller = scrollableAncestor(current, direction);
  if (!scroller) return;
  const vertical = direction === "up" || direction === "down";
  const amount = (vertical ? scroller.clientHeight : scroller.clientWidth) * 0.8;
  const sign = direction === "right" || direction === "down" ? 1 : -1;
  scroller.scrollBy({ left: vertical ? 0 : sign * amount, top: vertical ? sign * amount : 0, behavior: "smooth" });
  retryTimer = window.setTimeout(() => {
    const target = nextInDirection(current, direction);
    if (!target) return;
    target.focus();
    target.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
  }, 320);
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
      if (event.key === "Escape") {
        // Escape closes the topmost panel only; without this the player would also leave.
        if (closeOverlay()) { event.stopImmediatePropagation(); return; }
        // Details and collection folders have a back button only; Escape is the keyboard way out.
        const back = document.querySelector<HTMLElement>(".screen:not([hidden]) .back");
        if (back && !document.querySelector(".full-player")) {
          event.preventDefault();
          back.click();
        }
        return;
      }
      if (typing) return;
      if (document.querySelector(".player-full, .full-player")) return; // player keeps arrow keys for seek
      const directions: Record<string, "up" | "down" | "left" | "right"> = {
        ArrowUp: "up", ArrowDown: "down", ArrowLeft: "left", ArrowRight: "right",
      };
      if (event.ctrlKey || event.metaKey) {
        const key = event.key.toLowerCase();
        if (key === "k") {
          event.preventDefault();
          document.querySelector<HTMLElement>('[data-tab="search"]')?.click();
          window.setTimeout(() => document.querySelector<HTMLInputElement>('input[aria-label="Buscar"]')?.focus(), 80);
          markNavInput();
          return;
        }
        if (key === "tab") {
          event.preventDefault();
          const tabs = Array.from(document.querySelectorAll<HTMLElement>("[data-tab]"));
          const active = tabs.findIndex((el) => el === document.activeElement);
          const next = tabs[(active + (event.shiftKey ? -1 : 1) + tabs.length) % tabs.length];
          next?.focus();
          markNavInput();
          return;
        }
        if (key === "b") {
          event.preventDefault();
          document.querySelector<HTMLElement>(".sidebar")?.querySelector<HTMLElement>("button")?.focus();
          markNavInput();
          return;
        }
      }
      if (event.key === "F10" && event.shiftKey) {
        event.preventDefault();
        (document.activeElement as HTMLElement | null)?.querySelector<HTMLElement>(".fav-btn")?.click();
        markNavInput();
        return;
      }
      const direction = directions[event.key];
      if (!direction) return;
      event.preventDefault();
      markNavInput();
      move(direction);
    };
    let scrollTimer = 0;
    const onScroll = () => {
      window.clearTimeout(scrollTimer);
      scrollTimer = window.setTimeout(ensureVisibleFocus, 140);
    };
    const onMouse = () => {
      document.body.classList.remove("nav-input");
      document.body.classList.remove("gamepad-active");
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("mousemove", onMouse);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("mousemove", onMouse);
      window.clearTimeout(scrollTimer);
      window.clearTimeout(navHideTimer);
      window.clearTimeout(retryTimer);
    };
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
      markNavInput();
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
      if (down(3)) {
        document.querySelector<HTMLElement>('[data-tab="search"]')?.click();
        window.setTimeout(() => document.querySelector<HTMLInputElement>('input[aria-label="Buscar"]')?.focus(), 80);
      }
      if (down(2)) (document.activeElement as HTMLElement | null)?.querySelector<HTMLElement>(".fav-btn")?.click();
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

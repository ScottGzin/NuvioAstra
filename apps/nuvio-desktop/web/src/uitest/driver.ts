// Automated UI test (`--uitest <report.json>`). Drives the real UI with DOM clicks and typing,
// against the user's real addons, and checks what is on screen. Network use is bounded: one movie
// and one series from Home, the same two titles searched by name, the first playable source of
// that movie and of one episode (explicitly chosen by this script), no channel resolution.

import { api, notifyRemoteChange } from "../api";
import { invoke } from "../tauri";

interface Check { name: string; passed: boolean; detail: string }
interface RegionStats { whiteFraction: number; blackFraction: number; lumaStddev: number; motion: number }

const checks: Check[] = [];
const check = (name: string, passed: boolean, detail: string) => checks.push({ name, passed, detail });
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const $ = <T extends Element = HTMLElement>(sel: string, root: ParentNode = document) => root.querySelector<T>(sel);
const $$ = <T extends Element = HTMLElement>(sel: string, root: ParentNode = document) => Array.from(root.querySelectorAll<T>(sel));

async function waitFor<T>(what: () => T | null | undefined | false, timeoutMs: number): Promise<T | null> {
  const end = performance.now() + timeoutMs;
  while (performance.now() < end) {
    const value = what();
    if (value) return value;
    await sleep(150);
  }
  return null;
}

const visible = (el: Element | null): el is HTMLElement => !!el && (el as HTMLElement).offsetParent !== null;

function click(el: HTMLElement) {
  el.scrollIntoView({ block: "center" });
  el.click();
}

/** Types like a user: React sees a native input event. */
function typeInto(input: HTMLInputElement, text: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  setter.call(input, text);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

const capture = (name: string) => invoke<RegionStats>("uitest_capture", { name }).catch(() => null);

interface Opened { id: string; name: string; type: string }

/** Waits for the details page of `id` (or its error) and validates what it shows. */
async function expectDetails(label: string, id: string | null, type: string): Promise<Opened | null> {
  const page = await waitFor(() => {
    const d = $$(".details").find((el) => visible(el) && (!id || el.dataset.id === id));
    if (d) return d;
    const err = $$(".screen-message").find((el) => visible(el) && el.textContent?.includes("Não foi possível abrir"));
    return err ?? null;
  }, 30000);
  if (!page || !page.classList.contains("details")) {
    check(`${label}: details open`, false, page?.textContent?.trim() ?? "timeout (30 s)");
    return null;
  }
  const name = $("h1", page)?.textContent?.trim() || $<HTMLImageElement>(".details-logo", page)?.alt || "";
  const gotType = page.dataset.contentType ?? "";
  check(`${label}: details open`, !!name, `“${name}” (${gotType}, ${page.dataset.id})`);
  check(`${label}: content type kept`, gotType === type, `expected ${type}, got ${gotType}`);
  const desc = $(".details-desc", page)?.textContent?.length ?? 0;
  const facts = [$(".meta-line", page)?.textContent?.trim(), $$(".chip", $(".details-info", page) ?? page).length + " gêneros", `${$$(".person", page).length} no elenco`, `sinopse ${desc} caracteres`];
  const images = $$<HTMLImageElement>("img", page).filter((i) => i.complete && i.naturalWidth > 0).length;
  check(`${label}: real metadata shown`, !!name && (desc > 0 || images > 0), `${facts.join(" · ")} · ${images} imagens carregadas`);
  return { id: page.dataset.id ?? "", name, type: gotType };
}

/** Waits for the sources list, plays the first playable one, checks the picture, closes. */
async function playFirstSource(label: string, whilePlaying?: () => Promise<void>) {
  const list = await waitFor(() => {
    const s = $(".details .streams");
    if (s) return s;
    const msg = $$(".details .sources-panel .muted, .details .details-section .muted, .details .row-error").find((el) => /Nenhum addon|Falha/.test(el.textContent ?? ""));
    return msg ?? null;
  }, 45000);
  if (!list || !list.classList.contains("streams")) {
    check(`${label}: sources listed`, false, list?.textContent?.trim() ?? "timeout (45 s)");
    return;
  }
  const groups = $$(".stream-group", list).map((g) => $(".stream-group-title", g)?.textContent?.trim()).join(" | ");
  // Direct sources only (the P2P phase tests torrents with freely licensed content).
  const playable = $$<HTMLButtonElement>('.stream:not(:disabled)[data-kind="direct"]', list);
  const unavailable = $$(".stream:disabled", list).length + $$('.stream[data-kind="torrent"]', list).length;
  check(`${label}: sources listed`, true, `${groups} — ${playable.length} reproduzível(is), ${unavailable} indisponível(is) nesta versão`);
  if (!playable.length) {
    check(`${label}: playback`, false, "no direct source among the addon answers (torrent/debrid/YouTube are not playable yet)");
    return;
  }
  click(playable[0]);
  const full = await waitFor(() => $(".full-player"), 20000);
  if (!full) {
    const err = $(".details .row-error")?.textContent ?? "full player did not open";
    check(`${label}: playback`, false, err);
    return;
  }
  let lastError: string | null = null;
  const start = performance.now();
  let snapshotOk = false;
  let detail = "";
  while (performance.now() - start < 60000) {
    const snap = await api.playerSnapshot().catch(() => null);
    if (snap?.error) { lastError = snap.error; break; }
    if (snap && snap.loaded && snap.timePos > 1) {
      snapshotOk = true;
      detail = `time ${snap.timePos.toFixed(1)} s, ${snap.width}×${snap.height}, ${snap.hwdecCurrent ?? "software"}`;
      break;
    }
    await sleep(500);
  }
  if (!snapshotOk) {
    check(`${label}: playback`, false, lastError ?? "no progress in 60 s");
  } else {
    // A film may open on black (studio logo fade): up to 3 captures over ~8 s before failing.
    let stats: RegionStats | null = null;
    let picture = false;
    let tries = 0;
    for (; tries < 3 && !picture; tries++) {
      await sleep(tries === 0 ? 1500 : 3000);
      stats = await capture(`${label.replace(/\W+/g, "-")}-playing`);
      picture = !!stats && stats.motion > 0.3 && stats.whiteFraction < 0.98 && stats.blackFraction < 0.98;
    }
    check(`${label}: playback`, picture, `${detail}; window motion ${stats?.motion.toFixed(2)}, white ${Math.round((stats?.whiteFraction ?? 0) * 100)} %, black ${Math.round((stats?.blackFraction ?? 0) * 100)} %${tries > 1 ? ` (captura ${tries})` : ""}`);
    if (whilePlaying) await whilePlaying();
  }
  const close = $<HTMLButtonElement>('.full-player .full-player-top .back');
  if (close) close.click();
  const closed = await waitFor(() => !$(".full-player"), 10000);
  check(`${label}: player closed`, !!closed, closed ? "full player removed" : "still open");
}

async function openFromHome(type: string, label: string): Promise<Opened | null> {
  const row = await waitFor(() => $$(".row-section").find((r) => r.dataset.contentType === type), 30000);
  if (!row) {
    check(`${label}: catalog row`, false, `no ${type} row on Home`);
    return null;
  }
  row.scrollIntoView({ block: "center" }); // rows load lazily when they come into view
  const card = await waitFor(() => $<HTMLButtonElement>(".card[data-id]", row), 30000);
  if (!card) {
    check(`${label}: catalog row`, false, `${$("h2", row)?.textContent}: no items`);
    return null;
  }
  check(`${label}: catalog row`, true, `${$("h2", row)?.textContent} — ${$$(".card[data-id]", row).length} itens`);
  click(card);
  return expectDetails(label, card.dataset.id ?? null, type);
}

async function openFromSearch(title: Opened, label: string): Promise<Opened | null> {
  click($('[data-tab="search"]')!);
  const input = await waitFor(() => $<HTMLInputElement>('[data-testid="search-input"]'), 5000);
  if (!input) {
    check(`${label}: search`, false, "search input missing");
    return null;
  }
  typeInto(input, title.name);
  const card = await waitFor(() => {
    const cards = $$<HTMLButtonElement>(".search .card[data-id]");
    return cards.find((c) => c.dataset.id === title.id) ?? (cards.length && $(".search .spinner") === null ? cards.find((c) => c.dataset.contentType === title.type) : null);
  }, 40000);
  if (!card) {
    check(`${label}: search`, false, `no ${title.type} result for “${title.name}”: ${$(".search .screen-message, .search .row-error")?.textContent ?? "timeout"}`);
    return null;
  }
  check(`${label}: search`, true, `${$$(".search .row-section").length} seções; abrindo ${card.dataset.id} (${card.dataset.id === title.id ? "mesmo título da Home" : "primeiro resultado do tipo"})`);
  click(card);
  return expectDetails(label, card.dataset.id ?? null, title.type);
}

async function back() {
  const b = $$<HTMLButtonElement>(".details .back").find(visible);
  b?.click();
  await sleep(300);
}

/** Sets a <select>/<input> like a user (React sees the native event). */
function setValue(el: HTMLSelectElement | HTMLInputElement, value: string) {
  const proto = el instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(el, value);
  el.dispatchEvent(new Event(el instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }));
}

async function openSettings(sectionId: string): Promise<boolean> {
  if (!$(".settings")) click($('[data-tab="settings"]')!);
  if (!(await waitFor(() => $(".settings .settings-nav"), 20000))) return false;
  click($(`.settings-nav [data-section="${sectionId}"]`)!);
  await sleep(250);
  return true;
}

async function waitSaved(label: string) {
  await sleep(50); // the change itself switches the status to "Salvando…"
  const status = await waitFor(() => {
    const el = $('[data-testid="save-status"]');
    return el && /Salvo|Não foi possível/.test(el.textContent ?? "") ? el : null;
  }, 5000);
  check(`${label}: saved`, !!status && /Salvo/.test(status.textContent ?? ""), status?.textContent ?? "no feedback in 5 s");
  await sleep(200);
}

/** Read-only tour of the Settings page: every category opens and shows its controls. */
async function settingsTour() {
  if (!(await openSettings("home"))) {
    check("settings: page opens", false, "settings page did not render");
    return;
  }
  const nav = $(".settings-nav");
  const sidebarItem = $('[data-tab="settings"]');
  check("settings: sidebar item", !!sidebarItem && sidebarItem.classList.contains("active") && /Configurações/.test(sidebarItem.textContent ?? ""), "⚙ Configurações above the profile, highlighted when open");
  for (const id of ["home", "playback", "audio", "subtitles", "data", "pending"]) {
    const button = $(`.settings-nav [data-section="${id}"]`, nav!)!;
    click(button);
    const label = $("span:last-child", button)?.textContent?.trim() ?? "";
    // The body is re-created per category: wait for the one of this category.
    const body = await waitFor(() => ($(".settings-header h1")?.textContent?.trim() === label ? $(".settings-body") : null), 3000);
    const controls = $$("select, input, [role=switch]", body!).length;
    const rows = $$(".setting-row, .catalog-pref, .pending-list li", body!).length;
    check(`settings: ${id}`, rows > 0, `${$(".settings-header h1")?.textContent}: ${rows} linhas, ${controls} controles`);
  }
  const live = await api.settingsGet();
  check("settings: values loaded from Rust", true, `perfil ${live.profile.name}: imagem ${live.settings.player.resizeMode}, decodificador ${live.settings.player.decoderPriority}, áudio ${live.settings.player.preferredAudioLanguage}, legenda ${live.settings.player.subtitle.fontSizeSp} sp, destaque ${live.settings.home.heroEnabled}`);
}

/** Phase 1: change five settings through the UI, check storage, Home and the player. */
async function settingsWrite(): Promise<Record<string, unknown>> {
  const before = (await api.settingsGet()).settings;
  const original = {
    resizeMode: before.player.resizeMode, decoderPriority: before.player.decoderPriority,
    fontSizeSp: before.player.subtitle.fontSizeSp, preferredAudioLanguage: before.player.preferredAudioLanguage,
    heroEnabled: before.home.heroEnabled,
  };
  await openSettings("playback");
  setValue($<HTMLSelectElement>('select[data-setting="resizeMode"]')!, "Stretch");
  await waitSaved("set image mode Stretch");
  setValue($<HTMLSelectElement>('select[data-setting="decoderPriority"]')!, "2");
  await waitSaved("set decoder software only");
  await openSettings("audio");
  setValue($<HTMLSelectElement>('select[aria-label="Idioma de áudio preferido"]')!, "en");
  await waitSaved("set audio language en");
  await openSettings("subtitles");
  setValue($<HTMLInputElement>('input[aria-label="Tamanho da legenda"]')!, "24");
  await waitSaved("set subtitle size 24");
  await openSettings("home");
  const hero = $<HTMLButtonElement>('[role=switch][aria-label="Destaque no topo"]')!;
  if (hero.getAttribute("aria-checked") === "true") click(hero);
  await waitSaved("hero off");

  const after = (await api.settingsGet()).settings;
  check("settings: stored in Rust", after.player.resizeMode === "Stretch" && after.player.decoderPriority === 2 && after.player.preferredAudioLanguage === "en" && after.player.subtitle.fontSizeSp === 24 && !after.home.heroEnabled,
    JSON.stringify({ resize: after.player.resizeMode, decoder: after.player.decoderPriority, audio: after.player.preferredAudioLanguage, font: after.player.subtitle.fontSizeSp, hero: after.home.heroEnabled }));

  click($('[data-tab="home"]')!);
  await waitFor(() => $$(".row-section .card[data-id]").find(visible), 30000);
  await sleep(1500);
  check("settings: Home follows hero off", !$(".hero"), $(".hero") ? "hero still shown" : "no hero, rows shown");

  const movie = await openFromHome("movie", "settings → player");
  if (movie) {
    click($<HTMLButtonElement>('.details [data-testid="watch"]')!);
    await playFirstSource("settings → player", async () => {
      const prop = (name: string) => invoke<string | null>("uitest_player_property", { name }).catch(() => null);
      const [keepaspect, hwdec, font, alang] = await Promise.all([prop("keepaspect"), prop("hwdec"), prop("sub-font-size"), prop("alang")]);
      check("settings → player: image mode Stretch", keepaspect === "no", `keepaspect=${keepaspect}`);
      check("settings → player: software decoding", hwdec === "no", `hwdec=${hwdec}`);
      check("settings → player: subtitle size 24 sp", Number(font) === 72, `sub-font-size=${font} (24 sp × 3)`);
      check("settings → player: audio language en", (alang ?? "").split(",")[0] === "en", `alang=${alang}`);
    });
    await back();
  }
  return original;
}

/** Phase 2 (after a restart): the values are still there; then the originals are restored. */
async function settingsVerify(original: { resizeMode: string; decoderPriority: number; fontSizeSp: number; preferredAudioLanguage: string; heroEnabled: boolean }) {
  await openSettings("playback");
  const resize = $<HTMLSelectElement>('select[data-setting="resizeMode"]')?.value;
  const decoder = $<HTMLSelectElement>('select[data-setting="decoderPriority"]')?.value;
  await openSettings("subtitles");
  const font = $<HTMLInputElement>('input[aria-label="Tamanho da legenda"]')?.value;
  await openSettings("home");
  const hero = $('[role=switch][aria-label="Destaque no topo"]')?.getAttribute("aria-checked");
  check("settings after restart: shown in the UI", resize === "Stretch" && decoder === "2" && font === "24" && hero === "false", JSON.stringify({ resize, decoder, font, hero }));

  // Restore through the same command the UI uses.
  const current = (await api.settingsGet()).settings;
  const restored = await api.settingsSet({
    ...current,
    player: { ...current.player, resizeMode: original.resizeMode as never, decoderPriority: original.decoderPriority, preferredAudioLanguage: original.preferredAudioLanguage, subtitle: { ...current.player.subtitle, fontSizeSp: original.fontSizeSp } },
    home: { ...current.home, heroEnabled: original.heroEnabled },
  });
  check("settings: originals restored", restored.player.resizeMode === original.resizeMode && restored.player.decoderPriority === original.decoderPriority && restored.player.preferredAudioLanguage === original.preferredAudioLanguage && restored.player.subtitle.fontSizeSp === original.fontSizeSp && restored.home.heroEnabled === original.heroEnabled, JSON.stringify(original));
}

// ---------------- Library (phases `library-write` and `library-verify:<json>`) ----------------
// Runs on a copy of the user's NextGen data (Rust `uitest::data_dir`): real favorites, progress
// and history, never the user's own files. Network: the Home/Search catalogs, the sources of one
// movie (played twice) and of two episodes of one series, each source chosen by this script.

type Counts = Record<string, [number, number, number]>;
const storeCounts = () => invoke<Counts>("uitest_store_counts").catch(() => ({}) as Counts);
const heartOf = (card: Element | null) => card?.closest(".card-wrap")?.querySelector<HTMLButtonElement>(".fav-btn") ?? null;
const pressed = (b: Element | null | undefined) => b?.getAttribute("aria-pressed") === "true";
const fmt = (ms: number) => `${Math.floor(ms / 60000)}:${String(Math.floor((ms % 60000) / 1000)).padStart(2, "0")}`;

function othersUnchanged(before: Counts, after: Counts, active: number): [boolean, string] {
  const others = Object.keys({ ...before, ...after }).filter((p) => Number(p) !== active);
  const same = others.every((p) => JSON.stringify(before[p] ?? null) === JSON.stringify(after[p] ?? null));
  return [same, `perfil ativo ${active}: ${JSON.stringify(before[active] ?? [0, 0, 0])} → ${JSON.stringify(after[active] ?? [0, 0, 0])}; outros perfis ${others.map((p) => `${p}: ${JSON.stringify(after[p])}`).join(", ") || "nenhum"}`];
}

async function goTab(tab: string) {
  // From a details page the sidebar click also closes it.
  click($(`[data-tab="${tab}"]`)!);
  await sleep(250);
}

async function homeCards(type: string): Promise<HTMLButtonElement[]> {
  await goTab("home");
  const row = await waitFor(() => $$(".home .row-section").find((r) => r.dataset.contentType === type), 30000);
  if (!row) return [];
  row.scrollIntoView({ block: "center" });
  await waitFor(() => $(".card[data-id]", row), 30000);
  return $$<HTMLButtonElement>(".card[data-id]", row);
}
const homeCard = (id: string) => $$<HTMLButtonElement>(".home .card[data-id]").find((c) => c.dataset.id === id) ?? null;

async function searchCard(name: string, id: string): Promise<HTMLButtonElement | null> {
  await goTab("search");
  const input = await waitFor(() => $<HTMLInputElement>('[data-testid="search-input"]'), 5000);
  if (!input) return null;
  typeInto(input, name);
  return waitFor(() => $$<HTMLButtonElement>(".search .card[data-id]").find((c) => c.dataset.id === id), 40000);
}

async function toggleAndWait(button: () => HTMLButtonElement | null, want: boolean): Promise<boolean> {
  const b = button();
  if (!b) return false;
  click(b);
  const ok = await waitFor(() => pressed(button()) === want, 3000);
  await sleep(300); // Rust confirmed (the optimistic mark is undone on failure)
  return !!ok && pressed(button()) === want;
}

/** Favorite sync for one title: Home ↔ Search ↔ Details, persisted in Rust for the active profile. */
async function favoriteSync(type: "movie" | "series", label: string, active: number): Promise<Opened | null> {
  const cards = await homeCards(type);
  const card = cards[0];
  if (!card) {
    check(`${label}: Home card`, false, `no ${type} row`);
    return null;
  }
  const id = card.dataset.id!;
  const name = card.title;
  const key = `${card.dataset.contentType}/${id}`;
  if (pressed(heartOf(card))) await toggleAndWait(() => heartOf(homeCard(id)), false); // start unsaved

  const before = await storeCounts();
  const on = await toggleAndWait(() => heartOf(homeCard(id)), true);
  const keys = await api.libraryFavoriteKeys();
  const after = await storeCounts();
  const [othersSame, detail] = othersUnchanged(before, after, active);
  check(`${label}: favorite from Home`, on && keys.includes(key), `“${name}” (${key}) ♥ marcado e salvo no Rust`);
  check(`${label}: saved only for the active profile`, othersSame && (after[active]?.[0] ?? 0) === (before[active]?.[0] ?? 0) + 1, detail);

  // Search shows the same state for the same id, and can change it.
  const found = await searchCard(name, id);
  if (!found) {
    check(`${label}: Search shows the saved mark`, false, `a busca por “${name}” não retornou o id ${id}`);
  } else {
    check(`${label}: Search shows the saved mark`, pressed(heartOf(found)), `resultado ${id}: ${pressed(heartOf(found)) ? "♥" : "♡"}`);
    const off = await toggleAndWait(() => heartOf($$<HTMLButtonElement>(".search .card[data-id]").find((c) => c.dataset.id === id) ?? null), false);
    await goTab("home");
    const homeOff = await waitFor(() => homeCard(id) && !pressed(heartOf(homeCard(id))), 3000);
    check(`${label}: unfavorite in Search → Home`, off && !!homeOff && !(await api.libraryFavoriteKeys()).includes(key), "♡ na Busca e na Home, removido no Rust");
  }

  // Details: same state; change it there and Home follows.
  await goTab("home");
  const c = homeCard(id);
  if (!c) return null;
  click(c);
  const opened = await expectDetails(`${label} details`, id, type);
  if (!opened) return null;
  const pill = () => $<HTMLButtonElement>('.details [data-testid="favorite"]');
  const wasOn = pressed(pill());
  check(`${label}: Details shows the same state`, wasOn === (await api.libraryFavoriteKeys()).includes(key), `botão: “${pill()?.textContent}”`);
  const set = await toggleAndWait(pill, !wasOn);
  await back();
  const homeFollows = await waitFor(() => homeCard(id) && pressed(heartOf(homeCard(id))) === !wasOn, 3000);
  check(`${label}: change in Details → Home`, set && !!homeFollows, `Detalhes: ${!wasOn ? "salvo" : "removido"}; card da Home igual`);
  if (wasOn) await toggleAndWait(() => heartOf(homeCard(id)), true); // leave it saved for the restart check
  return { id, name, type: card.dataset.contentType ?? type };
}

async function titleEntry(id: string, season: number | null = null, episode: number | null = null) {
  const p = await api.libraryTitleProgress(id).catch(() => null);
  return p?.entries.find((e) => e.season === season && e.episode === episode) ?? null;
}

async function seekAndSettle(seconds: number) {
  await api.playerCommand({ type: "seek", seconds, relative: false });
  const end = performance.now() + 20000;
  while (performance.now() < end) {
    const s = await api.playerSnapshot().catch(() => null);
    if (s && Math.abs(s.timePos - seconds) < 5) break;
    await sleep(300);
  }
  await sleep(1500); // the tracker saves 700 ms after a seek settles
}

async function libraryNavigation() {
  const order = $$<HTMLButtonElement>(".sidebar ul .nav-item").map((b) => b.dataset.tab);
  const settingsLast = $(".sidebar > [data-tab=settings]") !== null;
  check("library: sidebar order", JSON.stringify(order) === JSON.stringify(["home", "tv", "search", "library"]) && settingsLast, `${order.join(" → ")}; Configurações embaixo: ${settingsLast}`);
  await goTab("library");
  check("library: sidebar item highlighted", !!$('[data-tab="library"].active'), "Biblioteca ativa");
  for (const id of ["continue", "favorites", "history"]) {
    click($(`.library [data-section="${id}"]`)!);
    const body = await waitFor(() => $<HTMLElement>(`[data-testid="library-${id}"], [data-testid="library-empty"]`), 10000);
    check(`library: ${id} opens`, !!body, body ? ((body as HTMLElement).dataset.testid === "library-empty" ? `vazio: ${body.textContent}` : `${$$(".cw-card, .card[data-id], .history-item", body).length} item(ns) na tela`) : "timeout");
  }
}

async function favoritesGrid(movie: Opened, series: Opened) {
  await goTab("library");
  click($('.library [data-section="favorites"]')!);
  const grid = await waitFor(() => $(".library-grid"), 10000);
  const cards = () => $$<HTMLButtonElement>(".library-grid .card[data-id]");
  const m = cards().find((c) => c.dataset.id === movie.id);
  const s = cards().find((c) => c.dataset.id === series.id);
  check("library favorites: both titles in the grid", !!grid && !!m && !!s, `${cards().length} favorito(s); filme: ${$(".card-type", m ?? document)?.textContent ?? "—"}, série: ${$(".card-type", s ?? document)?.textContent ?? "—"}`);
  check("library favorites: type, title, saved mark", $(".card-type", m!)?.textContent === "Filme" && $(".card-type", s!)?.textContent === "Série" && pressed(heartOf(m!)) && !!$(".card-title", m!)?.textContent, "selo Filme/Série, título, ♥");
  setValue($<HTMLSelectElement>('[data-testid="library-sort"]')!, "titleAsc");
  await sleep(300);
  const names = cards().map((c) => c.title);
  const sorted = [...names].sort((a, b) => a.localeCompare(b, "pt-BR", { sensitivity: "base" }));
  check("library favorites: sort by title", JSON.stringify(names) === JSON.stringify(sorted), names.slice(0, 5).join(" · "));
  const moviesChip = $$<HTMLButtonElement>(".library-toolbar .chip").find((b) => b.textContent?.startsWith("Filmes"));
  if (moviesChip) {
    click(moviesChip);
    await sleep(300);
    check("library favorites: filter Filmes", cards().every((c) => c.dataset.contentType === "movie") && cards().some((c) => c.dataset.id === movie.id), `${cards().length} filme(s)`);
    click($$<HTMLButtonElement>(".library-toolbar .chip").find((b) => b.textContent?.startsWith("Todos"))!);
  }
  setValue($<HTMLSelectElement>('[data-testid="library-sort"]')!, "addedDesc");
  click(m!);
  const opened = await expectDetails("library favorites → details", movie.id, movie.type);
  check("library favorites: opens details", !!opened, opened?.name ?? "");
  await back();
}

/** Movie: save on pause and on close, resume from Continue watching, complete at 95 %. */
async function movieProgress(movie: Opened): Promise<{ resumedFrom: number } | null> {
  await homeCards("movie");
  const card = homeCard(movie.id);
  if (!card) return null;
  click(card);
  if (!(await expectDetails("progress movie", movie.id, movie.type))) return null;
  click($<HTMLButtonElement>('.details [data-testid="watch"]')!);
  let target = 0;
  let paused: Awaited<ReturnType<typeof titleEntry>> = null;
  await playFirstSource("progress movie", async () => {
    const s = await api.playerSnapshot();
    target = Math.round(Math.min(300, s.duration * 0.3));
    await seekAndSettle(target);
    await api.playerCommand({ type: "setPaused", paused: true });
    await sleep(1200);
    paused = await titleEntry(movie.id);
    check("progress movie: saved on pause", !!paused && Math.abs(paused.positionMs - target * 1000) < 6000 && !paused.completed,
      paused ? `${fmt(paused.positionMs)} de ${fmt(paused.durationMs)}, concluído: ${paused.completed}` : "no entry");
    await api.playerCommand({ type: "setPaused", paused: false });
    await sleep(2500);
  });
  const closed = await titleEntry(movie.id);
  check("progress movie: saved on close", !!closed && !!paused && closed.positionMs >= (paused as { positionMs: number }).positionMs && !closed.completed,
    closed ? `${fmt(closed.positionMs)} salvo ao fechar; concluído: ${closed.completed} (fechar não conclui)` : "no entry");
  await back();
  if (!closed) return null;

  // Continue watching (Library) → details with the movie selected → first source resumes there.
  await goTab("library");
  click($('.library [data-section="continue"]')!);
  const cw = await waitFor(() => $<HTMLButtonElement>(`.library .cw-card[data-key="${CSS.escape(closed.key)}"]`), 10000);
  const firstKey = $(".library .cw-card")?.dataset.key;
  check("continue watching: movie listed first", !!cw && firstKey === closed.key, cw ? `${$$(".library .cw-card").length} item(ns); barra ${Math.round(closed.fraction * 100)} %; ${cw.textContent?.replace(/\s+/g, " ").slice(0, 90)}` : "missing");
  if (!cw) return null;
  click(cw);
  if (!(await expectDetails("continue watching → details", movie.id, movie.type))) return null;
  const note = await waitFor(() => $('[data-testid="resume-note"]'), 5000);
  check("continue watching: resume position shown", !!note && note.textContent!.includes(fmt(closed.positionMs).slice(0, 3)), note?.textContent ?? "no note");
  const historyBefore = (await api.libraryHistory(0, 200)).items.find((e) => e.key === closed.key);
  let resumedFrom = 0;
  await playFirstSource("resumed movie", async () => {
    const s = await api.playerSnapshot();
    resumedFrom = s.timePos;
    check("progress movie: playback resumed at the saved position", s.timePos >= closed.positionMs / 1000 - 15, `começou em ${s.timePos.toFixed(0)} s (salvo ${Math.round(closed.positionMs / 1000)} s)`);
    await seekAndSettle(Math.floor(s.duration * 0.95));
    const done = await titleEntry(movie.id);
    check("progress movie: completed at 90 % or more", !!done && done.completed, done ? `${fmt(done.positionMs)} de ${fmt(done.durationMs)} → concluído` : "no entry");
  });
  const history = (await api.libraryHistory(0, 200)).items.filter((e) => e.key === closed.key);
  check("history: replay updates the same record", history.length === 1 && !!historyBefore && history[0].lastWatchedEpochMs > historyBefore.lastWatchedEpochMs,
    `${history.length} registro(s) do filme; data ${historyBefore ? new Date(historyBefore.lastWatchedEpochMs).toLocaleTimeString() : "?"} → ${history[0] ? new Date(history[0].lastWatchedEpochMs).toLocaleTimeString() : "?"}`);
  const cwNow = await api.libraryContinueWatching();
  check("progress movie: completed leaves Continue watching", !cwNow.some((e) => e.key === closed.key), `${cwNow.length} item(ns) em Continuar assistindo`);
  const shown = await waitFor(() => /Assistido/.test($('[data-testid="movie-progress"]')?.textContent ?? "") ? $('[data-testid="movie-progress"]') : null, 5000);
  check("details: completed movie marked", !!shown, shown?.textContent ?? "no mark");
  await back();
  return { resumedFrom };
}

/** Two episodes of one series: separate progress, one Continue watching entry (the latest). */
async function episodeProgress(): Promise<{ seriesId: string; key: string; positionMs: number } | null> {
  const cards = await homeCards("series");
  for (let i = 0; i < Math.min(3, cards.length); i++) {
    await homeCards("series");
    const card = $$<HTMLButtonElement>(".home .row-section").find((r) => r.dataset.contentType === "series")?.querySelectorAll<HTMLButtonElement>(".card[data-id]")[i];
    if (!card) break;
    click(card);
    const opened = await expectDetails(`progress series #${i + 1}`, card.dataset.id ?? null, "series");
    if (!opened) continue;
    const seasons = $$<HTMLButtonElement>(".details .season-tabs .chip").filter((t) => !/Especiais/.test(t.textContent ?? ""));
    if (seasons.length) {
      click(seasons[seasons.length - 1]);
      await sleep(300);
    }
    const episodes = $$<HTMLButtonElement>(".details .episode");
    if (episodes.length < 2) {
      await back();
      continue;
    }
    const plays: { season: number; episode: number; seconds: number }[] = [];
    for (const [index, seconds] of [[0, 200], [1, 400]] as const) {
      const ep = $$<HTMLButtonElement>(".details .episode")[index];
      const [season, episode] = (ep.dataset.episode ?? "0:0").split(":").map(Number);
      click(ep);
      const before = checks.length;
      await playFirstSource(`progress episode T${season} E${episode}`, async () => {
        const s = await api.playerSnapshot();
        const at = Math.round(Math.min(seconds, s.duration * 0.5));
        await seekAndSettle(at);
        plays.push({ season, episode, seconds: at });
      });
      const result = checks.slice(before).find((c) => c.name.endsWith(": playback"));
      if (!result?.passed) break;
    }
    if (plays.length < 2) {
      await back();
      continue;
    }
    const [a, b] = plays;
    const ea = await titleEntry(opened.id, a.season, a.episode);
    const eb = await titleEntry(opened.id, b.season, b.episode);
    check("progress episodes: separate positions", !!ea && !!eb && ea.key !== eb.key && Math.abs(ea.positionMs - a.seconds * 1000) < 15000 && Math.abs(eb.positionMs - b.seconds * 1000) < 15000,
      `${ea?.key}: ${ea ? fmt(ea.positionMs) : "—"} · ${eb?.key}: ${eb ? fmt(eb.positionMs) : "—"}`);
    const cw = (await api.libraryContinueWatching()).filter((e) => e.metaId === opened.id);
    check("continue watching: one entry per series (latest episode)", cw.length === 1 && cw[0].key === eb?.key, cw.map((e) => e.key).join(", ") || "none");
    const rows = await waitFor(() => ($$('[data-testid="episode-progress"]').length >= 2 ? $$('[data-testid="episode-progress"]') : null), 5000);
    check("details: episode progress shown", !!rows, rows ? rows.map((r) => r.textContent).join(" | ") : "missing");
    const resume = $('[data-testid="resume-episode"]');
    check("details: series resume button", !!resume && resume.textContent!.includes(`E${b.episode}`), resume?.textContent ?? "missing");
    await back();
    return eb ? { seriesId: opened.id, key: eb.key, positionMs: eb.positionMs } : null;
  }
  check("progress episodes: two playable episodes", false, "no series with two playable episodes in 3 tries");
  return null;
}

async function historyChecks(removeKey: string | null) {
  const page = await api.libraryHistory(0, 200);
  const dates = page.items.map((e) => e.lastWatchedEpochMs);
  check("history: newest first", dates.every((d, i) => i === 0 || dates[i - 1] >= d), `${page.total} registro(s); primeiros: ${page.items.slice(0, 3).map((e) => e.key).join(", ")}`);
  await goTab("library");
  click($('.library [data-section="history"]')!);
  await waitFor(() => $(".history-item"), 10000);
  const first = $$(".history-item").length;
  if (page.total > first) {
    $$(".history-item").at(-1)?.scrollIntoView({ block: "end" });
    const more = await waitFor(() => ($$(".history-item").length > first ? $$(".history-item") : null), 8000);
    check("history: large list loads page by page", !!more, `${first} na primeira página, ${more?.length ?? first} após rolar, total ${page.total}`);
  } else {
    check("history: large list loads page by page", true, `total ${page.total} cabe em uma página (${first})`);
  }
  if (removeKey) {
    const row = $(`.history-item[data-key="${CSS.escape(removeKey)}"]`);
    row?.scrollIntoView({ block: "center" });
    if (row) click($<HTMLButtonElement>('[data-testid="history-remove"]', row)!);
    const gone = await waitFor(() => !$(`.history-item[data-key="${CSS.escape(removeKey)}"]`), 5000);
    const still = (await api.libraryHistory(0, 500)).items.some((e) => e.key === removeKey);
    check("history: remove a record", !!row && !!gone && !still, `${removeKey} removido`);
  }
}

async function libraryWrite(): Promise<Record<string, unknown>> {
  const boot = await api.bootstrap();
  const active = boot.activeProfile;
  const countsBefore = await storeCounts();
  check("library: data imported from the original", true, `migração v${boot.migration?.version}: ${JSON.stringify((boot.migration as unknown as { library?: unknown })?.library ?? null)}; por perfil ${JSON.stringify(countsBefore)}`);
  await libraryNavigation();
  const movie = await favoriteSync("movie", "favorites movie", active);
  const series = await favoriteSync("series", "favorites series", active);
  if (movie && series) await favoritesGrid(movie, series);
  if (movie) await movieProgress(movie);
  const ep = await episodeProgress();
  // Removes the first of the two episodes (the second stays for the restart check).
  const firstEpisode = ep ? (await api.libraryHistory(0, 50)).items.find((e) => e.metaId === ep.seriesId && e.key !== ep.key)?.key ?? null : null;
  await historyChecks(firstEpisode);
  const countsAfter = await storeCounts();
  const [same, detail] = othersUnchanged(countsBefore, countsAfter, active);
  check("profiles: only the active profile changed", same, detail);
  await capture("library-history");
  return { library: { active, movie, series, episode: ep, countsBefore } };
}

async function libraryVerify(info: { active: number; movie: Opened | null; series: Opened | null; episode: { seriesId: string; key: string; positionMs: number } | null; countsBefore: Counts }) {
  const keys = await api.libraryFavoriteKeys();
  const want = [info.movie, info.series].filter(Boolean).map((t) => `${t!.type}/${t!.id}`);
  check("after restart: favorites kept", want.length === 2 && want.every((k) => keys.includes(k)), want.join(", "));
  if (info.movie) {
    const e = await titleEntry(info.movie.id);
    check("after restart: completed movie kept", !!e && e.completed, e ? `${fmt(e.positionMs)} concluído` : "missing");
  }
  if (info.episode) {
    const cw = await api.libraryContinueWatching();
    const entry = cw.find((e) => e.key === info.episode!.key);
    check("after restart: Continue watching kept", !!entry && Math.abs(entry.positionMs - info.episode.positionMs) < 5000, entry ? `${entry.key} em ${fmt(entry.positionMs)}` : "missing");
    await goTab("home");
    const homeRow = await waitFor(() => $<HTMLElement>(`[data-testid="home-continue"] .cw-card[data-key="${CSS.escape(info.episode!.key)}"]`), 30000);
    check("Home: Continue watching row", !!homeRow, homeRow ? `${$$('[data-testid="home-continue"] .cw-card').length} item(ns) na fileira` : "missing");
    if (homeRow) {
      click(homeRow as HTMLElement);
      if (await expectDetails("Home continue → details", info.episode.seriesId, "series")) {
        const note = await waitFor(() => $('[data-testid="resume-note"]'), 8000);
        check("after restart: resume position offered", !!note && note.textContent!.includes(fmt(info.episode.positionMs).slice(0, 3)), note?.textContent ?? "no note");
        await back();
      }
    }
  }
  const [same, detail] = othersUnchanged(info.countsBefore, await storeCounts(), info.active);
  check("after restart: other profiles untouched", same, detail);

  // Removal straight from the Library down to the empty state (test copy of the data only).
  await goTab("library");
  click($('.library [data-section="favorites"]')!);
  await waitFor(() => $(".library-grid .card[data-id]"), 10000);
  for (let i = 0; i < 400 && $(".library-grid .fav-btn"); i++) {
    click($<HTMLButtonElement>(".library-grid .fav-btn")!);
    await sleep(120);
  }
  const empty = await waitFor(() => $('[data-testid="library-empty"]'), 8000);
  check("library: remove favorites in the Library and empty state", !!empty && (await api.libraryFavoriteKeys()).length === 0, empty?.textContent ?? "not empty");
  await capture("library-empty");
}

// ---------------- Addons and profiles (phases `manage-write` and `manage-verify:<json>`) ----------------
// Runs on a copy of the user's NextGen data. Network: the profile's own addons (as the app does), at
// most 3 public Stremio manifests tried for the install test (first one not installed is used),
// one movie source played to test a profile switch during playback. Confirm dialogs are answered
// by the script and their text is recorded.

let lastConfirm = "";
function answerConfirms(answer: boolean) {
  window.confirm = (message?: string) => {
    lastConfirm = String(message ?? "");
    return answer;
  };
}

const PUBLIC_ADDONS = [
  "https://opensubtitles-v3.strem.io/manifest.json",
  "https://watchhub.strem.io/manifest.json",
  "https://v3-cinemeta.strem.io/manifest.json",
];

async function openAddons(): Promise<HTMLElement | null> {
  await goTab("settings");
  if (!(await openSettings("addons"))) return null;
  return waitFor(() => $('[data-testid="addons"]'), 30000);
}

const addonCards = () => $$<HTMLLIElement>(".addon-list .addon-card");
const cardByKey = (key: string) => $<HTMLLIElement>(`.addon-list .addon-card[data-key="${key}"]`);
async function feedbackAfter(action: () => void, timeoutMs = 30000): Promise<string> {
  const before = $('[data-testid="addon-feedback"]')?.textContent;
  action();
  const el = await waitFor(() => {
    const f = $('[data-testid="addon-feedback"]');
    return f && !f.classList.contains("busy") && f.textContent !== before ? f : null;
  }, timeoutMs);
  return el ? `${el.classList.contains("error") ? "erro: " : ""}${el.textContent}` : "sem retorno";
}

async function homeRowsByAddon(): Promise<Record<string, number>> {
  await goTab("home");
  await waitFor(() => $$(".home .row-section[data-content-type]").length > 0, 30000);
  const out: Record<string, number> = {};
  for (const h of $$(".home .row-section[data-content-type] .row-header .muted")) {
    const name = h.textContent ?? "";
    out[name] = (out[name] ?? 0) + 1;
  }
  return out;
}

async function openProfileMenu(): Promise<HTMLElement | null> {
  if (!$('[data-testid="profile-menu"]')) click($('[data-testid="profile-chip"]')!);
  return waitFor(() => $('[data-testid="profile-menu"]'), 3000);
}

async function switchTo(index: number): Promise<boolean> {
  const menu = await openProfileMenu();
  const option = menu ? $<HTMLButtonElement>(`.profile-option[data-profile="${index}"]`, menu) : null;
  if (!option) return false;
  option.click();
  const ok = await waitFor(() => !$('[data-testid="profile-switching"]') && !$('[data-testid="profile-menu"]') ? true : null, 20000);
  return !!ok && (await api.profilesList()).active === index;
}

async function manageWrite(): Promise<Record<string, unknown>> {
  answerConfirms(true);
  const boot = await api.bootstrap();
  const profiles = await api.profilesList();
  const first = profiles.active;
  const other = profiles.profiles.find((p) => p.index !== first);
  check("profiles: imported list", profiles.profiles.length >= 1, `${profiles.profiles.map((p) => `${p.index}:${p.name}${p.index === first ? " (ativo)" : ""}`).join(", ")} · migração v${boot.migration?.version}`);

  const menu = await openProfileMenu();
  check("profiles: menu opens from the sidebar avatar", !!menu, menu ? `${$$(".profile-option[data-profile]", menu).length} perfil(is)` : "não abriu");
  await capture("profile-menu");
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
  await sleep(200);

  // ---- Addons: real list ----
  const page = await openAddons();
  await sleep(500);
  await capture("addons-page");
  const listed = await api.addonsList();
  check("addons: real installed list", !!page && addonCards().length === listed.addons.length && listed.addons.length > 0,
    `${listed.addons.length} addon(s): ${listed.addons.map((a) => `${a.name} [${a.status}]`).join(", ")}`);
  const pageText = page?.textContent ?? "";
  check("addons: no manifest address on screen", !/manifest\.json|https?:\/\//.test(pageText), "somente nome e host");
  const configurable = listed.addons.filter((a) => a.configurable).length;
  check("addons: details shown (logo, version, types, catalogs)", listed.addons.some((a) => a.status === "ok" && a.version && a.types.length > 0),
    `${listed.addons.filter((a) => a.logo).length} com logo, ${listed.addons.reduce((n, a) => n + a.catalogCount, 0)} catálogos, ${configurable} configurável(is) (botão Configurar abre a página do addon no navegador, não clicado no teste)`);

  // ---- Install: invalid, valid, duplicate ----
  const input = $<HTMLInputElement>('[data-testid="addon-url"]')!;
  typeInto(input, "endereco-que-nao-existe.invalid");
  await sleep(150); // React enables "Verificar" after the input event
  const invalid = await feedbackAfter(() => click($<HTMLButtonElement>('[data-testid="addon-check"]')!), 75000);
  check("addons install: invalid address rejected", /erro:/.test(invalid) && !$('[data-testid="addon-preview"]'), invalid);
  let installed: { key: string; name: string } | null = null;
  for (const url of PUBLIC_ADDONS) {
    typeInto(input, url);
    await sleep(150);
    const result = await feedbackAfter(() => click($<HTMLButtonElement>('[data-testid="addon-check"]')!)).catch(() => "");
    const preview = await waitFor(() => $('[data-testid="addon-preview"]'), 2000);
    if (!preview) {
      check(`addons install: ${new URL(url).host} skipped`, true, result);
      continue;
    }
    const name = $(".addon-title", preview)?.textContent ?? "";
    check("addons install: manifest validated and shown", true, `${name} · ${$(".addon-badges", preview)?.textContent} · ${$(".addon-catalogs", preview)?.textContent ?? "sem catálogos"}`);
    const before = (await api.addonsList()).addons.length;
    const done = await feedbackAfter(() => click($<HTMLButtonElement>('[data-testid="addon-install"]')!));
    const after = await api.addonsList();
    const added = after.addons.find((a) => !listed.addons.some((x) => x.key === a.key));
    check("addons install: installed for the active profile", after.addons.length === before + 1 && !!added && added.status === "ok", `${done}; ${after.addons.length} addons`);
    if (added) installed = { key: added.key, name: added.name };
    typeInto(input, url);
    await sleep(150);
    const dup = await feedbackAfter(() => click($<HTMLButtonElement>('[data-testid="addon-check"]')!));
    check("addons install: duplicate detected", /já está instalado/.test(dup), dup);
    break;
  }
  if (!installed) check("addons install: valid addon", false, "nenhum dos manifests públicos estava disponível ou todos já estavam instalados");

  // ---- Enable / disable: Home follows, no data lost ----
  const counts0 = await storeCounts();
  const withRows = await homeRowsByAddon();
  const target = (await api.addonsList()).addons.find((a) => a.status === "ok" && (withRows[a.name] ?? 0) > 0);
  if (target) {
    await openAddons();
    const off = await feedbackAfter(() => click($<HTMLButtonElement>('[data-testid="addon-toggle"]', cardByKey(target.key)!)!));
    const rowsOff = await homeRowsByAddon();
    check("addons: disable → Home without its catalogs", (rowsOff[target.name] ?? 0) === 0, `${off}; fileiras de ${target.name}: ${withRows[target.name]} → ${rowsOff[target.name] ?? 0}`);
    await openAddons();
    const on = await feedbackAfter(() => click($<HTMLButtonElement>('[data-testid="addon-toggle"]', cardByKey(target.key)!)!));
    const rowsOn = await homeRowsByAddon();
    check("addons: enable → catalogs back", (rowsOn[target.name] ?? 0) === withRows[target.name], `${on}; fileiras: ${rowsOn[target.name] ?? 0}`);
  } else {
    check("addons: disable/enable with Home rows", false, "nenhum addon com fileiras na Home");
  }
  if (installed) {
    await openAddons();
    const off = await feedbackAfter(() => click($<HTMLButtonElement>('[data-testid="addon-toggle"]', cardByKey(installed!.key)!)!));
    check("addons: installed addon left disabled for the restart check", (await api.addonsList()).addons.find((a) => a.key === installed!.key)?.status === "disabled", off);
    const refreshTarget = (await api.addonsList()).addons.find((a) => a.status === "ok");
    if (refreshTarget) {
      const r = await feedbackAfter(() => click($<HTMLButtonElement>('[data-testid="addon-refresh"]', cardByKey(refreshTarget.key)!)!));
      check("addons: manual refresh feedback", /atualizado/.test(r), r);
    }
  }
  const counts1 = await storeCounts();
  check("addons: library data untouched by addon changes", JSON.stringify(counts0) === JSON.stringify(counts1), JSON.stringify(counts1));

  // ---- Profiles: switch during playback ----
  if (!other) {
    check("profiles: second profile", false, "só existe um perfil");
    return { manage: { first, installed } };
  }
  const fav1 = await api.libraryFavoriteKeys();
  const cw1 = (await api.libraryContinueWatching()).map((e) => e.key);
  const settings1 = (await api.settingsGet()).settings;
  const addons1 = (await api.addonsList()).addons.map((a) => a.key);
  const countsBefore = await storeCounts();

  await goTab("home"); // rows load only while Home is visible
  const movie = await openFromHome("movie", "switch during playback");
  let movieId: string | null = null;
  if (movie) {
    movieId = movie.id;
    click($<HTMLButtonElement>('.details [data-testid="watch"]')!);
    const list = await waitFor(() => $(".details .streams"), 45000);
    const source = list ? $<HTMLButtonElement>(".stream:not(:disabled)", list) : null;
    if (source) {
      click(source);
      let snap = null;
      const end = performance.now() + 60000;
      while (performance.now() < end) {
        snap = await api.playerSnapshot().catch(() => null);
        if (snap?.loaded && snap.timePos > 1) break;
        await sleep(500);
      }
      if (snap?.loaded) await seekAndSettle(Math.round(Math.min(150, snap.duration * 0.3)));
      check("profiles: playback active before the switch", !!snap?.loaded && !!$(".full-player"), snap ? `${snap.timePos.toFixed(0)} s` : "não iniciou");
    } else {
      check("profiles: playback active before the switch", false, "sem fonte reproduzível");
    }
  }
  // The full player covers the sidebar; the switch is requested through the same menu (DOM click).
  const switched = await switchTo(other.index);
  check("profiles: switch without restart", switched, `perfil ${other.index} (${other.name}) ativo`);
  check("profiles: player closed by the switch", !$(".full-player") && !(await api.playerSnapshot().then(() => true).catch(() => false)), "sessão encerrada");
  const chip = $('[data-testid="profile-chip"]')?.textContent ?? "";
  check("profiles: sidebar shows the new profile", chip.includes(other.name), chip);

  const fav2 = await api.libraryFavoriteKeys();
  const cw2 = (await api.libraryContinueWatching()).map((e) => e.key);
  const settings2 = await api.settingsGet();
  const addons2 = await api.addonsList();
  check("profiles: addons of the new profile", addons2.profile === other.index && JSON.stringify(addons2.addons.map((a) => a.key)) !== JSON.stringify(addons1), `${addons2.addons.length} addon(s) do perfil ${addons2.addonProfile}`);
  check("profiles: favorites of the new profile", JSON.stringify(fav2) !== JSON.stringify(fav1) || fav1.length === 0, `${fav1.length} → ${fav2.length} favorito(s)`);
  check("profiles: Continue watching of the new profile", JSON.stringify(cw2) !== JSON.stringify(cw1), `${cw1.length} → ${cw2.length} item(ns)`);
  check("profiles: settings of the new profile", settings2.profile.index === other.index, `perfil ${settings2.profile.name}: áudio ${settings2.settings.player.preferredAudioLanguage}, destaque ${settings2.settings.home.heroEnabled} (antes: ${settings1.player.preferredAudioLanguage}, ${settings1.home.heroEnabled})`);
  if (movieId) {
    const p2 = await api.libraryTitleProgress(movieId);
    check("profiles: playback progress not inherited", !p2.entries.some((e) => e.lastWatchedEpochMs > Date.now() - 5 * 60000), `${p2.entries.length} registro(s) deste filme no perfil ${other.index}, nenhum recente`);
  }
  const rows = await homeRowsByAddon();
  const heartsOn = $$(".home .card-wrap .fav-btn.on").map((b) => b.closest(".card-wrap")?.querySelector<HTMLElement>(".card")?.dataset.id);
  check("profiles: Home reloaded for the new profile", Object.keys(rows).length > 0 && heartsOn.every((id) => fav2.some((k) => k.endsWith(`/${id}`))),
    `${Object.values(rows).reduce((a, b) => a + b, 0)} fileiras de ${Object.keys(rows).length} addon(s); ${heartsOn.length} ♥ todos deste perfil`);
  await goTab("library");
  click($('.library [data-section="favorites"]')!);
  const libShown = await waitFor(() => $('[data-testid="library-favorites"], [data-testid="library-empty"]'), 10000);
  const libIds = $$(".library-grid .card[data-id]").map((c) => `${c.dataset.contentType}/${c.dataset.id}`);
  check("profiles: Library of the new profile", !!libShown && libIds.every((k) => fav2.includes(k)) && libIds.length === Math.min(fav2.length, 60), `${libIds.length} favorito(s) na tela`);
  await goTab("tv");
  const tv = await waitFor(() => $(".channel") ?? $$(".livetv .screen-message").find((m) => !/Carregando/.test(m.textContent ?? "")), 60000);
  check("profiles: TV with the new profile's addons", !!tv, tv ? `${$$(".channel").length} canais` : "timeout");
  const countsAfter = await storeCounts();
  const p1Before = countsBefore[String(first)];
  const p1After = countsAfter[String(first)];
  check("profiles: no data mixed", JSON.stringify(countsBefore[String(other.index)]) === JSON.stringify(countsAfter[String(other.index)]) && p1After[0] === p1Before[0],
    `perfil ${first}: ${JSON.stringify(p1Before)} → ${JSON.stringify(p1After)} (progresso do filme salvo nele); perfil ${other.index}: ${JSON.stringify(countsAfter[String(other.index)])} sem mudança`);
  await capture("profile-switched");
  return { manage: { first, other: other.index, installed, movieId } };
}

async function manageVerify(info: { first: number; other?: number; installed: { key: string; name: string } | null; movieId?: string | null }) {
  answerConfirms(true);
  const profiles = await api.profilesList();
  check("after restart: active profile kept", info.other === undefined || profiles.active === info.other, `perfil ativo ${profiles.active}`);
  const menu = await openProfileMenu();
  const active = menu ? $('.profile-option[aria-checked="true"]', menu)?.textContent : null;
  check("profiles: menu lists profiles and marks the active one", !!menu && $$(".profile-option[data-profile]", menu!).length === profiles.profiles.length && !!active, `${$$(".profile-option[data-profile]", menu!).map((b) => b.textContent).join(" | ")}; ativo: ${active}`);
  check("profiles: switch back", await switchTo(info.first), `perfil ${info.first}`);
  if (info.movieId) {
    const p = await api.libraryTitleProgress(info.movieId);
    const recent = p.entries.find((e) => e.lastWatchedEpochMs > Date.now() - 60 * 60000);
    check("profiles: progress of the interrupted playback saved in its own profile", !!recent, recent ? `${Math.round(recent.positionMs / 1000)} s` : "sem registro");
  }
  const counts0 = await storeCounts();
  if (info.installed) {
    await openAddons();
    const a = (await api.addonsList()).addons.find((x) => x.key === info.installed!.key);
    check("after restart: addon and disabled state kept", a?.status === "disabled", a ? `${a.name}: ${a.status}` : "ausente");
    lastConfirm = "";
    const r = await feedbackAfter(() => click($<HTMLButtonElement>('[data-testid="addon-remove"]', cardByKey(info.installed!.key)!)!));
    check("addons: remove with confirmation", /Excluir addon/.test(lastConfirm) && !(await api.addonsList()).addons.some((x) => x.key === info.installed!.key), `${r} · confirmação: “${lastConfirm.split("\n")[0]}”`);
    check("addons: library kept after removal", JSON.stringify(await storeCounts()) === JSON.stringify(counts0), JSON.stringify(counts0));
  }
  const home = await homeRowsByAddon();
  check("regression: Home", Object.keys(home).length > 0, `${Object.values(home).reduce((a, b) => a + b, 0)} fileiras`);

  // Create, use and delete a profile through the dialog.
  await openProfileMenu();
  click((await waitFor(() => $<HTMLButtonElement>('[data-testid="profiles-manage"]'), 3000))!);
  const dialog = await waitFor(() => $('[data-testid="profiles-dialog"]'), 3000);
  const add = dialog ? $<HTMLButtonElement>('[data-testid="profile-add"]', dialog) : null;
  if (!add) {
    check("profiles: create", false, "limite atingido ou diálogo ausente");
    return;
  }
  click(add);
  const nameInput = await waitFor(() => $<HTMLInputElement>('[data-testid="profile-name"]'), 3000);
  if (!nameInput) {
    check("profiles: create", false, "formulário não abriu");
    return;
  }
  typeInto(nameInput, "Teste NextGen");
  await sleep(150);
  click($<HTMLButtonElement>(".swatch:nth-child(3)")!);
  await sleep(100);
  click($<HTMLButtonElement>('[data-testid="profile-save"]')!);
  const tile = await waitFor(() => $$<HTMLButtonElement>(".profile-tile[data-profile]").find((t) => /Teste NextGen/.test(t.textContent ?? "")), 5000);
  const newIndex = Number(tile?.dataset.profile ?? 0);
  check("profiles: create", !!tile, tile ? `perfil ${newIndex}` : "não criado");
  click($<HTMLButtonElement>('.modal [aria-label="Fechar"]')!);
  if (!newIndex) return;
  check("profiles: switch to the new profile", await switchTo(newIndex), `perfil ${newIndex}`);
  const empty = await Promise.all([api.libraryFavoriteKeys(), api.libraryContinueWatching(), api.addonsList()]);
  check("profiles: new profile starts empty", empty[0].length === 0 && empty[1].length === 0 && empty[2].addons.length === 0, `favoritos ${empty[0].length}, continuar ${empty[1].length}, addons ${empty[2].addons.length}`);
  await goTab("home");
  const msg = await waitFor(() => $$(".home .screen-message, .screen-message").find(visible), 15000);
  check("profiles: Home of a profile without addons", !!msg, msg?.textContent ?? "");
  check("profiles: back to the first profile", await switchTo(info.first), `perfil ${info.first}`);
  await openProfileMenu();
  click((await waitFor(() => $<HTMLButtonElement>('[data-testid="profiles-manage"]'), 3000))!);
  const t2 = await waitFor(() => $<HTMLButtonElement>(`.profile-tile[data-profile="${newIndex}"]`), 3000);
  if (t2) {
    click(t2);
    lastConfirm = "";
    click((await waitFor(() => $<HTMLButtonElement>('[data-testid="profile-delete"]'), 3000))!);
    const gone = await waitFor(() => !$(`.profile-tile[data-profile="${newIndex}"]`) && $$(".profile-tile[data-profile]").length > 0, 5000);
    check("profiles: delete with confirmation and backup", !!gone && /Excluir perfil/.test(lastConfirm) && !(await api.profilesList()).profiles.some((p) => p.index === newIndex), `confirmação: “${lastConfirm.split("\n")[0]}”`);
    click($<HTMLButtonElement>('.modal [aria-label="Fechar"]')!);
  }
  const countsEnd = await storeCounts();
  check("profiles: other profiles untouched", [info.first, info.other].filter((x): x is number => x !== undefined).every((p) => JSON.stringify(countsEnd[String(p)]) === JSON.stringify(counts0[String(p)])), JSON.stringify(countsEnd));
  await goTab("tv");
  const tv = await waitFor(() => $(".channel") ?? $$(".livetv .screen-message").find((m) => !/Carregando/.test(m.textContent ?? "")), 60000);
  check("regression: TV", !!tv, tv ? `${$$(".channel").length} canais` : "timeout");
  await capture("profiles-after");
}

// ---------------- Account UI (phases `account-ui` and `account-verify:<json>`) ----------------
// Copy of the user's data, signed out (the session file is not copied). Network: the account's
// public avatar catalog (read-only). No sign-in attempt, nothing written to any account.

async function accountUi(): Promise<Record<string, unknown>> {
  const status = await api.accountStatus();
  check("account: status from Rust", true, `configurado ${status.configured}, conectado ${status.signedIn}, estado ${status.state}, pendentes ${status.pending}`);
  check("account: no sync dot while signed out", !$('[data-testid="nav-sync"]'), "indicador oculto");
  await openSettings("account");
  const panel = await waitFor(() => $('[data-testid="account"]'), 10000);
  check("account: page in Settings → Conta", !!panel, panel ? ($(".settings-header h1")?.textContent ?? "") : "não abriu");
  if (status.configured) {
    const email = $<HTMLInputElement>('[data-testid="account-email"]');
    const password = $<HTMLInputElement>('[data-testid="account-password"]');
    const button = $<HTMLButtonElement>('[data-testid="sign-in"]');
    check("account: sign-in form", !!email && !!password && password.type === "password" && !!button?.disabled, "e-mail, senha (campo oculto), Entrar desabilitado vazio");
    if (email && password) {
      typeInto(email, "teste@exemplo.test");
      typeInto(password, "123456");
      await sleep(150);
      check("account: Entrar enabled when filled", !$<HTMLButtonElement>('[data-testid="sign-in"]')!.disabled, "habilitado");
      typeInto(password, "");
    }
    check("account: no token or key in the page", !/eyJ[A-Za-z0-9_-]{10,}|access_token|refresh_token/.test(document.documentElement.innerHTML), "nenhum token no DOM");
  }

  // Avatars of the account's catalog.
  const avatars = await api.avatarsCatalog().catch((e) => String(e));
  const ok = Array.isArray(avatars) && avatars.length > 0;
  check("avatars: catalog of the account", ok, Array.isArray(avatars) ? `${avatars.length} avatares, categorias ${[...new Set(avatars.map((a) => a.category))].join(", ")}` : avatars);
  let chosen: string | null = null;
  if (ok) {
    const profiles = await api.profilesList();
    await openProfileMenu();
    click((await waitFor(() => $<HTMLButtonElement>('[data-testid="profiles-manage"]'), 3000))!);
    click((await waitFor(() => $<HTMLButtonElement>(`.profile-tile[data-profile="${profiles.active}"]`), 3000))!);
    const picker = await waitFor(() => $(".avatar-picker"), 5000);
    const imgs = picker ? $$<HTMLImageElement>(".avatar-choice img", picker) : [];
    await waitFor(() => imgs.some((i) => i.complete && i.naturalWidth > 0), 10000);
    const loaded = imgs.filter((i) => i.complete && i.naturalWidth > 0).length;
    check("avatars: picker shows the catalog images", !!picker && loaded > 0, `${imgs.length} opções, ${loaded} imagens carregadas`);
    const pinText = $('[data-testid="pin-section"]')?.textContent ?? $$(".setting-row").find((r) => /Bloqueio por PIN/.test(r.textContent ?? ""))?.textContent ?? "";
    check("pin: needs the account when signed out", /Entre na conta Nuvio/.test(pinText), pinText.slice(0, 90));
    const choice = picker ? $<HTMLButtonElement>(".avatar-choice[data-avatar]:not(.active)", picker) : null;
    if (choice) {
      chosen = choice.dataset.avatar ?? null;
      click(choice);
      await sleep(150);
      click($<HTMLButtonElement>('[data-testid="profile-save"]')!);
      const saved = await waitFor(() => !$('[data-testid="profile-name"]') ? true : null, 5000);
      const after = await api.profilesList();
      check("avatars: chosen avatar saved", !!saved && after.profiles.find((p) => p.index === after.active)?.avatarId === chosen, `avatar ${chosen}`);
      click($<HTMLButtonElement>('.modal [aria-label="Fechar"]')!);
      const img = await waitFor(() => $<HTMLImageElement>('[data-testid="profile-chip"] .avatar-img img'), 5000);
      check("avatars: sidebar shows the avatar image", !!img, img ? "imagem do avatar na barra lateral" : "sem imagem");
    }
  }
  await capture("account-page");
  return { account: { chosen } };
}

async function accountVerify(info: { chosen: string | null }) {
  const profiles = await api.profilesList();
  const active = profiles.profiles.find((p) => p.index === profiles.active);
  check("after restart: avatar kept", !info.chosen || active?.avatarId === info.chosen, `avatar ${active?.avatarId}`);
  const img = await waitFor(() => $<HTMLImageElement>('[data-testid="profile-chip"] .avatar-img img'), 15000);
  check("after restart: avatar image in the sidebar", !info.chosen || !!img, img ? "imagem carregada" : "sem imagem");
  const state = await api.accountStatus();
  check("after restart: still signed out, data local", !state.signedIn && state.pending >= 1, `estado ${state.state}, ${state.pending} alteração(ões) local(is) guardada(s) para enviar quando houver conta`);
}

// ---------------- P2P (phase `p2p`) ----------------
// A freely licensed torrent (Big Buck Bunny, CC BY) added by Rust to the sources of a movie only
// in this phase. Real UI path: source → P2P notice → engine → app player; picture checked, then a
// seek far ahead (pieces fetched on demand, before the download ends), stats overlay, close, and
// the process's sockets/threads compared with before the session.

type Resources = { tcpListening: number; tcpEstablished: number; tcpOther: number; udp: number; threads: number };
const resources = () => invoke<Resources>("uitest_process_resources");

async function waitPicture(label: string, minTime: number, timeoutMs: number): Promise<boolean> {
  const start = performance.now();
  let detail = "";
  let ok = false;
  while (performance.now() - start < timeoutMs) {
    const snap = await api.playerSnapshot().catch(() => null);
    if (snap?.error) { detail = snap.error; break; }
    if (snap && snap.loaded && snap.timePos > minTime) {
      ok = true;
      detail = `time ${snap.timePos.toFixed(1)} s de ${snap.duration?.toFixed(0)} s, ${snap.width}×${snap.height}, ${snap.hwdecCurrent ?? "software"}`;
      break;
    }
    await sleep(500);
  }
  if (!ok) {
    check(`${label}: playing`, false, detail || `no progress past ${minTime} s in ${timeoutMs / 1000} s`);
    return false;
  }
  let stats: RegionStats | null = null;
  let picture = false;
  for (let tries = 0; tries < 3 && !picture; tries++) {
    await sleep(tries === 0 ? 1500 : 2500);
    stats = await capture(`${label.replace(/\W+/g, "-")}`);
    picture = !!stats && stats.motion > 0.3 && stats.whiteFraction < 0.98 && stats.blackFraction < 0.98;
  }
  check(`${label}: playing`, picture, `${detail}; window motion ${stats?.motion.toFixed(2)}, black ${Math.round((stats?.blackFraction ?? 0) * 100)} %`);
  return picture;
}

async function p2pPhase(): Promise<Record<string, unknown>> {
  const before = await resources();
  const movie = await openFromHome("movie", "p2p");
  if (!movie) return {};
  const watch = $<HTMLButtonElement>('.details [data-testid="watch"]');
  if (watch) click(watch);
  const source = await waitFor(() => $$<HTMLButtonElement>('.details .stream[data-kind="torrent"]').find((b) => /Big Buck Bunny/.test(b.textContent ?? "")), 45000);
  check("p2p: torrent source listed and playable", !!source && !source.disabled, source ? `${source.textContent?.trim()} (tag P2P: ${!!$(".stream-tag", source)})` : "missing");
  if (!source) return {};
  const status0 = await api.p2pStatus();
  check("p2p: no session before the click", status0.state === "idle" || status0.state === "", status0.state || "none");
  click(source);

  // P2P is off on this profile: the original's notice comes first.
  const notice = await waitFor(() => $<HTMLButtonElement>('[data-testid="p2p-accept"]'), 15000);
  const noticeText = $(".p2p-consent")?.textContent ?? "";
  check("p2p: notice before any connection", !!notice && /endereço IP ficará visível/.test(noticeText), notice ? "aviso P2P do original exibido" : "no notice");
  const during0 = await resources();
  check("p2p: nothing started before consent", during0.udp <= before.udp && during0.tcpListening <= before.tcpListening, JSON.stringify(during0));
  if (!notice) return {};
  const t0 = performance.now();
  click(notice);

  const phases = new Set<string>();
  const full = await waitFor(() => {
    const ph = $('[data-testid="torrent-phase"]')?.textContent?.trim();
    if (ph) phases.add(ph);
    const err = $('[data-testid="torrent-error"]')?.textContent;
    if (err) return err;
    return $(".full-player") ? "ok" : null;
  }, 150000);
  const startup = Math.round((performance.now() - t0) / 100) / 10;
  check("p2p: engine ready, player opened", full === "ok", `${full ?? "timeout 150 s"} em ${startup} s; fases: ${[...phases].join(" → ")}`);
  if (full !== "ok") return {};
  const status1 = await api.p2pStatus();
  check("p2p: file identified", status1.state === "streaming" && /Big Buck Bunny\.mp4/.test(status1.fileName ?? ""), `${status1.fileName} · ${(status1.fileSize / 1048576).toFixed(1)} MB`);

  const playing = await waitPicture("p2p-start", 3, 90000);
  const during = await resources();
  const overlay = await waitFor(() => $('[data-testid="p2p-stats"]'), 5000);
  const st = await api.p2pStatus();
  check("p2p: stats over the player", !!overlay && st.peers > 0, `${overlay?.textContent?.trim()} | peers ${st.peers}, seeds ${st.seeds}, ↓ ${st.downloadSpeed} B/s, baixado ${(st.downloadedBytes / 1048576).toFixed(1)} MB, ${Math.round(st.totalProgress * 100)} %`);
  check("p2p: started before the download ended", playing && st.totalProgress < 0.9, `progresso do arquivo ao tocar: ${(st.totalProgress * 100).toFixed(1)} %`);

  // Seek far ahead: those pieces are fetched on demand.
  await api.playerCommand({ type: "seek", seconds: 420, relative: false });
  await waitPicture("p2p-seek-420s", 421, 90000);
  const st2 = await api.p2pStatus();

  const close = $<HTMLButtonElement>('.full-player .full-player-top .back');
  if (close) close.click();
  const closed = await waitFor(() => !$(".full-player"), 10000);
  check("p2p: player closed", !!closed, closed ? "ok" : "still open");
  let stopped = false;
  for (let i = 0; i < 60 && !stopped; i++) {
    stopped = (await api.p2pStatus()).state === "idle";
    if (!stopped) await sleep(250);
  }
  check("p2p: session ended with the player", stopped, stopped ? "estado idle" : "still active");
  await sleep(4000);
  const after = await resources();
  check(
    "p2p: no socket or engine thread left",
    after.udp <= before.udp && after.tcpListening <= before.tcpListening && after.threads <= before.threads + 2,
    `antes ${JSON.stringify(before)} · durante ${JSON.stringify(during)} · depois ${JSON.stringify(after)}`,
  );
  await back();
  return { p2p: { startupSeconds: startup, downloadedMb: Math.round(st2.downloadedBytes / 1048576), progressAfterSeek: Math.round(st2.totalProgress * 1000) / 10 } };
}

// ---------------- Installed build (phase `install-check`) ----------------
// Run from the installed folder: native libraries found next to the exe and loaded (libmpv, P2P
// engine created and destroyed without any torrent), Home rows from the addons, the addons list,
// every Settings section and one details page. No playback, no download.

async function installCheck(): Promise<Record<string, unknown>> {
  type Native = { exeDir: string; engineDll: string | null; engineNextToExe: boolean; engine: { Ok?: string; Err?: string }; libmpv: { Ok?: string; Err?: string } };
  const n = await invoke<Native>("uitest_native_libraries");
  check("install: P2P engine next to the exe", n.engineNextToExe, `${n.engineDll ?? "não encontrado"} (pasta do exe: ${n.exeDir})`);
  check("install: P2P engine loads", !!n.engine.Ok, n.engine.Ok ?? n.engine.Err ?? "?");
  check("install: libmpv loads", !!n.libmpv.Ok, n.libmpv.Ok ? `libmpv API ${n.libmpv.Ok}` : n.libmpv.Err ?? "?");
  const rows = await waitFor(() => { const r = $$(".row-section"); return r.length ? r : null; }, 40000);
  check("install: Home rows from the addons", !!rows, rows ? `${rows.length} fileiras` : "nenhuma fileira em 40 s");
  const movie = await openFromHome("movie", "install: details");
  if (movie) await back();
  await openAddons();
  const addons = await waitFor(() => { const a = addonCards(); return a.length ? a : null; }, 20000);
  check("install: addons list", !!addons, addons ? `${addons.length} addon(s)` : "lista vazia");
  const sections = $$<HTMLButtonElement>(".settings-nav [data-section]").map((b) => b.dataset.section!);
  const opened: string[] = [];
  for (const id of sections) {
    click($(`.settings-nav [data-section="${id}"]`)!);
    await sleep(300);
    if ($(".settings-body h1")) opened.push(id);
  }
  check("install: every Settings section opens", opened.length === sections.length && sections.includes("p2p"), opened.join(", "));
  const p2p = await api.p2pStatus();
  check("install: no P2P session at start", p2p.state === "idle" || p2p.state === "", p2p.state || "none");
  return { native: n };
}

// ---------------- README screenshots (phase `readme-shots`) ----------------
// Captures of the main screens for the public README. Before each capture, everything that names
// an addon or shows the user's own data is hidden: addon names (rows, details, channels),
// Continue watching, favorite marks, progress notes and the profile name. Nothing is played and the
// search used for the picture is not added to the search history.

async function readmeShots(): Promise<Record<string, unknown>> {
  const style = document.createElement("style");
  style.textContent = `
    .row-header .muted.small, .details-info > .muted.small, .details-info [data-testid="movie-progress"],
    .channel-text .muted, .preview-title .muted, .profile-chip .nav-label { visibility: hidden !important; }
    [data-testid="home-continue"], .fav-btn.on, .settings-profile, .update-banner { display: none !important; }
  `;
  document.head.appendChild(style);
  const setItem = Storage.prototype.setItem;
  Storage.prototype.setItem = function (key: string, value: string) {
    if (key === "nuvio.search.history") return;
    return setItem.call(this, key, value);
  };
  const shots: Record<string, unknown> = {};
  const shot = async (name: string) => {
    await sleep(1800); // images settle
    const s = await capture(`readme-${name}`);
    shots[name] = !!s;
    check(`readme: ${name}`, !!s, s ? "captured" : "capture failed");
  };
  try {
    // Home
    await goTab("home");
    await waitFor(() => $$(".home .row-section .card[data-id]").length > 8, 40000);
    window.scrollTo(0, 0);
    await shot("home");
    // Details (no sources opened)
    const cards = await homeCards("movie");
    if (cards[0]) {
      click(cards[0]);
      await expectDetails("readme details", cards[0].dataset.id ?? null, "movie");
      await shot("details");
      await back();
    }
    // Search
    await goTab("search");
    const input = await waitFor(() => $<HTMLInputElement>('[data-testid="search-input"]'), 5000);
    if (input) {
      typeInto(input, "batman");
      await waitFor(() => $$(".search .row-section").length >= 2, 40000);
      await shot("search");
    }
    // Live TV (list and guide area; nothing tuned)
    await goTab("tv");
    await waitFor(() => $$(".livetv .channel").length > 5, 40000);
    const cat = $$<HTMLButtonElement>(".livetv-categories .cat-item")[3];
    if (cat) click(cat);
    await shot("livetv");
    // Settings → Reprodução
    await openSettings("playback");
    await shot("settings");
  } finally {
    Storage.prototype.setItem = setItem;
    style.remove();
  }
  return { shots };
}

// ---------------- Hero carousel + collections (phase `collections`) ----------------
// Hero: several titles, arrows change it, and it changes by itself after 8 s. Collections: a test
// collection (in the test data copy) made of the first Home catalogs, with an animated GIF cover,
// a still cover, an emoji and a wide card; its folder opens with tabs and a grid, a title opens
// from it and "back" returns to the folder, then to Home.
const TEST_GIF = "data:image/gif;base64,R0lGODlhPABaAIEAAC/n8g0OEgAAAAAAACH/C05FVFNDQVBFMi4wAwEAAAAh+QQAGQAAACwAAAAAPABaAAAIxAADCBxIsKDBgwgTKlzIsKHDhxAjSpxIsaLFixgzatzIsaPHjyBDihxJsqTJkyhTqlzJsqXLlzBjypxJs6bNmzhz6txJE4DPnwBkAh0KtCXRo0VTIl3qEyXTpyafSiUptarIqlhDYrX6cWtWj165dgw7dSzZshvPotWoFirHtkzNwj0qd+7QunZ/gs2rdy/foH75gvyrVXDhuVThRlXrlKzKsEbXuqTLs7Lly5gza97MubPnz6BDix5NurTp06hTq15tMyAAIfkEARkAAgAsCgAeACQAHwCB8lqMDQ4SAAAAAAAACJIABQgcSFBggIMHAShcCKCgw4cDESZkSHEhRIgSA1TcyPAiwYwcQyr0KACkSJEXTZ5E6VDlSpYRJb6c+VHmzJUxEd6kWdLmzpMGff4M2VPn0JUuj3JMqrQi06YMn0JVKHVqVahFJ07tmFXjVq5Xj+bUurWm0a8Fw/JMK1QsxrY7SardSDLo2bV1u77M+/auRZIBAQAh+QQBGQACACwPAB4AJAAfAIH6yDwNDhIAAAAAAAAIkgAFCBxIUGCAgwcBKFwIoKDDhwMRJmRIcSFEiBIDVNzI8CLBjBxDKvQoAKRIkRdNnkTpUOVKlhElvpz5UebMlTER3qRZ0ubOkwZ9/gzZU+fQlS6PckyqtCLTpgyfQlUodWpVqEUnTu2YVeNWrleP5tS6tabRrwXD8kwrVCzGtjtJqt1IMujZtXW7vsz79q5FkgEBACH5BAEZAAIALBQAHgAkAB8AgXhu+g0OEgAAAAAAAAiSAAUIHEhQYICDBwEoXAigoMOHAxEmZEhxIUSIEgNU3MjwIsGMHEMq9CgApEiRF02eROlQ5UqWESW+nPlR5syVMRHepFnS5s6TBn3+DNlT59CVLo9yTKq0ItOmDJ9CVSh1alWoRSdO7ZhV41auV4/m1Lq1ptGvBcPyTCtULMa2O0mq3Ugy6Nm1dbu+zPv2rkWSAQEAOw==";

async function collectionsPhase(): Promise<Record<string, unknown>> {
  // The hero may be off in the test data copy: turned on there (the copy only).
  const initial = await api.settingsGet();
  if (!initial.settings.home.heroEnabled) await api.settingsSet({ ...initial.settings, home: { ...initial.settings.home, heroEnabled: true } });
  check("hero: setting", true, initial.settings.home.heroEnabled ? "já ligado" : "estava desligado na cópia; ligado para o teste");
  await goTab("home");
  const hero = await waitFor(() => $<HTMLElement>(".home .hero-carousel"), 20000);
  let heroInfo = "";
  if (!hero) {
    const v = await api.homeRows();
    const src = v.heroSources;
    const pages = await Promise.all(src.map((key) => api.catalogPage(key, null).then((p) => p.items, () => [])));
    heroInfo = `enabled=${v.heroEnabled} placeholder=${!!$(".hero-placeholder")} sources=${src.length} items=${pages.flat().length} withArt=${pages.flat().filter((i) => i.landscapePoster || i.banner).length}`;
  }
  check("hero: carousel", !!hero, hero ? `${$$(".hero-dot").length} destaques` : `not shown (${heroInfo})`);
  let heroAuto = false;
  const coverRects: Record<string, number[]> = {};
  if (hero && $$(".hero-dot").length > 1) {
    window.scrollTo(0, 0);
    const start = hero.dataset.index;
    click($<HTMLButtonElement>(".hero-arrow.next")!);
    await sleep(400);
    const afterNext = hero.dataset.index;
    check("hero: next arrow", afterNext !== start, `${start} → ${afterNext}`);
    click($<HTMLButtonElement>(".hero-arrow.prev")!);
    await sleep(400);
    check("hero: previous arrow", hero.dataset.index === start, `${afterNext} → ${hero.dataset.index}`);
    await sleep(1500);
    await capture("collections-hero");
    const before = hero.dataset.index;
    heroAuto = !!(await waitFor(() => hero.dataset.index !== before, 9500));
    check("hero: changes by itself (8 s)", heroAuto, `${before} → ${hero.dataset.index}`);
  }

  const view = await api.homeRows();
  const catalogs = view.rows.filter((r) => !r.collection).slice(0, 3);
  const firstItems = catalogs[0] ? (await api.catalogPage(catalogs[0].key, null)).items : [];
  const source = (key: string) => {
    const [addonId, type, ...rest] = key.split(":");
    return { provider: "addon", addonId, type, catalogId: rest.join(":") };
  };
  const collection = {
    id: "uitest", title: "Coleção de teste", viewMode: "TABBED_GRID", showAllTab: true,
    folders: [
      { id: "gif", title: "Capa GIF", coverImageUrl: firstItems[0]?.poster ?? null, focusGifUrl: TEST_GIF, sources: catalogs.map((c) => source(c.key)) },
      { id: "gifonly", title: "Só GIF", coverImageUrl: TEST_GIF, sources: catalogs.slice(0, 1).map((c) => source(c.key)) },
      { id: "still", title: "Capa fixa", coverImageUrl: firstItems[1]?.poster ?? null, sources: catalogs.slice(0, 1).map((c) => source(c.key)) },
      { id: "emoji", title: "Emoji", coverEmoji: "🎬", sources: [{ provider: "tmdb", tmdbSourceType: "COLLECTION", tmdbId: 1 }] },
      { id: "wide", title: "Larga", tileShape: "landscape", coverImageUrl: firstItems[2]?.landscapePoster ?? firstItems[2]?.poster ?? null, sources: catalogs.slice(1, 2).map((c) => source(c.key)) },
    ],
  };
  await invoke("uitest_seed_collections", { collections: [collection] });
  // As if the account had put the collection between the catalogs (test data copy).
  const stored = (await api.settingsGet()).settings;
  const keys = view.rows.filter((r) => !r.collection).map((r) => r.key);
  keys.splice(1, 0, "collection_uitest");
  await api.settingsSet({ ...stored, home: { ...stored.home, items: keys.map((key, order) => ({ key, order, enabled: true, customTitle: "", heroSourceEnabled: true })) } });
  notifyRemoteChange(0, ["collections", "home"]);
  const row = await waitFor(() => $<HTMLElement>('.home .collection-row[data-collection="uitest"]'), 15000);
  check("collections: row on Home", !!row, row ? `${$$(".folder-card", row).length} pastas` : "not shown");
  if (!row) return { heroAuto };
  row.scrollIntoView({ block: "center" });
  await sleep(1500);
  const sections = $$(".home .row-section").filter((s) => s.dataset.collection || s.dataset.contentType);
  const at = sections.findIndex((s) => s.dataset.collection === "uitest");
  const pinned = view.rows.filter((r) => r.collection?.pinToTop).length;
  check("collections: between the catalogs", at === pinned + 1, `linha ${at + 1} de ${sections.length}`);
  check("GIF: none animated without the pointer", !$(".collection-row .folder-anim"), `${$$(".collection-row .folder-anim").length} animando`);
  for (const id of ["gif", "gifonly"]) {
    const card = $<HTMLElement>(`[data-folder="${id}"]`, row)!;
    card.scrollIntoView({ block: "center", inline: "nearest" });
    await sleep(500);
    const canvas = $<HTMLCanvasElement>("canvas.folder-still", card);
    check(`GIF ${id}: still drawn`, !!canvas?.dataset.drawn, canvas ? "canvas" : "missing");
    const r = card.querySelector(".card-art")!.getBoundingClientRect();
    coverRects[id] = [r.x, r.y, r.width, r.height].map((v) => v * window.devicePixelRatio);
    // Two captures not a multiple of the GIF's 1 s loop apart.
    await capture(`cover-${id}-still-a`);
    await sleep(370);
    await capture(`cover-${id}-still-b`);
    card.dispatchEvent(new PointerEvent("pointerover", { bubbles: true, relatedTarget: document.body }));
    const anim = await waitFor(() => $<HTMLImageElement>(".folder-anim", card), 2000);
    check(`GIF ${id}: animates under the pointer`, !!anim && anim.src.startsWith("data:image/gif"), anim ? "img GIF" : "missing");
    await sleep(400);
    for (const [i, wait] of [0, 90, 170, 230, 310, 430].entries()) {
      await sleep(wait);
      await capture(`cover-${id}-hover-${i}`);
    }
    card.dispatchEvent(new PointerEvent("pointerout", { bubbles: true, relatedTarget: document.body }));
    await sleep(200);
    check(`GIF ${id}: still again after leaving`, !$(".folder-anim", card) && !$(".folder-still.covered", card), "");
    await capture(`cover-${id}-left-a`);
    await sleep(370);
    await capture(`cover-${id}-left-b`);
  }
  check("collections: emoji cover", ($('[data-folder="emoji"] .folder-fallback', row)?.textContent ?? "") === "🎬", "emoji");
  const wide = $<HTMLElement>('[data-folder="wide"]', row);
  check("collections: wide card", !!wide && wide.offsetWidth > ($<HTMLElement>('[data-folder="still"]', row)?.offsetWidth ?? 0), `${wide?.offsetWidth}px`);
  await capture("collections-row");

  click($<HTMLButtonElement>('[data-folder="gif"]', row)!);
  const screen = await waitFor(() => $(".folder-screen"), 5000);
  const tabs = $$<HTMLButtonElement>(".folder-tabs .chip");
  check("folder: opens with tabs", !!screen && tabs.length === catalogs.length + 1, tabs.map((t) => t.textContent).join(" | "));
  const cards = await waitFor(() => ($$(".folder-grid .card[data-id]").length > 4 ? $$<HTMLButtonElement>(".folder-grid .card[data-id]") : null), 30000);
  check("folder: Todos grid", !!cards, `${cards?.length ?? 0} títulos`);
  await capture("collections-folder");
  if (tabs[1]) {
    click(tabs[1]);
    await sleep(300);
    const one = await waitFor(() => ($$(".folder-grid .card[data-id]").length > 0 ? $$<HTMLButtonElement>(".folder-grid .card[data-id]") : null), 30000);
    check("folder: catalog tab", !!one && tabs[1].classList.contains("active"), `${one?.length ?? 0} títulos`);
    if (one) {
      click(one[0]);
      const opened = await waitFor(() => $(".details"), 15000);
      check("folder: title opens", !!opened, one[0].dataset.id ?? "");
      await back();
      check("folder: back returns to the folder", visible($(".folder-screen")), "");
    }
  }
  click($$<HTMLButtonElement>(".folder-screen .back").find(visible)!);
  await sleep(400);
  check("folder: back returns to Home", !$(".folder-screen") && visible($(".home")), "");
  click($<HTMLButtonElement>('[data-folder="emoji"]')!);
  await waitFor(() => $(".folder-screen"), 5000);
  const message = $(".folder-screen .screen-message")?.textContent ?? "";
  check("folder: TMDB-only folder explains", /TMDB/.test(message), message);
  click($$<HTMLButtonElement>(".folder-screen .back").find(visible)!);
  await sleep(300);
  await openSettings("home");
  const pref = await waitFor(() => $('.catalog-pref[data-key="collection_uitest"]'), 5000);
  check("settings: collection listed in Início", !!pref, pref?.textContent ?? "missing");
  return { heroAuto, coverRects };
}

// ---------------- Video improvements (phase `enhance:<still image>`) ----------------
// Synthetic video with sound (volume 0) in the real window: playback rate and A/V offset with each
// motion and level, saving per profile (test data copy), and the watchdog with the broken
// swapchain put back on purpose. The `signal-*` captures ask the outer script to act on the window.
const SYNTH = (w: number, h: number, rate = "24000/1001") =>
  `av://lavfi:testsrc2=size=${w}x${h}:rate=${rate},format=yuv420p[out0];sine=frequency=300:sample_rate=48000[out1]`;
const prop = (name: string) => invoke<string | null>("uitest_player_property", { name });

async function playTest(url: string, motion = "off", enhancement = "off") {
  await invoke("player_video_config", { config: { motion, enhancement } });
  const start = () => invoke("player_open", { request: { url, headers: [], startSeconds: 0 } });
  window.dispatchEvent(new CustomEvent("uitest-play", { detail: start }));
  await waitFor(() => $(".full-player .controls"), 5000);
  await sleep(1500);
  await invoke("player_command", { command: { type: "setVolume", volume: 0 } });
}

async function syncOk(label: string, settleMs = 4000) {
  await sleep(settleMs);
  const read = async () => [Number(await prop("time-pos")), performance.now(), Number(await prop("frame-drop-count"))];
  const [p0, t0, d0] = await read();
  await sleep(4000);
  const [p1, t1, d1] = await read();
  const rate = (p1 - p0) / ((t1 - t0) / 1000);
  const offset = Number(await prop("avsync"));
  check(`${label}: 1x, audio in sync`, Math.abs(rate - 1) < 0.05 && Math.abs(offset) < 0.1,
    `rate=${rate.toFixed(3)} avsync=${offset.toFixed(4)} drops=${d1 - d0} sync=${await prop("video-sync")}`);
  return d1 - d0;
}

async function pick(selector: string) {
  if (!$(".pc-enhance-menu")) click($<HTMLButtonElement>(".pc-enhance > button")!);
  const tab = selector.includes("data-motion") ? "motion" : "quality";
  (await waitFor(() => $<HTMLButtonElement>(`.pc-enhance-menu [data-tab-id="${tab}"]`), 3000))?.click();
  const button = await waitFor(() => $<HTMLButtonElement>(`.pc-enhance-menu ${selector}`), 3000);
  button?.click();
}

const savedPlayer = async () => (await api.settingsGet()).settings.player;

function oneRow(label: string) {
  const row = $(".full-player .controls .row")!;
  const items = Array.from(row.children).filter((e) => (e as HTMLElement).offsetWidth > 0).map((e) => e.getBoundingClientRect());
  const middles = items.map((r) => (r.top + r.bottom) / 2);
  const inside = Math.max(...items.map((r) => r.right)) <= window.innerWidth;
  check(`controls ${label}: one row, inside the window`, inside && Math.max(...middles) - Math.min(...middles) < 8,
    `janela ${window.innerWidth}px, play ${$(".full-player .pc-play")!.getBoundingClientRect().height}px`);
}

async function enhancePhase(image: string): Promise<Record<string, unknown>> {
  await sleep(2500);
  await playTest(SYNTH(1920, 1080));
  await syncOk("original");
  oneRow("normal");

  for (const motion of ["fluid", "steady"]) {
    await pick(`[data-motion="${motion}"]`);
    await syncOk(`menu → ${motion}`);
    check(`${motion}: saved in the profile`, (await savedPlayer()).motionSmoothing === motion, (await savedPlayer()).motionSmoothing);
  }
  await pick('[data-motion="fluid"]');
  await api.setFullscreen(true);
  await syncOk("fluid, fullscreen");
  const shot = await capture("enhance-fullscreen");
  check("fluid: picture visible under the WebView", !!shot && shot.lumaStddev > 5, JSON.stringify(shot));

  for (const level of ["movies", "animeFast", "animeQuality"]) {
    await pick(`[data-level="${level}"]`);
    const drops = await syncOk(`fluid + ${level}`);
    check(`${level}: no overload`, drops <= 4, `${drops} drops`);
    const shaders = (await prop("glsl-shaders")) ?? "";
    const applied = level === "movies" ? (await prop("deband-iterations")) === "4" && !shaders
      : shaders.includes("Anime4K") && ((await prop("vo-passes")) ?? "").includes("Anime4K");
    check(`${level}: really rendering`, applied, shaders ? `${shaders.split("Anime4K_").length - 1} shaders` : "propriedades");
    check(`${level}: saved in the profile`, (await savedPlayer()).videoEnhancement === level, (await savedPlayer()).videoEnhancement);
  }
  $(".full-player")?.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
  await sleep(500);
  await capture("enhance-menu");

  await invoke("player_command", { command: { type: "seek", seconds: 5, relative: true } });
  await syncOk("after seek");
  await invoke("player_command", { command: { type: "togglePause" } });
  await sleep(2500);
  await invoke("player_command", { command: { type: "togglePause" } });
  await syncOk("after pause");

  await api.setFullscreen(false);
  await sleep(800);
  await capture("signal-minimize");
  await sleep(9000);
  check("minimized: motion suspended", (await prop("video-sync")) === "audio", String(await prop("video-sync")));
  await sleep(9000);
  await syncOk("restored", 2500);
  check("restored: motion resumed", (await prop("video-sync")) === "display-resample", String(await prop("video-sync")));

  for (const width of [1000, 780]) {
    await capture(`signal-width-${width}`);
    await sleep(2000);
    $(".full-player")?.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
    await sleep(500);
    oneRow(`${width}`);
    await capture(`enhance-width-${width}`);
  }
  await capture("signal-width-1280");
  await sleep(1500);

  await pick('[data-motion="off"]');
  await syncOk("menu → original");

  await playTest(SYNTH(1280, 720), "fluid", "animeQuality");
  await syncOk("on from the start, 720p");

  // The reported bug, on purpose: the flip swapchain under display sync.
  await api.settingsGet();
  await invoke("uitest_break_display_sync");
  const notice = await waitFor(() => $(".enhance-notice"), 14000);
  check("watchdog: broken motion detected and told", !!notice, notice?.textContent ?? "sem aviso");
  await syncOk("watchdog: after going back to original");
  check("watchdog: saved", (await savedPlayer()).motionSmoothing === "off", (await savedPlayer()).motionSmoothing);

  // Anime + RTX Super Resolution: Anime4K takes its place, and it comes back with another level.
  await invoke("player_video_config", { config: { enhancement: "animeQuality", motion: "fluid", nvidiaRtxSuperResolution: true } });
  const start = () => invoke("player_open", { request: { url: SYNTH(1920, 1080), headers: [], startSeconds: 0 } });
  window.dispatchEvent(new CustomEvent("uitest-play", { detail: start }));
  await sleep(1500);
  await invoke("player_command", { command: { type: "setVolume", volume: 0 } });
  const rtxDrops = await syncOk("anime + RTX VSR", 5000);
  check("anime + RTX VSR: no overload", rtxDrops <= 4 && !((await prop("vf")) ?? "").includes("nvidia"), `${rtxDrops} drops`);
  await pick('[data-level="movies"]');
  await sleep(1500);
  check("movies: RTX VSR back", ((await prop("vf")) ?? "").includes("nvidia"), String(await prop("vf")));

  await playTest(SYNTH(3840, 2160), "off", "animeQuality");
  await sleep(2000);
  check("4K: no Anime4K", !((await prop("glsl-shaders")) ?? "").includes("Anime4K"), String(await prop("glsl-shaders")));

  // An impossible load: the level must go down by itself, be told and be saved.
  await playTest(SYNTH(2048, 1152, "300"), "off", "animeQuality");
  const lowered = await waitFor(() => $(".enhance-notice"), 25000);
  check("watchdog: overload lowers the level", !!lowered, lowered?.textContent ?? "sem aviso");
  check("watchdog: lower level saved", (await savedPlayer()).videoEnhancement === "animeFast", (await savedPlayer()).videoEnhancement);

  // Visible change on the drawn test frame, through the app.
  await playTest(image);
  await sleep(1500);
  for (const level of ["off", "animeFast", "animeQuality"]) {
    await pick(`[data-level="${level}"]`);
    await sleep(3000);
    const menu = $<HTMLElement>(".full-player .controls")!;
    menu.style.visibility = "hidden";
    await sleep(400);
    await capture(`enhance-art-${level}`);
    menu.style.visibility = "";
  }
  return {};
}

// ---------------- Trailers (phase `trailer`) ----------------
// A movie and a series from Home whose details have a YouTube trailer: nothing is requested before
// the click, the trailer plays (picture + audio track) in the app's player, closing returns to the
// same details page, and watch progress / history counts stay the same.

async function trailerFor(type: "movie" | "series", label: string): Promise<Record<string, unknown> | null> {
  await goTab("home");
  const cards = await homeCards(type);
  for (const card of cards.slice(0, 6)) {
    click(card);
    const opened = await expectDetails(`${label} (${card.dataset.id})`, card.dataset.id ?? null, type);
    if (!opened) { await back(); continue; }
    const button = $<HTMLButtonElement>('.details [data-testid="watch-trailer"]');
    if (!button) { await back(); continue; }
    const counts = await invoke<Record<string, number[]>>("uitest_store_counts");
    const full0 = !!$(".full-player");
    check(`${label}: trailer button`, !full0, `“${opened.name}”: ${button.textContent?.trim()} (nada carregado antes do clique)`);
    const t0 = performance.now();
    click(button);
    const full = await waitFor(() => $(".full-player") ?? $(".details .row-error"), 40000);
    const started = Math.round((performance.now() - t0) / 100) / 10;
    if (!full || !full.classList.contains("full-player")) {
      check(`${label}: trailer plays`, false, full?.textContent ?? "timeout 40 s");
      await back();
      return null;
    }
    const subtitle = $(".full-player-top .muted")?.textContent ?? "";
    const ok = await waitPicture(`${label}-trailer`, 2, 40000);
    const snap = await api.playerSnapshot().catch(() => null);
    const audio = snap?.tracks.filter((t) => t.kind === "audio").length ?? 0;
    check(`${label}: trailer audio`, ok && audio > 0, `${audio} faixa(s) de áudio, vídeo ${snap?.width}×${snap?.height} ${snap?.videoCodec}, aberto em ${started} s; legenda da tela: ${subtitle}`);
    const close = $<HTMLButtonElement>('.full-player .full-player-top .back');
    if (close) close.click();
    const back2 = await waitFor(() => (!$(".full-player") && $$(".details").find((d) => visible(d) && d.dataset.id === opened.id)) || null, 10000);
    check(`${label}: back to the details`, !!back2, back2 ? `detalhes de “${opened.name}” mantidos` : "details not shown");
    await sleep(1500);
    const after = await invoke<Record<string, number[]>>("uitest_store_counts");
    check(`${label}: no history/progress change`, JSON.stringify(after) === JSON.stringify(counts), `antes ${JSON.stringify(counts)} · depois ${JSON.stringify(after)}`);
    await back();
    return { id: opened.id, startedSeconds: started };
  }
  check(`${label}: trailer button`, false, "nenhum dos 6 primeiros títulos tem trailer nos metadados do addon");
  return null;
}

async function trailerPhase(): Promise<Record<string, unknown>> {
  const movie = await trailerFor("movie", "trailer movie");
  const series = await trailerFor("series", "trailer series");
  return { trailers: { movie, series } };
}

export async function runUiTest() {
  const started = performance.now();
  const phase = await invoke<string>("uitest_phase").catch(() => "");
  let extra: Record<string, unknown> = {};
  if (phase) {
    try {
      if (phase === "settings-write") extra = { original: await settingsWrite() };
      else if (phase.startsWith("settings-verify:")) await settingsVerify(JSON.parse(decodeURIComponent(phase.slice("settings-verify:".length))));
      else if (phase === "library-write") extra = await libraryWrite();
      else if (phase === "manage-write") extra = await manageWrite();
      else if (phase === "account-ui") extra = await accountUi();
      else if (phase === "p2p") extra = await p2pPhase();
      else if (phase === "trailer") extra = await trailerPhase();
      else if (phase === "install-check") extra = await installCheck();
      else if (phase === "readme-shots") extra = await readmeShots();
      else if (phase === "collections") extra = await collectionsPhase();
      else if (phase.startsWith("enhance:")) extra = await enhancePhase(decodeURIComponent(phase.slice("enhance:".length)));
      else if (phase.startsWith("account-verify:")) await accountVerify(JSON.parse(decodeURIComponent(phase.slice("account-verify:".length))));
      else if (phase.startsWith("manage-verify:")) await manageVerify(JSON.parse(decodeURIComponent(phase.slice("manage-verify:".length))));
      else if (phase.startsWith("library-verify:")) await libraryVerify(JSON.parse(decodeURIComponent(phase.slice("library-verify:".length))));
    } catch (e) {
      check("driver", false, e instanceof Error ? `${e.message} ${e.stack?.split(String.fromCharCode(10)).slice(1, 3).join(" ")}` : String(e));
    }
    await invoke("uitest_report", { report: { phase, passed: checks.filter((c) => c.passed).length, failed: checks.filter((c) => !c.passed).length, durationMs: Math.round(performance.now() - started), checks, ...extra } });
    return;
  }
  try {
    // 1. Home → movie → sources → play.
    const movie = await openFromHome("movie", "movie (Home)");
    if (movie) {
      const watch = $<HTMLButtonElement>('.details [data-testid="watch"]');
      check("movie (Home): Assistir button", !!watch, watch ? "present" : "missing");
      if (watch) {
        click(watch);
        await playFirstSource("movie (Home)");
      }
      await back();
    }

    // 2. Home → series → seasons → episode → sources → play.
    const series = await openFromHome("series", "series (Home)");
    if (series) {
      const tabs = $$<HTMLButtonElement>(".details .season-tabs .chip");
      const episodes = $$<HTMLButtonElement>(".details .episode");
      check("series (Home): seasons and episodes", tabs.length > 0 && episodes.length > 0, `${tabs.length} temporada(s), ${episodes.length} episódio(s) na temporada inicial`);
      if (tabs.length > 1) {
        click(tabs[1]);
        await sleep(300);
        check("series (Home): season switch", $$(".details .episode").length > 0, `${tabs[1].textContent}: ${$$(".details .episode").length} episódio(s)`);
      }
      // Up to 3 series of the row (latest season, first episode): an old episode may simply have
      // no source in the addons; at most 3 source requests.
      for (let attempt = 0; attempt < 3; attempt++) {
        if (attempt > 0) {
          await back();
          const row = $$(".row-section").find((r) => r.dataset.contentType === "series");
          const card = row ? $$<HTMLButtonElement>(".card[data-id]", row)[attempt] : null;
          if (!card) break;
          click(card);
          if (!(await expectDetails(`series #${attempt + 1} (Home)`, card.dataset.id ?? null, "series"))) continue;
        }
        const seasonTabs = $$<HTMLButtonElement>(".details .season-tabs .chip").filter((t) => !/Especiais/.test(t.textContent ?? ""));
        if (seasonTabs.length) {
          click(seasonTabs[seasonTabs.length - 1]);
          await sleep(300);
        }
        const first = $<HTMLButtonElement>(".details .episode");
        if (!first) continue;
        const label = `episode (Home, ${$(".details .episode-title")?.textContent?.trim()})`;
        click(first);
        const before = checks.length;
        await playFirstSource(label);
        const result = checks.slice(before).find((c) => c.name.endsWith(": playback"));
        if (!result || result.passed || !/no direct source/.test(result.detail)) break;
        // Kept in the report as information: the addon had no source for that episode.
        result.passed = true;
        result.name = `${label}: no playable source in the addon answer, trying the next series`;
      }
      if (!checks.some((c) => c.name.startsWith("episode") && c.name.endsWith(": playback"))) {
        check("episode (Home): playback", false, "no playable episode source in 3 series");
      }
      await back();
    }

    // 3. Search → the same titles by name.
    if (movie) {
      await openFromSearch(movie, "movie (Search)");
      await capture("details-movie-search");
      await back();
    }
    if (series) {
      await openFromSearch(series, "series (Search)");
      await capture("details-series-search");
      await back();
    }

    // 4. Navigation regression: Home and TV still render (TV: list only, no channel is tuned).
    click($('[data-tab="home"]')!);
    check("navigation: Home", !!(await waitFor(() => $$(".row-section").find(visible), 10000)), "rows visible");
    click($('[data-tab="tv"]')!);
    const tv = await waitFor(() => $(".channel") ?? $$(".livetv .screen-message").find((m) => !/Carregando/.test(m.textContent ?? "")), 60000);
    check("navigation: TV", !!tv, tv ? `${$$(".channel").length} canais, ${$$(".livetv .livetv-categories .cat-item").length} filtros` : "timeout");

    // 5. Settings: every category opens with its controls (read only in this phase).
    await settingsTour();
    await capture("settings-page");
  } catch (e) {
    check("driver", false, String(e));
  }
  const report = {
    passed: checks.filter((c) => c.passed).length,
    failed: checks.filter((c) => !c.passed).length,
    durationMs: Math.round(performance.now() - started),
    checks,
  };
  await invoke("uitest_report", { report });
}

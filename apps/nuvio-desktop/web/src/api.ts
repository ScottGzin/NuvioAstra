import { invoke } from "./tauri";
import { message } from "./rustErrors";
import type {
  EpgView,
  SubtitleFont,
  SubtitleFontList,
  SubtitleList,
  Moments,
  TorboxStatus,
  DebridStatus,
  ParentalWarning,
  TmdbEntityPage,
  TmdbStatus,
  SimklStatus,
  M3uPlaylist,
  PluginsView,
  SportsMatch,
  MatchChannel,
  DiscordPresence,
  DeviceCode,
  DevicePoll,
  MdbListStatus,
  ExternalRating,
  EditCollection,
  CollectionCatalogOption,
  DebridCheck,
  DownloadRequest,
  DownloadItem,
  DownloadsFolder,
  UpdateInfo,
  UpdateStatus,
  P2pStatus,
  TorrentPrepared,
  AddonPreview, AddonsView, Bootstrap, CatalogPage, ChannelList, CssRect, Details, HistoryEntry, HistoryPage, HomeView, LibraryItem,
  AccountStatus, AvatarItem, PinResult, SyncPreview, PlaybackItem, PlayerCommand, ProfilesView, PlayerInfo, PlayerSnapshot, ProfileSettings, SearchSection, SettingsView, StreamGroup, SearchAddon, DiscoverCatalog,
  TitleProgress,
} from "./types";

// Session caches: the same catalog page or title is asked to the addons once, so scrolling back,
// returning to Home or reopening a title never repeats the request. Failures are not cached.
const pageCache = new Map<string, Promise<CatalogPage>>();
const detailsCache = new Map<string, Promise<Details>>();
const ratingsCache = new Map<string, Promise<ExternalRating[]>>();
const homeListeners = new Set<() => void>();

const remoteSettingsListeners = new Set<() => void>();
/** The account changed settings, the Home layout or the collections (not a change made in this app). */
export function onRemoteSettingsChanged(listener: () => void): () => void {
  remoteSettingsListeners.add(listener);
  return () => remoteSettingsListeners.delete(listener);
}

// Data epoch: changes when the active profile or its addons change. Every cache tied to them is
// dropped, and an answer started before the change is ignored by whoever compares the epoch.
let epoch = 0;
const epochListeners = new Set<(reason: "profile" | "addons") => void>();
export const dataEpoch = () => epoch;
function newEpoch(reason: "profile" | "addons") {
  epoch++;
  pageCache.clear();
  detailsCache.clear();
  epochListeners.forEach((l) => l(reason));
}
function tmdbChanged() {
  detailsCache.clear();
  pageCache.clear();
  homeListeners.forEach((l) => l());
}
export function onDataEpoch(listener: (reason: "profile" | "addons") => void): () => void {
  epochListeners.add(listener);
  return () => epochListeners.delete(listener);
}

// What a sync brought from the account: addon changes reload the addon-based screens (data epoch);
// profile list changes (names, avatars, PIN) only refresh the profile list. Library, progress and
// watched changes arrive as `library-changed` (Library, Details, Continue watching re-read).
const profileListeners = new Set<() => void>();
export function onProfilesChanged(listener: () => void): () => void {
  profileListeners.add(listener);
  return () => profileListeners.delete(listener);
}
/** Reloads every screen of the active profile (after unlocking it with its PIN). */
export function reloadProfileData() {
  newEpoch("profile");
}
export function notifyRemoteChange(_profile: number, changed: string[]) {
  if (changed.includes("addons")) newEpoch("addons");
  if (changed.includes("profiles")) profileListeners.forEach((l) => l());
  if (changed.includes("collections") || changed.includes("home")) {
    pageCache.clear(); // "ocultar não lançados" may have changed
    homeListeners.forEach((l) => l());
  }
  if (["collections", "home", "settings"].some((k) => changed.includes(k))) remoteSettingsListeners.forEach((l) => l());
}

function cached<T>(cache: Map<string, Promise<T>>, key: string, load: () => Promise<T>): Promise<T> {
  let value = cache.get(key);
  if (!value) {
    value = load();
    value.catch(() => cache.delete(key));
    cache.set(key, value);
  }
  return value;
}

export const api = {
  bootstrap: () => invoke<Bootstrap>("app_bootstrap"),
  homeRows: () => invoke<HomeView>("home_rows"),
  /** `genre`: a collection folder's catalog filtered by genre. */
  catalogPage: (key: string, skip: number | null, genre: string | null = null) =>
    cached(pageCache, `${key}#${genre ?? ""}#${skip ?? 0}`, () => invoke<CatalogPage>("catalog_page", { key, skip, genre })),
  search: (query: string) => invoke<SearchSection[]>("search_all", { query }),
  details: (contentType: string, id: string) =>
    cached(detailsCache, `${contentType}/${id}`, () => invoke<Details>("meta_details", { contentType, id })),
  /** The details completed from TMDB (fails when TMDB is off, has no key or lacks the title). */
  detailsEnriched: (contentType: string, id: string) =>
    cached(detailsCache, `tmdb:${contentType}/${id}`, () => invoke<Details>("meta_enrich", { contentType, id })),
  uiLanguageSet: (language: string) => invoke<void>("ui_language_set", { language }).catch(() => {}),
  tmdbStatus: () => invoke<TmdbStatus>("tmdb_status"),
  async tmdbSaveKey(key: string): Promise<TmdbStatus> {
    const status = await invoke<TmdbStatus>("tmdb_save_key", { key });
    tmdbChanged();
    return status;
  },
  async tmdbRemoveKey(): Promise<TmdbStatus> {
    const status = await invoke<TmdbStatus>("tmdb_remove_key");
    tmdbChanged();
    return status;
  },
  /** TMDB settings changed: details are completed again and Home re-reads its collections. */
  tmdbChanged: () => tmdbChanged(),
  discordEnabled: () => invoke<boolean>("discord_enabled"),
  discordSetEnabled: (enabled: boolean) => invoke<void>("discord_set_enabled", { enabled }),
  discordPresence: (presence: DiscordPresence) => invoke<void>("discord_presence", { presence }).catch(() => {}),
  simklStatus: () => invoke<SimklStatus>("simkl_status"),
  simklDeviceStart: (openBrowser: boolean) => invoke<DeviceCode>("simkl_device_start", { openBrowser }),
  simklDevicePoll: () => invoke<DevicePoll>("simkl_device_poll"),
  simklDeviceCancel: () => invoke<void>("simkl_device_cancel").catch(() => {}),
  simklDisconnect: () => invoke<SimklStatus>("simkl_disconnect"),
  simklImport: () => invoke<number>("simkl_import"),
  mdblistStatus: () => invoke<MdbListStatus>("mdblist_status"),
  async mdblistSaveKey(key: string): Promise<MdbListStatus> {
    const status = await invoke<MdbListStatus>("mdblist_save_key", { key });
    ratingsCache.clear();
    return status;
  },
  async mdblistRemoveKey(): Promise<MdbListStatus> {
    const status = await invoke<MdbListStatus>("mdblist_remove_key");
    ratingsCache.clear();
    return status;
  },
  mdblistOpenKeyPage: () => invoke<void>("mdblist_open_key_page"),
  /** MDBList ratings of a title (fails when MDBList is off or has no key). */
  mdblistRatings: (contentType: string, id: string) =>
    cached(ratingsCache, `${contentType}/${id}`, () => invoke<ExternalRating[]>("mdblist_ratings", { contentType, id })),
  /** MDBList settings changed: ratings are asked again. */
  mdblistChanged: () => ratingsCache.clear(),
  tmdbEntity: (kind: string, id: number) => invoke<TmdbEntityPage>("tmdb_entity", { kind, id }),
  tmdbEntityTitles: (kind: string, id: number, media: string, page: number) => invoke<CatalogPage>("tmdb_entity_titles", { kind, id, media, page }),
  /** `refresh`: asks the addons again ("Recarregar" in the sources panel). */
  /** Sources of a title. With `request`, each addon's answer also arrives as `streams-part`. */
  streams: (contentType: string, id: string, refresh = false, request: number | null = null) =>
    invoke<StreamGroup[]>("title_streams", { contentType, id, refresh, request }),
  /** "Reutilizar último link": whether this movie/episode has a fresh link, and playing it. */
  lastLinkHas: (progress: PlaybackItem) => invoke<boolean>("last_link_has", { progress }),
  lastLinkPlay: (startSeconds: number | null, progress: PlaybackItem) => invoke<PlayerInfo>("last_link_play", { startSeconds, progress }),
  streamPlayYoutube: (key: string, startSeconds: number | null, progress: PlaybackItem | null) =>
    invoke<PlayerInfo>("stream_play_youtube", { key, startSeconds, progress }),
  streamOpenExternal: (key: string) => invoke<void>("stream_open_external", { key }),
  /** Content warnings of a title (IMDb parents guide), for the start of playback. */
  parentalGuide: (id: string) => invoke<ParentalWarning[]>("parental_guide", { id }),
  /** The trailer in the language of Settings → Áudio, searched on YouTube when clicked. */
  trailerPlayLocalized: (name: string, year: string | null) => invoke<PlayerInfo>("trailer_play_localized", { name, year }),
  searchAddons: () => invoke<SearchAddon[]>("search_addons"),
  searchAddonSet: (key: string, enabled: boolean) => invoke<SearchAddon[]>("search_addon_set", { key, enabled }),
  discoverCatalogs: () => invoke<DiscoverCatalog[]>("discover_catalogs"),
  settingsGet: () => invoke<SettingsView>("settings_get"),
  /** Saves and returns what was stored (validated by Rust). `previous`: what was loaded before the
   *  edit, so only the edit is written. `profile`: the one edited. Home changes drop the cached pages. */
  async settingsSet(settings: ProfileSettings, previous?: ProfileSettings | null, profile?: number): Promise<ProfileSettings> {
    const saved = await invoke<ProfileSettings>("settings_set", { settings, previous: previous ?? null, profile: profile ?? null });
    pageCache.clear();
    homeListeners.forEach((l) => l());
    return saved;
  },
  collectionsEdit: () => invoke<{ collections: EditCollection[]; catalogs: CollectionCatalogOption[] }>("collections_edit"),
  /** Saves the edited collections (and sends them to the account); Home reloads its rows. */
  async collectionsSave(list: EditCollection[]): Promise<void> {
    await invoke<void>("collections_save", { collectionsList: list });
    pageCache.clear();
    homeListeners.forEach((l) => l());
  },
  /** Home re-reads its rows when the profile's Home settings change. */
  onHomeSettingsChanged(listener: () => void): () => void {
    homeListeners.add(listener);
    return () => homeListeners.delete(listener);
  },
  subtitleFonts: () => invoke<SubtitleFontList>("subtitle_fonts"),
  /** Opens the Windows file dialog; `null` = cancelled. */
  subtitleFontImport: () => invoke<SubtitleFont | null>("subtitle_font_import"),
  subtitleFontRemove: (id: string) => invoke<void>("subtitle_font_remove", { id }),
  subtitleFontData: (id: string) => invoke<ArrayBuffer>("subtitle_font_data", { id }),
  appMode: () => invoke<"app" | "selftest" | "uitest">("app_mode"),
  liveTvChannels: () => invoke<ChannelList>("livetv_channels"),
  m3uList: () => invoke<M3uPlaylist[]>("m3u_list"),
  pluginsList: () => invoke<PluginsView>("plugins_list"),
  pluginsAdd: (url: string) => invoke<PluginsView>("plugins_add", { url }),
  pluginsRefresh: (key: string) => invoke<PluginsView>("plugins_refresh", { key }),
  pluginsSetEnabled: (enabled: boolean) => invoke<PluginsView>("plugins_set_enabled", { enabled }),
  pluginsSetScraperEnabled: (key: string, enabled: boolean) => invoke<PluginsView>("plugins_set_scraper_enabled", { key, enabled }),
  pluginsRemove: (key: string) => invoke<PluginsView>("plugins_remove", { key }),
  liveTvMatches: (tzOffsetMinutes: number) => invoke<{ matches: SportsMatch[]; error: string | null }>("livetv_matches", { tzOffsetMinutes }),
  liveTvMatchChannels: (matchId: string, channels: { key: string; name: string; genres: string[]; logo: string | null }[], tzOffsetMinutes: number) =>
    invoke<MatchChannel[]>("livetv_match_channels", { matchId, channels, tzOffsetMinutes }),
  m3uAdd: (name: string, source: string) => invoke<M3uPlaylist[]>("m3u_add", { name, source }),
  m3uRemove: (id: string) => invoke<M3uPlaylist[]>("m3u_remove", { id }),
  liveTvFavorites: () => invoke<string[]>("livetv_favorites"),
  liveTvToggleFavorite: (key: string) => invoke<string[]>("livetv_toggle_favorite", { key }),
  /** Asks only the channel's own addon, only when the user picks the channel. */
  liveTvResolve: (key: string) => invoke<StreamGroup[]>("livetv_resolve", { key }),

  // Addons of the active profile (Settings → Addons). Every change starts a new data epoch.
  addonsList: () => invoke<AddonsView>("addons_list"),
  addonPreview: (address: string) => invoke<AddonPreview>("addon_preview", { address }),
  addonInstall: (key: string, trackingConsent: boolean) => changed("addons", invoke<AddonsView>("addon_install", { key, trackingConsent })),
  addonSetEnabled: (key: string, enabled: boolean) => changed("addons", invoke<AddonsView>("addon_set_enabled", { key, enabled })),
  addonRemove: (key: string) => changed("addons", invoke<AddonsView>("addon_remove", { key })),
  addonMove: (key: string, delta: number) => changed("addons", invoke<AddonsView>("addon_move", { key, delta })),
  addonRefresh: (key: string) => changed("addons", invoke<AddonsView>("addon_refresh", { key })),
  addonConfigure: (key: string) => invoke<void>("addon_configure", { key }),

  // Profiles. A switch closes the player in Rust and starts a new data epoch.
  profilesList: () => invoke<ProfilesView>("profiles_list"),
  /** Asks again whether the PIN was removed in Nuvio. */
  profilePinRefresh: (index: number) => invoke<ProfilesView>("profile_pin_refresh", { index }),
  /** The UI listens to the sync events: Rust starts the first sync. */
  appReady: () => invoke<void>("app_ready"),
  welcomeNeeded: () => invoke<boolean>("welcome_needed"),
  welcomeDone: () => invoke<void>("welcome_done"),
  /** After signing in: receives the account's data and joins it with this computer's. */
  accountConnect: (confirmOther: boolean) => invoke<AccountStatus>("account_connect", { confirmOther }),
  profileSwitch: (index: number) => changed("profile", invoke<ProfilesView>("profile_switch", { index })),
  profileCreate: (name: string, color: string, usesPrimaryAddons: boolean) => invoke<ProfilesView>("profile_create", { name, color, usesPrimaryAddons }),
  /** `avatarId`: an id of the account's catalog, "" for color and initial, undefined to keep it. */
  profileUpdate: (index: number, name: string, color: string, usesPrimaryAddons: boolean, avatarId?: string) =>
    changed("profile", invoke<ProfilesView>("profile_update", { index, name, color, usesPrimaryAddons, avatarId: avatarId ?? null })),
  /** Validates a profile's PIN in Rust (account, or the offline hash) before opening it. */
  profileUnlock: (index: number, pin: string) => invoke<PinResult>("profile_unlock", { index, pin }),
  profilePinSet: (index: number, pin: string, currentPin: string | null) => invoke<ProfilesView>("profile_pin_set", { index, pin, currentPin }),
  profilePinClear: (index: number, currentPin: string | null) => invoke<ProfilesView>("profile_pin_clear", { index, currentPin }),

  // Nuvio account: the session stays in Rust (DPAPI); the UI only sees this status.
  accountStatus: () => invoke<AccountStatus>("account_status"),
  accountSignIn: (email: string, password: string) => invoke<AccountStatus>("account_sign_in", { email, password }),
  accountSignUp: (email: string, password: string) => invoke<{ signedIn: boolean; message: string }>("account_sign_up", { email, password }),
  accountSignOut: () => invoke<AccountStatus>("account_sign_out"),
  accountSyncNow: (confirmLink = false) => invoke<AccountStatus>("account_sync_now", { confirmLink }),
  /** Read-only comparison with the account (nothing is written). */
  accountSyncPreview: () => invoke<SyncPreview>("account_sync_preview"),
  avatarsCatalog: () => invoke<AvatarItem[]>("avatars_catalog"),
  profileDelete: (index: number) => changed("profile", invoke<ProfilesView>("profile_delete", { index })),

  // Library: NextGen's own data, no addon request.
  libraryFavoriteKeys: () => invoke<string[]>("library_favorite_keys"),
  libraryFavorites: () => invoke<LibraryItem[]>("library_favorites"),
  libraryToggleFavorite: (item: Omit<LibraryItem, "savedAtEpochMs">) => invoke<boolean>("library_toggle_favorite", { item }),
  libraryRemoveFavorite: (contentType: string, id: string) => invoke<boolean>("library_remove_favorite", { contentType, id }),
  libraryContinueWatching: () => invoke<HistoryEntry[]>("library_continue_watching"),
  /** The row completed with next episodes (asks the addons for the episode lists it lacks). */
  libraryNextUp: () => invoke<HistoryEntry[]>("library_next_up"),
  libraryHistory: (offset: number, limit: number) => invoke<HistoryPage>("library_history", { offset, limit }),
  libraryRemoveHistory: (key: string) => invoke<boolean>("library_remove_history", { key }),
  libraryTitleProgress: (id: string) => invoke<TitleProgress>("library_title_progress", { id }),
  /** Marks (or unmarks) a movie, episodes or a season as watched. */
  librarySetWatched: (
    meta: { contentType: string; id: string; name: string; poster: string | null; releaseInfo: string | null },
    targets: { season: number | null; episode: number | null; videoId: string | null }[],
    watched: boolean,
  ) => invoke<number>("library_set_watched", { ...meta, targets, watched }),

  /** `progress`: the movie/episode, so Rust saves its position (absent for live TV). */
  playStream: (key: string, startSeconds: number | null, progress: PlaybackItem | null = null, previousFinished = false) =>
    invoke<PlayerInfo>("stream_play", { key, startSeconds, progress, previousFinished }),
  playerSubtitles: (session: number) => invoke<SubtitleList>("player_subtitles", { session }),
  playerSubtitleLoad: (session: number, index: number) => invoke<void>("player_subtitle_load", { session, index }),
  /** Aligns the selected addon subtitle with the video's embedded subtitles (sets the delay). */
  playerSubtitleAutosync: (session: number) =>
    invoke<{ status: "applied" | "no-reference" | "no-match" | "not-addon" | "no-subtitle"; offsetMs: number | null }>("player_subtitle_autosync", { session }),
  playerSubtitleStyle: () => invoke<void>("player_subtitle_style"),
  playerMoments: (session: number) => invoke<Moments>("player_moments", { session }),
  playerCommand: (command: PlayerCommand) => invoke<void>("player_command", { command }).catch(() => {}),
  /** `rect` in CSS pixels; `null` = whole window. */
  playerRect: (rect: CssRect | null) =>
    invoke<void>("player_video_rect", { rect, scale: window.devicePixelRatio }).catch(() => {}),
  playerSnapshot: () => invoke<PlayerSnapshot>("player_snapshot"),
  playerClose: () => invoke<void>("player_close"),
  /** Starts the P2P engine for a torrent stream (rejects with `P2P_CONSENT_REQUIRED` while P2P is off). */
  torboxStatus: () => invoke<TorboxStatus>("torbox_status"),
  torboxSaveKey: (key: string) => invoke<TorboxStatus>("torbox_save_key", { key }),
  torboxRemoveKey: () => invoke<TorboxStatus>("torbox_remove_key"),
  /** Which torrent/TorBox sources of the last searches TorBox has cached. */
  /** What the connected debrid services can do with these sources. */
  torboxCheck: (keys: string[]) => invoke<DebridCheck>("torbox_check", { keys }),
  /** The link of a source from the first connected debrid service that has it. */
  torboxPrepare: (key: string, season: number | null, episode: number | null) =>
    invoke<{ filename: string | null; size: number | null; code: string | null }>("torbox_prepare", { key, season, episode }),
  debridStatus: () => invoke<DebridStatus>("debrid_status"),
  debridSaveKey: (service: string, key: string) => invoke<DebridStatus>("debrid_save_key", { service, key }),
  debridRemoveKey: (service: string) => invoke<DebridStatus>("debrid_remove_key", { service }),
  debridSetPreferred: (service: string) => invoke<DebridStatus>("debrid_set_preferred", { service }),
  downloadStart: (request: DownloadRequest) => invoke<DownloadItem>("download_start", { request }),
  downloadsList: () => invoke<DownloadItem[]>("downloads_list"),
  downloadPause: (id: string) => invoke<void>("download_pause", { id }),
  downloadResume: (id: string) => invoke<void>("download_resume", { id }),
  downloadRemove: (id: string, deleteFile: boolean) => invoke<void>("download_remove", { id, deleteFile }),
  downloadShow: (id: string) => invoke<void>("download_show", { id }),
  downloadPlay: (id: string, startSeconds: number | null) => invoke<PlayerInfo>("download_play", { id, startSeconds }),
  downloadsFolder: () => invoke<DownloadsFolder>("downloads_folder"),
  downloadsChooseFolder: () => invoke<DownloadsFolder>("downloads_choose_folder"),
  torrentPrepare: (key: string) => invoke<TorrentPrepared>("torrent_prepare", { key }),
  torrentStop: () => invoke<void>("torrent_stop"),
  /** Program guide of one channel (cached in Rust until the current program ends). */
  liveTvEpg: (key: string, tzOffsetMinutes: number) => invoke<EpgView>("livetv_epg", { key, tzOffsetMinutes }),
  /** Fetches again the manifests and channels of the channel addons (all, or the ones named). */
  liveTvReload: (addonNames: string[] | null) => invoke<ChannelList>("livetv_reload", { addonNames }),
  updateStatus: () => invoke<UpdateStatus>("update_status"),
  /** Asks the release endpoint for a newer version (nothing downloaded). */
  updateCheck: () => invoke<UpdateInfo | null>("update_check"),
  /** Downloads, verifies the signature and runs the installer (the app then closes). */
  updateInstall: () => invoke<void>("update_install"),
  /** Resolves the YouTube trailer (only now, on the user's click) and opens it in the player. */
  trailerPlay: (key: string) => invoke<PlayerInfo>("trailer_play", { key }),
  p2pStatus: () => invoke<P2pStatus>("p2p_status"),
  p2pClearCache: () => invoke<number>("p2p_clear_cache"),
  p2pCacheSize: () => invoke<number>("p2p_cache_size"),
  pictureInPicture: (enabled: boolean, title: string) => invoke<void>("player_picture_in_picture", { enabled, title }),
  setFullscreen: (fullscreen: boolean) => invoke<void>("window_set_fullscreen", { fullscreen }),
};

async function changed<T>(reason: "profile" | "addons", call: Promise<T>): Promise<T> {
  const value = await call;
  newEpoch(reason);
  return value;
}

export function errorText(error: unknown): string {
  const raw = typeof error === "string" ? error : error instanceof Error ? error.message : "erro desconhecido";
  // Rust commands answer in Portuguese; message() handles both the exact dictionary and the
  // messages Rust builds with values (HTTP codes, engine output).
  return message(raw);
}

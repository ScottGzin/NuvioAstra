import { t } from "./i18n";
// Contracts of the Rust commands (apps/nuvio-desktop/src/api.rs, main.rs). Display data only:
// stream URLs, addon URLs and tokens never reach the UI.

export interface ProfileInfo {
  index: number; name: string; avatarId: string | null; avatarColorHex: string; usesPrimaryAddons: boolean;
  /** Protected by a PIN set with the Nuvio account. */
  pinEnabled: boolean;
}
export interface ProfilesView { active: number; locked: boolean; profiles: ProfileInfo[]; max: number; colors: string[] }

// ---- Nuvio account (nuvio-sync). Tokens never reach the UI. ----
export type SyncStateName = "notConfigured" | "signedOut" | "expired" | "idle" | "syncing" | "pending" | "offline" | "error" | "needsReview";
export interface SyncReport {
  finishedAtEpochMs: number; profile: number; sent: number; received: number; failed: string[]; changed: string[];
}
export interface AccountStatus {
  configured: boolean; signedIn: boolean; email: string | null; state: SyncStateName; pending: number;
  lastSyncEpochMs: number; lastError: string | null; lastReport: SyncReport | null; linkQuestion: string | null;
}
export interface PinResult { unlocked: boolean; retryAfterSeconds: number; message: string | null }
export interface AvatarItem { id: string; displayName: string; category: string; sortOrder: number; bgColor: string | null; imageUrl: string }
export interface MigrationRecord {
  version: number; sourceDir: string; backupDir: string; backupFiles: number;
  profiles: number; liveTvFavorites: number; completedAtEpochMs: number;
}
export interface Bootstrap {
  activeProfile: number; profiles: ProfileInfo[]; addonsEnabled: number; addonsFailed: number;
  migration: MigrationRecord | null;
}

export type PosterShape = "Poster" | "Square" | "Landscape";
export interface MetaPreview {
  id: string; contentType: string; name: string; poster: string | null; banner: string | null;
  logo: string | null; posterShape: PosterShape; description: string | null; releaseInfo: string | null;
  imdbRating: string | null; genres: string[]; landscapePoster: string | null; runtime: string | null;
}
export interface CatalogPage { items: MetaPreview[]; rawItemCount: number; nextSkip: number | null }
export interface CatalogGenres { options: string[]; required: boolean }
export interface HomeRow {
  key: string; title: string; addonName: string; contentType: string; catalogId: string; supportsPagination: boolean;
  /** Set for a collection row (key `collection_<id>`). */
  collection?: CollectionView;
}

// ---- Collections (services::collections) ----
export interface FolderSource {
  /** `addon` (a catalog of an installed addon) or `tmdb` (a discover query resolved in Rust). */
  provider: string;
  key: string; genre: string | null; label: string; addonName: string; contentType: string; supportsPagination: boolean;
  /** Stored source id of a TMDB source. */
  sourceId?: string | null;
}
export interface FolderView {
  id: string; title: string;
  /** GIF (when enabled) or cover; a `.mp4`/`.webm` URL is a video. */
  image: string | null; stillImage: string | null; emoji: string | null;
  shape: "poster" | "landscape" | "square"; hideTitle: boolean;
  backdrop: string | null; logo: string | null;
  sources: FolderSource[]; skippedSources: number;
}
export interface CollectionView {
  id: string; title: string; backdrop: string | null; pinToTop: boolean;
  viewMode: "TABBED_GRID" | "ROWS" | "FOLLOW_LAYOUT" | string; showAllTab: boolean;
  folders: FolderView[];
}
export interface HomeView {
  heroEnabled: boolean;
  /** Catalog keys whose first page feeds the hero (at most 2, hidden rows included). */
  heroSources: string[];
  rows: HomeRow[];
  continueWatching: ContinueWatchingSettings;
}

// ---- Settings (nuvio-store::settings, per profile) ----
export interface SubtitleSettings {
  textColor: string; backgroundColor: string; outlineColor: string; outlineEnabled: boolean;
  outlineWidth: number; bold: boolean; fontSizeSp: number; bottomOffset: number; stripSdh: boolean;
  fontId: string; shadowEnabled: boolean; shadowOpacity: number; shadowSize: number;
  italic: boolean; spacing: number; blur: number; align: "left" | "center" | "right"; overrideAss: boolean;
}
export type MotionName = "off" | "steady" | "fluid";
export type AnimeModeName = "auto" | "a" | "b" | "c" | "aa" | "bb" | "ca";
export interface PictureSettings {
  mode: AnimeModeName; denoise: number; deblur: number; thin: number; darken: number; deband: number;
  brightness: number; contrast: number; saturation: number; gamma: number; hue: number;
}
export interface SubtitleFont { id: string; name: string; family: string }
export interface SubtitleFontList { imported: SubtitleFont[]; system: string[] }
export type EnhancementName = "off" | "movies" | "animeFast" | "animeQuality";
export type ResizeModeName = "Fit" | "Fill" | "Zoom" | "Stretch";
export interface PlayerSettings {
  resizeMode: ResizeModeName; decoderPriority: number; nvidiaRtxSuperResolution: boolean; nvidiaRtxHdr: boolean;
  motionSmoothing: MotionName; videoEnhancement: EnhancementName; picture: PictureSettings;
  preferredAudioLanguage: string; secondaryPreferredAudioLanguage: string | null;
  preferredSubtitleLanguage: string; secondaryPreferredSubtitleLanguage: string | null;
  subtitle: SubtitleSettings;
  skipSegmentsEnabled: boolean; autoSkipSegments: string[];
  nextEpisodeThresholdMode: "PERCENTAGE" | "MINUTES_BEFORE_END";
  nextEpisodeThresholdPercent: number; nextEpisodeThresholdMinutes: number; watchedPercent: number;
  // Sources / automatic playback
  reuseLastLinkEnabled: boolean; reuseLastLinkCacheHours: number;
  autoPlayFallbackOnFailure: boolean; autoPlayFallbackMaxSources: number; autoPlayFallbackStartupTimeoutSeconds: number;
  autoPlayMode: "manual" | "firstStream" | "regexMatch";
  autoPlaySource: "allSources" | "installedAddonsOnly" | "enabledPluginsOnly";
  autoPlaySelectedAddons: string[]; autoPlaySelectedPlugins: string[];
  autoPlayRegex: string; autoPlayTimeoutSeconds: number;
  // Next episode / chapters / hold / warnings
  autoPlayNextEpisodeEnabled: boolean; autoPlayNextEpisodeFallbackEnabled: boolean;
  autoPlayPreferBingeGroup: boolean; autoPlayReuseBingeGroup: boolean;
  preloadNextEpisodeSources: boolean; chapterTitleEnabled: boolean;
  holdToSpeedEnabled: boolean; holdToSpeedValue: number; showParentalGuide: boolean;
  // Audio
  rememberAudioDelayPerDevice: boolean; persistAudioAmplification: boolean;
  audioAmplificationDb: number; downmixEnabled: boolean; audioOutputChannels: string;
  maintainOriginalAudioOnDownmix: boolean; centerMixLevelDb: number;
  audioPassthrough: boolean; subtitleCodepage: string;
  // Buffer / cache
  bufferEnabled: boolean; minBufferMs: number; maxBufferMs: number; bufferForPlaybackMs: number;
  bufferForPlaybackAfterRebufferMs: number; backBufferDurationMs: number; targetBufferSizeMb: number;
  vodCacheEnabled: boolean; vodCacheSizeMode: "auto" | "manual"; vodCacheSizeMb: number;
  librarySource: string; progressSource: string; cards?: CardStyle; appLanguage?: string | null;
}
export interface HomeCatalogPreference { key: string; customTitle: string; enabled: boolean; heroSourceEnabled: boolean; order: number }
export interface HomeCatalogSettings { heroEnabled: boolean; showCatalogType: boolean; hideUnreleasedContent: boolean; items: HomeCatalogPreference[]; heroCatalogs: string[] | null }
export type P2pProfileName = "SOFT" | "BALANCED" | "FAST";
export type P2pCacheName = "NONE" | "GB_2" | "GB_5" | "GB_10";
export interface P2pSettings { enabled: boolean; enableUpload: boolean; hideTorrentStats: boolean; torrentProfile: P2pProfileName; cacheSize: P2pCacheName }
export interface ContinueWatchingSettings {
  visible: boolean; style: string; upNextFromFurthestEpisode: boolean; useEpisodeThumbnails: boolean;
  showUnairedNextUp: boolean; blurNextUp: boolean; showResumePromptOnLaunch: boolean;
  sortMode: string; clickAction: string;
}
export interface MetaSectionSetting { key: string; enabled: boolean; order: number }
export interface MetaScreenSettings {
  sections: MetaSectionSetting[]; backgroundMode: string; heroTrailerPlayback: boolean; tabLayout: boolean;
  episodeCardStyle: string; blurUnwatchedEpisodes: boolean; posterTransitionEnabled: boolean;
  showOverallRatings: boolean; episodeRatingsVisibility: string;
}
export interface DiscordActivity { title: string; state?: string | null; poster?: string | null; playing: boolean; positionMs?: number | null; durationMs?: number | null }
export interface ProfileSettings { player: PlayerSettings; home: HomeCatalogSettings; p2p: P2pSettings; continueWatching: ContinueWatchingSettings; metaScreen: MetaScreenSettings; discordRpcEnabled: boolean; debrid?: DebridSettings }
/** A movie/episode watched mark (Settings → Library); ids stay opaque. */
export interface WatchedItem {
  id: string; contentType: string; name: string; poster: string | null; releaseInfo: string | null;
  season: number | null; episode: number | null; videoId: string | null; markedAtEpochMs: number;
}
/** Selected file of a torrent prepared by the P2P engine (the local URL stays in Rust). */
export interface TorrentPrepared { fileIndex: number; fileSize: number; fileName: string; fileCount: number }
export interface P2pStatus {
  state: "idle" | "connecting" | "streaming" | "error" | "";
  phase: "engine" | "metadata" | "preparing" | "";
  peers: number; seeds: number; downloadSpeed: number; uploadSpeed: number; downloadedBytes: number;
  fileName: string | null; fileSize: number; totalProgress: number; readyAheadBytes: number; error: string | null;
}
export interface CatalogChoice { key: string; defaultTitle: string; addonName: string; contentType: string; enabled: boolean; customTitle: string }
export interface SettingsView {
  profile: ProfileInfo; settings: ProfileSettings; defaults: ProfileSettings; catalogs: CatalogChoice[];
  deviceLanguages: string[]; dataDir: string; migration: (MigrationRecord & { settingsProfiles?: number }) | null;
  /** Version of the app (Cargo package version). */
  appVersion: string;
}

export interface SearchSection { key: string; title: string; addonName: string; items: MetaPreview[] }

// ---- Discover (services::discover) ----
export interface DiscoverSource {
  key: string; addonName: string; contentType: string; catalogId: string; catalogName: string;
  genreOptions: string[]; genreRequired: boolean; defaultGenre: string | null; supportsPagination: boolean;
}
export interface DiscoverView { sources: DiscoverSource[]; typeOptions: string[]; selectedKey: string | null }
export interface DiscoverPage {
  items: MetaPreview[]; nextSkip: number | null; duplicatePages: number; supportsPagination: boolean;
}

export interface MetaPerson { name: string; role: string | null; photo: string | null }
export interface MetaVideo {
  id: string; title: string; released: string | null; available: boolean; thumbnail: string | null;
  season: number | null; episode: number | null; overview: string | null;
}
export interface MetaTrailer { id: string; key: string; name: string; site: string; type: string; official: boolean; seasonNumber: number | null }
export interface MetaDetails {
  id: string; contentType: string; name: string; poster: string | null; background: string | null;
  logo: string | null; description: string | null; releaseInfo: string | null; imdbRating: string | null;
  ageRating: string | null; status: string | null; runtime: string | null; genres: string[]; director: string[];
  writer: string[]; cast: MetaPerson[]; country: string | null; awards: string | null; language: string | null;
  website: string | null; defaultVideoId: string | null; videos: MetaVideo[];
}

/** What the details page needs to ask `meta_details`: the addon's own type and id, never guessed. */
export interface MetaRef { contentType: string; id: string; name: string; resume?: ResumeTarget }

/** Builds a `MetaRef` from a catalog/search item, or explains why it cannot. */
export function metaRef(item: Pick<MetaPreview, "contentType" | "id" | "name">): MetaRef | string {
  const contentType = typeof item.contentType === "string" ? item.contentType.trim() : "";
  const id = typeof item.id === "string" ? item.id.trim() : "";
  if (!id) return t("o addon não informou o identificador deste título");
  if (!contentType) return t("o addon não informou o tipo deste título (filme/série)");
  return { contentType, id, name: item.name };
}
export interface ExternalRating { source: string; value: number; certified: boolean }
export interface TrackingCredentialsView { traktConfigured: boolean; simklConfigured: boolean; traktConnected: boolean; simklConnected: boolean; traktFromBuild: boolean; simklFromBuild: boolean; mdblistConnected: boolean; mdblistFromBuild: boolean }
export interface RemoteResume { season: number | null; episode: number | null; progressPercent: number; pausedAt: string | null }
export interface TraktDeviceStart { userCode: string; verificationUrl: string; intervalSecs: number }
export type TrackingPollResult = "pending" | "authorized" | "expired" | "denied" | "slowDown" | "noSession" | "error";
export interface TrackingSyncResult { movies: number; episodes: number }
export interface ServerConnection { id: string; provider: string; name: string; address: string; userId: string; userName: string; enabled: boolean }
export interface ServerLibrary { id: string; name: string; kind: string }
export type DownloadStatus = "downloading" | "paused" | "completed" | "failed" | "queued" | "preparing" | "done";
export interface StreamsSettings {
  showSizeBadges: boolean | null; showAddonLogo: boolean; showInPlayer: boolean;
  playerBadgeSource: string; badgePlacement: string; backgroundMode: string; badgeUrls: string[];
}
export interface StreamsSettingsView { settings: StreamsSettings; importedFilters: number }
export interface CardStyle { width: number; radius: number; landscape: boolean; hideLabels: boolean }
export interface DebridSettings { enabled: boolean }
export interface DownloadsFolder { folder: string; freeBytes: number | null }
export interface TorboxStatus { configured: boolean; enabled: boolean; account: string | null }
export interface DownloadRequest { key: string; title: string; subtitle: string | null; poster: string | null; season: number | null; episode: number | null; playback: PlaybackItem | null }
export interface DownloadItem {
  id: string; poster?: string | null; contentType: string; parentMetaId: string; parentMetaType: string; videoId: string;
  title: string; seasonNumber: number | null; episodeNumber: number | null; episodeTitle: string | null;
  streamTitle: string; streamSubtitle: string | null; providerName: string; sourceUrl: string;
  fileName: string; localFileUri: string | null; status: DownloadStatus; downloadedBytes: number;
  totalBytes: number | null; errorMessage: string | null; createdAtEpochMs: number; updatedAtEpochMs: number;
  /** Upstream aliases used by the account's Downloads list component. */
  kind: string; subtitle?: string | null; folder: string; downloaded: number; total: number | null;
  speed: number; connections: number; error: string | null; createdAt: number; sourceName: string;
}
export interface ShuffleView { available: boolean; enabled: boolean; includeWatched: boolean }
export interface ParentalGuide { nudity: string | null; violence: string | null; profanity: string | null; alcohol: string | null; frightening: string | null }
export interface Details { meta: MetaDetails; seasons: number[]; addonName: string; trailer: MetaTrailer | null; ratings: ExternalRating[]; watchedKeys: string[] }

export type StreamKind = "direct" | "torrent" | "debrid" | "external" | "youTube";
export interface StreamEntry {
  key: string; name: string | null; title: string | null; description: string | null;
  filename: string | null; videoSize: number | null; kind: StreamKind;
  /** Played through the connected debrid service instead of local P2P. */
  debridPlayable: boolean;
  /** Name of the active provider (TorBox, Real-Debrid…) when this source uses debrid. */
  debridProvider: string | null;
  headerNames: string[]; notWebReady: boolean;
  /** Upstream alias of videoSize used by the stream badge parser. */
  size?: number | null;
  torbox?: boolean;
  /** `externalUrl` source: safe only as a link to open in the browser. */
  externalUrl: string | null;
  /** `ytId` source: resolved in Rust by the YouTube extractor. */
  youtubeId: string | null;
}
export interface StreamGroup { addonName: string; streams: StreamEntry[]; error: string | null }

/** Badge rules imported in the original app (read only); matched by regex on the stream text. */
export interface StreamBadgeFilter {
  name: string; pattern: string; imageUrl: string; tagColor: string; tagStyle: string;
  textColor: string; borderColor: string; groupId: string;
}
export interface StreamBadgeRules { filters: StreamBadgeFilter[]; showFileSizeBadges: boolean; placement: string }

/** Settings → Services → MDBList Ratings. */
export interface MdblistSettingsView {
  enabled: boolean; apiKeySet: boolean; apiKeyFromAccount: boolean; apiKeyFromOriginal: boolean;
  useImdb: boolean; useTmdb: boolean; useTomatoes: boolean; useMetacritic: boolean;
  useTrakt: boolean; useLetterboxd: boolean; useAudience: boolean; useMal: boolean;
}
export interface MdblistSettingsInput {
  enabled: boolean; apiKey?: string | null;
  useImdb: boolean; useTmdb: boolean; useTomatoes: boolean; useMetacritic: boolean;
  useTrakt: boolean; useLetterboxd: boolean; useAudience: boolean; useMal: boolean;
}

/** Settings → Services → Connected Services (debrid). Keys are never returned. */
export interface DebridProviderView {
  id: string; label: string; connected: boolean; fromAccount: boolean; fromOriginal: boolean;
  account: string | null;
}
export interface DebridSettingsView {
  enabled: boolean; providers: DebridProviderView[]; active: string | null;
  // Kept for older builds of the UI.
  torboxConnected: boolean; torboxFromAccount: boolean; torboxFromOriginal: boolean;
  account: string | null;
}
export interface DebridSettingsInput {
  enabled: boolean; torboxApiKey?: string | null;
  keys?: Record<string, string | null>; active?: string | null;
}

/** Settings → Addons → Plugins (Nuvio scraper repositories). The manifest URL stays in Rust. */
export interface PluginRepositoryView {
  key: string; name: string; version: string; description: string | null;
  scraperCount: number; lastUpdated: number; refreshDue: boolean;
}
export interface PluginScraperView {
  key: string; repositoryKey: string; name: string; version: string;
  supportedTypes: string[]; enabled: boolean; manifestEnabled: boolean;
}
export interface PluginsView {
  profile: number; enabled: boolean; repositories: PluginRepositoryView[];
  scrapers: PluginScraperView[]; error: string | null;
}

/** Settings → Services → TMDB (the key never leaves Rust). */
export interface TmdbSettingsView {
  enabled: boolean; language: string; useMoreLikeThis: boolean; useCollections: boolean;
  useArtwork: boolean; useBasicInfo: boolean; useDetails: boolean; useCredits: boolean;
  useEpisodes: boolean; useTrailers: boolean;
  apiKeySet: boolean; apiKeyFromAccount: boolean; apiKeyFromOriginal: boolean;
}
export interface TmdbSettingsInput {
  enabled: boolean; language: string; useMoreLikeThis: boolean; useCollections: boolean;
  useArtwork: boolean; useBasicInfo: boolean; useDetails: boolean; useCredits: boolean;
  useEpisodes: boolean; useTrailers: boolean;
  /** null keeps the current key; "" clears Astra's key (the original's is used again). */
  apiKey?: string | null;
}

export interface Channel {
  key: string; metaId: string; contentType: string; name: string; logo: string | null;
  genres: string[]; addonName: string;
}
export interface ChannelList { channels: Channel[]; categories: [string, number][]; failedCatalogs: number }

export interface Chapter { time: number; title: string }
export interface SubtitleOption { index: number; source: string; language: string; title: string }
export interface SubtitleList { options: SubtitleOption[]; addons: number; failed: number }
export interface Moment {
  kind: "intro" | "recap" | "outro" | "movie-credits" | "post-credits";
  start: number; end: number; target: number; toPostCredits: boolean; auto: boolean;
}
export interface Moments { segments: Moment[]; nextEpisodeAt: number | null }
export interface Track {
  id: number; kind: string; title: string | null; lang: string | null; codec: string | null;
  selected: boolean; external: boolean; default: boolean; forced: boolean;
}
export interface PlayerSnapshot {
  session: number; loaded: boolean; ended: boolean; paused: boolean; buffering: boolean;
  timePos: number; duration: number; cacheSeconds: number; volume: number; mute: boolean; speed: number;
  subDelayMs: number; audioDelayMs: number; subVisible: boolean; hwdecCurrent: string | null;
  videoCodec: string | null; width: number; height: number; fps: number; droppedFrames: number;
  decoderDroppedFrames: number; aid: string | null; sid: string | null; tracks: Track[]; chapters: Chapter[];
  pictureInPicture: boolean; motion: MotionName; enhancement: EnhancementName; picture: PictureSettings;
  autoChanges: number; autoReason: "motion" | "enhancement" | "effects" | null; error: string | null;
}
export type ResizeMode = "fit" | "fill" | "zoom" | "stretch";
export type PlayerCommand =
  | { type: "togglePause" }
  | { type: "setPaused"; paused: boolean }
  | { type: "seek"; seconds: number; relative: boolean }
  | { type: "setVolume"; volume: number }
  | { type: "setMute"; mute: boolean }
  | { type: "setSpeed"; speed: number }
  | { type: "setAudioTrack"; id: string }
  | { type: "setSubtitleTrack"; id: string }
  | { type: "setSubtitleDelay"; ms: number }
  | { type: "setSubtitleVisible"; visible: boolean }
  | { type: "setAudioDelay"; ms: number }
  | { type: "setResizeMode"; mode: ResizeMode }
  | { type: "setMotion"; motion: MotionName }
  | { type: "setEnhancement"; enhancement: EnhancementName }
  | { type: "setPicture"; picture: PictureSettings }
  | { type: "setCompare"; active: boolean };
export interface PlayerInfo { session: number; libmpvApi: string }
export interface CssRect { x: number; y: number; width: number; height: number }

// ---- Library (nuvio-store::library, per profile) ----
/** A saved movie/series with the display data saved with it (identity: contentType + id). */
export interface LibraryItem {
  id: string; contentType: string; name: string; poster: string | null; banner: string | null; logo: string | null;
  description: string | null; releaseInfo: string | null; imdbRating: string | null; genres: string[];
  posterShape: PosterShape; savedAtEpochMs: number;
}
/** A played movie/episode (progress or watched mark). `key` = progress key (`<metaId>` or `<metaId>_s<S>e<E>`). */
export interface HistoryEntry {
  key: string; metaId: string; metaType: string; videoId: string | null; title: string; poster: string | null;
  background: string | null; season: number | null; episode: number | null; episodeTitle: string | null;
  positionMs: number; durationMs: number; fraction: number; completed: boolean; inProgress: boolean;
  lastWatchedEpochMs: number;
  /** The next episode of a series, after finishing the previous one. */
  nextUp: boolean;
  /** Date of a next episode that is not out yet. */
  airs: string | null;
}
export interface HistoryPage { items: HistoryEntry[]; total: number; nextOffset: number | null }
export interface TitleProgress { resume: HistoryEntry | null; entries: HistoryEntry[] }
/** What `stream_play` records progress for (no URLs). */
export interface PlaybackItem {
  contentType: string; parentMetaId: string; parentMetaType: string; videoId: string; title: string;
  logo: string | null; poster: string | null; background: string | null; seasonNumber: number | null;
  episodeNumber: number | null; episodeTitle: string | null; episodeThumbnail: string | null; releaseInfo: string | null;
}
/** Where Details should resume (Continue watching / history). */
export interface ResumeTarget { videoId: string | null; season: number | null; episode: number | null }

// ---- Addons (manage.rs). Manifest URLs stay in Rust: an addon is a key + its host. ----
export interface AddonCatalogView { name: string; contentType: string }
export interface AddonView {
  key: string; enabled: boolean; status: "ok" | "error" | "disabled"; error: string | null;
  name: string; description: string; version: string; logo: string | null; types: string[];
  catalogs: AddonCatalogView[]; catalogCount: number; resources: string[];
  configurable: boolean; configurationRequired: boolean; tracking: boolean; host: string; configured: boolean;
}
export interface AddonsView { profile: number; addonProfile: number; readOnly: boolean; addons: AddonView[] }
export interface AddonPreview { key: string; addon: AddonView; tracking: boolean }

/** One kind of data on both sides, by identifier (read-only comparison with the account). */
export interface SyncCompare {
  local: number;
  remote: number;
  both: number;
  onlyLocal: number;
  onlyRemote: number;
  differing: number;
  localNewer: number;
  localDuplicates: number;
  remoteDuplicates: number;
}

export interface SyncProfilePreview {
  index: number;
  localName: string | null;
  remoteName: string | null;
  profileDiffers: boolean;
  sharesPrimaryAddons: boolean;
  addons: SyncCompare;
  addonOrderDiffers: boolean;
  favorites: SyncCompare;
  progress: SyncCompare;
  watched: SyncCompare;
  pending: number;
}

export interface SyncPreview {
  generatedAtEpochMs: number;
  firstSync: boolean;
  otherAccount: boolean;
  profiles: SyncCompare;
  entries: SyncProfilePreview[];
  pending: number;
  toSend: number;
  toReceive: number;
  conflicts: number;
}

/** App updates (official Tauri updater). */
export interface UpdateStatus { configured: boolean; currentVersion: string; repository: string | null }
export interface UpdateInfo { version: string; currentVersion: string; notes: string | null; date: string | null }
export interface UpdateProgress { phase: "downloading" | "verifying" | "installing" | "error"; downloaded: number; total: number | null; error: string | null }

/** Program guide of a live channel (times in UTC ms). */
export interface EpgProgram { startMs: number; endMs: number; title: string; description: string | null }
export interface EpgView { nowMs: number; programs: EpgProgram[]; hasSchedule: boolean }

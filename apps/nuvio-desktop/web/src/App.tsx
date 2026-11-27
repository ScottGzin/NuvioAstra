// App shell: sidebar + current screen. Screens stay mounted while hidden behind the full-window
// player, so returning keeps scroll position and loaded rows.

import { useCallback, useEffect, useState } from "react";
import { start as startAccount } from "./account";
import { api, errorText, onDataEpoch, onProfilesChanged, reloadProfileData } from "./api";
import { listen } from "./tauri";
import { LockScreen } from "./screens/LockScreen";
import { Welcome } from "./screens/Welcome";
import { startAppLanguage } from "./appLanguage";
import { startCardStyle } from "./cardStyle";
import { useToasts } from "./toast";
import { HoverPreview } from "./components/HoverPreview";
import { ResumePrompt, WhoIsWatching, chooserOnLaunch } from "./components/LaunchScreens";
import { PinDialog } from "./components/PinDialog";
import { ProfileMenu, ProfilesDialog } from "./components/ProfileMenu";
import { Sidebar, type Tab } from "./components/Sidebar";
import { UpdateBanner } from "./components/UpdateBanner";
import { FullPlayer } from "./player/FullPlayer";
import { usePlayer } from "./player/PlayerContext";
import { Details } from "./screens/Details";
import { Folder } from "./screens/Folder";
import { TmdbEntity } from "./screens/TmdbEntity";
import { DiscordPresenceSync } from "./components/DiscordPresence";
import { Home } from "./screens/Home";
import { Library } from "./screens/Library";
import { resumeTarget } from "./library";
import { LiveTv } from "./screens/LiveTv";
import { Search } from "./screens/Search";
import { Settings } from "./screens/Settings";
import { metaRef, type Bootstrap, type CollectionView, type FolderView, type HistoryEntry, type MetaPreview, type MetaRef, type ProfilesView, type TmdbEntityKind } from "./types";
import { t } from "./i18n";


export function App() {
  const player = usePlayer();
  const [appMode, setAppMode] = useState<string>("");
  useEffect(() => {
    api.appMode().then(setAppMode, () => setAppMode("app"));
  }, []);
  const [boot, setBoot] = useState<Bootstrap | null>(null);
  const [bootError, setBootError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("home");
  /** Pages over the tabs: a title's details, and from it other titles or a person's/studio's page
   *  (TMDB); "Voltar" returns to the one below. */
  const [stack, setStack] = useState<Overlay[]>([]);
  const top = stack.length > 0 ? stack[stack.length - 1] : null;
  const details = top?.kind === "details" ? top.ref : null;
  const setDetails = useCallback((ref: MetaRef | null) => setStack(ref ? [{ kind: "details", ref }] : []), []);
  const back = useCallback(() => setStack((s) => s.slice(0, -1)), []);
  /** Collection folder opened from Home (stays under the details of a title opened from it). */
  const [folder, setFolder] = useState<{ collection: CollectionView; folder: FolderView } | null>(null);
  const [openError, setOpenError] = useState<string | null>(null);
  const [profiles, setProfiles] = useState<ProfilesView | null>(null);
  const [profilesFailed, setProfilesFailed] = useState(false);
  /** The account screen of the first opening; it waits while a profile is locked. */
  const [welcome, setWelcome] = useState<"unknown" | "waiting" | "open" | "none">("unknown");
  const [menu, setMenu] = useState(false);
  const [manage, setManage] = useState(false);
  const [switching, setSwitching] = useState<string | null>(null);
  /** Protected profile waiting for its PIN before the switch. */
  const [pinFor, setPinFor] = useState<number | null>(null);
  // Screens are keyed by these: a profile switch remounts every screen, an addon change the ones
  // that show addon data. Module caches are dropped by `onDataEpoch` in each module.
  const [profileEpoch, setProfileEpoch] = useState(0);
  const [addonsEpoch, setAddonsEpoch] = useState(0);

  useEffect(() => {
    api.bootstrap().then(setBoot, (e) => setBootError(errorText(e)));
    api.profilesList().then(setProfiles, () => setProfilesFailed(true));
    api.welcomeNeeded().then((needed) => setWelcome(needed ? "waiting" : "none"), () => setWelcome("none"));
    startAccount();
    startCardStyle();
    startAppLanguage();
    // The event listeners just registered must hear the first sync.
    const ready = window.setTimeout(() => void api.appReady().catch(() => {}), 200);
    return () => window.clearTimeout(ready);
  }, []);

  // Locked or unlocked by Rust (PIN removed in Nuvio, protected profile received).
  useEffect(
    () =>
      listen<null>("profile-lock", () => {
        api.profilesList().then(setProfiles, () => {});
        reloadProfileData();
      }),
    [],
  );

  useEffect(() => {
    if (welcome === "waiting" && profiles && !profiles.locked && appMode) setWelcome(appMode === "app" ? "open" : "none");
  }, [welcome, profiles, appMode]);

  const endWelcome = useCallback(() => {
    setWelcome("none");
    api.profilesList().then(setProfiles, () => {});
    reloadProfileData();
  }, []);
  const openFromLock = useCallback((next: ProfilesView) => {
    setProfiles(next);
    reloadProfileData();
  }, []);

  // Names, avatars or PIN changed by the account (another device).
  useEffect(() => onProfilesChanged(() => void api.profilesList().then(setProfiles, () => {})), []);

  useEffect(
    () =>
      onDataEpoch((reason) => {
        if (reason === "profile") {
          setProfileEpoch((e) => e + 1);
          setDetails(null);
          setFolder(null);
          setOpenError(null);
          setBoot(null);
          api.profilesList().then(setProfiles, () => {});
        } else {
          setAddonsEpoch((e) => e + 1);
          setFolder(null); // its catalogs were resolved with the previous addons
        }
        api.bootstrap().then(setBoot, (e) => setBootError(errorText(e)));
      }),
    [],
  );

  /** Closes the player (its progress is saved for the current profile), then switches. */
  const switchProfile = useCallback(
    async (index: number, pinChecked = false) => {
      setMenu(false);
      if (!pinChecked && profiles?.profiles.find((p) => p.index === index)?.pinEnabled) {
        setPinFor(index);
        return;
      }
      const name = profiles?.profiles.find((p) => p.index === index)?.name ?? "";
      setSwitching(name);
      try {
        if (player.session) await player.close();
        setProfiles(await api.profileSwitch(index));
      } catch (e) {
        setOpenError(t("Não foi possível trocar de perfil: {error}", { error: errorText(e) }));
      } finally {
        setSwitching(null);
      }
    },
    [player, profiles],
  );

  const select = useCallback(
    (next: Tab) => {
      // The Live TV preview belongs to its screen: leaving the screen stops it.
      if (next !== "tv" && player.session?.view === "embedded") void player.close();
      setDetails(null);
      setFolder(null);
      setTab(next);
    },
    [player],
  );

  // Home and Search hand over the catalog item as the addon sent it; its type is checked here,
  // so a malformed item gives a clear message instead of a failed command.
  const open = useCallback((item: MetaPreview) => {
    const ref = metaRef(item);
    if (typeof ref === "string") {
      setOpenError(t("Não foi possível abrir “{name}”: {error}.", { name: item.name, error: ref }));
      return;
    }
    setOpenError(null);
    setDetails(ref);
  }, [setDetails]);
  /** Another title opened from a details or TMDB page: on top of it. */
  const push = useCallback((item: MetaPreview) => {
    const ref = metaRef(item);
    if (typeof ref === "string") {
      setOpenError(t("Não foi possível abrir “{name}”: {error}.", { name: item.name, error: ref }));
      return;
    }
    setOpenError(null);
    setStack((s) => [...s.slice(-15), { kind: "details", ref }]);
  }, []);
  const pushEntity = useCallback((kind: TmdbEntityKind, id: number, name: string) => {
    setStack((s) => [...s.slice(-15), { kind: "entity", entity: { kind, id, name } }]);
  }, []);

  // Continue watching / history: the title's details with that movie/episode selected and its
  // sources listed; the chosen source starts at the saved position (the original's STREAM action).
  const resume = useCallback((entry: HistoryEntry) => {
    setOpenError(null);
    setDetails({ contentType: entry.metaType, id: entry.metaId, name: entry.title, resume: resumeTarget(entry) });
  }, [setDetails]);
  const openEntry = useCallback((entry: HistoryEntry) => {
    setOpenError(null);
    setDetails({ contentType: entry.metaType, id: entry.metaId, name: entry.title });
  }, [setDetails]);

  const toasts = useToasts();
  // "Quem está assistindo?" once per launch, with more than one profile.
  const [chooser, setChooser] = useState(chooserOnLaunch);
  const full = player.session?.view === "full";
  const profile = profiles?.profiles.find((p) => p.index === profiles.active) ?? null;
  const dataKey = `${profileEpoch}-${addonsEpoch}`;

  // Nothing of the profile is drawn before it is known to be open.
  const starting = (!profiles && !profilesFailed) || welcome === "unknown" || (welcome === "waiting" && !profiles?.locked && !profilesFailed);
  if (starting) return <div className="gate" aria-busy />;
  if (welcome === "open" || profiles?.locked) {
    return (
      <div className="gate-page">
        <UpdateBanner automatic={appMode === "app"} />
        {welcome === "open" ? <Welcome onDone={endWelcome} /> : <LockScreen view={profiles!} onOpen={openFromLock} />}
      </div>
    );
  }

  if (chooser && profiles && profiles.profiles.length > 1) {
    return (
      <div className="gate-page">
        <WhoIsWatching
          view={profiles}
          onPick={(index) => {
            setChooser(false);
            if (index !== profiles.active) void switchProfile(index);
          }}
          onManage={() => {
            setChooser(false);
            setManage(true);
          }}
        />
      </div>
    );
  }

  return (
    <>
      <div className={`shell ${full ? "behind-player" : ""}`} aria-hidden={full}>
        <Sidebar current={tab} onSelect={select} profile={profile} onProfile={() => setMenu((m) => !m)} />
        {menu && profiles && (
          <ProfileMenu view={profiles} busy={switching !== null} onSwitch={(i) => void switchProfile(i)} onManage={() => { setMenu(false); setManage(true); }} onClose={() => setMenu(false)} />
        )}
        {pinFor !== null && profiles && (
          <PinDialog
            profile={profiles.profiles.find((p) => p.index === pinFor)!}
            onUnlocked={() => { const i = pinFor; setPinFor(null); void switchProfile(i, true); }}
            onCancel={() => setPinFor(null)}
          />
        )}
        {manage && profiles && <ProfilesDialog view={profiles} onChanged={setProfiles} onClose={() => setManage(false)} />}
        {switching !== null && (
          <div className="switching" role="status" data-testid="profile-switching">{t("Trocando para")} {switching || t("outro perfil")}…</div>
        )}
        <main className="content">
          <UpdateBanner automatic={appMode === "app"} waiting={full} />
          {bootError && <div className="banner error">{t("Não foi possível ler os dados do Nuvio Astra:")} {bootError}</div>}
          {openError && <div className="banner error" role="alert">{openError}</div>}
          {boot && boot.addonsFailed > 0 && (
            <div className="banner">{t("{failed} de {total} addon(s) não responderam; os demais estão disponíveis.", { failed: boot.addonsFailed, total: boot.addonsEnabled })}</div>
          )}
          <div className="screen" hidden={top !== null || folder !== null || tab !== "home"}>
            <Home key={dataKey} onOpen={open} onResume={resume} onOpenEntry={openEntry} onOpenFolder={(collection, f) => setFolder({ collection, folder: f })} />
          </div>
          {folder && tab === "home" && (
            <div className="screen" hidden={top !== null}>
              <Folder key={`${dataKey}/${folder.collection.id}/${folder.folder.id}`} collection={folder.collection} folder={folder.folder} onOpen={open} onBack={() => setFolder(null)} />
            </div>
          )}
          {tab === "search" && <div className="screen" hidden={top !== null}><Search key={dataKey} onOpen={open} /></div>}
          {tab === "tv" && <div className="screen screen-fill"><LiveTv key={dataKey} /></div>}
          {tab === "library" && <div className="screen" hidden={top !== null}><Library key={profileEpoch} onOpen={open} onResume={resume} onOpenEntry={openEntry} /></div>}
          {tab === "settings" && <div className="screen screen-fill" hidden={top !== null}><Settings key={profileEpoch} /></div>}
          {top?.kind === "entity" && (
            <div className="screen">
              <TmdbEntity key={`${stack.length}/${top.entity.kind}/${top.entity.id}`} {...top.entity} onBack={back} onOpen={push} />
            </div>
          )}
          {details && (
            <div className="screen">
              <Details key={`${stack.length}/${details.contentType}/${details.id}/${details.resume?.videoId ?? ""}`} item={details} onBack={back} onOpen={push} onEntity={pushEntity} />
            </div>
          )}
        </main>
      </div>
      {full && <FullPlayer />}
      <HoverPreview />
      <DiscordPresenceSync tab={tab} viewing={details?.name ?? (top?.kind === "entity" ? top.entity.name : null)} />
      {!full && <ResumePrompt onResume={resume} />}
      {toasts.length > 0 && (
        <div className="toasts" role="status" aria-live="polite">
          {toasts.map((t) => <div key={t.id} className={`toast ${t.error ? "error" : ""}`}>{t.text}</div>)}
        </div>
      )}
    </>
  );
}

type Overlay = { kind: "details"; ref: MetaRef } | { kind: "entity"; entity: { kind: TmdbEntityKind; id: number; name: string } };

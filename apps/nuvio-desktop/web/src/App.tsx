// App shell: sidebar + current screen. Screens stay mounted while hidden behind the full-window
// player, so returning keeps scroll position and loaded rows.

import { useCallback, useEffect, useRef, useState } from "react";
import { start as startAccount } from "./account";
import { startAppLanguage } from "./appLanguage";
import { api, errorText, onDataEpoch, onProfilesChanged, reloadProfileData } from "./api";
import { useGamepad, useInputNavigation } from "./input/navigation";
import { listen } from "./tauri";
import { LockScreen } from "./screens/LockScreen";
import { Welcome } from "./screens/Welcome";
import { PinDialog } from "./components/PinDialog";
import { ProfileMenu } from "./components/ProfileMenu";
import { ProfileSelection } from "./components/ProfileSelection";
import { ResumePrompt } from "./components/ResumePrompt";
import { Sidebar, type Tab } from "./components/Sidebar";
import { UpdateBanner } from "./components/UpdateBanner";
import { FullPlayer } from "./player/FullPlayer";
import { usePlayer } from "./player/PlayerContext";
import { Details } from "./screens/Details";
import { Folder } from "./screens/Folder";
import { Home } from "./screens/Home";
import { Library } from "./screens/Library";
import { resumeTarget } from "./library";
import { LiveTv } from "./screens/LiveTv";
import { Search } from "./screens/Search";
import { Discover } from "./screens/Discover";
import { Settings } from "./screens/Settings";
import { metaRef, type CollectionView, type FolderView, type HistoryEntry, type MetaPreview, type MetaRef, type ProfilesView } from "./types";
import { t } from "./i18n";


export function App() {
  useInputNavigation();
  useGamepad();
  const player = usePlayer();
  const [appMode, setAppMode] = useState<string>("");
  useEffect(() => {
    api.appMode().then(setAppMode, () => setAppMode("app"));
  }, []);

  // OAuth callbacks arrive through the `nuvio://` scheme; forward them to the settings panel.
  // On Linux/Windows the scheme starts a new instance, so also read the URL that started this one.
  useEffect(() => {
    const forward = (urls: string[] | null) => {
      const url = urls?.[0] ?? "";
      if (url.startsWith("nuvio://auth/simkl")) {
        window.dispatchEvent(new CustomEvent("nuvio-simkl-callback", { detail: url }));
      }
    };
    const unlisten = listen<string[]>("deep-link://new-url", forward);
    void api.deepLinkCurrent().then(forward, () => {});
    return unlisten;
  }, []);
  const [bootError, setBootError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("home");
  const [details, setDetails] = useState<MetaRef | null>(null);
  /** Collection folder opened from Home (stays under the details of a title opened from it). */
  const [folder, setFolder] = useState<{ collection: CollectionView; folder: FolderView } | null>(null);
  const [openError, setOpenError] = useState<string | null>(null);
  const [profiles, setProfiles] = useState<ProfilesView | null>(null);
  const [profilesFailed, setProfilesFailed] = useState(false);
  /** The account screen of the first opening; it waits while a profile is locked. */
  const [welcome, setWelcome] = useState<"unknown" | "waiting" | "open" | "none">("unknown");
  /** The original asks "Who's watching?" on every launch, before opening the last profile. */
  const [needChooser, setNeedChooser] = useState(true);
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
    api.bootstrap().catch((e) => setBootError(errorText(e)));
    api.profilesList().then(setProfiles, () => setProfilesFailed(true));
    api.welcomeNeeded().then((needed) => setWelcome(needed ? "waiting" : "none"), () => setWelcome("none"));
    startAccount();
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
    setNeedChooser(false);
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
          api.profilesList().then(setProfiles, () => {});
        } else {
          setAddonsEpoch((e) => e + 1);
          setFolder(null); // its catalogs were resolved with the previous addons
        }
        api.bootstrap().catch((e) => setBootError(errorText(e)));
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
  }, []);

  // Continue watching / history: the title's details with that movie/episode selected and its
  // sources listed; the chosen source starts at the saved position (the original's STREAM action).
  const resume = useCallback((entry: HistoryEntry) => {
    setOpenError(null);
    setDetails({ contentType: entry.metaType, id: entry.metaId, name: entry.title, resume: resumeTarget(entry) });
  }, []);
  // Resume prompt on launch shows the most recent title once per app session.
  const [resumeEntry, setResumeEntry] = useState<HistoryEntry | null>(null);
  const resumePrompted = useRef(false);
  useEffect(() => {
    if (resumePrompted.current) return;
    const timer = window.setTimeout(() => {
      void Promise.all([api.settingsGet(), api.libraryContinueWatching()]).then(([settings, entries]) => {
        if (resumePrompted.current) return;
        resumePrompted.current = true;
        if (settings.settings.continueWatching.showResumePromptOnLaunch && entries.length > 0) {
          setResumeEntry(entries[0]);
        }
      }, () => {});
    }, 1500);
    return () => window.clearTimeout(timer);
  }, []);

  const openEntry = useCallback((entry: HistoryEntry) => {
    setOpenError(null);
    setDetails({ contentType: entry.metaType, id: entry.metaId, name: entry.title });
  }, []);

  const full = player.session?.view === "full";
  const profile = profiles?.profiles.find((p) => p.index === profiles.active) ?? null;
  const dataKey = `${profileEpoch}-${addonsEpoch}`;

  // Nothing of the profile is drawn before it is known to be open.
  const starting = (!profiles && !profilesFailed) || welcome === "unknown" || (welcome === "waiting" && !profiles?.locked && !profilesFailed);
  if (starting) return <div className="gate" aria-busy />;
  if (welcome === "open" || profiles?.locked || needChooser) {
    return (
      <div className="gate-page">
        <UpdateBanner automatic={appMode === "app"} />
        {welcome === "open" ? <Welcome onDone={endWelcome} /> : <LockScreen view={profiles!} onOpen={openFromLock} initialMode={needChooser ? "profiles" : "pin"} />}
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
        {manage && profiles && (
          <ProfileSelection
            view={profiles}
            initialManage
            onOpen={setProfiles}
            onChanged={setProfiles}
            onDone={() => setManage(false)}
          />
        )}
        {switching !== null && (
          <div className="switching" role="status" data-testid="profile-switching">{t("Trocando para")} {switching || t("outro perfil")}…</div>
        )}
        <main className="content">
          <UpdateBanner automatic={appMode === "app"} waiting={full} />
          {bootError && <div className="banner error">{t("Não foi possível ler os dados do Nuvio Astra:")} {bootError}</div>}
          {openError && <div className="banner error" role="alert">{openError}</div>}
          <div className="screen" hidden={details !== null || folder !== null || tab !== "home"}>
            <Home key={dataKey} onOpen={open} onResume={resume} onOpenFolder={(collection, f) => setFolder({ collection, folder: f })} />
          </div>
          {folder && tab === "home" && (
            <div className="screen" key={`folder-${folder.collection.id}-${folder.folder.id}`} hidden={details !== null}>
              <Folder key={`${dataKey}/${folder.collection.id}/${folder.folder.id}`} collection={folder.collection} folder={folder.folder} onOpen={open} onBack={() => setFolder(null)} />
            </div>
          )}
          {tab === "search" && <div className="screen" hidden={details !== null}><Search key={dataKey} onOpen={open} /></div>}
          {tab === "discover" && <div className="screen" hidden={details !== null}><Discover key={dataKey} onOpen={open} /></div>}
          {tab === "tv" && <div className="screen screen-fill"><LiveTv key={dataKey} /></div>}
          {tab === "library" && <div className="screen" hidden={details !== null}><Library key={profileEpoch} onOpen={open} onResume={resume} onOpenEntry={openEntry} /></div>}
          {tab === "settings" && <div className="screen screen-fill" hidden={details !== null}><Settings key={profileEpoch} /></div>}
          {details && (
            <div className="screen" key={`details-${details.contentType}-${details.id}`}>
              <Details key={`${details.contentType}/${details.id}/${details.resume?.videoId ?? ""}`} item={details} onBack={() => setDetails(null)} onOpen={open} />
            </div>
          )}
        </main>
      </div>
      {resumeEntry && !full && (
        <ResumePrompt
          entry={resumeEntry}
          onResume={() => { const entry = resumeEntry; setResumeEntry(null); resume(entry); }}
          onDismiss={() => setResumeEntry(null)}
        />
      )}
      <div className="gamepad-hints" aria-hidden>
        <span>✕ / A — {t("Selecionar")}</span><span>○ / B — {t("Voltar")}</span><span>✛ — {t("Navegar")}</span>
      </div>
      {full && <FullPlayer />}
    </>
  );
}

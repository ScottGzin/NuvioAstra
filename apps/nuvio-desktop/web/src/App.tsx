// App shell: sidebar + current screen. Screens stay mounted while hidden behind the full-window
// player, so returning keeps scroll position and loaded rows.

import { useCallback, useEffect, useState } from "react";
import { api, errorText, onDataEpoch, onProfilesChanged, reloadProfileData } from "./api";
import { PinDialog } from "./components/PinDialog";
import { ProfileMenu, ProfilesDialog } from "./components/ProfileMenu";
import { Sidebar, type Tab } from "./components/Sidebar";
import { UpdateBanner } from "./components/UpdateBanner";
import { FullPlayer } from "./player/FullPlayer";
import { usePlayer } from "./player/PlayerContext";
import { Details } from "./screens/Details";
import { Home } from "./screens/Home";
import { Library } from "./screens/Library";
import { resumeTarget } from "./library";
import { LiveTv } from "./screens/LiveTv";
import { Search } from "./screens/Search";
import { Settings } from "./screens/Settings";
import { metaRef, type Bootstrap, type HistoryEntry, type MetaPreview, type MetaRef, type ProfilesView } from "./types";


export function App() {
  const player = usePlayer();
  const [appMode, setAppMode] = useState<string>("");
  useEffect(() => {
    api.appMode().then(setAppMode, () => setAppMode("app"));
  }, []);
  const [boot, setBoot] = useState<Bootstrap | null>(null);
  const [bootError, setBootError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("home");
  const [details, setDetails] = useState<MetaRef | null>(null);
  const [openError, setOpenError] = useState<string | null>(null);
  const [profiles, setProfiles] = useState<ProfilesView | null>(null);
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
    api.profilesList().then(setProfiles, () => {});
  }, []);

  // Names, avatars or PIN changed by the account (another device).
  useEffect(() => onProfilesChanged(() => void api.profilesList().then(setProfiles, () => {})), []);

  useEffect(
    () =>
      onDataEpoch((reason) => {
        if (reason === "profile") {
          setProfileEpoch((e) => e + 1);
          setDetails(null);
          setOpenError(null);
          setBoot(null);
          api.profilesList().then(setProfiles, () => {});
        } else {
          setAddonsEpoch((e) => e + 1);
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
        setOpenError(`Could not switch profile: ${errorText(e)}`);
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
      setTab(next);
    },
    [player],
  );

  // Home and Search hand over the catalog item as the addon sent it; its type is checked here,
  // so a malformed item gives a clear message instead of a failed command.
  const open = useCallback((item: MetaPreview) => {
    const ref = metaRef(item);
    if (typeof ref === "string") {
      setOpenError(`Could not open “${item.name}”: ${ref}.`);
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
  const openEntry = useCallback((entry: HistoryEntry) => {
    setOpenError(null);
    setDetails({ contentType: entry.metaType, id: entry.metaId, name: entry.title });
  }, []);

  const full = player.session?.view === "full";
  const profile = profiles?.profiles.find((p) => p.index === profiles.active) ?? null;
  const dataKey = `${profileEpoch}-${addonsEpoch}`;

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
        {profiles?.locked && (
          <PinDialog
            profile={profiles.profiles.find((p) => p.index === profiles.active)!}
            onUnlocked={() => { setProfiles({ ...profiles, locked: false }); reloadProfileData(); }}
          />
        )}
        {manage && profiles && <ProfilesDialog view={profiles} onChanged={setProfiles} onClose={() => setManage(false)} />}
        {switching !== null && (
          <div className="switching" role="status" data-testid="profile-switching">Switching to {switching || "another profile"}…</div>
        )}
        <main className="content">
          <UpdateBanner automatic={appMode === "app"} />
          {bootError && <div className="banner error">Could not read the Nuvio Astra data: {bootError}</div>}
          {openError && <div className="banner error" role="alert">{openError}</div>}
          {boot && boot.addonsFailed > 0 && (
            <div className="banner">{boot.addonsFailed} of {boot.addonsEnabled} addon(s) did not respond; the rest are available.</div>
          )}
          <div className="screen" hidden={details !== null || tab !== "home"}><Home key={dataKey} onOpen={open} onResume={resume} /></div>
          {tab === "search" && <div className="screen" hidden={details !== null}><Search key={dataKey} onOpen={open} /></div>}
          {tab === "tv" && <div className="screen screen-fill"><LiveTv key={dataKey} /></div>}
          {tab === "library" && <div className="screen" hidden={details !== null}><Library key={profileEpoch} onOpen={open} onResume={resume} onOpenEntry={openEntry} /></div>}
          {tab === "settings" && <div className="screen screen-fill" hidden={details !== null}><Settings key={profileEpoch} /></div>}
          {details && (
            <div className="screen">
              <Details key={`${details.contentType}/${details.id}/${details.resume?.videoId ?? ""}`} item={details} onBack={() => setDetails(null)} />
            </div>
          )}
        </main>
      </div>
      {full && <FullPlayer />}
    </>
  );
}

// Settings of the active profile. Only options that NextGen applies for real are shown; each one
// is saved through Rust (`settings_set`, validated into the original's ranges) and read back.
// The original's other settings pages are listed under "Not available yet" (no controls).

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { AccountIcon, AddonsIcon, AudioIcon, HomeIcon, P2pIcon, PendingIcon, PlaybackIcon, ProfileDataIcon, SubtitlesIcon, UpdatesIcon } from "../components/NavIcons";
import { api, errorText } from "../api";
import { AccountPanel } from "../components/AccountPanel";
import { AddonsManager } from "../components/AddonsManager";
import { checkForUpdates, installUpdate, useUpdates } from "../updates";
import { Avatar } from "../components/ProfileMenu";
import { LANGUAGES } from "../languages";
import { P2P_CONSENT_TEXT, megabytes } from "../components/TorrentStart";
import type { CatalogChoice, P2pCacheName, P2pProfileName, P2pSettings, PlayerSettings, ProfileSettings, ResizeModeName, SettingsView, SubtitleSettings } from "../types";

type Section = "account" | "home" | "addons" | "playback" | "p2p" | "audio" | "subtitles" | "data" | "updates" | "pending";

const SECTIONS: { id: Section; label: string; icon: ReactNode }[] = [
  { id: "account", label: "Account", icon: <AccountIcon /> },
  { id: "home", label: "Home", icon: <HomeIcon /> },
  { id: "addons", label: "Addons", icon: <AddonsIcon /> },
  { id: "playback", label: "Playback", icon: <PlaybackIcon /> },
  { id: "p2p", label: "Torrent / P2P", icon: <P2pIcon /> },
  { id: "audio", label: "Audio", icon: <AudioIcon /> },
  { id: "subtitles", label: "Subtitles", icon: <SubtitlesIcon /> },
  { id: "data", label: "Profile and data", icon: <ProfileDataIcon /> },
  { id: "updates", label: "Updates", icon: <UpdatesIcon /> },
  { id: "pending", label: "Not available yet", icon: <PendingIcon /> },
];

type Status = { kind: "idle" } | { kind: "saving" } | { kind: "saved"; note?: string } | { kind: "error"; message: string };

// ---------- small controls ----------

function Row({ title, description, children }: { title: string; description?: string; children: ReactNode }) {
  return (
    <div className="setting-row">
      <div className="setting-text">
        <div className="setting-title">{title}</div>
        {description && <div className="muted small">{description}</div>}
      </div>
      <div className="setting-control">{children}</div>
    </div>
  );
}

function Toggle({ checked, onChange, label }: { checked: boolean; onChange(v: boolean): void; label: string }) {
  return (
    <button role="switch" aria-checked={checked} aria-label={label} className={`switch ${checked ? "on" : ""}`} onClick={() => onChange(!checked)}>
      <span className="knob" />
    </button>
  );
}

function Slider({ value, min, max, onChange, unit, label }: { value: number; min: number; max: number; onChange(v: number): void; unit?: string; label: string }) {
  return (
    <span className="slider">
      <input type="range" min={min} max={max} value={value} aria-label={label} onChange={(e) => onChange(Number(e.target.value))} />
      <output>{value}{unit}</output>
    </span>
  );
}

/** `#AARRGGBB` ⇄ color input (`#rrggbb`) + optional opacity. */
function ColorField({ value, onChange, withOpacity, label }: { value: string; onChange(v: string): void; withOpacity?: boolean; label: string }) {
  const alpha = value.slice(1, 3);
  const rgb = `#${value.slice(3, 9).toLowerCase()}`;
  const opacity = Math.round((parseInt(alpha, 16) / 255) * 100);
  const hex2 = (n: number) => Math.round(n).toString(16).padStart(2, "0").toUpperCase();
  return (
    <span className="color-field">
      <input type="color" value={rgb} aria-label={label} onChange={(e) => onChange(`#${alpha}${e.target.value.slice(1).toUpperCase()}`)} />
      {withOpacity && (
        <Slider label={`${label} — opacity`} value={opacity} min={0} max={100} unit="%" onChange={(o) => onChange(`#${hex2((o / 100) * 255)}${value.slice(3)}`)} />
      )}
    </span>
  );
}

function LanguageSelect({ value, onChange, special, label }: { value: string | null; onChange(v: string | null): void; special: [string, string][]; label: string }) {
  const known = LANGUAGES.some(([c]) => c.toLowerCase() === (value ?? "").toLowerCase()) || special.some(([c]) => c === (value ?? ""));
  return (
    <select aria-label={label} value={value ?? ""} onChange={(e) => onChange(e.target.value || null)}>
      {special.map(([code, name]) => <option key={code} value={code}>{name}</option>)}
      {!known && value && <option value={value}>{value}</option>}
      {LANGUAGES.map(([code, name]) => <option key={code} value={code}>{name}</option>)}
    </select>
  );
}

// ---------- page ----------

export function Settings() {
  const [view, setView] = useState<SettingsView | null>(null);
  const [draft, setDraft] = useState<ProfileSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [section, setSection] = useState<Section>("home");
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const timer = useRef<number>(0);
  const pending = useRef<{ next: ProfileSettings; note?: string } | null>(null);
  /** Latest settings, including changes not rendered yet (several quick changes in a row). */
  const draftRef = useRef<ProfileSettings | null>(null);

  useEffect(() => {
    api.settingsGet().then(
      (v) => {
        setView(v);
        draftRef.current = v.settings;
        setDraft(v.settings);
      },
      (e) => setError(errorText(e)),
    );
  }, []);

  const flush = useCallback(async () => {
    const job = pending.current;
    if (!job) return;
    pending.current = null;
    setStatus({ kind: "saving" });
    try {
      const saved = await api.settingsSet(job.next);
      // Rust may have corrected a value (ranges of the original): show what was stored, unless
      // the user changed something else meanwhile (that change is saved next).
      if (draftRef.current === job.next) {
        draftRef.current = saved;
        setDraft(saved);
      }
      if (!pending.current) setStatus({ kind: "saved", note: job.note });
    } catch (e) {
      setStatus({ kind: "error", message: errorText(e) });
    }
  }, []);

  // Leaving the page (or the app tab) never drops a change still waiting for its debounce.
  useEffect(
    () => () => {
      window.clearTimeout(timer.current);
      void flush();
    },
    [flush],
  );

  /** Applies a change locally and saves it (debounced for sliders and typing). */
  const update = useCallback(
    (change: (s: ProfileSettings) => ProfileSettings, note?: string, delay = 300) => {
      const current = pending.current?.next ?? draftRef.current;
      if (!current) return;
      const next = change(current);
      draftRef.current = next;
      pending.current = { next, note };
      setDraft(next);
      setStatus({ kind: "saving" });
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => void flush(), delay);
    },
    [flush],
  );

  const player = (change: (p: PlayerSettings) => PlayerSettings) =>
    update((s) => ({ ...s, player: change(s.player) }), "Applies from the next playback.");
  const subtitle = (change: (p: SubtitleSettings) => SubtitleSettings) => player((p) => ({ ...p, subtitle: change(p.subtitle) }));

  const resetSection = (what: Section) => {
    if (!view || !draft) return;
    const name = SECTIONS.find((s) => s.id === what)?.label ?? "";
    if (!window.confirm(`Restore the defaults of “${name}” for profile ${view.profile.name}? Changes in this section will be discarded.`)) return;
    const d = view.defaults;
    update((s) => {
      switch (what) {
        case "home": return { ...s, home: { ...d.home } };
        case "playback": return { ...s, player: { ...s.player, resizeMode: d.player.resizeMode, decoderPriority: d.player.decoderPriority, nvidiaRtxSuperResolution: d.player.nvidiaRtxSuperResolution, nvidiaRtxHdr: d.player.nvidiaRtxHdr } };
        case "audio": return { ...s, player: { ...s.player, preferredAudioLanguage: d.player.preferredAudioLanguage, secondaryPreferredAudioLanguage: d.player.secondaryPreferredAudioLanguage } };
        case "subtitles": return { ...s, player: { ...s.player, preferredSubtitleLanguage: d.player.preferredSubtitleLanguage, secondaryPreferredSubtitleLanguage: d.player.secondaryPreferredSubtitleLanguage, subtitle: { ...d.player.subtitle } } };
        default: return s;
      }
    }, "Defaults restored.", 0);
  };

  if (error) return <div className="screen-message">Could not open settings: {error}</div>;
  if (!view || !draft) return <div className="screen-message">Loading settings…</div>;
  const p = draft.player;
  const sub = p.subtitle;

  return (
    <div className="settings">
      <nav className="settings-nav" aria-label="Settings categories">
        <div className="settings-profile">
          <Avatar profile={view.profile} />
          <div>
            <div className="setting-title">{view.profile.name}</div>
            <div className="muted small">Settings for this profile</div>
          </div>
        </div>
        {SECTIONS.map((s) => (
          <button key={s.id} className={`nav-item ${section === s.id ? "active" : ""}`} data-section={s.id} onClick={() => setSection(s.id)}>
            <span className="nav-icon" aria-hidden>{s.icon}</span>
            <span>{s.label}</span>
          </button>
        ))}
      </nav>

      <div className="settings-body" key={section}>
        <header className="settings-header">
          <h1>{SECTIONS.find((s) => s.id === section)?.label}</h1>
          <span className={`save-status ${status.kind}`} role="status" data-testid="save-status">
            {status.kind === "saving" && "Saving…"}
            {status.kind === "saved" && `✓ Saved${status.note ? ` · ${status.note}` : ""}`}
            {status.kind === "error" && `Could not save: ${status.message}`}
          </span>
          {["home", "playback", "audio", "subtitles"].includes(section) && (
            <button className="ghost" onClick={() => resetSection(section)}>Restore defaults</button>
          )}
        </header>

        {section === "home" && (
          <HomeSection settings={draft} catalogs={view.catalogs} update={update} />
        )}

        {section === "account" && <AccountPanel />}
        {section === "addons" && <AddonsManager />}

        {section === "playback" && (
          <div className="settings-group">
            <Row title="Default picture mode" description="How the video fills the screen on start (can be changed during playback).">
              <select aria-label="Default picture mode" data-setting="resizeMode" value={p.resizeMode} onChange={(e) => player((x) => ({ ...x, resizeMode: e.target.value as ResizeModeName }))}>
                <option value="Fit">Fit</option>
                <option value="Fill">Fill</option>
                <option value="Zoom">Zoom</option>
                <option value="Stretch">Stretch</option>
              </select>
            </Row>
            <Row title="Video decoding" description="Decoder priority (libmpv).">
              <select aria-label="Video decoding" data-setting="decoderPriority" value={p.decoderPriority} onChange={(e) => player((x) => ({ ...x, decoderPriority: Number(e.target.value) }))}>
                <option value={1}>Hardware, with software as fallback (default)</option>
                <option value={0}>Hardware only</option>
                <option value={2}>Software only</option>
              </select>
            </Row>
            <Row title="NVIDIA RTX Video Super Resolution" description="Requires an NVIDIA RTX card and a compatible driver; no effect on other cards.">
              <Toggle label="NVIDIA RTX Video Super Resolution" checked={p.nvidiaRtxSuperResolution} onChange={(v) => player((x) => ({ ...x, nvidiaRtxSuperResolution: v }))} />
            </Row>
            <Row title="NVIDIA RTX Video HDR" description="Converts SDR to HDR on HDR displays with an RTX card.">
              <Toggle label="NVIDIA RTX Video HDR" checked={p.nvidiaRtxHdr} onChange={(v) => player((x) => ({ ...x, nvidiaRtxHdr: v }))} />
            </Row>
          </div>
        )}

        {section === "p2p" && <P2pSection settings={draft.p2p} update={(change) => update((s) => ({ ...s, p2p: change(s.p2p) }), "Applies from the next torrent source.", 0)} />}

        {section === "audio" && (
          <div className="settings-group">
            <Row title="Preferred audio language" description={`“Device language” = ${view.deviceLanguages.join(", ") || "not detected"} on this PC.`}>
              <LanguageSelect label="Preferred audio language" value={p.preferredAudioLanguage} special={[["device", "Device language"], ["default", "File default"], ["original", "Original language"]]} onChange={(v) => player((x) => ({ ...x, preferredAudioLanguage: v ?? "device" }))} />
            </Row>
            <Row title="Secondary audio language" description="Used when the preferred one is not in the file.">
              <LanguageSelect label="Secondary audio language" value={p.secondaryPreferredAudioLanguage} special={[["", "None"]]} onChange={(v) => player((x) => ({ ...x, secondaryPreferredAudioLanguage: v }))} />
            </Row>
            <p className="muted small pad-top">“Original language” uses the device language while Nuvio Astra does not know the title's original language (same rule as the original Nuvio when that information is missing).</p>
          </div>
        )}

        {section === "subtitles" && (
          <div className="settings-group">
            <Row title="Preferred subtitle language">
              <LanguageSelect label="Preferred subtitle language" value={p.preferredSubtitleLanguage} special={[["none", "None (forced only)"], ["device", "Device language"], ["forced", "Forced only"]]} onChange={(v) => player((x) => ({ ...x, preferredSubtitleLanguage: v ?? "none" }))} />
            </Row>
            <Row title="Secondary subtitle language">
              <LanguageSelect label="Secondary subtitle language" value={p.secondaryPreferredSubtitleLanguage} special={[["", "None"]]} onChange={(v) => player((x) => ({ ...x, secondaryPreferredSubtitleLanguage: v }))} />
            </Row>
            <div className="subtitle-preview" aria-hidden>
              <span
                style={{
                  color: `#${sub.textColor.slice(3)}`,
                  opacity: parseInt(sub.textColor.slice(1, 3), 16) / 255,
                  fontSize: `${Math.max(11, sub.fontSizeSp)}px`,
                  fontWeight: sub.bold ? 700 : 400,
                  background: `rgba(${parseInt(sub.backgroundColor.slice(3, 5), 16)},${parseInt(sub.backgroundColor.slice(5, 7), 16)},${parseInt(sub.backgroundColor.slice(7, 9), 16)},${parseInt(sub.backgroundColor.slice(1, 3), 16) / 255})`,
                  WebkitTextStroke: sub.outlineEnabled ? `${sub.outlineWidth / 2}px #${sub.outlineColor.slice(3)}` : undefined,
                  bottom: `${8 + sub.bottomOffset / 5}px`,
                }}
              >
                Subtitle example
              </span>
            </div>
            <Row title="Size"><Slider label="Subtitle size" value={sub.fontSizeSp} min={6} max={40} onChange={(v) => subtitle((s) => ({ ...s, fontSizeSp: v }))} /></Row>
            <Row title="Distance from bottom edge"><Slider label="Distance from bottom edge" value={sub.bottomOffset} min={0} max={200} onChange={(v) => subtitle((s) => ({ ...s, bottomOffset: v }))} /></Row>
            <Row title="Text color"><ColorField label="Text color" value={sub.textColor} onChange={(v) => subtitle((s) => ({ ...s, textColor: v }))} /></Row>
            <Row title="Bold"><Toggle label="Bold" checked={sub.bold} onChange={(v) => subtitle((s) => ({ ...s, bold: v }))} /></Row>
            <Row title="Outline"><Toggle label="Outline" checked={sub.outlineEnabled} onChange={(v) => subtitle((s) => ({ ...s, outlineEnabled: v }))} /></Row>
            {sub.outlineEnabled && (
              <>
                <Row title="Outline color"><ColorField label="Outline color" value={sub.outlineColor} onChange={(v) => subtitle((s) => ({ ...s, outlineColor: v }))} /></Row>
                <Row title="Outline thickness"><Slider label="Outline thickness" value={sub.outlineWidth} min={1} max={5} onChange={(v) => subtitle((s) => ({ ...s, outlineWidth: v }))} /></Row>
              </>
            )}
            <Row title="Background"><ColorField label="Background color" withOpacity value={sub.backgroundColor} onChange={(v) => subtitle((s) => ({ ...s, backgroundColor: v }))} /></Row>
            <Row title="Remove descriptions for the deaf (SDH)" description="Hides cues such as [music] and (laughter).">
              <Toggle label="Remove SDH" checked={sub.stripSdh} onChange={(v) => subtitle((s) => ({ ...s, stripSdh: v }))} />
            </Row>
          </div>
        )}

        {section === "data" && (
          <div className="settings-group">
            <div className="about" data-testid="about">
              <img src="./nuvio-astra-128.png" alt="" width={56} height={56} draggable={false} />
              <div>
                <div className="about-name">Nuvio Astra</div>
                <div className="muted small">Version {view.appVersion} · Windows x64 · fork of Nuvio Desktop, compatible with your addons and data</div>
              </div>
            </div>
            <Row title="Active profile" description="Each profile has its own settings, as in the original Nuvio.">
              <span>{view.profile.name}</span>
            </Row>
            <Row title="Nuvio Astra data folder" description="Atomic writes with a .bak copy for recovery. The technical folder name was kept to preserve existing data.">
              <code className="path">{view.dataDir}</code>
            </Row>
            {view.migration && (
              <>
                <Row title="Migration from the original Nuvio" description="The original files are not modified.">
                  <span>
                    version {view.migration.version} · {view.migration.backupFiles} files backed up · {view.migration.profiles} profiles · settings of {view.migration.settingsProfiles ?? 0} profile(s)
                  </span>
                </Row>
                <Row title="Verified backup"><code className="path">{view.migration.backupDir}</code></Row>
              </>
            )}
          </div>
        )}

        {section === "updates" && <UpdatesSection />}

        {section === "pending" && (
          <div className="settings-group">
            <p className="muted">Features of the original Nuvio that Nuvio Astra does not have yet. They will appear in Settings when added — no option is shown unless it works.</p>
            <ul className="pending-list">
              <li><b>Details screen</b> — choose which sections are shown, similar titles.</li>
              <li><b>Continue watching</b> — style, sorting and click action (the row itself already works).</li>
              <li><b>Sources and autoplay</b> — autoplay, regex filters, automatic source switching, reusing the last link.</li>
              <li><b>Next episode and skip intro/credits</b> — automatic next episode, AnimeSkip, IntroDB.</li>
              <li><b>Subtitles</b> — addon subtitles and automatic synchronization.</li>
              <li><b>Player buffer and cache</b>, <b>advanced audio</b> (amplification, downmix).</li>
              <li><b>Debrid</b> — Real-Debrid, Premiumize, TorBox.</li>
              <li><b>Integrations</b> — Trakt, Simkl, MDBList, TMDB; <b>media servers</b> (Plex, Jellyfin/Emby, Silo).</li>
              <li><b>Plugins</b>, member-exclusive avatars, Watch Party and signing in with another device (QR).</li>
              <li>Syncing player preferences with other devices, provider credentials, collections; Discord Rich Presence, app language.</li>
            </ul>
          </div>
        )}
      </div>
    </div>
  );
}

// ---------- Atualizações ----------

function UpdatesSection() {
  const u = useUpdates();
  const s = u.status;
  const when = u.lastCheck ? new Date(u.lastCheck).toLocaleString("en-US", { hour: "2-digit", minute: "2-digit", day: "2-digit", month: "2-digit" }) : null;
  return (
    <div className="settings-group" data-testid="updates-settings">
      <Row title="Installed version" description={s?.repository ? `Updates published at github.com/${s.repository}` : undefined}>
        <span>{s?.currentVersion ?? "…"}</span>
      </Row>
      {s && !s.configured && <p className="muted">This build has no automatic updates configured (local build). Versions published on GitHub do.</p>}
      {s?.configured && (
        <>
          <Row title="Check for updates" description="Nuvio Astra also checks on its own at launch and every 6 hours. Every update is verified by its signature before it is installed.">
            <button className="ghost" disabled={u.checking || !!u.installing} onClick={() => void checkForUpdates()} data-testid="update-check">
              {u.checking ? "Checking…" : "Check now"}
            </button>
          </Row>
          {u.checkError && <div className="row-error">{u.checkError}</div>}
          {!u.checking && !u.checkError && u.lastCheck && !u.available && <div className="addon-feedback ok" role="status">You are on the latest version (checked at {when}).</div>}
          {u.available && (
            <div className="update-card">
              <div className="setting-title">New version {u.available.version}</div>
              {u.available.notes && <pre className="update-notes">{u.available.notes}</pre>}
              <div className="account-actions">
                <button className="primary" disabled={!!u.installing && u.installing.phase !== "error"} onClick={() => void installUpdate()}>
                  {u.installing && u.installing.phase !== "error" ? "Updating…" : "Update now"}
                </button>
              </div>
              {u.installing?.phase === "error" && <div className="row-error">{u.installing.error}</div>}
            </div>
          )}
          <p className="muted small">Updating keeps profiles, addons, settings, library and the account session: the installer replaces only the program files.</p>
        </>
      )}
    </div>
  );
}

// ---------- Torrent / P2P ----------

function P2pSection({ settings, update }: { settings: P2pSettings; update(change: (s: P2pSettings) => P2pSettings): void }) {
  const [cache, setCache] = useState<number | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    api.p2pCacheSize().then(setCache, () => setCache(null));
  }, []);

  const enable = (on: boolean) => {
    if (on && !window.confirm(P2P_CONSENT_TEXT.join("\n\n"))) return;
    update((s) => ({ ...s, enabled: on }));
  };
  const clear = async () => {
    setBusy(true);
    setMessage(null);
    try {
      const freed = await api.p2pClearCache();
      setMessage(`Cache cleared: ${megabytes(freed)} freed.`);
      setCache(await api.p2pCacheSize());
    } catch (e) {
      setMessage(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="settings-group" data-testid="p2p-settings">
      <Row title="P2P streaming" description="Plays addon torrent sources in the built-in player, downloading in pieces while you watch.">
        <Toggle label="P2P streaming" checked={settings.enabled} onChange={enable} />
      </Row>
      <Row title="Upload data" description="Shares the pieces already downloaded with other peers while the video plays.">
        <Toggle label="Upload data" checked={settings.enableUpload} onChange={(v) => update((s) => ({ ...s, enableUpload: v }))} />
      </Row>
      <Row title="Hide torrent stats" description="Peers, speed and progress over the player.">
        <Toggle label="Hide stats" checked={settings.hideTorrentStats} onChange={(v) => update((s) => ({ ...s, hideTorrentStats: v }))} />
      </Row>
      <Row title="Connection profile" description="How many connections and requests the engine opens at the same time.">
        <select aria-label="Connection profile" value={settings.torrentProfile} onChange={(e) => update((s) => ({ ...s, torrentProfile: e.target.value as P2pProfileName }))}>
          <option value="SOFT">Light</option>
          <option value="BALANCED">Balanced (default)</option>
          <option value="FAST">Fast</option>
        </select>
      </Row>
      <Row title="Cache size" description="Disk space for pieces already downloaded (reused when watching again).">
        <select aria-label="Cache size" value={settings.cacheSize} onChange={(e) => update((s) => ({ ...s, cacheSize: e.target.value as P2pCacheName }))}>
          <option value="NONE">No cache</option>
          <option value="GB_2">2 GB (default)</option>
          <option value="GB_5">5 GB</option>
          <option value="GB_10">10 GB</option>
        </select>
      </Row>
      <Row title="Cache in use" description="Cannot be cleared during P2P playback.">
        <span className="account-actions" style={{ marginTop: 0 }}>
          <span>{cache === null ? "—" : megabytes(cache)}</span>
          <button className="ghost" disabled={busy} onClick={() => void clear()}>{busy ? "Clearing…" : "Clear cache"}</button>
        </span>
      </Row>
      {message && <div className="addon-feedback ok" role="status">{message}</div>}
    </div>
  );
}

// ---------- Início ----------

function HomeSection({ settings, catalogs, update }: {
  settings: ProfileSettings;
  catalogs: CatalogChoice[];
  update(change: (s: ProfileSettings) => ProfileSettings, note?: string, delay?: number): void;
}) {
  const note = "The Home screen is updated.";
  const home = settings.home;
  // Current order: saved preferences first (by order), then catalogs without one.
  const ordered = [...catalogs].sort((a, b) => {
    const pa = home.items.find((i) => i.key === a.key)?.order ?? Number.MAX_SAFE_INTEGER;
    const pb = home.items.find((i) => i.key === b.key)?.order ?? Number.MAX_SAFE_INTEGER;
    return pa - pb || catalogs.indexOf(a) - catalogs.indexOf(b);
  });
  const pref = (c: CatalogChoice) => home.items.find((i) => i.key === c.key);

  /** Rewrites the preference list for every catalog, in `order`, keeping hero-source flags. */
  const saveCatalogs = (order: CatalogChoice[], patch?: { key: string; enabled?: boolean; customTitle?: string }, delay?: number) =>
    update(
      (s) => ({
        ...s,
        home: {
          ...s.home,
          items: order.map((c, index) => {
            const old = s.home.items.find((i) => i.key === c.key);
            const base = { key: c.key, customTitle: old?.customTitle ?? c.customTitle, enabled: old?.enabled ?? c.enabled, heroSourceEnabled: old?.heroSourceEnabled ?? true, order: index };
            return patch && patch.key === c.key ? { ...base, ...(patch.enabled !== undefined && { enabled: patch.enabled }), ...(patch.customTitle !== undefined && { customTitle: patch.customTitle }) } : base;
          }),
        },
      }),
      note,
      delay,
    );

  const move = (index: number, dir: -1 | 1) => {
    const next = [...ordered];
    const [item] = next.splice(index, 1);
    next.splice(index + dir, 0, item);
    saveCatalogs(next, undefined, 0);
  };

  return (
    <>
      <div className="settings-group">
        <Row title="Featured at top" description="Shows the featured title at the top of the screen.">
          <Toggle label="Featured at top" checked={home.heroEnabled} onChange={(v) => update((s) => ({ ...s, home: { ...s.home, heroEnabled: v } }), note)} />
        </Row>
        <Row title="Show type in catalog name" description="E.g. “Trending - Movies”.">
          <Toggle label="Show type in catalog name" checked={home.showCatalogType} onChange={(v) => update((s) => ({ ...s, home: { ...s.home, showCatalogType: v } }), note)} />
        </Row>
        <Row title="Hide unreleased content" description="Removes titles with a future release date from catalogs.">
          <Toggle label="Hide unreleased content" checked={home.hideUnreleasedContent} onChange={(v) => update((s) => ({ ...s, home: { ...s.home, hideUnreleasedContent: v } }), note)} />
        </Row>
      </div>
      <h2 className="settings-subtitle">Catalogs ({ordered.filter((c) => pref(c)?.enabled ?? c.enabled).length} of {ordered.length} visible)</h2>
      <div className="settings-group catalogs">
        {ordered.map((c, index) => {
          const enabled = pref(c)?.enabled ?? c.enabled;
          const title = pref(c)?.customTitle ?? c.customTitle;
          return (
            <div key={c.key} className={`catalog-pref ${enabled ? "" : "off"}`} data-key={c.key}>
              <Toggle label={`Show ${c.defaultTitle}`} checked={enabled} onChange={(v) => saveCatalogs(ordered, { key: c.key, enabled: v }, 0)} />
              <div className="catalog-pref-text">
                <input
                  className="catalog-title-input"
                  value={title}
                  placeholder={c.defaultTitle}
                  aria-label={`Name of ${c.defaultTitle}`}
                  onChange={(e) => saveCatalogs(ordered, { key: c.key, customTitle: e.target.value }, 600)}
                />
                <span className="muted small">{c.addonName}</span>
              </div>
              <button className="icon-btn" aria-label={`Move up ${c.defaultTitle}`} disabled={index === 0} onClick={() => move(index, -1)}>↑</button>
              <button className="icon-btn" aria-label={`Move down ${c.defaultTitle}`} disabled={index === ordered.length - 1} onClick={() => move(index, 1)}>↓</button>
            </div>
          );
        })}
      </div>
    </>
  );
}

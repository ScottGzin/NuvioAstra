# Changelog

From the first public release on, every push to `main` publishes a new version
(the last number goes up to 9, then the middle one advances: `0.2.9` → `0.3.0`); its notes are the commit messages since the previous release and appear in the
app's update notice and on the GitHub Release page.

## 0.1.0 — first version (before GitHub releases)

- **Home, search, details** with seasons and episodes, cast, metadata from the installed Stremio
  addons; **live TV** with preview and channel list.
- **Library**: favorites, history, Continue watching, resume position, per profile.
- **Addons and profiles**: install, configure, enable/disable, reorder, remove; up to 6 profiles
  with colors, avatars and PIN.
- **Nuvio account**: e-mail and password sign-in, encrypted session (Windows DPAPI), cloud sync of
  profiles, addons, favorites, progress and watched marks, offline queue, first-sync preview that
  writes nothing until confirmed.
- **Player**: embedded libmpv (hardware decoding, audio/subtitle tracks and styles, delays, speed,
  picture-in-picture, resize modes), cyan controls with buffered range on the timeline.
- **Torrent / P2P** sources played in the app's player with the Nuvio Engine (libtorrent): playback
  before the download ends, seeking, stats, per-profile settings, consent notice, cache control.
- **YouTube trailers** on movie and series details (resolved only on click, never saved to history).
- **Migration** from Nuvio Desktop's data with verified backups (originals never modified).
- **Windows installer** (per user, no administrator rights) and portable folder.
- **Automatic updates** (official Tauri updater): signed packages, notice at the top with the
  version and its notes, "Atualizar agora" / "Depois", download progress, manual check in
  Settings → Atualizações. Data, profiles, addons, settings and the account session are kept.

Postponed: next episode, skip intro/credits, automatic source fallback, subtitle auto-sync, debrid,
`.torrent` file sources, YouTube addon streams.

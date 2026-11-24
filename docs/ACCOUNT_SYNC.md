# Nuvio account, PIN, avatars and cloud sync (stage 12)

Nuvio Astra signs in to the **same Nuvio account** as the original app and talks to the same backend
(Supabase: GoTrue for auth, PostgREST RPCs for data). The contracts were taken from the original's
Kotlin sources (`composeApp/src/commonMain/.../features/{auth,sync,profiles,library,watchprogress,
watched,addons}`); nothing was invented on the server side and no table or function was added.

## Architecture

```
React (first-open account screen, lock screen, Settings → Conta, sidebar dot, avatar picker)
   │  IPC: account_status / sign_in / sign_up / sign_out / sync_now, avatars_catalog,
   │       profile_unlock / profile_pin_set / profile_pin_clear      (no token ever crosses)
   ▼
apps/nuvio-desktop/src/account.rs   automatic sync loop (see Triggers), events
   │                                 `sync-status`, `sync-applied`, `library-changed`; PIN gate
   ▼
crates/nuvio-sync                   Engine: session, refresh, sync steps, PIN, avatars
   ├─ remote.rs   HTTP to Supabase (anon key + user JWT), fallback URL, pt-BR errors
   └─ crypto.rs   DPAPI protect/unprotect, SHA-256 and random bytes (Windows BCrypt)
   ▼
crates/nuvio-store::sync_state      pending changes (offline queue), cursors, merge rules
   ▼
%APPDATA%\NuvioNextGen              local data stays the source the UI reads
```

- The local store is always what the screens read; sync only merges into it. Without an account,
  or offline, everything works as before and changes are queued.
- Every local write (favorite, progress, watched, history removal, addons, profiles) records a
  `Change` with a revision number in `sync-state.json`. A push sends the snapshot of pending keys;
  only the keys whose revision did not change meanwhile are cleared (`clear_sent`), so an edit made
  during a push is sent again — no change is lost and resending is idempotent (upserts by key).

## Server configuration and secrets

- `NUVIO_SUPABASE_URL`, `NUVIO_SUPABASE_ANON_KEY`, optional `NUVIO_SUPABASE_FALLBACK_URL` are read at
  build time from the environment or from the repository root `local.properties` (git-ignored), as in
  the original. `build.rs` refuses a key whose JWT role is not `anon` (service/admin keys can never be
  embedded). Without them the build works and the account shows "Conta indisponível nesta versão".
- Only https URLs are accepted (http only for `127.0.0.1`, used by the tests' mock server).
- The anon key is public by design (row-level security on the server); all user data access uses the
  user's own access token.

## Session security

- The session (access token, refresh token, expiry, user id, e-mail) lives only in Rust and is saved
  in `account.dat` encrypted with **DPAPI** (`CryptProtectData`, current Windows user, extra entropy
  `NuvioAstra.Account.v1`). Another Windows user or another machine cannot decrypt it; a damaged file
  means "signed out".
- The frontend only receives `AccountStatus` (configured, signed in, e-mail, state, pending count,
  last sync, last error, last report). No token in the DOM, `localStorage`, IPC or logs (checked by the
  UI test). The password is sent once to GoTrue and never stored.
- Refresh: before a call when the token is near expiry, and once after a 401. A refused refresh ends
  in the "Sessão expirada" state (data and pending changes kept; signing in again resumes).
- Sign-out calls `/auth/v1/logout?scope=local` (5 s limit), deletes `account.dat`, keeps local data
  and the queue; a sync running at that moment is discarded (generation counter).

## Login methods

| Method | Original | Astra |
|---|---|---|
| E-mail + password | ✅ | ✅ `grant_type=password` |
| Create account | ✅ | ✅ `/auth/v1/signup` (when the server requires e-mail confirmation, the UI says so) |
| Anonymous / guest | ✅ (anonymous sign-in) | ⏳ local mode without account covers the use case |
| TV/device link (QR) | ✅ | ⏳ |

## Data synchronized

| Entity | Local source | Remote (RPC) | Identifier | Push | Pull | Conflict rule (original) |
|---|---|---|---|---|---|---|
| Profiles (name, color, avatar, uses primary addons) | `profiles.json` | `sync_pull_profiles`, `sync_push_profiles {p_client_max_profiles: 6, p_profiles, p_origin_client_id}`, `sync_delete_profile_data` | profile index 1–6 | whole list when dirty | every sync | the list is received first; a local change waiting survives it, then is pushed |
| Addons per profile | `addons-<p>.json` | `sync_push_addons {p_addons}`, table `addons?profile_id=eq.N` | manifest URL | whole list when dirty | every sync (active profile) | same as profiles; enabled state + order kept; new manifests fetched by URL |
| Saved items (library) | `library-<p>.json` | `sync_push_library_items`, `sync_delete_library_items`, `sync_pull_library`, `sync_pull_library_delta`, `sync_get_library_delta_cursor` | `content_id` + `content_type` | pending keys | snapshot first time, then delta by event id | snapshot replaces local except keys with pending local upsert/delete; deltas applied in order skipping pending keys |
| Watch progress | `progress-<p>.json` | `sync_*_watch_progress*` | `<id>` / `<id>_s<S>e<E>` | pending keys, on player flush (pause, seek, end, close) | snapshot then delta | remote wins unless the local entry is pending **and** newer; a delta delete never removes a pending entry |
| Watched marks | `watched-<p>.json` | `sync_*_watched_items*` (pages from 1) | `content_id` + season + episode | pending keys | snapshot then delta | same as library |
| Collections | `collections-<p>.json` | `sync_pull_collections` | collection id | — (no editor) | every sync (active profile) | account copy |
| Home layout (order, visibility, titles, 2 switches) | `settings.json` + `sync-base-home-<p>.json` | `sync_pull/push_home_catalog_settings` (`home_catalog_shared`) | `<addon>:<type>:<catalog>`, `collection_<id>` | when changed | every sync | three-way per row; the order is one value; unknown fields and `server:` rows kept |
| Player, audio and subtitle preferences | `settings.json` + `sync-base-player-<p>.json` | `sync_pull/push_profile_settings_blob` (`desktop`) | field | changed fields patched into the blob just read | every sync | three-way per field |
| PIN | server only (+ offline hash) | `verify_profile_pin`, `set_profile_pin`, `clear_profile_pin` | profile index | immediate | with profiles (`pin_enabled`, `updated_at`) | server is the authority |
| Avatars | `avatars.json` cache | `get_avatar_catalog`, image `storage/v1/object/public/avatars/<path>` | avatar id | with profiles | on demand | — |

The settings blob holds every preference of Nuvio Desktop and a desktop replaces its own with it, so
Astra only patches the fields it has into the blob it just read, and never creates one: without a
blob on the account these preferences stay local. The first time, the account's preferences win and
what was on the computer is kept in `settings-before-sync.json`.

The blob also carries the poster card format (`poster_card_style_settings_payload`) and the
interface language (`theme_settings.selected_app_language`: `pt-BR`, `en` or `ar`). The original
keeps that language on the device and drops the key when it pushes its theme, so a blob without it
leaves the language of this computer as it is, and "follow the computer" is never sent.

Not synchronized: Astra-only options (motion, image improvement, P2P), hero choices (the original's payload has
no field for them; Astra follows the ones of the Nuvio Desktop on the same computer, read only) and live TV
favorites (local in the original too), provider/debrid credentials, media servers, plugins, watch
party, member avatars, and every other feature of the settings blob (kept untouched).

## Sync flow

1. Profile deletions queued locally (`sync_delete_profile_data`).
2. Pull profiles, apply, then push local profile changes.
3. Per profile (the active one; **every** profile on the first run after opening and on the first
   link): pull and merge addons, library, progress, watched, collections, Home and preferences, then
   push what waits here.
4. Send what changed in the other profiles.

Nothing waiting is sent alone (`push_pending`) before a run received the account's data.

- **Initial sync**: snapshot pull (pages of 500), the delta cursor is stored afterwards.
- **Incremental**: `*_delta {p_since_event_id}`; the cursor moves to the largest event id applied.
- **Per profile**: every RPC carries `p_profile_id`; queues and cursors are per profile; data of one
  profile never goes to another. Profile 1 with `uses_primary_addons` sharing follows the original.
- **Stale answers**: each run carries the session generation; a run started before a sign-out or
  account change is dropped, and a merge never overwrites a key changed locally after the request.
- **Failures**: each step is independent; failed steps are reported (`last_report`) and retried on the
  next run; offline aborts the run and the state shows "Sem conexão".
- **Triggers** (all automatic, there is no manual sync): as soon as the UI is ready, sign-in, profile switch and
  PIN unlock; 0.8 s after a local change; window focused again (at most every 10 s); every 5 min
  while focused, 15 min otherwise; after a failure in 30 s, 1 min, 2 min… up to 15 min.
- **Playback position**: each saved position (every minute, pause, seek, end, close) is pushed alone
  (`Engine::push_pending`), without pulling.
- **Closing**: with changes waiting the window hides and the app sends them for up to 5 s; what is
  left goes on the next start.
- **Continue watching**: the account keeps ids and positions only; titles and artwork are resolved
  with the profile's metadata addons after each sync (`services/enrich.rs`).

### First link of an account (Astra decision)

Astra may already have local data (migrated from the original, or used without an account):

- A computer never linked sees the account screen once at opening ("Continuar sem conta" skips it).
  A linked install never sees it.
- Signing in starts the first sync at once (`account_connect`), after a verified copy of every local
  document. Saved items, progress and watched marks of both sides are kept; the account's Home order
  and preferences win. Deletions made before the link never delete account data.
- Data that came from **another** account: nothing is written, locally or remotely, until the user
  confirms.
- The session of Nuvio Desktop is never read or copied. Profiles copied from it are not a sign-in.

### PIN lock at opening

- Nothing of a locked profile is drawn. Only the PIN's state is read from the account, so a PIN
  removed in Nuvio lifts the lock (checked every 15 s, on focus and on demand).
- The PIN is checked by the account only with a session of the account the profiles came from;
  otherwise by the offline hash (Astra's, or the one Nuvio Desktop keeps for a profile copied from it).
- Profiles copied and never linked, signed out: a PIN removed in the Nuvio of the same computer is
  followed too.
- The lock screen can sign in or change profile.

`sync-log.json` keeps the last 50 previews, backups and runs with counts, entity names and error
texts only (no token, e-mail, title or address); `tools/migration-audit/audit-sync.ps1` reads it with
the sync state from outside the app package.

## PIN

- 4 digits, validated by the server (`verify_profile_pin` → `unlocked`, `retry_after_seconds`); the
  server's lockout is shown ("tente novamente em N s").
- Setting/changing/removing needs the account (and the current PIN to change/remove), as in the original.
- Offline: after a successful online check the app keeps `salt + SHA-256("profile:<i>:<salt>:<pin>")`
  in `pin-cache.json`; it unlocks offline only with that hash, and the cache is dropped when the
  profile's `updated_at` changes. The PIN is never stored in clear and is not the account password.
- Enforcement is in Rust: switching to a protected profile, and every command that reads the active
  profile's data while it is locked (`ensure_unlocked`), fail until the PIN is accepted. A protected
  active profile is locked at every start.

## UI

- Settings → **Conta**: sign in / create account, e-mail, status, last sync time, pending changes,
  the first-sync confirmation, last report, sign out.
- Sidebar: dot on Configurações — green synced, blue syncing, amber pending/confirmation, grey
  offline/signed out, red error/expired (with screen-reader text).
- Profile editor: avatar grid from the account catalog (by category), PIN section.

## Tests

- `crates/nuvio-sync/tests/contract.rs` against an in-memory Supabase (`tests/mock`, std TCP server
  implementing every route used, with failure switches): auth + DPAPI file, refresh + expiry, first
  sync join then incremental, deletions not resurrected, progress conflicts + offline queue, failed
  steps + idempotent retry, different-account confirmation + pre-link deletions, PIN online/offline/
  lockout + avatars, first sync only after the reviewed read-only preview (exact counts, backup, all
  profiles, newer local progress kept, addon names kept, identical comparison afterwards). 9/9.
- `nuvio-store::sync_state` unit tests (4), `crypto` tests, `manage.rs` IPC test with the PIN gate.
- UI phases `account-ui` (11) and `account-verify` (3) on a copy of the data, signed out; the only
  network access is the public avatar catalog. The real account is never used by tests.

## Limitations

- Not tested against the user's real account (by rule: no destructive operation on it). The contracts
  are the original's and the anon configuration was verified, but a first real sign-in is the first
  end-to-end run; it is non-destructive by design (join).
- Anonymous sign-in, TV/QR device link, password reset screen, Trakt/Simkl, collection editor,
  watch party: not implemented.
- While the app is open, non-active profiles pull when they become active (all of them at each opening).
- No realtime channel: other devices' changes arrive when the app is focused again, or within 5 min.

//! Account and sync contract tests against an in-memory Supabase (tests/mock). No real account.

mod mock;

use mock::{EMAIL, PASSWORD, USER};
use nuvio_store::library::{LibraryItem, PlaybackItem};
use nuvio_store::nextgen::AppStore;
use nuvio_sync::{AccountError, Engine, RemoteError, ServerConfig};
use serde_json::{json, Value};
use std::path::PathBuf;

fn anon_key() -> String {
    // header {"alg":"HS256"} . payload {"role":"anon"} . signature (not checked by the client)
    "eyJhbGciOiJIUzI1NiJ9.eyJyb2xlIjoiYW5vbiJ9.sig".to_string()
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("nuvio-sync-contract-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn engine(base: &str, d: &PathBuf, owner: Option<&str>) -> Engine {
    Engine::new(d.clone(), ServerConfig::new(base, None, &anon_key()), owner.map(str::to_string))
}

fn fav(id: &str) -> LibraryItem {
    serde_json::from_value(json!({ "id": id, "contentType": "movie", "name": id, "posterShape": "Poster" })).unwrap()
}

fn play(store: &AppStore, profile: i64, id: &str, pos: i64, at: i64) {
    let item: PlaybackItem = serde_json::from_value(json!({ "contentType": "movie", "parentMetaId": id, "parentMetaType": "movie", "videoId": id, "title": id })).unwrap();
    store.record_progress(profile, item.entry(pos, 6_000_000, false, at, 90).unwrap(), None).unwrap();
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap()
}

fn seed_profiles(m: &mut mock::Mock) {
    m.profiles = vec![
        json!({ "profile_index": 1, "name": "Ana", "avatar_color_hex": "#E53935", "avatar_id": "avatar_a", "pin_enabled": false, "updated_at": "2026-01-01T00:00:00Z" }),
    ];
}

#[test]
fn sign_in_errors_session_protection_and_sign_out_keep_local_data() {
    let (base, m) = mock::start();
    let d = dir("auth");
    rt().block_on(async {
        let e = engine(&base, &d, None);
        assert_eq!(e.status().state, "signedOut");
        let bad = e.sign_in(EMAIL, "errada").await.unwrap_err();
        assert_eq!(bad, AccountError::Remote(RemoteError::Auth("E-mail ou senha incorretos.".into())));
        let st = e.sign_in(EMAIL, PASSWORD).await.unwrap();
        assert!(st.signed_in && st.email.as_deref() == Some(EMAIL));
        // Session file protected with DPAPI: no token in clear.
        let raw = std::fs::read(d.join("account.dat")).unwrap();
        let access = m.lock().unwrap().access.clone();
        assert!(!raw.windows(access.len()).any(|w| w == access.as_bytes()));
        // Persistent: a new engine on the same folder is signed in.
        let again = engine(&base, &d, None);
        assert!(again.status().signed_in);

        let store = AppStore::open(&d).unwrap();
        store.toggle_favorite(1, fav("tt-local")).unwrap();
        let out = again.sign_out().await;
        assert_eq!(out.state, "signedOut");
        assert!(!d.join("account.dat").exists());
        assert_eq!(store.favorites(1).unwrap().len(), 1, "local data kept");
        assert!(out.pending > 0, "unsent change kept for the next sign-in");
        assert!(m.lock().unwrap().calls.iter().any(|c| c.starts_with("/auth/v1/logout")));
    });
}

#[test]
fn refresh_on_expired_token_and_session_end_when_refresh_is_refused() {
    let (base, m) = mock::start();
    let d = dir("refresh");
    rt().block_on(async {
        seed_profiles(&mut m.lock().unwrap());
        let e = engine(&base, &d, None);
        e.sign_in(EMAIL, PASSWORD).await.unwrap();
        m.lock().unwrap().expire_next = true; // 401 JWT expired on the next call → refresh + retry
        e.sync(1, true).await.unwrap();
        assert!(m.lock().unwrap().token_counter >= 2, "token refreshed");
        assert_eq!(e.status().state, "idle");

        let store = AppStore::open(&d).unwrap();
        let before = store.favorites(1).unwrap();
        {
            let mut mk = m.lock().unwrap();
            mk.refresh_revoked = true;
            mk.expire_next = true;
        }
        let err = e.sync(1, false).await.unwrap_err();
        assert_eq!(err, AccountError::SignedOut);
        let st = e.status();
        assert_eq!((st.state.as_str(), st.signed_in), ("expired", false));
        assert_eq!(store.favorites(1).unwrap(), before, "local data untouched");
    });
}

#[test]
fn first_sync_merges_both_origins_then_incremental_only() {
    let (base, m) = mock::start();
    let d = dir("first");
    let store = AppStore::open(&d).unwrap();
    // This computer: profiles 1 and 2, favorites, progress, an addon.
    store.create_profile("Dois", "#43A047", false).unwrap().unwrap();
    store.toggle_favorite(1, fav("tt-local")).unwrap();
    store.toggle_favorite(1, fav("tt-both")).unwrap();
    store.toggle_favorite(1, fav("tt-pending")).unwrap();
    store.toggle_favorite(2, fav("tt-p2")).unwrap();
    // tt-local / tt-both: data that already existed here (no local change pending); tt-pending: a
    // change made here and not sent yet.
    store
        .update_sync_state(|s| {
            let p = s.profiles.get_mut(&1).unwrap();
            p.library_upserts.remove("movie/tt-local");
            p.library_upserts.remove("movie/tt-both");
        })
        .unwrap();
    play(&store, 1, "tt-play", 600_000, 5_000);
    store.install_addon(1, "https://local.test/manifest.json", Default::default()).unwrap().unwrap();
    // The account (another device): profile 1, favorites, progress, addon.
    {
        let mut mk = m.lock().unwrap();
        seed_profiles(&mut mk);
        mk.remote_library_upsert(1, "tt-both", "Ambos (conta)");
        mk.remote_library_upsert(1, "tt-remote", "Só na conta");
        mk.remote_library_upsert(1, "tt-pending", "Versão antiga da conta");
        mk.remote_progress(1, "tt-remote-play", 300_000, 4_000);
        mk.addons.insert(1, vec![json!({ "url": "https://remote.test/manifest.json", "name": "R", "enabled": true, "sort_order": 0 })]);
        mk.collections.insert(1, json!([{ "id": "c1", "title": "Estúdios", "folders": [{ "id": "f1", "title": "Marvel", "focusGifUrl": "https://img.test/m.gif" }] }]));
        mk.home.insert(1, json!({ "items": [
            { "key": "r:movie:top", "order": 0 },
            { "key": "collection_c1", "is_collection": true, "collection_id": "c1", "order": 1 },
            { "key": "r:series:new", "order": 2 }
        ]}));
    }
    rt().block_on(async {
        let e = engine(&base, &d, Some(USER));
        e.sign_in(EMAIL, PASSWORD).await.unwrap();
        let report = e.sync(1, true).await.unwrap();
        assert!(report.failed.is_empty(), "{:?}", report.failed);

        // Collections: read from the account and kept as they are; never pushed.
        assert_eq!(store.collections(1).unwrap().unwrap()[0]["folders"][0]["focusGifUrl"], "https://img.test/m.gif");
        assert!(report.changed.contains(&"collections".to_string()), "{:?}", report.changed);
        assert!(!m.lock().unwrap().calls.iter().any(|c| c == "sync_push_collections"));

        // Home layout: the account's order, read only.
        let order = |store: &AppStore| store.profile_settings(1).unwrap().home.items.iter().map(|p| p.key.clone()).collect::<Vec<_>>();
        assert_eq!(order(&store), ["r:movie:top", "collection_c1", "r:series:new"]);
        assert!(report.changed.contains(&"home".to_string()), "{:?}", report.changed);
        assert!(!m.lock().unwrap().calls.iter().any(|c| c.starts_with("sync_push_home") || c.starts_with("sync_push_profile_settings")), "a first sync only adopts");

        // Local: union, no duplicates.
        let keys: Vec<String> = store.favorites(1).unwrap().iter().map(|i| i.key()).collect();
        for k in ["movie/tt-local", "movie/tt-both", "movie/tt-remote"] {
            assert_eq!(keys.iter().filter(|x| *x == k).count(), 1, "{k} once");
        }
        assert_eq!(store.favorites(1).unwrap().iter().find(|i| i.id == "tt-both").unwrap().name, "Ambos (conta)", "both, nothing pending: account version");
        assert_eq!(store.favorites(1).unwrap().iter().find(|i| i.id == "tt-pending").unwrap().name, "tt-pending", "pending local change wins");
        assert_eq!(m.lock().unwrap().library[&(1, "movie/tt-pending".to_string())]["name"], "tt-pending", "and is sent");
        let progress: Vec<String> = store.progress(1).unwrap().iter().map(|p| p.key()).collect();
        assert!(progress.contains(&"tt-play".to_string()) && progress.contains(&"tt-remote-play".to_string()));
        let addons: Vec<String> = store.addons(1).unwrap().iter().map(|a| a.manifest_url.clone()).collect();
        assert_eq!(addons, vec!["https://remote.test/manifest.json", "https://local.test/manifest.json"]);
        assert_eq!(m.lock().unwrap().addons[&1][0]["name"], "R", "the account's addon name is sent back unchanged");
        assert_eq!(store.profiles().unwrap().profiles.len(), 2, "local-only profile 2 kept");

        // Account: local-only items uploaded, per profile.
        {
            let mk = m.lock().unwrap();
            assert!(mk.library.contains_key(&(1, "movie/tt-local".into())));
            assert!(mk.library.contains_key(&(2, "movie/tt-p2".into())), "profile 2 data under profile_id 2");
            assert!(!mk.library.contains_key(&(1, "movie/tt-p2".into())), "never mixed");
            assert!(mk.progress.contains_key(&(1, "tt-play".into())));
            assert_eq!(mk.addons[&1].len(), 2);
            assert_eq!(mk.profiles.len(), 2);
            assert!(mk.origins.iter().all(|o| o.starts_with("nuvio-astra-")));
        }
        assert_eq!(e.status().pending, 0);

        // Second run: deltas only, nothing re-sent.
        m.lock().unwrap().calls.clear();
        let quiet = e.sync(1, false).await.unwrap();
        assert_eq!(quiet.sent, 0);
        let calls = m.lock().unwrap().calls.clone();
        assert!(!calls.iter().any(|c| c == "sync_pull_library" || c == "sync_pull_watch_progress" || c == "sync_pull_watched_items"), "{calls:?}");
        assert!(calls.iter().any(|c| c == "sync_pull_library_delta"));

        // Another device changes; this computer changes one item: only that goes up.
        {
            let mut mk = m.lock().unwrap();
            mk.remote_library_upsert(1, "tt-new", "Novo");
            mk.remote_library_delete(1, "tt-remote");
            mk.calls.clear();
        }
        store.toggle_favorite(1, fav("tt-mine")).unwrap();
        let inc = e.sync(1, false).await.unwrap();
        assert_eq!(inc.sent, 1);
        let keys: Vec<String> = store.favorites(1).unwrap().iter().map(|i| i.key()).collect();
        assert!(keys.contains(&"movie/tt-new".into()) && !keys.contains(&"movie/tt-remote".into()));
        assert!(m.lock().unwrap().library.contains_key(&(1, "movie/tt-mine".into())));
    });
}

#[test]
fn deletes_do_not_come_back_and_pending_local_items_survive_remote_deletes() {
    let (base, m) = mock::start();
    let d = dir("deletes");
    let store = AppStore::open(&d).unwrap();
    seed_profiles(&mut m.lock().unwrap());
    rt().block_on(async {
        let e = engine(&base, &d, None);
        e.sign_in(EMAIL, PASSWORD).await.unwrap();
        store.toggle_favorite(1, fav("tt-x")).unwrap();
        e.sync(1, true).await.unwrap();
        // Removed here → removed on the account, and a later sync does not bring it back.
        store.remove_favorite(1, "movie", "tt-x").unwrap();
        e.sync(1, false).await.unwrap();
        assert!(!m.lock().unwrap().library.contains_key(&(1, "movie/tt-x".into())));
        e.sync(1, false).await.unwrap();
        assert!(store.favorites(1).unwrap().is_empty());

        // Saved here while offline, deleted on another device meanwhile: the local intent wins.
        m.lock().unwrap().remote_library_upsert(1, "tt-y", "Y");
        e.sync(1, false).await.unwrap();
        m.lock().unwrap().down = true;
        store.remove_favorite(1, "movie", "tt-y").unwrap();
        store.toggle_favorite(1, fav("tt-y")).unwrap();
        assert!(e.sync(1, false).await.is_err());
        {
            let mut mk = m.lock().unwrap();
            mk.down = false;
            mk.remote_library_delete(1, "tt-y");
        }
        e.sync(1, false).await.unwrap();
        assert!(store.favorites(1).unwrap().iter().any(|i| i.id == "tt-y"));
        assert!(m.lock().unwrap().library.contains_key(&(1, "movie/tt-y".into())));
    });
}

#[test]
fn progress_conflicts_follow_the_original_and_offline_changes_are_sent_later() {
    let (base, m) = mock::start();
    let d = dir("progress");
    let store = AppStore::open(&d).unwrap();
    seed_profiles(&mut m.lock().unwrap());
    rt().block_on(async {
        let e = engine(&base, &d, None);
        e.sign_in(EMAIL, PASSWORD).await.unwrap();
        e.sync(1, true).await.unwrap();

        // Offline: the player keeps saving locally.
        m.lock().unwrap().down = true;
        play(&store, 1, "tt-movie", 1_800_000, 9_000);
        let err = e.sync(1, false).await.unwrap_err();
        assert_eq!(err, AccountError::Remote(RemoteError::Offline));
        let st = e.status();
        assert_eq!(st.state, "offline");
        assert!(st.pending >= 1);

        // Back online; meanwhile another device saved an OLDER position: the local fresher one wins.
        {
            let mut mk = m.lock().unwrap();
            mk.down = false;
            mk.remote_progress(1, "tt-movie", 300_000, 5_000);
        }
        e.sync(1, false).await.unwrap();
        assert_eq!(store.progress(1).unwrap().iter().find(|p| p.key() == "tt-movie").unwrap().last_position_ms, 1_800_000);
        assert_eq!(m.lock().unwrap().progress[&(1, "tt-movie".to_string())]["position"], 1_800_000, "sent after reconnecting");
        assert_eq!(e.status().pending, 0);

        // A NEWER position from another device replaces the local one.
        m.lock().unwrap().remote_progress(1, "tt-movie", 2_400_000, 20_000);
        e.sync(1, false).await.unwrap();
        assert_eq!(store.progress(1).unwrap().iter().find(|p| p.key() == "tt-movie").unwrap().last_position_ms, 2_400_000);
    });
}

#[test]
fn failed_steps_are_reported_kept_pending_and_retries_do_not_duplicate() {
    let (base, m) = mock::start();
    let d = dir("retry");
    let store = AppStore::open(&d).unwrap();
    seed_profiles(&mut m.lock().unwrap());
    rt().block_on(async {
        let e = engine(&base, &d, None);
        e.sign_in(EMAIL, PASSWORD).await.unwrap();
        e.sync(1, true).await.unwrap();
        store.toggle_favorite(1, fav("tt-a")).unwrap();
        play(&store, 1, "tt-b", 500_000, 7_000);
        m.lock().unwrap().failing = vec!["sync_push_watch_progress".into()];
        let report = e.sync(1, false).await.unwrap();
        assert!(report.failed.iter().any(|f| f.starts_with("progresso")), "{:?}", report.failed);
        assert_eq!(e.status().state, "error");
        assert!(e.status().pending > 0, "nothing marked as sent");
        // Retry: the library item is sent again (idempotent upsert), no duplicate rows.
        m.lock().unwrap().failing.clear();
        e.sync(1, false).await.unwrap();
        e.sync(1, false).await.unwrap();
        let mk = m.lock().unwrap();
        assert_eq!(mk.library.keys().filter(|k| k.1 == "movie/tt-a").count(), 1);
        assert_eq!(mk.progress.keys().filter(|k| k.1 == "tt-b").count(), 1);
        drop(mk);
        assert_eq!(e.status().pending, 0);
        assert_eq!(e.status().state, "idle");
    });
}

#[test]
fn data_from_another_account_needs_confirmation_and_old_local_deletes_never_delete_account_data() {
    let (base, m) = mock::start();
    let d = dir("link");
    let store = AppStore::open(&d).unwrap();
    {
        let mut mk = m.lock().unwrap();
        seed_profiles(&mut mk);
        mk.remote_library_upsert(1, "tt-keep", "Conta");
    }
    // Before any link: a local profile 2 deleted, a favorite with the same key removed.
    store.create_profile("Dois", "#43A047", false).unwrap().unwrap();
    store.toggle_favorite(1, fav("tt-keep")).unwrap();
    store.remove_favorite(1, "movie", "tt-keep").unwrap();
    rt().block_on(async {
        let e = engine(&base, &d, Some("someone-else"));
        e.sign_in(EMAIL, PASSWORD).await.unwrap();
        assert_eq!(e.sync(1, false).await.unwrap_err(), AccountError::NeedsReview);
        assert_eq!(e.status().state, "needsReview");
        assert!(e.status().link_question.is_some(), "data of another account is announced");
        assert!(m.lock().unwrap().calls.iter().all(|c| !c.starts_with("sync_push")), "nothing sent before the confirmation");
        e.sync(1, true).await.unwrap();
        assert!(m.lock().unwrap().library.contains_key(&(1, "movie/tt-keep".into())), "old local delete did not delete account data");
        assert!(store.favorites(1).unwrap().iter().any(|i| i.id == "tt-keep"));
    });
}

#[test]
fn pin_online_offline_lockout_and_avatars() {
    let (base, m) = mock::start();
    let d = dir("pin");
    seed_profiles(&mut m.lock().unwrap());
    rt().block_on(async {
        let e = engine(&base, &d, None);
        // Signed out: setting a PIN needs the account.
        assert!(e.set_pin(1, "1234", None).await.unwrap_err().to_string().contains("Conecte-se"));
        e.sign_in(EMAIL, PASSWORD).await.unwrap();
        e.sync(1, true).await.unwrap();
        assert!(e.set_pin(1, "12a4", None).await.is_err(), "4 digits");
        e.set_pin(1, "1234", None).await.unwrap();
        let store = AppStore::open(&d).unwrap();
        assert!(store.profiles().unwrap().get(1).unwrap().pin_enabled);
        let cache = std::fs::read_to_string(d.join("pin-cache.json")).unwrap();
        assert!(!cache.contains("1234"), "PIN never stored in clear");

        assert!(e.verify_pin(1, "1234").await.unlocked);
        let wrong = e.verify_pin(1, "0000").await;
        assert_eq!((wrong.unlocked, wrong.message.as_deref()), (false, Some("PIN incorreto")));
        // Changing needs the current PIN.
        assert!(e.set_pin(1, "5678", Some("0000")).await.is_err());
        e.set_pin(1, "5678", Some("1234")).await.unwrap();
        // Offline: the cached hash of the last success.
        m.lock().unwrap().down = true;
        assert!(e.verify_pin(1, "5678").await.unlocked);
        assert!(!e.verify_pin(1, "1234").await.unlocked);
        assert!(e.clear_pin(1, Some("5678")).await.unwrap_err().to_string().contains("Conecte-se"));
        m.lock().unwrap().down = false;
        // Server lockout after repeated failures.
        for _ in 0..3 {
            e.verify_pin(1, "9999").await;
        }
        let locked = e.verify_pin(1, "5678").await;
        assert_eq!(locked.retry_after_seconds, 30);
        assert_eq!(locked.message.as_deref(), Some("Bloqueado. Tente novamente em 30s"));
        m.lock().unwrap().pin_failures.clear();
        e.clear_pin(1, Some("5678")).await.unwrap();
        assert!(!store.profiles().unwrap().get(1).unwrap().pin_enabled);

        // Avatars: active ones, ordered, public image URL; offline from the cache.
        let avatars = e.avatars().await.unwrap();
        assert_eq!(avatars.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), vec!["avatar_a", "avatar_b"]);
        assert_eq!(avatars[0].image_url, format!("{base}/storage/v1/object/public/avatars/a.png"));
        m.lock().unwrap().down = true;
        assert_eq!(e.avatars().await.unwrap().len(), 2);
        let _: Value = json!(null);
    });
}

fn files(d: &PathBuf) -> std::collections::BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(d)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_file() && e.file_name() != "sync-log.json" && e.file_name() != "account.dat")
        .map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap()))
        .collect()
}

#[test]
fn first_sync_waits_for_the_reviewed_preview_which_is_read_only_and_exact() {
    let (base, m) = mock::start();
    let d = dir("preview");
    let store = AppStore::open(&d).unwrap();
    // This computer (data migrated from the original, nothing pending): profiles 1 and 2.
    store.create_profile("Dois", "#43A047", false).unwrap().unwrap();
    store.toggle_favorite(1, fav("tt-local")).unwrap();
    store.toggle_favorite(1, fav("tt-both")).unwrap();
    store.toggle_favorite(2, fav("tt-p2")).unwrap();
    play(&store, 1, "tt-newer-here", 900_000, 9_000);
    play(&store, 1, "tt-newer-there", 100_000, 1_000);
    store.install_addon(1, "https://local.test/manifest.json", Default::default()).unwrap().unwrap();
    store.update_sync_state(|s| *s = Default::default()).unwrap();
    {
        let mut mk = m.lock().unwrap();
        seed_profiles(&mut mk);
        mk.remote_library_upsert(1, "tt-both", "Ambos");
        mk.remote_library_upsert(1, "tt-remote", "Só na conta");
        mk.remote_progress(1, "tt-newer-here", 300_000, 4_000);
        mk.remote_progress(1, "tt-newer-there", 500_000, 8_000);
        mk.addons.insert(1, vec![json!({ "url": "https://remote.test/manifest.json", "name": "Addon da conta", "enabled": true, "sort_order": 0 })]);
    }
    rt().block_on(async {
        let e = engine(&base, &d, None);
        e.sign_in(EMAIL, PASSWORD).await.unwrap();
        assert_eq!(e.status().state, "needsReview", "signed in, first sync not started");
        // The background loop's run does nothing before the confirmation.
        let before = files(&d);
        m.lock().unwrap().calls.clear();
        assert_eq!(e.sync(1, false).await.unwrap_err(), AccountError::NeedsReview);
        assert!(m.lock().unwrap().calls.is_empty(), "no request at all");

        // Preview: reads only, no local change, exact counts.
        let p = e.preview().await.unwrap();
        let calls = m.lock().unwrap().calls.clone();
        let reads = ["sync_pull_profiles", "/rest/v1/addons", "sync_pull_library", "sync_pull_watch_progress", "sync_pull_watched_items"];
        assert!(calls.iter().all(|c| reads.contains(&c.as_str())), "{calls:?}");
        assert_eq!(files(&d), before, "local data untouched by the preview");
        assert!(p.first_sync && !p.other_account);
        assert_eq!((p.profiles.local, p.profiles.remote, p.profiles.only_local), (2, 1, 1));
        let p1 = p.entries.iter().find(|e| e.index == 1).unwrap();
        assert_eq!((p1.favorites.only_local, p1.favorites.only_remote, p1.favorites.both), (1, 1, 1));
        assert_eq!((p1.progress.differing, p1.progress.local_newer), (2, 1));
        assert_eq!((p1.addons.only_local, p1.addons.only_remote), (1, 1));
        let p2 = p.entries.iter().find(|e| e.index == 2).unwrap();
        assert_eq!((p2.remote_name.as_deref(), p2.favorites.only_local), (None, 1));
        // Sent: tt-local, tt-p2, the newer local progress, the local addon, profile 2 (+ profile 2's
        // own addons, if it has any).
        assert_eq!(p.to_send, 5 + p2.addons.only_local);
        // Received: tt-remote, the newer account progress, the account addon, + profile 1 if it differs.
        assert_eq!(p.to_receive, 3 + usize::from(p1.profile_differs));
        assert_eq!(p.conflicts, 2 + usize::from(p1.profile_differs));
        assert_eq!(e.status().state, "needsReview");

        // Confirmed by the user: verified backup first, then every profile joined.
        e.sync(1, true).await.unwrap();
        let backups: Vec<String> = std::fs::read_dir(d.join("backups")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        let backup = backups.iter().find(|b| b.starts_with("before-first-sync-")).expect("backup made");
        assert_eq!(&std::fs::read(d.join("backups").join(backup).join("library-1.json")).unwrap(), &before["library-1.json"]);
        let pos = |key: &str| store.progress(1).unwrap().iter().find(|p| p.key() == key).unwrap().last_position_ms;
        assert_eq!(pos("tt-newer-here"), 900_000, "a newer local position is never replaced");
        assert_eq!(pos("tt-newer-there"), 500_000);
        {
            let mk = m.lock().unwrap();
            assert_eq!(mk.progress[&(1, "tt-newer-here".to_string())]["position"], 900_000, "and is sent");
            assert!(mk.library.contains_key(&(2, "movie/tt-p2".into())), "profile 2 joined in the same run");
            assert_eq!(mk.addons[&1][0]["name"], "Addon da conta");
        }
        assert_eq!(e.status().state, "idle");
        assert_eq!(e.status().pending, 0);
        assert!(store.sync_state().unwrap().profiles[&2].library.initialized, "profile 2 cursor set");

        // Check afterwards: same comparison, both sides identical, nothing written by it.
        m.lock().unwrap().calls.clear();
        let after = e.preview().await.unwrap();
        assert!(!after.first_sync);
        assert_eq!((after.to_send, after.to_receive, after.conflicts, after.pending), (0, 0, 0, 0), "{after:#?}");
        assert!(after.entries.iter().all(|e| e.favorites.remote_duplicates + e.progress.remote_duplicates + e.watched.remote_duplicates == 0));
        assert!(m.lock().unwrap().calls.iter().all(|c| reads.contains(&c.as_str())));
        // Idempotent: another run sends nothing.
        assert_eq!(e.sync(1, false).await.unwrap().sent, 0);

        // The log has counts only.
        let log = std::fs::read_to_string(d.join("sync-log.json")).unwrap();
        let access = m.lock().unwrap().access.clone();
        assert!(!log.contains(&access) && !log.contains(EMAIL) && !log.contains("Dois") && !log.contains("tt-"));
        assert!(log.contains("\"preview\"") && log.contains("\"backup\"") && log.contains("\"sync\""));
    });
}

fn desktop_blob() -> Value {
    json!({ "version": 4, "unknown_top": "keep", "features": {
        "theme_settings": { "selected_theme": { "type": "string", "value": "OCEAN" } },
        "poster_card_style_settings_payload": "{\"widthDp\":140}",
        "debrid_settings": { "debrid_enabled": { "type": "boolean", "value": true } },
        "player_settings": {
            "skip_intro_enabled": { "type": "boolean", "value": false },
            "hold_to_speed_value": { "type": "float", "value": 2.5 },
            "subtitle_appearance": { "type": "string", "value": "{\"fontId\":\"inter\"}" },
            "preferred_audio_language": { "type": "string", "value": "ja" },
            "subtitle_font_size_sp": { "type": "int", "value": 22 }
        }
    }})
}

#[test]
fn preferences_sync_both_ways_between_computers_without_losing_the_rest() {
    let (base, m) = mock::start();
    let (da, db) = (dir("prefs-a"), dir("prefs-b"));
    let (a, b) = (AppStore::open(&da).unwrap(), AppStore::open(&db).unwrap());
    a.update_profile_settings(1, |s| s.player.preferred_audio_language = "en".into()).unwrap();
    {
        let mut mk = m.lock().unwrap();
        seed_profiles(&mut mk);
        mk.blobs.insert(1, desktop_blob());
        mk.home.insert(1, json!({ "show_catalog_type": true, "future": [1, 2], "items": [
            { "key": "x:movie:top", "order": 0, "enabled": true, "extra": "keep" },
            { "key": "collection_c1", "is_collection": true, "collection_id": "c1", "order": 1 },
            { "key": "server:plex:movies", "order": 2 }
        ]}));
    }
    rt().block_on(async {
        let ea = engine(&base, &da, Some(USER));
        ea.sign_in(EMAIL, PASSWORD).await.unwrap();
        // The mock keeps a single session: the second computer uses the same one.
        std::fs::copy(da.join("account.dat"), db.join("account.dat")).unwrap();
        let eb = engine(&base, &db, Some(USER));
        ea.sync(1, true).await.unwrap();
        assert_eq!(a.profile_settings(1).unwrap().player.preferred_audio_language, "ja", "first sync: the account's");
        assert!(da.join("settings-before-sync.json").exists(), "what was here is kept");
        assert_eq!(m.lock().unwrap().blobs[&1], desktop_blob(), "nothing written by a first sync");

        eb.sync(1, true).await.unwrap();
        assert_eq!(b.profile_settings(1).unwrap().player.subtitle.font_size_sp, 22);

        // Each computer changes another field before either syncs.
        a.update_profile_settings(1, |s| s.player.subtitle.bold = true).unwrap();
        a.update_profile_settings(1, |s| {
            for row in s.home.items.iter_mut() {
                match row.key.as_str() {
                    "collection_c1" => row.order = 0,
                    "x:movie:top" => row.order = 1,
                    _ => {}
                }
            }
        })
        .unwrap();
        b.update_profile_settings(1, |s| {
            s.player.preferred_audio_language = "pt".into();
            s.player.motion_smoothing = "fluid".into();
            s.home.items.iter_mut().find(|r| r.key == "x:movie:top").unwrap().custom_title = "Em alta".into();
        })
        .unwrap();
        assert!(ea.status().pending > 0 && eb.status().pending > 0, "waiting to be sent");

        ea.sync(1, false).await.unwrap();
        eb.sync(1, false).await.unwrap();
        ea.sync(1, false).await.unwrap();

        for store in [&a, &b] {
            let s = store.profile_settings(1).unwrap();
            assert!(s.player.subtitle.bold);
            assert_eq!(s.player.preferred_audio_language, "pt");
            let row = |key: &str| s.home.items.iter().find(|r| r.key == key).unwrap();
            assert_eq!((row("collection_c1").order, row("x:movie:top").order), (0, 1));
            assert_eq!(row("x:movie:top").custom_title, "Em alta");
        }
        assert_eq!((ea.status().pending, eb.status().pending), (0, 0));
        assert_eq!(a.profile_settings(1).unwrap().player.motion_smoothing, "off", "Astra-only options stay on each computer");

        // The account: exactly the original blob plus the two changed fields.
        let mut expected = desktop_blob();
        expected["features"]["player_settings"]["subtitle_bold"] = json!({ "type": "boolean", "value": true });
        expected["features"]["player_settings"]["preferred_audio_language"] = json!({ "type": "string", "value": "pt" });
        assert_eq!(m.lock().unwrap().blobs[&1], expected);
        let home = m.lock().unwrap().home[&1].clone();
        assert_eq!(home["future"], json!([1, 2]));
        let items = home["items"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert!(items.contains(&json!({ "key": "server:plex:movies", "order": 2 })), "{home}");
        assert!(items.iter().any(|i| i["extra"] == "keep" && i["custom_title"] == "Em alta"));

        // Settled: no more writes.
        m.lock().unwrap().calls.clear();
        ea.sync(1, false).await.unwrap();
        eb.sync(1, false).await.unwrap();
        assert!(!m.lock().unwrap().calls.iter().any(|c| c.starts_with("sync_push_")), "{:?}", m.lock().unwrap().calls);

        // The account clears a field: the default here, not sent back.
        m.lock().unwrap().blobs.get_mut(&1).unwrap()["features"]["player_settings"].as_object_mut().unwrap().remove("subtitle_bold");
        ea.sync(1, false).await.unwrap();
        assert!(!a.profile_settings(1).unwrap().player.subtitle.bold);
        assert!(m.lock().unwrap().blobs[&1]["features"]["player_settings"].get("subtitle_bold").is_none());

        // An editor opened before that sync saves only its own edit.
        let mut opened = a.profile_settings(1).unwrap();
        opened.player.subtitle.bold = true; // what it still shows
        let mut edited = opened.clone();
        edited.player.resize_mode = "Zoom".into();
        let saved = a.edit_profile_settings(1, &opened, &edited).unwrap();
        assert_eq!((saved.player.resize_mode.as_str(), saved.player.subtitle.bold), ("Zoom", false));
    });
}

#[test]
fn failed_preference_push_waits_and_a_missing_blob_is_never_created() {
    let (base, m) = mock::start();
    let d = dir("prefs-retry");
    let store = AppStore::open(&d).unwrap();
    {
        let mut mk = m.lock().unwrap();
        seed_profiles(&mut mk);
        mk.home.insert(1, json!({ "items": [{ "key": "x:movie:top", "order": 0 }] }));
    }
    rt().block_on(async {
        let e = engine(&base, &d, Some(USER));
        e.sign_in(EMAIL, PASSWORD).await.unwrap();
        e.sync(1, true).await.unwrap();

        store.update_profile_settings(1, |s| { s.player.subtitle.bold = true; s.home.items[0].enabled = false; }).unwrap();
        m.lock().unwrap().failing.push("sync_push_home_catalog_settings".into());
        let report = e.sync(1, false).await.unwrap();
        assert!(!report.failed.is_empty());
        assert!(!m.lock().unwrap().blobs.contains_key(&1), "no partial blob");
        assert_eq!(e.status().pending, 1, "the Home change waits");
        assert!(!store.profile_settings(1).unwrap().home.items[0].enabled, "and is not undone");

        m.lock().unwrap().failing.clear();
        e.sync(1, false).await.unwrap();
        assert_eq!(m.lock().unwrap().home[&1]["items"][0]["enabled"], false);
        assert_eq!(e.status().pending, 0);

        // A saved position goes out by itself, without reading the account.
        play(&store, 1, "tt-minute", 120_000, 9_000);
        m.lock().unwrap().calls.clear();
        assert_eq!(e.push_pending(1).await.unwrap().sent, 1);
        assert_eq!(m.lock().unwrap().calls, vec!["sync_push_watch_progress".to_string()]);
    });
}

#[test]
fn opening_receives_every_profile_before_anything_is_sent() {
    let (base, m) = mock::start();
    let d = dir("receive-first");
    let store = AppStore::open(&d).unwrap();
    {
        let mut mk = m.lock().unwrap();
        seed_profiles(&mut mk);
        mk.profiles.push(json!({ "profile_index": 2, "name": "Bia", "avatar_color_hex": "#43A047", "pin_enabled": false, "updated_at": "2026-01-01T00:00:00Z" }));
    }
    rt().block_on(async {
        let e = engine(&base, &d, None);
        e.sign_in(EMAIL, PASSWORD).await.unwrap();
        e.sync(1, true).await.unwrap();
        // Closed with an older position waiting; another device went further meanwhile.
        play(&store, 1, "tt-movie", 600_000, 5_000);
        m.lock().unwrap().remote_progress(1, "tt-movie", 2_700_000, 8_000);
        m.lock().unwrap().remote_progress(2, "tt-other", 900_000, 8_000);

        let reopened = engine(&base, &d, None);
        assert_eq!(reopened.push_pending(1).await.unwrap_err(), AccountError::Busy, "nothing goes out before receiving");
        m.lock().unwrap().calls.clear();
        reopened.sync(1, false).await.unwrap();
        {
            let mk = m.lock().unwrap();
            let pull = mk.calls.iter().position(|c| c.starts_with("sync_pull_watch_progress")).unwrap();
            let push = mk.calls.iter().position(|c| c == "sync_push_watch_progress").unwrap_or(usize::MAX);
            assert!(pull < push, "{:?}", mk.calls);
            assert_eq!(mk.progress[&(1, "tt-movie".to_string())]["position"], 2_700_000, "the newer one was not overwritten");
        }
        assert_eq!(store.progress(1).unwrap().iter().find(|p| p.key() == "tt-movie").unwrap().last_position_ms, 2_700_000);
        assert_eq!(store.progress(2).unwrap().iter().find(|p| p.key() == "tt-other").unwrap().last_position_ms, 900_000, "also the profiles not on screen");
        assert_eq!(reopened.status().pending, 0);
    });
}

#[test]
fn pin_removed_elsewhere_is_followed_and_another_account_never_opens_a_profile() {
    let (base, m) = mock::start();
    rt().block_on(async {
        seed_profiles(&mut m.lock().unwrap());
        // Profiles copied from the original, with a PIN, no session yet.
        let d = dir("pin-removed");
        let store = AppStore::open(&d).unwrap();
        store.set_profile_pin_state(1, true).unwrap();
        store.create_profile("Local", "#1E88E5", false).unwrap().unwrap();
        store.set_profile_pin_state(1, true).unwrap();
        let e = engine(&base, &d, Some("someone-else"));
        e.sign_in(EMAIL, PASSWORD).await.unwrap();
        assert_eq!(e.refresh_pin(1).await, None, "another account says nothing about this PIN");
        assert!(!e.verify_pin(1, "1234").await.unlocked);
        assert!(store.profiles().unwrap().get(1).unwrap().pin_enabled);

        let owner = engine(&base, &d, Some(USER));
        assert_eq!(owner.refresh_pin(1).await, Some(false));
        assert!(!store.profiles().unwrap().get(1).unwrap().pin_enabled, "removed on the account");
    });
}

#[test]
fn offline_first_unlock_registers_a_local_pin() {
    let d = dir("offline-pin");
    let store = AppStore::open(&d).unwrap();
    store.create_profile("Protegido", "#112233", false).unwrap().unwrap();
    store.set_profile_pin_state(2, true).unwrap();
    let e = engine("http://127.0.0.1:1", &d, None);
    let rt = rt();
    assert!(rt.block_on(e.verify_pin(2, "6969")).unlocked, "first offline unlock registers the PIN");
    assert!(!rt.block_on(e.verify_pin(2, "0000")).unlocked, "wrong PIN rejected afterwards");
    assert!(rt.block_on(e.verify_pin(2, "6969")).unlocked, "the registered PIN keeps working");
    let cache = std::fs::read_to_string(d.join("pin-cache.json")).unwrap();
    assert!(!cache.contains("6969"), "PIN never stored in clear");
}

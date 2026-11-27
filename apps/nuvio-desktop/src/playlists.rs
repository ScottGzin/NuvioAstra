//! Live TV → "Listas M3U": the playlists the user added (`services/m3u.rs`), kept encrypted with
//! DPAPI in `m3u.dat` for every profile of this computer. Their channels join the addons' ones in
//! `livetv_channels`; one plays through the normal stream path (`livetv_resolve`). The playlist
//! addresses never go back to the interface whole: only the site's name is shown.

use crate::services::livetv::{self, Channel, ChannelList};
use crate::services::m3u::{self, Entry, Playlist};
use crate::streams::{self, StreamGroup};
use crate::AppState;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tauri::State;

fn file(state: &AppState) -> std::path::PathBuf {
    state.app_store.dir().join("m3u.dat")
}

fn stored(state: &AppState) -> Vec<Playlist> {
    let Ok(cipher) = std::fs::read(file(state)) else { return Vec::new() };
    nuvio_sync::crypto::unprotect(&cipher).ok().and_then(|plain| serde_json::from_slice(&plain).ok()).unwrap_or_default()
}

fn save(state: &AppState, list: &[Playlist]) -> Result<(), String> {
    let path = file(state);
    if list.is_empty() {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    let plain = serde_json::to_vec(list).map_err(|e| e.to_string())?;
    let cipher = nuvio_sync::crypto::protect(&plain)?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, cipher).and_then(|()| std::fs::rename(&tmp, &path)).map_err(|e| e.to_string())
}

/// Channels read from each playlist, and why one failed (until the next reload).
static LOADED: Mutex<Option<HashMap<String, Result<Vec<Entry>, String>>>> = Mutex::new(None);

/// Reads a playlist: an address (60 s, at most `MAX_BYTES`) or a local file.
async fn fetch(source: &str) -> Result<Vec<Entry>, String> {
    let text = if source.starts_with("http://") || source.starts_with("https://") {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(60))
            .user_agent(concat!("NuvioAstra/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        let mut response = client.get(source).send().await.map_err(|_| "Não foi possível baixar a lista.".to_string())?;
        if !response.status().is_success() {
            return Err(format!("O servidor da lista respondeu com o código {}.", response.status().as_u16()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "A lista chegou incompleta.".to_string())? {
            if bytes.len() + chunk.len() > m3u::MAX_BYTES {
                return Err("Lista grande demais (mais de 40 MB).".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    } else {
        let meta = std::fs::metadata(source).map_err(|_| "Arquivo da lista não encontrado.".to_string())?;
        if meta.len() as usize > m3u::MAX_BYTES {
            return Err("Lista grande demais (mais de 40 MB).".into());
        }
        String::from_utf8_lossy(&std::fs::read(source).map_err(|_| "Não foi possível ler o arquivo da lista.".to_string())?).into_owned()
    };
    if !m3u::looks_like_playlist(&text) {
        return Err("Isso não é uma lista M3U.".into());
    }
    let entries = m3u::parse(&text);
    if entries.is_empty() {
        return Err("A lista não tem canais.".into());
    }
    Ok(entries)
}

/// Reads the playlists not read yet (all of them with `again`), at the same time.
async fn ensure_loaded(list: &[Playlist], again: bool) {
    let missing: Vec<Playlist> = {
        let loaded = LOADED.lock().expect("m3u");
        list.iter().filter(|p| again || loaded.as_ref().is_none_or(|l| !l.contains_key(&p.id))).cloned().collect()
    };
    let mut handles = Vec::new();
    for p in missing {
        handles.push(tauri::async_runtime::spawn(async move { (p.id.clone(), fetch(&p.source).await) }));
    }
    for handle in handles {
        if let Ok((id, result)) = handle.await {
            LOADED.lock().expect("m3u").get_or_insert_with(HashMap::new).insert(id, result);
        }
    }
}

fn channels_of(list: &[Playlist]) -> (Vec<Channel>, usize) {
    let loaded = LOADED.lock().expect("m3u");
    let mut out = Vec::new();
    let mut failed = 0;
    for p in list {
        match loaded.as_ref().and_then(|l| l.get(&p.id)) {
            Some(Ok(entries)) => out.extend(m3u::channels(p, entries)),
            Some(Err(_)) => failed += 1,
            None => {}
        }
    }
    (out, failed)
}

/// The addons' channels with the playlists' ones after them (`livetv_channels`, `livetv_reload`).
pub async fn with_playlists(state: &AppState, mut list: ChannelList, reload: bool) -> ChannelList {
    let playlists = stored(state);
    if playlists.is_empty() {
        return list;
    }
    ensure_loaded(&playlists, reload).await;
    let (channels, failed) = channels_of(&playlists);
    list.channels.extend(channels);
    list.categories = livetv::categories(&list.channels);
    list.failed_catalogs += failed;
    list
}

/// A playlist channel as one source (`livetv_resolve`); `None` when the key is not a playlist's.
pub fn resolve(state: &AppState, key: &str) -> Option<Result<Vec<StreamGroup>, String>> {
    let (playlist, index) = m3u::split(key)?;
    let name = stored(state).into_iter().find(|p| p.id == playlist).map(|p| p.name);
    let entry = LOADED.lock().expect("m3u").as_ref().and_then(|l| l.get(playlist)).and_then(|r| r.as_ref().ok()).and_then(|e| e.get(index)).cloned();
    Some(match (name, entry) {
        (Some(name), Some(entry)) => Ok(streams::register(&state.streams, name, &m3u::stream_payload(&entry), &format!("m3u:{playlist}"))),
        _ => Err("Este canal não está mais na lista. Recarregue os canais.".into()),
    })
}

// ---------------------------------------------------------------- the dialog

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistView {
    id: String,
    name: String,
    /// Where it comes from, without the path or the query (they often carry a user and password).
    origin: String,
    channels: Option<usize>,
    error: Option<String>,
}

fn origin(source: &str) -> String {
    match reqwest::Url::parse(source) {
        Ok(url) if url.scheme().starts_with("http") => url.host_str().unwrap_or("").to_string(),
        _ => std::path::Path::new(source).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "arquivo local".into()),
    }
}

fn views(list: &[Playlist]) -> Vec<PlaylistView> {
    let loaded = LOADED.lock().expect("m3u");
    list.iter()
        .map(|p| {
            let read = loaded.as_ref().and_then(|l| l.get(&p.id));
            PlaylistView {
                id: p.id.clone(),
                name: p.name.clone(),
                origin: origin(&p.source),
                channels: read.and_then(|r| r.as_ref().ok()).map(Vec::len),
                error: read.and_then(|r| r.as_ref().err()).cloned(),
            }
        })
        .collect()
}

#[tauri::command]
pub async fn m3u_list(state: State<'_, AppState>) -> Result<Vec<PlaylistView>, String> {
    let list = stored(&state);
    ensure_loaded(&list, false).await;
    Ok(views(&list))
}

/// Adds a playlist after reading it (a broken address is not kept).
#[tauri::command]
pub async fn m3u_add(state: State<'_, AppState>, name: String, source: String) -> Result<Vec<PlaylistView>, String> {
    let source = source.trim().trim_matches('"').to_string();
    let name = name.trim().chars().take(60).collect::<String>();
    let local = std::path::Path::new(&source).is_absolute();
    if !(source.starts_with("http://") || source.starts_with("https://") || local) {
        return Err("Use um endereço http(s) ou o caminho completo de um arquivo .m3u.".into());
    }
    let mut list = stored(&state);
    if list.iter().any(|p| p.source == source) {
        return Err("Essa lista já foi adicionada.".into());
    }
    let entries = fetch(&source).await?;
    let id = nuvio_sync::crypto::sha256_hex(&format!("{source}|{}", nuvio_store::library::now_ms()))[..12].to_string();
    let name = if name.is_empty() { origin(&source) } else { name };
    list.push(Playlist { id: id.clone(), name, source });
    save(&state, &list)?;
    LOADED.lock().expect("m3u").get_or_insert_with(HashMap::new).insert(id, Ok(entries));
    Ok(views(&list))
}

#[tauri::command]
pub async fn m3u_remove(state: State<'_, AppState>, id: String) -> Result<Vec<PlaylistView>, String> {
    let mut list = stored(&state);
    list.retain(|p| p.id != id);
    save(&state, &list)?;
    if let Some(l) = LOADED.lock().expect("m3u").as_mut() {
        l.remove(&id);
    }
    Ok(views(&list))
}

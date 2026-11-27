//! Debrid services besides TorBox (`torbox.rs`): Real-Debrid, AllDebrid, Premiumize and
//! Debrid-Link, each with the user's own API key. Same model as TorBox: a torrent source of the
//! addons is opened as the service's HTTP link when the service already has it ("cached"), and
//! otherwise keeps playing through P2P.
//!
//! - Only Premiumize can say beforehand which torrents it has (`cache/check`); the others are
//!   asked when the source is opened. A torrent they do not have is removed from the account at
//!   once, so trying a source never leaves a download running in the user's account.
//! - The file inside a season pack is chosen with the same rules as TorBox (`select_file`).
//! - The key goes only to its service (header, or the `apikey` query Premiumize and AllDebrid
//!   use) and never appears in an error text.

use crate::stream::StreamItem;
use crate::torbox::{select_file, File, Resolved, Wanted};
use crate::CoreError;
use serde_json::Value;
use std::collections::HashSet;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    RealDebrid,
    AllDebrid,
    Premiumize,
    DebridLink,
}

impl Provider {
    pub const ALL: [Provider; 4] = [Self::RealDebrid, Self::AllDebrid, Self::Premiumize, Self::DebridLink];

    pub fn id(self) -> &'static str {
        match self {
            Self::RealDebrid => "realdebrid",
            Self::AllDebrid => "alldebrid",
            Self::Premiumize => "premiumize",
            Self::DebridLink => "debridlink",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::RealDebrid => "Real-Debrid",
            Self::AllDebrid => "AllDebrid",
            Self::Premiumize => "Premiumize",
            Self::DebridLink => "Debrid-Link",
        }
    }

    /// Short mark on a source card: RD / AD / PM / DL.
    pub fn code(self) -> &'static str {
        match self {
            Self::RealDebrid => "RD",
            Self::AllDebrid => "AD",
            Self::Premiumize => "PM",
            Self::DebridLink => "DL",
        }
    }

    /// Accepts the ids above and the names addons use in `clientResolve.service`.
    pub fn parse(value: &str) -> Option<Self> {
        let v: String = value.trim().to_ascii_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        match v.as_str() {
            "realdebrid" | "rd" => Some(Self::RealDebrid),
            "alldebrid" | "ad" => Some(Self::AllDebrid),
            "premiumize" | "pm" => Some(Self::Premiumize),
            "debridlink" | "dl" => Some(Self::DebridLink),
            _ => None,
        }
    }

    /// Whether the service can tell which torrents it has without adding them.
    pub fn can_check(self) -> bool {
        self == Self::Premiumize
    }

    fn base(self) -> &'static str {
        match self {
            Self::RealDebrid => option_env!("NUVIO_RD_API_URL").unwrap_or("https://api.real-debrid.com/rest/1.0"),
            Self::AllDebrid => option_env!("NUVIO_AD_API_URL").unwrap_or("https://api.alldebrid.com"),
            Self::Premiumize => option_env!("NUVIO_PM_API_URL").unwrap_or("https://www.premiumize.me/api"),
            Self::DebridLink => option_env!("NUVIO_DL_API_URL").unwrap_or("https://debrid-link.com/api/v2"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DebridError {
    #[error("A chave do {} foi recusada. Confira em Configurações → Debrid.", .0.label())]
    Unauthorized(Provider),
    #[error("Este torrent não está no cache do {}.", .0.label())]
    NotCached(Provider),
    #[error("O {} não encontrou um vídeo desse episódio no torrent.", .0.label())]
    NoFile(Provider),
    #[error("Sem conexão com o {}.", .0.label())]
    Offline(Provider),
    #[error("O {} respondeu com erro ({}).", .0.label(), .1)]
    Failed(Provider, String),
}

/// AllDebrid asks every client to name itself.
const AGENT: &str = "NuvioAstra";

#[derive(Clone)]
pub struct Debrid {
    http: reqwest::Client,
}

fn text(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn id_text(v: Option<&Value>) -> Option<String> {
    text(v).or_else(|| v.and_then(Value::as_i64).map(|n| n.to_string()))
}

/// A torrent's files as `File`s (`id` = position) plus the service's own value for each.
fn files_of<'a>(items: impl Iterator<Item = (String, Option<u64>, &'a Value)>) -> (Vec<File>, Vec<&'a Value>) {
    let mut files = Vec::new();
    let mut raw = Vec::new();
    for (i, (name, size, value)) in items.enumerate() {
        files.push(File { id: i as i64, name, mime: String::new(), size });
        raw.push(value);
    }
    (files, raw)
}

fn pick<'a>(provider: Provider, files: &[File], raw: &[&'a Value], wanted: &Wanted) -> Result<(File, &'a Value), DebridError> {
    let file = select_file(files, wanted).ok_or(DebridError::NoFile(provider))?.clone();
    let value = raw.get(file.id as usize).copied().ok_or(DebridError::NoFile(provider))?;
    Ok((file, value))
}

/// AllDebrid's file tree (`n` name, `s` size, `l` link, `e` children) as a flat list.
fn flatten_tree<'a>(nodes: &'a [Value], prefix: &str, out: &mut Vec<(String, Option<u64>, &'a Value)>) {
    for node in nodes {
        let name = text(node.get("n")).unwrap_or_default();
        let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        match node.get("e").and_then(Value::as_array) {
            Some(children) => flatten_tree(children, &path, out),
            None => out.push((path, node.get("s").and_then(Value::as_u64), node)),
        }
    }
}

impl Debrid {
    pub fn new() -> Result<Self, CoreError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("NuvioAstra/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { http })
    }

    async fn send(&self, provider: Provider, request: reqwest::RequestBuilder) -> Result<(u16, Value), DebridError> {
        let response = request.send().await.map_err(|e| match CoreError::from(e) {
            CoreError::HttpStatus { status } => DebridError::Failed(provider, status.to_string()),
            _ => DebridError::Offline(provider),
        })?;
        let status = response.status().as_u16();
        if status == 401 {
            return Err(DebridError::Unauthorized(provider));
        }
        let body: Value = response.json().await.unwrap_or(Value::Null);
        Ok((status, body))
    }

    fn url(&self, provider: Provider, path: &str) -> String {
        format!("{}{path}", provider.base())
    }

    // ------------------------------------------------------------ per service requests

    async fn rd(&self, request: reqwest::RequestBuilder, key: &str) -> Result<(u16, Value), DebridError> {
        let (status, body) = self.send(Provider::RealDebrid, request.bearer_auth(key)).await?;
        if status == 403 && body.get("error_code").and_then(Value::as_i64) == Some(8) {
            return Err(DebridError::Unauthorized(Provider::RealDebrid));
        }
        if status >= 400 {
            let detail = text(body.get("error")).unwrap_or_else(|| status.to_string());
            return Err(DebridError::Failed(Provider::RealDebrid, detail));
        }
        Ok((status, body))
    }

    /// AllDebrid: `{status: "success", data}` or `{status: "error", error: {code, message}}`.
    async fn ad(&self, path: &str, key: &str, query: &[(&str, &str)]) -> Result<Value, DebridError> {
        let request = self.http.get(self.url(Provider::AllDebrid, path)).bearer_auth(key).query(&[("agent", AGENT)]).query(query);
        let (_, body) = self.send(Provider::AllDebrid, request).await?;
        if body.get("status").and_then(Value::as_str) == Some("success") {
            return Ok(body.get("data").cloned().unwrap_or(Value::Null));
        }
        let code = text(body.pointer("/error/code")).unwrap_or_default();
        if code.starts_with("AUTH_") {
            return Err(DebridError::Unauthorized(Provider::AllDebrid));
        }
        Err(DebridError::Failed(Provider::AllDebrid, if code.is_empty() { "?".into() } else { code }))
    }

    /// Premiumize: `{status: "success", ...}` or `{status: "error", message}`.
    fn pm_ok(body: Value) -> Result<Value, DebridError> {
        if body.get("status").and_then(Value::as_str) == Some("success") {
            return Ok(body);
        }
        let message = text(body.get("message")).unwrap_or_default();
        let lower = message.to_ascii_lowercase();
        if lower.contains("not logged") || lower.contains("apikey") || lower.contains("api key") || lower.contains("customer") {
            return Err(DebridError::Unauthorized(Provider::Premiumize));
        }
        if lower.contains("cache") {
            return Err(DebridError::NotCached(Provider::Premiumize));
        }
        Err(DebridError::Failed(Provider::Premiumize, if message.is_empty() { "?".into() } else { message }))
    }

    /// Debrid-Link: `{success: true, value}` or `{success: false, error}`.
    async fn dl(&self, request: reqwest::RequestBuilder, key: &str) -> Result<Value, DebridError> {
        let (_, body) = self.send(Provider::DebridLink, request.bearer_auth(key)).await?;
        if body.get("success").and_then(Value::as_bool) == Some(true) {
            return Ok(body.get("value").cloned().unwrap_or(Value::Null));
        }
        let error = text(body.get("error")).unwrap_or_default();
        if matches!(error.as_str(), "badToken" | "notDebrid" | "authorization_pending" | "accountLocked") {
            return Err(DebridError::Unauthorized(Provider::DebridLink));
        }
        Err(DebridError::Failed(Provider::DebridLink, if error.is_empty() { "?".into() } else { error }))
    }

    // ------------------------------------------------------------ account

    /// The account the key belongs to (e-mail or user name), which also validates the key.
    pub async fn account(&self, provider: Provider, key: &str) -> Result<String, DebridError> {
        let key = key.trim();
        match provider {
            Provider::RealDebrid => {
                let (_, body) = self.rd(self.http.get(self.url(provider, "/user")), key).await?;
                Ok(text(body.get("email")).or_else(|| text(body.get("username"))).unwrap_or_else(|| "conta Real-Debrid".into()))
            }
            Provider::AllDebrid => {
                let data = self.ad("/v4/user", key, &[]).await?;
                let user = data.get("user").unwrap_or(&data);
                Ok(text(user.get("email")).or_else(|| text(user.get("username"))).unwrap_or_else(|| "conta AllDebrid".into()))
            }
            Provider::Premiumize => {
                let (_, body) = self.send(provider, self.http.get(self.url(provider, "/account/info")).query(&[("apikey", key)])).await?;
                let body = Self::pm_ok(body)?;
                Ok(id_text(body.get("customer_id")).map(|id| format!("cliente {id}")).unwrap_or_else(|| "conta Premiumize".into()))
            }
            Provider::DebridLink => {
                let value = self.dl(self.http.get(self.url(provider, "/account/infos")), key).await?;
                Ok(text(value.get("email")).or_else(|| text(value.get("pseudo"))).unwrap_or_else(|| "conta Debrid-Link".into()))
            }
        }
    }

    // ------------------------------------------------------------ cache check

    /// Which of `hashes` the service has. Only Premiumize can answer (`can_check`); the others
    /// return an empty set and are tried when the source is opened.
    pub async fn check_cached(&self, provider: Provider, key: &str, hashes: &[String]) -> Result<HashSet<String>, DebridError> {
        if provider != Provider::Premiumize || hashes.is_empty() {
            return Ok(HashSet::new());
        }
        let mut found = HashSet::new();
        for batch in hashes.chunks(100) {
            let mut query: Vec<(&str, &str)> = vec![("apikey", key)];
            query.extend(batch.iter().map(|h| ("items[]", h.as_str())));
            let (_, body) = self.send(provider, self.http.get(self.url(provider, "/cache/check")).query(&query)).await?;
            let body = Self::pm_ok(body)?;
            let answers = body.get("response").and_then(Value::as_array).cloned().unwrap_or_default();
            for (hash, cached) in batch.iter().zip(answers) {
                if cached.as_bool() == Some(true) {
                    found.insert(hash.to_lowercase());
                }
            }
        }
        Ok(found)
    }

    // ------------------------------------------------------------ resolve

    /// The HTTP link of the wanted file, only when the service already has the torrent.
    pub async fn resolve(&self, provider: Provider, key: &str, wanted: &Wanted) -> Result<Resolved, DebridError> {
        let key = key.trim();
        match provider {
            Provider::RealDebrid => self.resolve_rd(key, wanted).await,
            Provider::AllDebrid => self.resolve_ad(key, wanted).await,
            Provider::Premiumize => self.resolve_pm(key, wanted).await,
            Provider::DebridLink => self.resolve_dl(key, wanted).await,
        }
    }

    async fn rd_info(&self, key: &str, id: &str) -> Result<Value, DebridError> {
        Ok(self.rd(self.http.get(self.url(Provider::RealDebrid, &format!("/torrents/info/{id}"))), key).await?.1)
    }

    async fn rd_delete(&self, key: &str, id: &str) {
        let _ = self.rd(self.http.delete(self.url(Provider::RealDebrid, &format!("/torrents/delete/{id}"))), key).await;
    }

    async fn resolve_rd(&self, key: &str, wanted: &Wanted) -> Result<Resolved, DebridError> {
        const P: Provider = Provider::RealDebrid;
        let (_, created) = self.rd(self.http.post(self.url(P, "/torrents/addMagnet")).form(&[("magnet", wanted.magnet.as_str())]), key).await?;
        let id = id_text(created.get("id")).ok_or(DebridError::NotCached(P))?;
        let result = async {
            // The magnet is converted first (a moment); then the file is chosen.
            let mut info = self.rd_info(key, &id).await?;
            for _ in 0..8 {
                if text(info.get("status")).as_deref() != Some("magnet_conversion") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(600)).await;
                info = self.rd_info(key, &id).await?;
            }
            let items = info.get("files").and_then(Value::as_array).cloned().unwrap_or_default();
            let (files, raw) = files_of(items.iter().map(|f| (text(f.get("path")).unwrap_or_default(), f.get("bytes").and_then(Value::as_u64), f)));
            let (file, value) = pick(P, &files, &raw, wanted)?;
            let file_id = id_text(value.get("id")).ok_or(DebridError::NoFile(P))?;
            self.rd(self.http.post(self.url(P, &format!("/torrents/selectFiles/{id}"))).form(&[("files", file_id.as_str())]), key).await?;
            // A torrent the service has is "downloaded" right after the selection.
            for attempt in 0..4 {
                info = self.rd_info(key, &id).await?;
                if text(info.get("status")).as_deref() == Some("downloaded") {
                    break;
                }
                if attempt < 3 {
                    tokio::time::sleep(Duration::from_millis(700)).await;
                }
            }
            if text(info.get("status")).as_deref() != Some("downloaded") {
                return Err(DebridError::NotCached(P));
            }
            let link = info.get("links").and_then(Value::as_array).and_then(|l| l.first()).and_then(Value::as_str).map(str::to_string).ok_or(DebridError::NoFile(P))?;
            let (_, unrestricted) = self.rd(self.http.post(self.url(P, "/unrestrict/link")).form(&[("link", link.as_str())]), key).await?;
            let url = text(unrestricted.get("download")).ok_or(DebridError::NoFile(P))?;
            Ok(Resolved { url, filename: Some(file.name.rsplit('/').next().unwrap_or(&file.name).to_string()), size: file.size })
        }
        .await;
        // Never leave a download the user did not ask for running in the account.
        if matches!(result, Err(DebridError::NotCached(_) | DebridError::NoFile(_))) {
            self.rd_delete(key, &id).await;
        }
        result
    }

    async fn resolve_ad(&self, key: &str, wanted: &Wanted) -> Result<Resolved, DebridError> {
        const P: Provider = Provider::AllDebrid;
        let data = self.ad("/v4/magnet/upload", key, &[("magnets[]", wanted.magnet.as_str())]).await?;
        let magnet = data.get("magnets").and_then(Value::as_array).and_then(|m| m.first()).cloned().unwrap_or(Value::Null);
        if magnet.get("error").is_some() {
            return Err(DebridError::NotCached(P));
        }
        let id = id_text(magnet.get("id")).ok_or(DebridError::NotCached(P))?;
        let result = async {
            if magnet.get("ready").and_then(Value::as_bool) != Some(true) {
                return Err(DebridError::NotCached(P));
            }
            let files = self.ad("/v4/magnet/files", key, &[("id[]", id.as_str())]).await?;
            let nodes = files.pointer("/magnets/0/files").and_then(Value::as_array).cloned().unwrap_or_default();
            let mut flat = Vec::new();
            flatten_tree(&nodes, "", &mut flat);
            let (files, raw) = files_of(flat.into_iter());
            let (file, value) = pick(P, &files, &raw, wanted)?;
            let link = text(value.get("l")).ok_or(DebridError::NoFile(P))?;
            let unlocked = self.ad("/v4/link/unlock", key, &[("link", link.as_str())]).await?;
            let url = text(unlocked.get("link")).ok_or(DebridError::NoFile(P))?;
            Ok(Resolved { url, filename: Some(file.name.rsplit('/').next().unwrap_or(&file.name).to_string()), size: file.size })
        }
        .await;
        if matches!(result, Err(DebridError::NotCached(_) | DebridError::NoFile(_))) {
            let _ = self.ad("/v4/magnet/delete", key, &[("id", id.as_str())]).await;
        }
        result
    }

    async fn resolve_pm(&self, key: &str, wanted: &Wanted) -> Result<Resolved, DebridError> {
        const P: Provider = Provider::Premiumize;
        // `directdl` only answers from the cache: nothing is added to the account.
        let request = self.http.post(self.url(P, "/transfer/directdl")).query(&[("apikey", key)]).form(&[("src", wanted.magnet.as_str())]);
        let (_, body) = self.send(P, request).await?;
        let body = Self::pm_ok(body)?;
        let items = body.get("content").and_then(Value::as_array).cloned().unwrap_or_default();
        if items.is_empty() {
            return Err(DebridError::NotCached(P));
        }
        let (files, raw) = files_of(items.iter().map(|f| (text(f.get("path")).unwrap_or_default(), f.get("size").and_then(Value::as_u64), f)));
        let (file, value) = pick(P, &files, &raw, wanted)?;
        let url = text(value.get("link")).or_else(|| text(value.get("stream_link"))).ok_or(DebridError::NoFile(P))?;
        Ok(Resolved { url, filename: Some(file.name.rsplit('/').next().unwrap_or(&file.name).to_string()), size: file.size })
    }

    async fn resolve_dl(&self, key: &str, wanted: &Wanted) -> Result<Resolved, DebridError> {
        const P: Provider = Provider::DebridLink;
        let added = self
            .dl(self.http.post(self.url(P, "/seedbox/add")).json(&serde_json::json!({ "url": wanted.magnet, "async": true })), key)
            .await?;
        let id = id_text(added.get("id")).ok_or(DebridError::NotCached(P))?;
        let result = async {
            let items = added.get("files").and_then(Value::as_array).cloned().unwrap_or_default();
            let (files, raw) = files_of(items.iter().map(|f| (text(f.get("name")).unwrap_or_default(), f.get("size").and_then(Value::as_u64), f)));
            let (file, value) = pick(P, &files, &raw, wanted)?;
            if value.get("downloadPercent").and_then(Value::as_f64).unwrap_or(0.0) < 100.0 {
                return Err(DebridError::NotCached(P));
            }
            let url = text(value.get("downloadUrl")).ok_or(DebridError::NoFile(P))?;
            Ok(Resolved { url, filename: Some(file.name.clone()), size: file.size })
        }
        .await;
        if matches!(result, Err(DebridError::NotCached(_) | DebridError::NoFile(_))) {
            let _ = self.dl(self.http.delete(self.url(P, &format!("/seedbox/{id}/remove"))), key).await;
        }
        result
    }
}

/// The service of an addon's own debrid source (`clientResolve` with `type: "debrid"` and
/// `isCached: true`), e.g. `"realdebrid"`; TorBox ones are `torbox::is_torbox_client_stream`.
pub fn client_stream_provider(stream: &StreamItem) -> Option<Provider> {
    let resolve = stream.client_resolve.as_ref()?;
    let kind = resolve.get("type").and_then(Value::as_str)?;
    if !kind.eq_ignore_ascii_case("debrid") || resolve.get("isCached").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    Provider::parse(resolve.get("service").and_then(Value::as_str)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn providers_parse_their_ids_and_addon_names() {
        for p in Provider::ALL {
            assert_eq!(Provider::parse(p.id()), Some(p));
        }
        assert_eq!(Provider::parse("real-debrid"), Some(Provider::RealDebrid));
        assert_eq!(Provider::parse("Debrid_Link"), Some(Provider::DebridLink));
        assert_eq!(Provider::parse("torbox"), None);
        assert!(Provider::Premiumize.can_check() && !Provider::RealDebrid.can_check());
    }

    #[test]
    fn alldebrid_file_tree_is_flattened_with_paths() {
        let tree = json!([
            {"n": "Show S01", "e": [{"n": "Show.S01E01.mkv", "s": 100, "l": "https://a/1"}, {"n": "Show.S01E02.mkv", "s": 120, "l": "https://a/2"}]},
            {"n": "readme.txt", "s": 1, "l": "https://a/3"}
        ]);
        let mut flat = Vec::new();
        flatten_tree(tree.as_array().unwrap(), "", &mut flat);
        let names: Vec<&str> = flat.iter().map(|(n, _, _)| n.as_str()).collect();
        assert_eq!(names, ["Show S01/Show.S01E01.mkv", "Show S01/Show.S01E02.mkv", "readme.txt"]);
        let (files, raw) = files_of(flat.into_iter());
        let wanted = Wanted { season: Some(1), episode: Some(2), ..Default::default() };
        let (file, value) = pick(Provider::AllDebrid, &files, &raw, &wanted).unwrap();
        assert_eq!(file.name, "Show S01/Show.S01E02.mkv");
        assert_eq!(value.get("l").and_then(Value::as_str), Some("https://a/2"));
    }

    #[test]
    fn premiumize_entries_without_ids_are_picked_by_position() {
        let content = json!([{"path": "M/sample.mkv", "size": 5, "link": "https://p/s"}, {"path": "M/Movie.2024.mkv", "size": 900, "link": "https://p/m"}]);
        let items = content.as_array().unwrap();
        let (files, raw) = files_of(items.iter().map(|f| (text(f.get("path")).unwrap_or_default(), f.get("size").and_then(Value::as_u64), f)));
        let (_, value) = pick(Provider::Premiumize, &files, &raw, &Wanted::default()).unwrap();
        assert_eq!(value.get("link").and_then(Value::as_str), Some("https://p/m"), "largest video");
    }

    #[test]
    fn addon_debrid_streams_name_their_service() {
        let streams = crate::stream::parse_streams(
            r#"{"streams":[
                {"clientResolve":{"type":"debrid","service":"realdebrid","isCached":true,"infoHash":"1111111111111111111111111111111111111111"}},
                {"clientResolve":{"type":"debrid","service":"alldebrid","isCached":false,"infoHash":"1111111111111111111111111111111111111111"}},
                {"clientResolve":{"type":"debrid","service":"torbox","isCached":true,"infoHash":"1111111111111111111111111111111111111111"}}
            ]}"#,
            "a",
            "b",
            None,
        )
        .unwrap();
        assert_eq!(client_stream_provider(&streams[0]), Some(Provider::RealDebrid));
        assert_eq!(client_stream_provider(&streams[1]), None, "not cached at the service");
        assert_eq!(client_stream_provider(&streams[2]), None, "TorBox has its own path");
    }
}

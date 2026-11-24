//! Live compatibility gate: fetch a real Nuvio scraper repository, run its scripts in the sandbox
//! and print what they returned. No app state, no debrid keys, no account.
//!
//!   cargo run -p nuvio-plugins --example live_check -- \
//!       https://raw.githubusercontent.com/michat88/nuvio-providers/main/manifest.json 550 movie 3

use nuvio_core::client::AddonClient;
use nuvio_plugins::manifest::{parse_manifest, resolve_code_url};
use nuvio_plugins::runtime::{get_streams, validate_module, PluginCall};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let repo = args
        .get(1)
        .map(String::as_str)
        .unwrap_or("https://raw.githubusercontent.com/michat88/nuvio-providers/main/manifest.json");
    let tmdb_id = args.get(2).map(String::as_str).unwrap_or("550");
    let media_type = args.get(3).map(String::as_str).unwrap_or("movie");
    let limit: usize = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(3);
    let filter = args.get(5).map(String::as_str);

    let client = AddonClient::new().map_err(|e| e.to_string())?;
    let payload = client.get_text(repo, true).await.map_err(|e| e.to_string())?;
    let manifest = parse_manifest(&payload).map_err(|e| e.to_string())?;
    println!("repo: {} v{} — {} scrapers", manifest.name, manifest.version, manifest.scrapers.len());

    for entry in manifest.scrapers.iter().filter(|e| filter.is_none_or(|f| e.id.contains(f))).take(limit) {
        let code_url = resolve_code_url(repo, &entry.filename).map_err(|e| e.to_string())?;
        let code = match client.get_text(&code_url, true).await {
            Ok(code) => code.to_string(),
            Err(e) => {
                println!("[{}] download failed: {e}", entry.id);
                continue;
            }
        };
        if let Err(e) = validate_module(&code).await {
            println!("[{}] rejected by sandbox: {e}", entry.id);
            continue;
        }
        let call = PluginCall {
            tmdb_id: tmdb_id.to_string(),
            media_type: media_type.to_string(),
            season: None,
            episode: None,
            scraper_id: entry.id.clone(),
            settings: json!({}),
            tmdb_api_key: String::new(),
        };
        let output = match get_streams(&code, &call).await {
            Ok(output) => output,
            Err(e) => {
                println!("[{}] runtime error: {e}", entry.id);
                continue;
            }
        };
        println!("[{}] {} streams", entry.id, output.results.len());
        for stream in output.results.iter().take(3) {
            println!("    · {} | {} | {}", stream.title, stream.quality.as_deref().unwrap_or("-"), stream.url);
        }
        if !output.log.is_empty() {
            println!("    log: {}", output.log.lines().take(3).collect::<Vec<_>>().join(" / "));
        }
    }
    Ok(())
}

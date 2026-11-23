//! Diagnostic: verify the Linux engine ABI loads and can be created (no network use).
#[cfg(not(windows))]
fn main() {
    let path = nuvio_p2p::locate_engine().expect("engine not found");
    println!("engine: {}", path.display());
    let api = nuvio_p2p::load_api(&path).expect("load API");
    println!("api version: {}", unsafe { (api.api_version)() });
    let dir = std::env::temp_dir().join(format!("nuvio-p2p-probe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let config = nuvio_p2p::EngineConfig {
        data_dir: dir.join("data"),
        cache_dir: dir.join("cache"),
        disk_cache_bytes: 256 * 1024 * 1024,
        upload: false,
        profile: 1,
    };
    let engine = nuvio_p2p::Engine::create(api, &config).expect("create engine");
    println!("version: {}", engine.version());
    println!("stats: {:?}", engine.stats().map(|s| (s.active_torrents, s.connected_peers)));
}

#[cfg(windows)]
fn main() {
    println!("Use the Windows build for the Windows engine.");
}

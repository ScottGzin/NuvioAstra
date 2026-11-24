//! Build script: Tauri resources, and the Nuvio account server of this build.
//!
//! Like the original (`runtimeConfigValue` in composeApp/build.gradle.kts), the server comes from
//! `NUVIO_SUPABASE_URL`, `NUVIO_SUPABASE_ANON_KEY` and `NUVIO_SUPABASE_FALLBACK_URL`: environment
//! variables first, else the repository's `local.properties` (gitignored). Only a public key (role
//! `anon` / `sb_publishable_`) is accepted: a service key makes the build fail. Without values the app
//! is built without account features.
//!
//! `NUVIO_UPDATE_REPOSITORY` (`owner/repo`, same sources) selects the GitHub releases the built app
//! follows for automatic updates; without it the build has updates off.
//!
//! `local.properties` is looked up in this repository's root (`Nuvio Astra` repository layout) and
//! one folder above it (development tree next to the original app).

fn main() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let candidates = [manifest.join("../../local.properties"), manifest.join("../../../local.properties")];
    for props in &candidates {
        println!("cargo:rerun-if-changed={}", props.display());
    }
    for key in [
        "NUVIO_SUPABASE_URL", "NUVIO_SUPABASE_ANON_KEY", "NUVIO_SUPABASE_FALLBACK_URL", "NUVIO_UPDATE_REPOSITORY",
        "TRAKT_CLIENT_ID", "TRAKT_CLIENT_SECRET", "SIMKL_CLIENT_ID", "MDBLIST_CLIENT_ID",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    let file = candidates.iter().find_map(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let value = |key: &str| -> String {
        std::env::var(key).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).unwrap_or_else(|| {
            file.lines()
                .map(|l| l.trim_start_matches('\u{feff}').trim())
                .filter(|l| !l.starts_with('#'))
                .find_map(|l| l.split_once('=').filter(|(k, _)| k.trim() == key).map(|(_, v)| v.trim().to_string()))
                .unwrap_or_default()
        })
    };
    let url = value("NUVIO_SUPABASE_URL");
    let key = value("NUVIO_SUPABASE_ANON_KEY");
    let fallback = value("NUVIO_SUPABASE_FALLBACK_URL");
    if !url.is_empty() && !key.is_empty() {
        if !is_public_key(&key) {
            panic!("NUVIO_SUPABASE_ANON_KEY is not a public key (role anon / sb_publishable_): refusing to embed it");
        }
        println!("cargo:rustc-env=NUVIO_SUPABASE_URL={url}");
        println!("cargo:rustc-env=NUVIO_SUPABASE_ANON_KEY={key}");
        println!("cargo:rustc-env=NUVIO_SUPABASE_FALLBACK_URL={fallback}");
    }
    // Optional Trakt/Simkl OAuth app credentials from the same gitignored local.properties. They
    // are compile-time defaults only: a user can still enter their own in Settings, and no value is
    // ever embedded in a committed source file.
    for (key, env_name) in [
        ("TRAKT_CLIENT_ID", "NUVIO_TRAKT_CLIENT_ID"),
        ("TRAKT_CLIENT_SECRET", "NUVIO_TRAKT_CLIENT_SECRET"),
        ("SIMKL_CLIENT_ID", "NUVIO_SIMKL_CLIENT_ID"),
        ("MDBLIST_CLIENT_ID", "NUVIO_MDBLIST_CLIENT_ID"),
    ] {
        let from_file = value(key);
        if !from_file.is_empty() {
            println!("cargo:rustc-env={env_name}={from_file}");
        }
    }
    let repository = value("NUVIO_UPDATE_REPOSITORY");
    if !repository.is_empty() {
        let ok = repository.split('/').count() == 2
            && repository.chars().all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c));
        if !ok {
            panic!("NUVIO_UPDATE_REPOSITORY must be `owner/repo`");
        }
        println!("cargo:rustc-env=NUVIO_UPDATE_REPOSITORY={repository}");
    }
    tauri_build::build()
}

fn is_public_key(key: &str) -> bool {
    if key.starts_with("sb_publishable_") {
        return true;
    }
    let Some(payload) = key.split('.').nth(1) else { return false };
    let mut bits = 0u32;
    let mut n = 0;
    let mut out = Vec::new();
    for c in payload.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => continue,
        } as u32;
        bits = (bits << 6) | v;
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
            bits &= (1 << n) - 1;
        }
    }
    let text = String::from_utf8_lossy(&out);
    text.contains("\"role\":\"anon\"")
}

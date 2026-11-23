//! End-to-end header check: plays `<url>` (the local header proxy) headless with the exact header
//! formatting used in production (`player::http_header_lines` as a node array) and reports whether a frame was
//! decoded. The proxy answers 403 unless every expected header arrives intact.
//!
//! Usage: NUVIO_LIBMPV=<dll> cargo run --release -p nuvio-player --example header_check -- <url> [no-headers]

use nuvio_player::mpv::*;
#[cfg(windows)]
use nuvio_player::player::http_header_lines;
use std::time::{Duration, Instant};

#[cfg(not(windows))]
fn http_header_lines(headers: &[(String, String)]) -> Vec<String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            let (name, value) = (name.trim(), value.trim());
            (!name.is_empty() && !value.is_empty()).then(|| format!("{name}: {value}"))
        })
        .collect()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let url = args.next().expect("url");
    let send_headers = args.next().as_deref() != Some("no-headers");
    let headers = vec![
        ("Referer".to_string(), "https://nuvio.test/referer".to_string()),
        ("Cookie".to_string(), "session=abc123; region=br".to_string()),
        ("User-Agent".to_string(), "NuvioNextGen-HeaderTest/1.0".to_string()),
        ("X-Nuvio-Token".to_string(), "token,with\\specials".to_string()),
        ("Range".to_string(), "".to_string()), // blank values are skipped like the original
    ];
    let api = MpvApi::load(std::path::Path::new(&std::env::var("NUVIO_LIBMPV").expect("NUVIO_LIBMPV"))).expect("libmpv");
    let mpv = Mpv::create(api).expect("create");
    for (k, v) in [("config", "no"), ("terminal", "no"), ("vo", "null"), ("ao", "null"), ("hwdec", "no")] {
        mpv.set_option(k, v).expect(k);
    }
    if send_headers {
        mpv.set_option_string_list("http-header-fields", &http_header_lines(&headers)).expect("headers");
    }
    mpv.initialize().expect("init");
    let start = Instant::now();
    mpv.command(&["loadfile", &url]).expect("loadfile");
    while start.elapsed() < Duration::from_secs(30) {
        // SAFETY: single waiting thread.
        let event = unsafe { &*mpv.wait_event(0.05) };
        if event.event_id == EVENT_END_FILE {
            println!("headers sent: {send_headers} -> FAILED to play (end-file)");
            return;
        }
        if mpv.get_f64("time-pos").is_some_and(|p| p > 0.0) {
            println!("headers sent: {send_headers} -> PLAYED (first frame after {} ms)", start.elapsed().as_millis());
            return;
        }
    }
    println!("headers sent: {send_headers} -> timeout");
}

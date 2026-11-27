//! Auto sync against an embedded subtitle, end to end over HTTP: a Matroska file laid out like
//! mkvmerge writes it (SeekHead, Info, Tracks, ~2 s Clusters with a video SimpleBlock and the
//! subtitle in BlockGroups with BlockDuration, Cues at the end), ~140 MB, served with ranges by a
//! local server. The embedded track is read with ranged requests (`embedded_subs`) and an addon
//! subtitle 2.3 s late is aligned against it (`autosync`).

use nuvio_core::autosync::{compute_alignment_v2, parse_cues};
use nuvio_core::embedded_subs::load_embedded_tracks;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

fn vint_size(n: u64) -> Vec<u8> {
    for len in 1..=8u32 {
        let max = (1u64 << (7 * len)) - 2;
        if n <= max {
            let mut out = vec![0u8; len as usize];
            let marked = n | (1u64 << (7 * len));
            for i in 0..len as usize {
                out[i] = (marked >> (8 * (len as usize - 1 - i))) as u8;
            }
            return out;
        }
    }
    panic!("too big")
}

fn el(id: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    out.extend(vint_size(payload.len() as u64));
    out.extend_from_slice(payload);
    out
}

fn uint(id: &[u8], v: u64) -> Vec<u8> {
    let bytes = v.to_be_bytes();
    let first = bytes.iter().position(|b| *b != 0).unwrap_or(7);
    el(id, &bytes[first..])
}

fn text(id: &[u8], v: &str) -> Vec<u8> {
    el(id, v.as_bytes())
}

struct Cue {
    start_ms: u64,
    end_ms: u64,
    text: String,
}

fn cues() -> Vec<Cue> {
    // 45 minutes, a line roughly every 6.5 s with varying lengths (speech-like gaps).
    let mut out = Vec::new();
    let mut t = 4_000u64;
    let mut i = 0u64;
    while t < 45 * 60 * 1000 {
        let len = 1_200 + (i * 37 % 2_600);
        out.push(Cue { start_ms: t, end_ms: t + len, text: format!("Linha {i}") });
        t += len + 900 + (i * 53 % 5_400);
        i += 1;
    }
    out
}

fn build_mkv(cues: &[Cue]) -> Vec<u8> {
    let ebml = el(&[0x1A, 0x45, 0xDF, 0xA3], &[uint(&[0x42, 0x86], 1), uint(&[0x42, 0xF7], 1), uint(&[0x42, 0xF2], 4), uint(&[0x42, 0xF3], 8), text(&[0x42, 0x82], "matroska"), uint(&[0x42, 0x87], 4), uint(&[0x42, 0x85], 2)].concat());
    let info = el(&[0x15, 0x49, 0xA9, 0x66], &[uint(&[0x2A, 0xD7, 0xB1], 1_000_000), el(&[0x44, 0x89], &(45.0f64 * 60_000.0).to_be_bytes()), text(&[0x4D, 0x80], "test"), text(&[0x57, 0x41], "mkvmerge-like")].concat());
    let video = el(&[0xAE], &[uint(&[0xD7], 1), uint(&[0x73, 0xC5], 11), uint(&[0x83], 1), text(&[0x86], "V_MPEG4/ISO/AVC"), el(&[0xE0], &[uint(&[0xB0], 1920), uint(&[0xBA], 1080)].concat())].concat());
    let sub = el(&[0xAE], &[uint(&[0xD7], 2), uint(&[0x73, 0xC5], 22), uint(&[0x83], 0x11), text(&[0x86], "S_TEXT/UTF8"), text(&[0x22, 0xB5, 0x9C], "eng")].concat());
    let tracks = el(&[0x16, 0x54, 0xAE, 0x6B], &[video, sub].concat());

    // Clusters every 2 s: one big video frame + the subtitle lines that start in it.
    let frame = vec![0x42u8; 100_000];
    let mut clusters: Vec<(u64, Vec<u8>)> = Vec::new();
    let total_ms = 45 * 60 * 1000u64;
    let mut at = 0u64;
    while at < total_ms {
        let mut body = uint(&[0xE7], at);
        let mut sb = vec![0x81, 0, 0, 0x80];
        sb.extend_from_slice(&frame);
        body.extend(el(&[0xA3], &sb));
        for cue in cues.iter().filter(|c| c.start_ms >= at && c.start_ms < at + 2_000) {
            let rel = (cue.start_ms - at) as i16;
            let mut block = vec![0x82];
            block.extend_from_slice(&rel.to_be_bytes());
            block.push(0x00);
            block.extend_from_slice(cue.text.as_bytes());
            body.extend(el(&[0xA0], &[el(&[0xA1], &block), uint(&[0x9B], cue.end_ms - cue.start_ms)].concat()));
        }
        clusters.push((at, el(&[0x1F, 0x43, 0xB6, 0x75], &body)));
        at += 2_000;
    }

    // Segment payload: SeekHead (fixed size), Info, Tracks, Clusters, Cues.
    let seek = |id: &[u8], pos: u64| el(&[0x4D, 0xBB], &[el(&[0x53, 0xAB], id), el(&[0x53, 0xAC], &pos.to_be_bytes())].concat());
    let seekhead_len = el(&[0x11, 0x4D, 0x9B, 0x74], &[seek(&[0x15, 0x49, 0xA9, 0x66], 0), seek(&[0x16, 0x54, 0xAE, 0x6B], 0), seek(&[0x1C, 0x53, 0xBB, 0x6B], 0)].concat()).len() as u64;
    let info_pos = seekhead_len;
    let tracks_pos = info_pos + info.len() as u64;
    let mut pos = tracks_pos + tracks.len() as u64;
    // Like mkvmerge: a CuePoint per video keyframe and per subtitle frame (with CueDuration).
    let mut points: Vec<(u64, u64, Vec<u8>)> = Vec::new();
    for (time, cluster) in &clusters {
        points.push((*time, 1, el(&[0xBB], &[uint(&[0xB3], *time), el(&[0xB7], &[uint(&[0xF7], 1), uint(&[0xF1], pos)].concat())].concat())));
        for cue in cues.iter().filter(|c| c.start_ms >= *time && c.start_ms < *time + 2_000) {
            let track = el(&[0xB7], &[uint(&[0xF7], 2), uint(&[0xF1], pos), uint(&[0xF0], 100_020), uint(&[0xB2], cue.end_ms - cue.start_ms)].concat());
            points.push((cue.start_ms, 2, el(&[0xBB], &[uint(&[0xB3], cue.start_ms), track].concat())));
        }
        pos += cluster.len() as u64;
    }
    points.sort_by_key(|(t, track, _)| (*t, *track));
    let cue_points: Vec<u8> = points.into_iter().flat_map(|(_, _, b)| b).collect();
    let cues_el = el(&[0x1C, 0x53, 0xBB, 0x6B], &cue_points);
    let seekhead = el(&[0x11, 0x4D, 0x9B, 0x74], &[seek(&[0x15, 0x49, 0xA9, 0x66], info_pos), seek(&[0x16, 0x54, 0xAE, 0x6B], tracks_pos), seek(&[0x1C, 0x53, 0xBB, 0x6B], pos)].concat());
    assert_eq!(seekhead.len() as u64, seekhead_len);
    let mut segment = Vec::new();
    segment.extend(seekhead);
    segment.extend(info);
    segment.extend(tracks);
    for (_, c) in clusters {
        segment.extend(c);
    }
    segment.extend(cues_el);
    let mut file = ebml;
    file.extend(el(&[0x18, 0x53, 0x80, 0x67], &segment));
    file
}

/// Minimal HTTP/1.1 file server with `Range` support (one request per connection).
fn serve(file: Arc<Vec<u8>>, served: Arc<AtomicU64>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let file = file.clone();
            let served = served.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut range: Option<(u64, Option<u64>)> = None;
                let mut head = false;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    if line.starts_with("HEAD") {
                        head = true;
                    }
                    let lower = line.to_ascii_lowercase();
                    if let Some(v) = lower.strip_prefix("range: bytes=") {
                        let (a, b) = v.trim().split_once('-').unwrap();
                        range = Some((a.parse().unwrap(), b.parse().ok()));
                    }
                    if line == "\r\n" {
                        break;
                    }
                }
                let len = file.len() as u64;
                let mut out = stream;
                let (start, end, status) = match range {
                    Some((a, b)) => (a.min(len), b.unwrap_or(len - 1).min(len - 1), "206 Partial Content"),
                    None => (0, len - 1, "200 OK"),
                };
                let body = &file[start as usize..=end as usize];
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: video/x-matroska\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{len}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = out.write_all(header.as_bytes());
                if !head {
                    served.fetch_add(body.len() as u64, Ordering::Relaxed);
                    let _ = out.write_all(body);
                }
            });
        }
    });
    format!("http://{addr}/movie.mkv")
}

#[tokio::test(flavor = "multi_thread")]
async fn embedded_subtitle_aligns_a_late_addon_subtitle() {
    let cues = cues();
    let file = Arc::new(build_mkv(&cues));
    let served = Arc::new(AtomicU64::new(0));
    let url = serve(file.clone(), served.clone());
    println!("arquivo: {} MB, {} falas", file.len() / 1_000_000, cues.len());

    let tracks = load_embedded_tracks(&url, &BTreeMap::new()).await.expect("embedded tracks");
    for t in &tracks {
        println!("faixa lida: {} {:?} {} falas, {} MB baixados", t.id, t.language, t.cues.len(), served.load(Ordering::Relaxed) / 1_000_000);
    }
    let track = tracks.iter().find(|t| t.cues.len() >= 4).expect("a subtitle track with cues");
    println!("faixa {} ({:?}): {} falas lidas, {} MB baixados", track.id, track.language, track.cues.len(), served.load(Ordering::Relaxed) / 1_000_000);
    assert_eq!(track.language.as_deref(), Some("eng"));
    assert_eq!(track.cues.len(), cues.len(), "every line, from the index");
    assert_eq!(track.cues[0].start_ms, cues[0].start_ms as i64);
    assert_eq!(track.cues[5].end_ms, cues[5].end_ms as i64, "durations from CueDuration");
    assert!(served.load(Ordering::Relaxed) < 2_000_000, "only the index is read, not the clusters");

    // The addon subtitle: the same lines, 2.3 s late.
    let mut srt = String::new();
    let fmt = |ms: u64| format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    for (i, c) in cues.iter().enumerate() {
        srt.push_str(&format!("{}\n{} --> {}\n{}\n\n", i + 1, fmt(c.start_ms + 2_300), fmt(c.end_ms + 2_300), c.text));
    }
    let target = parse_cues(&srt, "addon.srt");
    let alignment = compute_alignment_v2(&target, &[track.cues.clone()]).expect("an alignment");
    println!("atraso encontrado: {} ms", alignment.offset_ms);
    assert!((alignment.offset_ms + 2_300).abs() <= 150, "offset {}", alignment.offset_ms);
}

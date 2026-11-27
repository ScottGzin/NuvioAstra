//! Player-independent embedded subtitle timeline reading for auto-sync.
//!
//! This module reads embedded subtitle tracks directly from seekable HTTP(S) sources with small
//! ranged requests: MP4/MOV sample tables (`moov` → `trak` → `mdia` → `stbl`) and Matroska/WebM
//! EBML metadata plus `Cluster`/`SimpleBlock` payloads. It is deliberately best-effort and bounded:
//! malformed containers yield empty results rather than errors or panics, and both the per-range
//! and the cumulative download size are capped.
//!
//! The pure parsing entry points ([`parse_mp4_from_bytes`], [`parse_mkv_from_bytes`],
//! [`parse_mkv_document`]) and the cue builders ([`mp4_cues_from_samples`],
//! [`mkv_cues_from_blocks`]) are public so they can be unit tested without network access.

use crate::autosync::{SyncCue, parse_cues};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::collections::BTreeMap;
use std::time::Duration;

const MAX_RANGE_BYTES: usize = 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024;
const INITIAL_PROBE_BYTES: usize = 512 * 1024;
const MAX_MOOV_BYTES: usize = 24 * 1024 * 1024;
const CHUNK_ALIGN: u64 = 64 * 1024;
const MAX_CACHE_CHUNKS: usize = 48;
const MAX_SAMPLES: usize = 200_000;
const MAX_BLOCKS: usize = 200_000;
const MAX_BOXES: usize = 512;
const MAX_EBML_CHILDREN: usize = 100_000;
const MAX_CLUSTERS_PER_WINDOW: usize = 4_096;
const MAX_MKV_WINDOWS: usize = 32;
const DEFAULT_MKV_SCALE_NS: u64 = 1_000_000;
const DEFAULT_CUE_DURATION_MS: i64 = 5_000;
const MAX_ESTIMATED_DURATION_MS: i64 = 4_000;
const LAST_CUE_DURATION_MS: i64 = 2_000;

// ---------------------------------------------------------------- public API

/// One embedded subtitle track and the cues recovered from its samples/blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedTrack {
    /// Stable identifier, e.g. `mp4:3` or `mkv:2`.
    pub id: String,
    /// ISO 639 language code when the container declares one.
    pub language: Option<String>,
    /// Track kind; currently always `"sub"`.
    pub kind: String,
    /// Recovered cues, sorted by start time.
    pub cues: Vec<SyncCue>,
}

/// Timing and file location of one MP4 subtitle sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mp4SampleInfo {
    pub start_ms: i64,
    pub end_ms: i64,
    /// Absolute offset of the sample payload in the file.
    pub offset: u64,
    pub size: usize,
}

/// A parsed MP4 subtitle(text) track, including its sample timing table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mp4TrackInfo {
    pub track_id: u32,
    pub language: Option<String>,
    pub handler: String,
    pub codec: String,
    pub timescale: u64,
    pub samples: Vec<Mp4SampleInfo>,
    /// 1-based sync sample numbers from `stss` (when present).
    pub sync_samples: Vec<u32>,
}

/// A Matroska subtitle track entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MkvTrackInfo {
    pub number: u64,
    pub codec_id: String,
    pub language: Option<String>,
    pub name: Option<String>,
}

/// One subtitle text block from a Matroska `Cluster`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MkvBlockInfo {
    pub track_number: u64,
    /// Timecode in `TimecodeScale` ticks, already relative to the segment start.
    pub time_ticks: u64,
    pub duration_ticks: Option<u64>,
    /// Raw (possibly WebVTT/SRT/ASS) frame payload.
    pub payload: Vec<u8>,
}

/// The subset of a Matroska document needed for subtitle timing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MkvDocumentInfo {
    /// Nanoseconds per timecode tick (defaults to 1,000,000).
    pub timecode_scale_ns: u64,
    pub tracks: Vec<MkvTrackInfo>,
    pub blocks: Vec<MkvBlockInfo>,
}

/// Loads embedded subtitle tracks from an HTTP(S) video URL using ranged requests.
///
/// Caller headers are forwarded (excluding hop-by-hop/`Range` headers). The initial probe is at
/// most 512 KiB; subsequent ranges are at most 1 MiB each with a 32 MiB cumulative budget.
pub async fn load_embedded_tracks(
    url: &str,
    headers: &BTreeMap<String, String>,
) -> Result<Vec<EmbeddedTrack>, String> {
    let mut store = ByteStore::new(url, headers).await?;
    let initial = store.initial().to_vec();

    if looks_like_mkv(&initial) {
        load_mkv(&mut store).await
    } else if looks_like_mp4(&initial) {
        load_mp4(&mut store).await
    } else {
        Err("embedded subs: unsupported container (no EBML Segment or MP4 moov found)".to_string())
    }
}

/// Parses subtitle(text) tracks out of a complete `moov` box (or a whole small MP4 file).
pub fn parse_mp4_from_bytes(bytes: &[u8]) -> Vec<Mp4TrackInfo> {
    let Some(moov) = find_moov(bytes) else {
        return Vec::new();
    };
    let children = box_children(bytes, &moov, MAX_BOXES);
    let movie_timescale = child_slice(bytes, &children, *b"mvhd")
        .and_then(parse_mvhd_timescale)
        .unwrap_or(1_000)
        .max(1);

    let mut tracks = Vec::new();
    for trak in children.iter().filter(|b| b.typ == *b"trak") {
        if let Some(track) = parse_mp4_track(bytes, *trak, movie_timescale) {
            tracks.push(track);
        }
    }
    tracks
}

/// Parses Matroska subtitle tracks, timecode scale and text blocks out of an in-memory byte slice.
pub fn parse_mkv_from_bytes(bytes: &[u8]) -> Vec<MkvTrackInfo> {
    parse_mkv_document(bytes).tracks
}

/// Full (bounded) Matroska parse used by tests and by the network loader.
pub fn parse_mkv_document(bytes: &[u8]) -> MkvDocumentInfo {
    let mut result = MkvDocumentInfo {
        timecode_scale_ns: DEFAULT_MKV_SCALE_NS,
        tracks: Vec::new(),
        blocks: Vec::new(),
    };
    let mut pos = 0usize;
    let mut top = 0usize;
    while pos + 4 <= bytes.len() && top < 32 {
        let Some(element) = read_element(bytes, pos, bytes.len()) else {
            break;
        };
        if element.id == ID_SEGMENT {
            parse_mkv_segment(bytes, &element, &mut result);
            break;
        }
        if element.data_end <= pos {
            break;
        }
        pos = element.data_end;
        top += 1;
    }
    result
}

/// Builds cues from already-downloaded MP4 sample payloads (aligned with `Mp4TrackInfo::samples`).
pub fn mp4_cues_from_samples(track: &Mp4TrackInfo, samples: &[Option<Vec<u8>>]) -> Vec<SyncCue> {
    let mut cues = Vec::new();
    for (index, sample) in track.samples.iter().enumerate() {
        let Some(data) = samples.get(index).and_then(|value| value.as_ref()) else {
            continue;
        };
        let Some(text) = sample_text(&track.codec, data, 0) else {
            continue;
        };
        cues.extend(cues_for_text(&text, sample.start_ms, sample.end_ms));
    }
    cues.sort_by_key(|cue| cue.start_ms);
    cues.dedup_by(|a, b| a.start_ms == b.start_ms && a.text == b.text);
    cues
}

/// Builds per-track embedded timelines from Matroska blocks.
pub fn mkv_cues_from_blocks(
    scale_ns: u64,
    tracks: &[MkvTrackInfo],
    blocks: &[MkvBlockInfo],
) -> Vec<EmbeddedTrack> {
    let scale = scale_ns.max(1) as i128;
    let mut result = Vec::new();
    for track in tracks {
        let mut items: Vec<&MkvBlockInfo> = blocks
            .iter()
            .filter(|block| block.track_number == track.number)
            .take(MAX_BLOCKS)
            .collect();
        items.sort_by_key(|block| block.time_ticks);
        items.dedup_by_key(|block| block.time_ticks);

        let mut cues = Vec::with_capacity(items.len());
        for (index, block) in items.iter().enumerate() {
            let start_ms = mul_div(block.time_ticks as i128, scale, 1_000_000);
            let end_ms = match block.duration_ticks {
                Some(duration) if duration > 0 => {
                    start_ms.saturating_add(mul_div(duration as i128, scale, 1_000_000))
                }
                _ => {
                    let next_ms = items
                        .get(index + 1)
                        .map(|next| mul_div(next.time_ticks as i128, scale, 1_000_000));
                    match next_ms {
                        Some(next) if next > start_ms => {
                            start_ms.saturating_add((next - start_ms).min(MAX_ESTIMATED_DURATION_MS))
                        }
                        _ => start_ms.saturating_add(LAST_CUE_DURATION_MS),
                    }
                }
            };
            let text = String::from_utf8_lossy(&block.payload).replace('\0', "");
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            cues.extend(cues_for_text(text, start_ms, end_ms.max(start_ms + 1)));
        }
        cues.sort_by_key(|cue| cue.start_ms);
        cues.dedup_by(|a, b| a.start_ms == b.start_ms && a.text == b.text);
        result.push(EmbeddedTrack {
            id: format!("mkv:{}", track.number),
            language: track.language.clone(),
            kind: "sub".to_string(),
            cues,
        });
    }
    result
}

/// Turns text into cues: explicit WebVTT/SRT/ASS timings are honoured, otherwise the container
/// sample timing is used.
pub fn cues_for_text(text: &str, start_ms: i64, end_ms: i64) -> Vec<SyncCue> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let parsed = parse_cues(trimmed, "embedded.vtt");
    if !parsed.is_empty() {
        return parsed;
    }
    vec![SyncCue {
        start_ms,
        end_ms: end_ms.max(start_ms + 1),
        text: trimmed.to_string(),
    }]
}

// -------------------------------------------------------------- small helpers

fn mul_div(value: i128, multiplier: i128, divisor: i128) -> i64 {
    if divisor == 0 {
        return 0;
    }
    let scaled = value.saturating_mul(multiplier) / divisor;
    scaled.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

fn u32be(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 4)?;
    Some(u32::from_be_bytes(slice.try_into().ok()?))
}

fn u64be(bytes: &[u8], at: usize) -> Option<u64> {
    let slice = bytes.get(at..at + 8)?;
    Some(u64::from_be_bytes(slice.try_into().ok()?))
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn skip(&mut self, count: usize) -> Option<()> {
        self.pos = self.pos.checked_add(count)?;
        (self.pos <= self.bytes.len()).then_some(())
    }

    fn u8(&mut self) -> Option<u8> {
        let value = *self.bytes.get(self.pos)?;
        self.pos += 1;
        Some(value)
    }

    fn u16(&mut self) -> Option<u16> {
        let value = u16::from_be_bytes(self.bytes.get(self.pos..self.pos + 2)?.try_into().ok()?);
        self.pos += 2;
        Some(value)
    }

    fn u32(&mut self) -> Option<u32> {
        let value = u32be(self.bytes, self.pos)?;
        self.pos += 4;
        Some(value)
    }

    fn i32(&mut self) -> Option<i32> {
        self.u32().map(|value| value as i32)
    }
}

// ------------------------------------------------------------------ MP4 boxes

#[derive(Clone, Copy)]
struct Mp4Box {
    typ: [u8; 4],
    start: usize,
    end: usize,
    header: usize,
}

impl Mp4Box {
    fn data(&self) -> usize {
        self.start + self.header
    }
}

fn read_box_header(bytes: &[u8], pos: usize, limit: usize) -> Option<([u8; 4], u64, usize)> {
    if limit > bytes.len() || pos.checked_add(8)? > limit {
        return None;
    }
    let size32 = u32be(bytes, pos)? as u64;
    let typ: [u8; 4] = bytes.get(pos + 4..pos + 8)?.try_into().ok()?;
    let (size, header) = match size32 {
        0 => (u64::MAX, 8),
        1 => {
            if pos.checked_add(16)? > limit {
                return None;
            }
            let high = u32be(bytes, pos + 8)? as u64;
            let low = u32be(bytes, pos + 12)? as u64;
            ((high << 32) | low, 16)
        }
        other => (other, 8),
    };
    if (size != u64::MAX && size < header as u64) || size > (u64::MAX - 8) {
        return None;
    }
    Some((typ, size, header))
}

fn read_box(bytes: &[u8], pos: usize, limit: usize) -> Option<Mp4Box> {
    let (typ, size, header) = read_box_header(bytes, pos, limit)?;
    let size = if size == u64::MAX {
        (limit - pos) as u64
    } else {
        size
    };
    let end = (pos as u64).checked_add(size)?;
    if end > limit as u64 || size < header as u64 {
        return None;
    }
    Some(Mp4Box {
        typ,
        start: pos,
        end: end as usize,
        header,
    })
}

fn box_children(bytes: &[u8], parent: &Mp4Box, max: usize) -> Vec<Mp4Box> {
    let mut result = Vec::new();
    let mut pos = parent.data();
    while pos < parent.end && result.len() < max {
        match read_box(bytes, pos, parent.end) {
            Some(box_) => {
                if box_.end <= pos {
                    break;
                }
                pos = box_.end;
                result.push(box_);
            }
            None => break,
        }
    }
    result
}

fn child_slice<'a>(bytes: &'a [u8], children: &[Mp4Box], typ: [u8; 4]) -> Option<&'a [u8]> {
    children
        .iter()
        .find(|box_| box_.typ == typ)
        .map(|box_| &bytes[box_.data()..box_.end])
}

fn is_mp4_container(typ: &[u8; 4]) -> bool {
    *typ == *b"moov"
        || *typ == *b"trak"
        || *typ == *b"mdia"
        || *typ == *b"minf"
        || *typ == *b"stbl"
        || *typ == *b"edts"
        || *typ == *b"udta"
        || *typ == *b"mvex"
        || *typ == *b"dinf"
}

fn find_moov(bytes: &[u8]) -> Option<Mp4Box> {
    let mut pos = 0usize;
    let mut count = 0usize;
    while pos + 8 <= bytes.len() && count < 64 {
        match read_box(bytes, pos, bytes.len()) {
            Some(box_) => {
                if box_.typ == *b"moov" {
                    return Some(box_);
                }
                if box_.end <= pos {
                    break;
                }
                pos = box_.end;
                count += 1;
            }
            None => break,
        }
    }
    find_moov_recursive(bytes, 0, bytes.len(), 0)
}

fn find_moov_recursive(bytes: &[u8], start: usize, limit: usize, depth: usize) -> Option<Mp4Box> {
    if depth > 6 || limit > bytes.len() {
        return None;
    }
    let mut pos = start;
    let mut count = 0usize;
    while pos + 8 <= limit && count < MAX_BOXES {
        let box_ = match read_box(bytes, pos, limit) {
            Some(box_) => box_,
            None => break,
        };
        if box_.typ == *b"moov" {
            return Some(box_);
        }
        if is_mp4_container(&box_.typ) {
            if let Some(found) = find_moov_recursive(bytes, box_.data(), box_.end, depth + 1) {
                return Some(found);
            }
        }
        if box_.end <= pos {
            break;
        }
        pos = box_.end;
        count += 1;
    }
    None
}

fn looks_like_mp4(bytes: &[u8]) -> bool {
    if find_moov(bytes).is_some() {
        return true;
    }
    let mut pos = 0usize;
    let mut count = 0usize;
    while pos + 8 <= bytes.len() && count < 16 {
        let Some((typ, size, _)) = read_box_header(bytes, pos, bytes.len()) else {
            break;
        };
        if typ == *b"ftyp"
            || typ == *b"moov"
            || typ == *b"mdat"
            || typ == *b"free"
            || typ == *b"skip"
            || typ == *b"wide"
            || typ == *b"styp"
        {
            return true;
        }
        if size == u64::MAX || size < 8 || pos as u64 > u64::MAX - size {
            break;
        }
        pos += size as usize;
        count += 1;
    }
    false
}

fn parse_mvhd_timescale(payload: &[u8]) -> Option<u64> {
    let mut cursor = Cursor::new(payload);
    let version = cursor.u32()? >> 24;
    cursor.skip(if version == 1 { 16 } else { 8 })?;
    cursor.u32().map(u64::from)
}

fn parse_tkhd_id(payload: &[u8]) -> Option<u32> {
    let mut cursor = Cursor::new(payload);
    let version = cursor.u32()? >> 24;
    cursor.skip(if version == 1 { 16 } else { 8 })?;
    cursor.u32()
}

fn parse_mdhd(payload: &[u8]) -> Option<(u64, Option<String>)> {
    let mut cursor = Cursor::new(payload);
    let version = cursor.u32()? >> 24;
    cursor.skip(if version == 1 { 16 } else { 8 })?;
    let timescale = cursor.u32()? as u64;
    cursor.skip(if version == 1 { 8 } else { 4 })?;
    let packed = cursor.u16()?;
    let letters = [
        (((packed >> 10) & 0x1f) as u8).wrapping_add(0x60),
        (((packed >> 5) & 0x1f) as u8).wrapping_add(0x60),
        ((packed & 0x1f) as u8).wrapping_add(0x60),
    ];
    let language = letters.iter().all(|byte| byte.is_ascii_lowercase());
    let language = language
        .then(|| String::from_utf8_lossy(&letters).into_owned())
        .filter(|text| text != "und");
    Some((timescale.max(1), language))
}

fn parse_hdlr(payload: &[u8]) -> Option<String> {
    let fourcc = payload.get(8..12)?;
    Some(
        fourcc
            .iter()
            .map(|&byte| if byte.is_ascii_graphic() { byte as char } else { '.' })
            .collect(),
    )
}

fn is_text_handler(handler: &str) -> bool {
    matches!(handler, "sbtl" | "text" | "subp" | "subt" | "clcp")
}

fn is_text_codec(codec: &str) -> bool {
    matches!(
        codec,
        "tx3g" | "wvtt" | "TTML" | "stpp" | "c608" | "mp4s" | "sbtl"
    )
}

fn parse_stsd_codec(payload: &[u8]) -> Option<String> {
    let mut cursor = Cursor::new(payload);
    cursor.skip(4)?; // version + flags
    let count = cursor.u32()? as usize;
    let mut last = None;
    for _ in 0..count.min(MAX_BOXES) {
        let entry_start = cursor.pos;
        let size = cursor.u32()? as usize;
        let typ = cursor.u32()?;
        if size < 8 {
            break;
        }
        let name = typ.to_be_bytes();
        let name = name
            .iter()
            .map(|&byte| if byte.is_ascii_graphic() { byte as char } else { '.' })
            .collect::<String>();
        if is_text_codec(&name) {
            last = Some(name);
        }
        cursor.pos = entry_start.checked_add(size)?;
        if cursor.pos > payload.len() {
            break;
        }
    }
    last
}

fn parse_stts(payload: &[u8]) -> Option<Vec<i64>> {
    let mut cursor = Cursor::new(payload);
    cursor.skip(4)?;
    let entries = cursor.u32()? as usize;
    let mut deltas = Vec::new();
    for _ in 0..entries.min(10_000) {
        let count = cursor.u32()? as usize;
        let delta = cursor.i32()? as i64;
        if deltas.len() >= MAX_SAMPLES {
            break;
        }
        deltas.extend(std::iter::repeat_n(delta, count.min(MAX_SAMPLES - deltas.len())));
    }
    Some(deltas)
}

fn parse_ctts(payload: &[u8]) -> Option<Vec<i64>> {
    let mut cursor = Cursor::new(payload);
    cursor.skip(4)?;
    let entries = cursor.u32()? as usize;
    let mut offsets = Vec::new();
    for _ in 0..entries.min(10_000) {
        let count = cursor.u32()? as usize;
        let offset = cursor.i32()? as i64;
        if offsets.len() >= MAX_SAMPLES {
            break;
        }
        offsets.extend(std::iter::repeat_n(offset, count.min(MAX_SAMPLES - offsets.len())));
    }
    Some(offsets)
}

fn parse_stsz(payload: &[u8]) -> Option<Vec<usize>> {
    let mut cursor = Cursor::new(payload);
    cursor.skip(4)?;
    let fixed = cursor.u32()? as usize;
    let count = (cursor.u32()? as usize).min(MAX_SAMPLES);
    if fixed > 0 {
        return Some(vec![fixed; count]);
    }
    let mut sizes = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        sizes.push(cursor.u32()? as usize);
    }
    Some(sizes)
}

fn parse_stz2(payload: &[u8]) -> Option<Vec<usize>> {
    let mut cursor = Cursor::new(payload);
    cursor.skip(4)?;
    let field_size = cursor.u8()?;
    let count = (cursor.u32()? as usize).min(MAX_SAMPLES);
    let mut sizes = Vec::with_capacity(count.min(4096));
    match field_size {
        8 => {
            for _ in 0..count {
                sizes.push(cursor.u8()? as usize);
            }
        }
        16 => {
            for _ in 0..count {
                sizes.push(cursor.u16()? as usize);
            }
        }
        4 => {
            for index in 0..count {
                let byte = cursor.u8()? as usize;
                sizes.push(if index % 2 == 0 { byte >> 4 } else { byte & 0x0f });
            }
        }
        _ => return None,
    }
    Some(sizes)
}

fn parse_chunk_offsets(payload: &[u8], wide: bool) -> Option<Vec<u64>> {
    let mut cursor = Cursor::new(payload);
    cursor.skip(4)?;
    let count = (cursor.u32()? as usize).min(MAX_SAMPLES);
    let mut offsets = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        if wide {
            offsets.push(u64be(cursor.bytes, cursor.pos)?);
            cursor.skip(8)?;
        } else {
            offsets.push(u64::from(cursor.u32()?));
        }
    }
    Some(offsets)
}

fn parse_stsc(payload: &[u8]) -> Option<Vec<(u32, u32)>> {
    let mut cursor = Cursor::new(payload);
    cursor.skip(4)?;
    let count = (cursor.u32()? as usize).min(10_000);
    let mut entries = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        let first_chunk = cursor.u32()?;
        let samples_per_chunk = cursor.u32()?;
        cursor.skip(4)?; // sample_description_index
        entries.push((first_chunk, samples_per_chunk));
    }
    Some(entries)
}

fn parse_stss(payload: &[u8]) -> Option<Vec<u32>> {
    let mut cursor = Cursor::new(payload);
    cursor.skip(4)?;
    let count = (cursor.u32()? as usize).min(MAX_SAMPLES);
    let mut numbers = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        if let Some(number) = cursor.u32() {
            numbers.push(number);
        } else {
            break;
        }
    }
    Some(numbers)
}

fn parse_elst(payload: &[u8]) -> Option<Vec<(i64, i64)>> {
    let mut cursor = Cursor::new(payload);
    let version = cursor.u32()? >> 24;
    let count = (cursor.u32()? as usize).min(1024);
    let mut edits = Vec::with_capacity(count);
    for _ in 0..count {
        let duration = if version == 1 {
            i64::try_from(u64be(cursor.bytes, cursor.pos)?).unwrap_or(i64::MAX)
        } else {
            i64::from(cursor.u32()?)
        };
        if version == 1 {
            cursor.skip(8)?;
        }
        let media_time = if version == 1 {
            i64::try_from(u64be(cursor.bytes, cursor.pos)?).unwrap_or(i64::MAX)
        } else {
            i64::from(cursor.i32()?)
        };
        if version == 1 {
            cursor.skip(8)?;
        }
        cursor.skip(4)?; // media rate + reserved
        edits.push((duration, media_time));
    }
    Some(edits)
}

fn parse_mp4_track(bytes: &[u8], trak: Mp4Box, movie_timescale: u64) -> Option<Mp4TrackInfo> {
    let trak_children = box_children(bytes, &trak, MAX_BOXES);
    let mdia = trak_children.iter().find(|box_| box_.typ == *b"mdia").copied()?;
    let mdia_children = box_children(bytes, &mdia, MAX_BOXES);
    let handler = child_slice(bytes, &mdia_children, *b"hdlr")
        .and_then(parse_hdlr)
        .unwrap_or_default();
    let (timescale, language) = child_slice(bytes, &mdia_children, *b"mdhd")
        .and_then(parse_mdhd)
        .unwrap_or((1_000, None));
    let minf = mdia_children.iter().find(|box_| box_.typ == *b"minf").copied()?;
    let minf_children = box_children(bytes, &minf, MAX_BOXES);
    let stbl = minf_children.iter().find(|box_| box_.typ == *b"stbl").copied()?;
    let stbl_children = box_children(bytes, &stbl, MAX_BOXES);
    let codec = child_slice(bytes, &stbl_children, *b"stsd")
        .and_then(parse_stsd_codec)
        .unwrap_or_default();
    if !is_text_handler(&handler) && !is_text_codec(&codec) {
        return None;
    }

    let track_id = child_slice(bytes, &trak_children, *b"tkhd")
        .and_then(parse_tkhd_id)
        .unwrap_or(0);
    let sizes = child_slice(bytes, &stbl_children, *b"stsz")
        .and_then(parse_stsz)
        .or_else(|| child_slice(bytes, &stbl_children, *b"stz2").and_then(parse_stz2))?;
    let count = sizes.len().min(MAX_SAMPLES);
    let deltas = child_slice(bytes, &stbl_children, *b"stts").and_then(parse_stts).unwrap_or_default();
    let ctts = child_slice(bytes, &stbl_children, *b"ctts").and_then(parse_ctts).unwrap_or_default();
    let stco = child_slice(bytes, &stbl_children, *b"stco");
    let co64 = child_slice(bytes, &stbl_children, *b"co64");
    let chunk_offsets = stco
        .and_then(|payload| parse_chunk_offsets(payload, false))
        .or_else(|| co64.and_then(|payload| parse_chunk_offsets(payload, true)))
        .unwrap_or_default();
    let stsc_entries = child_slice(bytes, &stbl_children, *b"stsc")
        .and_then(parse_stsc)
        .unwrap_or_default();
    let sync_samples = child_slice(bytes, &stbl_children, *b"stss").and_then(parse_stss).unwrap_or_default();
    let edits = child_slice(bytes, &trak_children, *b"edts")
        .and_then(|edts| {
            let root = Mp4Box {
                typ: *b"edts",
                start: 0,
                end: edts.len(),
                header: 0,
            };
            box_children(edts, &root, MAX_BOXES)
                .iter()
                .find(|box_| box_.typ == *b"elst")
                .and_then(|elst| parse_elst(&edts[elst.data()..elst.end]))
        })
        .unwrap_or_default();

    // Map every sample to its absolute file offset via stsc + stco/co64.
    let mut offsets = vec![0u64; count];
    let mut sample_index = 0usize;
    for (chunk_index, &chunk_offset) in chunk_offsets.iter().enumerate() {
        if sample_index >= count {
            break;
        }
        let samples_per_chunk = stsc_entries
            .iter()
            .rev()
            .find(|(first_chunk, _)| (*first_chunk as usize) <= chunk_index + 1)
            .map(|(_, samples)| *samples as usize)
            .unwrap_or(1)
            .max(1);
        let mut cursor_offset = chunk_offset;
        for _ in 0..samples_per_chunk {
            if sample_index >= count {
                break;
            }
            offsets[sample_index] = cursor_offset;
            cursor_offset = cursor_offset.saturating_add(sizes[sample_index] as u64);
            sample_index += 1;
        }
    }

    // Apply edit list: a zero-duration edit shifts by its media time; a normal single edit shifts
    // and clips to its window. Dwell edits and multi-edit timelines are intentionally simplified.
    let mut shift_ticks = 0i64;
    let mut window: Option<(i64, i64)> = None;
    if let Some(&(edit_duration, edit_media_time)) = edits.first() {
        if edit_duration == 0 {
            shift_ticks = -edit_media_time;
        } else if edit_media_time >= 0 {
            shift_ticks = -edit_media_time;
            let duration_ticks = mul_div(edit_duration as i128, timescale as i128, movie_timescale as i128);
            window = Some((edit_media_time, edit_media_time.saturating_add(duration_ticks)));
        }
    }

    let mut raw: Vec<(i64, i64, usize, u64)> = Vec::with_capacity(count);
    let mut ticks = 0i64;
    for index in 0..count {
        let delta = deltas.get(index).copied().unwrap_or(0);
        let composition = ctts.get(index).copied().unwrap_or(0);
        let sample_ticks = ticks.saturating_add(composition);
        if window.is_none_or(|(low, high)| sample_ticks >= low && sample_ticks < high) {
            raw.push((
                sample_ticks.saturating_add(shift_ticks),
                delta,
                sizes[index],
                offsets[index],
            ));
        }
        ticks = ticks.saturating_add(delta);
    }
    if raw.is_empty() {
        return Some(Mp4TrackInfo {
            track_id,
            language,
            handler,
            codec,
            timescale,
            samples: Vec::new(),
            sync_samples,
        });
    }

    let starts: Vec<i64> = raw
        .iter()
        .map(|(time, _, _, _)| mul_div(*time as i128, 1_000, timescale as i128))
        .collect();
    let mut samples = Vec::with_capacity(raw.len());
    for (index, (_, delta, size, offset)) in raw.into_iter().enumerate() {
        let start_ms = starts[index];
        let duration_ms = if delta > 0 {
            mul_div(delta as i128, 1_000, timescale as i128).max(1)
        } else {
            DEFAULT_CUE_DURATION_MS
        };
        let end_ms = match starts.get(index + 1) {
            Some(&next) if next > start_ms => next,
            _ => start_ms.saturating_add(duration_ms),
        };
        samples.push(Mp4SampleInfo {
            start_ms,
            end_ms,
            offset,
            size,
        });
    }

    Some(Mp4TrackInfo {
        track_id,
        language,
        handler,
        codec,
        timescale,
        samples,
        sync_samples,
    })
}

// ----------------------------------------------------------- sample text (MP4)

fn sample_text(codec: &str, data: &[u8], depth: usize) -> Option<String> {
    let text = match codec {
        "tx3g" => {
            if data.len() < 4 || depth > 4 {
                return None;
            }
            let length = u16::from_be_bytes([data[0], data[1]]) as usize;
            let end = length.saturating_add(4).min(data.len());
            String::from_utf8_lossy(&data[4..end]).into_owned()
        }
        "wvtt" => wvtt_payload(data, depth),
        _ => String::from_utf8_lossy(data).into_owned(),
    };
    let text = text.replace('\0', "");
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn wvtt_payload(data: &[u8], depth: usize) -> String {
    if depth > 4 {
        return String::new();
    }
    let mut result = String::new();
    let mut pos = 0usize;
    let mut boxes = 0usize;
    while pos + 8 <= data.len() && boxes < MAX_BOXES {
        let Some(box_) = read_box(data, pos, data.len()) else {
            break;
        };
        if box_.typ == *b"payl" {
            result.push_str(&String::from_utf8_lossy(&data[box_.data()..box_.end]));
        } else if box_.typ == *b"vttc"
            || box_.typ == *b"vtts"
            || box_.typ == *b"mdat"
            || box_.typ == *b"moof"
            || box_.typ == *b"traf"
        {
            result.push_str(&wvtt_payload(&data[box_.data()..box_.end], depth + 1));
        }
        pos = box_.end;
        boxes += 1;
    }
    if result.is_empty() {
        String::from_utf8_lossy(data).into_owned()
    } else {
        result
    }
}

// ------------------------------------------------------------------ EBML / MKV

const ID_SEGMENT: u64 = 0x1853_8067;
const ID_INFO: u64 = 0x1549_A966;
const ID_TIMESTAMP_SCALE: u64 = 0x2AD7_B1;
const ID_TRACKS: u64 = 0x1654_AE6B;
const ID_TRACK_ENTRY: u64 = 0xAE;
const ID_TRACK_NUMBER: u64 = 0xD7;
const ID_TRACK_TYPE: u64 = 0x83;
const ID_CODEC_ID: u64 = 0x86;
const ID_LANGUAGE: u64 = 0x22B5_9C;
const ID_LANGUAGE_IETF: u64 = 0x22B5_9D;
const ID_NAME: u64 = 0x536E;
const ID_CLUSTER: u64 = 0x1F43_B675;
const ID_CLUSTER_TIMESTAMP: u64 = 0xE7;
const ID_SIMPLE_BLOCK: u64 = 0xA3;
const ID_BLOCK_GROUP: u64 = 0xA0;
const ID_BLOCK: u64 = 0xA1;
const ID_BLOCK_DURATION: u64 = 0x9B;

#[derive(Clone, Copy)]
struct EbmlElement {
    id: u64,
    data_start: usize,
    data_end: usize,
}

fn vint_marker(byte: u8) -> Option<(u8, usize)> {
    if byte == 0 {
        return None;
    }
    let mut mask = 0x80u8;
    let mut length = 1usize;
    while length <= 8 && (byte & mask) == 0 {
        mask >>= 1;
        length += 1;
    }
    (length <= 8).then_some((mask, length))
}

fn ebml_id(bytes: &[u8], pos: usize, limit: usize) -> Option<(u64, usize)> {
    let first = *bytes.get(pos)?;
    let (_, length) = vint_marker(first)?;
    if length > 4 || pos.checked_add(length)? > limit {
        return None;
    }
    let mut id = 0u64;
    for &byte in bytes.get(pos..pos + length)? {
        id = (id << 8) | u64::from(byte);
    }
    Some((id, length))
}

fn ebml_size(bytes: &[u8], pos: usize, limit: usize) -> Option<(Option<u64>, usize)> {
    let first = *bytes.get(pos)?;
    let (mask, length) = vint_marker(first)?;
    if pos.checked_add(length)? > limit {
        return None;
    }
    let mut value = u64::from(first & mask.wrapping_sub(1));
    for &byte in bytes.get(pos + 1..pos + length)? {
        value = (value << 8) | u64::from(byte);
    }
    let all_ones = if length == 8 {
        value == u64::MAX
    } else {
        value == (1u64 << (7 * length)) - 1
    };
    Some(((!all_ones).then_some(value), length))
}

fn ebml_uint(bytes: &[u8], pos: usize, limit: usize) -> Option<(u64, usize)> {
    let first = *bytes.get(pos)?;
    let (mask, length) = vint_marker(first)?;
    if pos.checked_add(length)? > limit {
        return None;
    }
    let mut value = u64::from(first & mask.wrapping_sub(1));
    for &byte in bytes.get(pos + 1..pos + length)? {
        value = (value << 8) | u64::from(byte);
    }
    Some((value, length))
}

fn read_element(bytes: &[u8], pos: usize, limit: usize) -> Option<EbmlElement> {
    if pos >= limit || limit > bytes.len() {
        return None;
    }
    let (id, id_len) = ebml_id(bytes, pos, limit)?;
    let (size, size_len) = ebml_size(bytes, pos + id_len, limit)?;
    let data_start = pos.checked_add(id_len)?.checked_add(size_len)?;
    let data_end = match size {
        Some(size) => data_start.checked_add(usize::try_from(size).ok()?)?,
        None => limit,
    };
    if data_end > limit {
        return None;
    }
    Some(EbmlElement {
        id,
        data_start,
        data_end,
    })
}

fn ebml_children(bytes: &[u8], parent: &EbmlElement, max: usize) -> Vec<EbmlElement> {
    let mut result = Vec::new();
    let mut pos = parent.data_start;
    while pos < parent.data_end && result.len() < max {
        match read_element(bytes, pos, parent.data_end) {
            Some(element) => {
                if element.data_end <= pos {
                    break;
                }
                pos = element.data_end;
                result.push(element);
            }
            None => break,
        }
    }
    result
}

fn ebml_uint_value(bytes: &[u8], element: &EbmlElement) -> Option<u64> {
    ebml_uint(bytes, element.data_start, element.data_end).map(|(value, _)| value)
}

fn ebml_string(bytes: &[u8], element: &EbmlElement) -> String {
    String::from_utf8_lossy(&bytes[element.data_start..element.data_end])
        .replace('\0', "")
        .trim()
        .to_string()
}

fn looks_like_mkv(bytes: &[u8]) -> bool {
    let mut pos = 0usize;
    let mut count = 0usize;
    while pos + 4 <= bytes.len() && count < 16 {
        let Some(element) = read_element(bytes, pos, bytes.len()) else {
            break;
        };
        if element.id == ID_SEGMENT {
            return true;
        }
        if element.data_end <= pos {
            break;
        }
        pos = element.data_end;
        count += 1;
    }
    bytes
        .iter()
        .take(8_192)
        .cloned()
        .collect::<Vec<u8>>()
        .windows(4)
        .any(|window| window == [0x18, 0x53, 0x80, 0x67])
}

fn parse_mkv_segment(bytes: &[u8], segment: &EbmlElement, out: &mut MkvDocumentInfo) {
    let children = ebml_children(bytes, segment, MAX_EBML_CHILDREN);
    for child in &children {
        match child.id {
            ID_INFO => {
                for info in ebml_children(bytes, child, MAX_BOXES) {
                    if info.id == ID_TIMESTAMP_SCALE {
                        if let Some(scale) = ebml_uint_value(bytes, &info) {
                            if scale > 0 {
                                out.timecode_scale_ns = scale;
                            }
                        }
                    }
                }
            }
            ID_TRACKS => parse_mkv_tracks(bytes, child, &mut out.tracks),
            ID_CLUSTER => parse_mkv_cluster(bytes, child, &mut out.blocks),
            _ => {}
        }
    }
}

fn parse_mkv_tracks(bytes: &[u8], tracks: &EbmlElement, out: &mut Vec<MkvTrackInfo>) {
    for entry in ebml_children(bytes, tracks, MAX_BOXES) {
        if entry.id != ID_TRACK_ENTRY {
            continue;
        }
        let mut number = 0u64;
        let mut track_type = 0u64;
        let mut codec_id = String::new();
        let mut language = None;
        let mut name = None;
        for field in ebml_children(bytes, &entry, MAX_BOXES) {
            match field.id {
                ID_TRACK_NUMBER => number = ebml_uint_value(bytes, &field).unwrap_or(0),
                ID_TRACK_TYPE => track_type = ebml_uint_value(bytes, &field).unwrap_or(0),
                ID_CODEC_ID => codec_id = ebml_string(bytes, &field),
                ID_LANGUAGE | ID_LANGUAGE_IETF => {
                    let value = ebml_string(bytes, &field);
                    if !value.is_empty() && (language.is_none() || field.id == ID_LANGUAGE_IETF) {
                        language = Some(value);
                    }
                }
                ID_NAME => {
                    let value = ebml_string(bytes, &field);
                    if !value.is_empty() {
                        name = Some(value);
                    }
                }
                _ => {}
            }
        }
        if track_type == 17 && codec_id.starts_with("S_TEXT/") && number > 0 {
            out.push(MkvTrackInfo {
                number,
                codec_id,
                language,
                name,
            });
        }
    }
}

fn parse_mkv_cluster(bytes: &[u8], cluster: &EbmlElement, out: &mut Vec<MkvBlockInfo>) {
    let children = ebml_children(bytes, cluster, MAX_EBML_CHILDREN);
    let mut base_ticks = 0u64;
    for child in &children {
        if child.id == ID_CLUSTER_TIMESTAMP {
            base_ticks = ebml_uint_value(bytes, child).unwrap_or(0);
        }
    }
    for child in &children {
        match child.id {
            ID_SIMPLE_BLOCK => parse_mkv_block(bytes, child, base_ticks, None, out),
            ID_BLOCK_GROUP => {
                let group_children = ebml_children(bytes, child, MAX_BOXES);
                let duration = group_children
                    .iter()
                    .find(|item| item.id == ID_BLOCK_DURATION)
                    .and_then(|item| ebml_uint_value(bytes, item));
                if let Some(block) = group_children.iter().find(|item| item.id == ID_BLOCK) {
                    parse_mkv_block(bytes, block, base_ticks, duration, out);
                }
            }
            _ => {}
        }
    }
}

fn parse_mkv_block(
    bytes: &[u8],
    block: &EbmlElement,
    base_ticks: u64,
    duration_ticks: Option<u64>,
    out: &mut Vec<MkvBlockInfo>,
) {
    if out.len() >= MAX_BLOCKS {
        return;
    }
    let Some((track_number, number_len)) =
        ebml_uint(bytes, block.data_start, block.data_end)
    else {
        return;
    };
    let relative_at = block.data_start + number_len;
    let relative = match bytes.get(relative_at..relative_at + 3) {
        Some(slice) => i16::from_be_bytes([slice[0], slice[1]]),
        None => return,
    };
    let flags = bytes[relative_at + 2];
    let mut payload_at = relative_at + 3;
    let end = block.data_end;
    let lacing = (flags >> 1) & 0x03;
    if lacing != 0 {
        payload_at = match skip_mkv_lacing(bytes, payload_at, end, lacing) {
            Some(position) => position,
            None => return,
        };
    }
    if payload_at > end {
        return;
    }
    let time_ticks = if relative >= 0 {
        base_ticks.saturating_add(relative as u64)
    } else {
        base_ticks.saturating_sub(relative.unsigned_abs() as u64)
    };
    out.push(MkvBlockInfo {
        track_number,
        time_ticks,
        duration_ticks,
        payload: bytes[payload_at..end].to_vec(),
    });
}

fn skip_mkv_lacing(bytes: &[u8], pos: usize, end: usize, mode: u8) -> Option<usize> {
    let first = *bytes.get(pos)?;
    let frame_count = first as usize + 1;
    match mode {
        1 => {
            // Xiph lacing: each size is a chain of 0xff bytes terminated by a final byte.
            let mut at = pos + 1;
            for _ in 0..frame_count.saturating_sub(1) {
                let mut size = 0usize;
                loop {
                    let byte = *bytes.get(at)?;
                    if at >= end {
                        return None;
                    }
                    at += 1;
                    size = size.saturating_add(byte as usize);
                    if byte != 0xff {
                        break;
                    }
                }
                let _ = size;
            }
            Some(at)
        }
        2 => {
            // Fixed-size lacing: first frame is 1/n of the remaining payload.
            let at = pos + 1;
            if at >= end {
                return None;
            }
            Some(at + (end - at) / frame_count)
        }
        3 => {
            // EBML lacing: only the first frame size is needed here.
            let (size, length) = ebml_uint(bytes, pos + 1, end)?;
            Some(pos + 1 + length + usize::try_from(size).ok()?)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------- network path

struct RangeClient {
    client: reqwest::Client,
    url: String,
    base_headers: BTreeMap<String, String>,
    used: u64,
}

impl RangeClient {
    fn new(url: &str, headers: &BTreeMap<String, String>) -> Result<Self, String> {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err("embedded subs: only HTTP(S) URLs are supported".to_string());
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(6))
            .build()
            .map_err(|error| format!("embedded subs: HTTP client error: {error}"))?;
        let mut base_headers = BTreeMap::new();
        for (name, value) in headers {
            let lower = name.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "range"
                    | "accept-encoding"
                    | "content-length"
                    | "host"
                    | "connection"
                    | "transfer-encoding"
                    | "if-range"
            ) {
                continue;
            }
            base_headers.insert(name.clone(), value.clone());
        }
        Ok(Self {
            client,
            url: url.to_string(),
            base_headers,
            used: 0,
        })
    }

    async fn fetch(&mut self, start: u64, length: usize) -> Result<Vec<u8>, String> {
        self.fetch_inner(start, length, false).await
    }

    /// The initial probe tolerates a short response: small files may be returned in full with 200.
    async fn fetch_initial(&mut self, length: usize) -> Result<Vec<u8>, String> {
        self.fetch_inner(0, length, true).await
    }

    async fn fetch_inner(
        &mut self,
        start: u64,
        length: usize,
        allow_short: bool,
    ) -> Result<Vec<u8>, String> {
        if length == 0 || length > MAX_RANGE_BYTES {
            return Err("embedded subs: invalid range length".to_string());
        }
        let end = start
            .checked_add(length as u64 - 1)
            .ok_or_else(|| "embedded subs: range overflow".to_string())?;
        let allowed = MAX_TOTAL_BYTES.saturating_sub(self.used);
        if length as u64 > allowed {
            return Err("embedded subs: download budget exhausted".to_string());
        }

        let mut headers = HeaderMap::new();
        for (name, value) in &self.base_headers {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|error| format!("embedded subs: bad header name: {error}"))?;
            let value = HeaderValue::from_str(value)
                .map_err(|error| format!("embedded subs: bad header value: {error}"))?;
            headers.insert(name, value);
        }
        let range = HeaderValue::from_str(&format!("bytes={start}-{end}"))
            .map_err(|error| format!("embedded subs: bad range header: {error}"))?;
        headers.insert(reqwest::header::RANGE, range);
        headers.insert(
            reqwest::header::ACCEPT_ENCODING,
            HeaderValue::from_static("identity"),
        );

        let response = self
            .client
            .get(&self.url)
            .headers(headers)
            .send()
            .await
            .map_err(|error| format!("embedded subs: request failed: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("embedded subs: HTTP {}", status.as_u16()));
        }
        if start > 0 && status.as_u16() != 206 {
            return Err("embedded subs: server ignored range request".to_string());
        }
        if let Some(content_range) = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
        {
            if status.as_u16() == 206
                && !content_range
                    .trim()
                    .to_ascii_lowercase()
                    .starts_with(&format!("bytes {start}-"))
            {
                return Err("embedded subs: Content-Range mismatch".to_string());
            }
        } else if status.as_u16() == 206 {
            return Err("embedded subs: missing Content-Range".to_string());
        }
        if status.as_u16() == 200 {
            if let Some(total) = response.content_length() {
                if total > 16 * 1024 * 1024 {
                    return Err("embedded subs: oversized non-range response".to_string());
                }
            }
        }
        let body = response
            .bytes()
            .await
            .map_err(|error| format!("embedded subs: body read failed: {error}"))?;
        if body.len() < length && !allow_short {
            return Err("embedded subs: short range response".to_string());
        }
        let received = body.len().min(length);
        self.used = self.used.saturating_add(received as u64);
        Ok(body[..received].to_vec())
    }
}

/// In-memory prefix plus ranged HTTP reads with a small aligned chunk cache.
struct ByteStore {
    initial: Vec<u8>,
    client: Option<RangeClient>,
    cache: BTreeMap<u64, Vec<u8>>,
}

impl ByteStore {
    #[cfg_attr(not(test), allow(dead_code))]
    fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            initial: bytes,
            client: None,
            cache: BTreeMap::new(),
        }
    }

    async fn new(url: &str, headers: &BTreeMap<String, String>) -> Result<Self, String> {
        let mut client = RangeClient::new(url, headers)?;
        let initial = client.fetch_initial(INITIAL_PROBE_BYTES).await?;
        Ok(Self {
            initial,
            client: Some(client),
            cache: BTreeMap::new(),
        })
    }

    fn initial(&self) -> &[u8] {
        &self.initial
    }

    fn from_cache(&self, start: u64, length: usize) -> Option<Vec<u8>> {
        for (base, bytes) in &self.cache {
            let offset = start.checked_sub(*base)?;
            let offset = usize::try_from(offset).ok()?;
            if offset.checked_add(length)? <= bytes.len() {
                return Some(bytes[offset..offset + length].to_vec());
            }
        }
        None
    }

    async fn read_exact(&mut self, start: u64, length: usize) -> Result<Vec<u8>, String> {
        if let Some(end) = start.checked_add(length as u64) {
            if end <= self.initial.len() as u64 {
                let from = start as usize;
                return Ok(self.initial[from..from + length].to_vec());
            }
        }
        if let Some(bytes) = self.from_cache(start, length) {
            return Ok(bytes);
        }
        match self.client.as_mut() {
            Some(client) => client.fetch(start, length).await,
            None => Err("embedded subs: no bytes available".to_string()),
        }
    }

    async fn read_cached(&mut self, start: u64, length: usize) -> Result<Vec<u8>, String> {
        if let Some(end) = start.checked_add(length as u64) {
            if end <= self.initial.len() as u64 {
                let from = start as usize;
                return Ok(self.initial[from..from + length].to_vec());
            }
        }
        if let Some(bytes) = self.from_cache(start, length) {
            return Ok(bytes);
        }
        if length > MAX_RANGE_BYTES {
            return Err("embedded subs: sample too large".to_string());
        }
        let aligned = start & !(CHUNK_ALIGN - 1);
        let offset = (start - aligned) as usize;
        if offset + length > MAX_RANGE_BYTES {
            return self.read_exact(start, length).await;
        }
        let chunk_len = (offset + length).max(CHUNK_ALIGN as usize).min(MAX_RANGE_BYTES);
        let bytes = self.read_exact(aligned, chunk_len).await?;
        let result = bytes[offset..offset + length].to_vec();
        if self.cache.len() >= MAX_CACHE_CHUNKS {
            if let Some(key) = self.cache.keys().next().copied() {
                self.cache.remove(&key);
            }
        }
        self.cache.insert(aligned, bytes);
        Ok(result)
    }
}

async fn find_moov_location(store: &mut ByteStore) -> Result<(u64, usize), String> {
    let initial = store.initial().to_vec();
    let mut pos = 0usize;
    for _ in 0..64 {
        let (typ, size, header) = if pos + 8 <= initial.len() {
            read_box_header(&initial, pos, initial.len())
                .ok_or_else(|| "embedded subs: bad MP4 box header".to_string())?
        } else {
            let probe = store
                .read_exact(pos as u64, 16)
                .await?;
            read_box_header(&probe, 0, probe.len())
                .ok_or_else(|| "embedded subs: bad MP4 box header".to_string())?
        };
        if typ == *b"moov" {
            if size == u64::MAX {
                let size = (initial.len() - pos).min(MAX_MOOV_BYTES);
                return Ok((pos as u64, size));
            }
            let length = usize::try_from(size)
                .ok()
                .filter(|size| *size >= header && *size <= MAX_MOOV_BYTES)
                .ok_or_else(|| "embedded subs: invalid moov size".to_string())?;
            return Ok((pos as u64, length));
        }
        if size == u64::MAX || size < 8 || pos as u64 > u64::MAX - size {
            return Err("embedded subs: invalid MP4 box size".to_string());
        }
        pos = pos
            .checked_add(size as usize)
            .ok_or_else(|| "embedded subs: MP4 offset overflow".to_string())?;
    }
    Err("embedded subs: no moov box found".to_string())
}

async fn load_mp4(store: &mut ByteStore) -> Result<Vec<EmbeddedTrack>, String> {
    let (offset, length) = find_moov_location(store).await?;
    let moov = store.read_exact(offset, length).await?;
    let tracks = parse_mp4_from_bytes(&moov);
    let mut result = Vec::with_capacity(tracks.len());
    for track in tracks {
        let mut payloads = Vec::with_capacity(track.samples.len());
        for sample in &track.samples {
            if sample.size == 0 || sample.size > MAX_RANGE_BYTES {
                payloads.push(None);
            } else {
                payloads.push(store.read_cached(sample.offset, sample.size).await.ok());
            }
        }
        let cues = mp4_cues_from_samples(&track, &payloads);
        result.push(EmbeddedTrack {
            id: format!("mp4:{}", track.track_id),
            language: track.language.clone(),
            kind: "sub".to_string(),
            cues,
        });
    }
    Ok(result)
}

fn find_clusters(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut result = Vec::new();
    let mut pos = 0usize;
    while pos + 4 <= bytes.len() && result.len() < MAX_CLUSTERS_PER_WINDOW {
        if bytes[pos] == 0x1F
            && bytes[pos + 1] == 0x43
            && bytes[pos + 2] == 0xB6
            && bytes[pos + 3] == 0x75
        {
            if let Some(element) = read_element(bytes, pos, bytes.len()) {
                let length = element.data_end.saturating_sub(pos);
                result.push((pos, length));
                if element.data_end > pos {
                    pos = element.data_end;
                    continue;
                }
            }
        }
        pos += 1;
    }
    result
}

async fn scan_mkv_window(
    store: &mut ByteStore,
    window: &[u8],
    base: u64,
    blocks: &mut Vec<MkvBlockInfo>,
) {
    for (offset, length) in find_clusters(window) {
        if blocks.len() >= MAX_BLOCKS {
            return;
        }
        if length == 0 || length > MAX_RANGE_BYTES {
            continue;
        }
        let cluster = if offset + length <= window.len() {
            window[offset..offset + length].to_vec()
        } else {
            let absolute = base.saturating_add(offset as u64);
            match store.read_exact(absolute, length).await {
                Ok(bytes) => bytes,
                Err(_) => continue,
            }
        };
        if let Some(element) = read_element(&cluster, 0, cluster.len()) {
            if element.id == ID_CLUSTER {
                parse_mkv_cluster(&cluster, &element, blocks);
            }
        }
    }
}

fn has_useful_span(document: &MkvDocumentInfo) -> bool {
    // Group directly from the track numbers found in the blocks.
    let mut numbers: Vec<u64> = document.blocks.iter().map(|block| block.track_number).collect();
    numbers.sort_unstable();
    numbers.dedup();
    numbers.into_iter().any(|number| {
        let mut times: Vec<u64> = document
            .blocks
            .iter()
            .filter(|block| block.track_number == number)
            .map(|block| block.time_ticks)
            .collect();
        times.sort_unstable();
        times.dedup();
        if times.len() < 8 {
            return false;
        }
        let span_ticks = times.last().unwrap_or(&0).saturating_sub(*times.first().unwrap_or(&0));
        let span_ms = (u128::from(span_ticks) * u128::from(document.timecode_scale_ns)) / 1_000_000;
        span_ms >= 30_000
    })
}


// ---------------------------------------------------------------- Matroska Cues index
// The original's `EmbeddedSubtitleTimelineLoader.loadMatroskaCueIndex`: Matroska recommends
// indexing every subtitle frame in Cues (mkvmerge does, with CueDuration), so the whole subtitle
// timeline comes from the SeekHead, Info, Tracks and Cues elements alone (a few hundred KB),
// without reading the clusters. Only timing is needed for the alignment.

mod mkv_index {
    //! See the comment above.
    use super::{ByteStore, EmbeddedTrack, SyncCue, BTreeMap, DEFAULT_MKV_SCALE_NS, LAST_CUE_DURATION_MS, MAX_EBML_CHILDREN, MAX_RANGE_BYTES};

const ID_SEGMENT: u64 = 0x1853_8067;
const ID_SEEKHEAD: u64 = 0x114D_9B74;
const ID_SEEK: u64 = 0x4DBB;
const ID_SEEK_ID: u64 = 0x53AB;
const ID_SEEK_POSITION: u64 = 0x53AC;
const ID_INFO: u64 = 0x1549_A966;
const ID_TIMECODE_SCALE: u64 = 0x2A_D7B1;
const ID_TRACKS: u64 = 0x1654_AE6B;
const ID_TRACK_ENTRY: u64 = 0xAE;
const ID_TRACK_NUMBER: u64 = 0xD7;
const ID_TRACK_TYPE: u64 = 0x83;
const ID_CODEC_ID: u64 = 0x86;
const ID_LANGUAGE: u64 = 0x22_B59C;
const ID_LANGUAGE_BCP47: u64 = 0x22_B59D;
const ID_CUES: u64 = 0x1C53_BB6B;
const ID_CUE_POINT: u64 = 0xBB;
const ID_CUE_TIME: u64 = 0xB3;
const ID_CUE_TRACK_POSITIONS: u64 = 0xB7;
const ID_CUE_TRACK: u64 = 0xF7;
const ID_CUE_DURATION: u64 = 0xB2;
const ID_CLUSTER: u64 = 0x1F43_B675;
/// The Cues element may be several MB on long files with many keyframes.
const MAX_CUES_BYTES: u64 = 12 * 1024 * 1024;

/// (id, header length, payload length; `None` = unknown size).
fn ebml_header(bytes: &[u8], at: usize) -> Option<(u64, usize, Option<u64>)> {
    let first = *bytes.get(at)?;
    let id_len = (first.leading_zeros() + 1) as usize;
    if id_len > 4 {
        return None;
    }
    let mut id = 0u64;
    for i in 0..id_len {
        id = (id << 8) | u64::from(*bytes.get(at + i)?);
    }
    let size_first = *bytes.get(at + id_len)?;
    let size_len = (size_first.leading_zeros() + 1) as usize;
    if size_len > 8 {
        return None;
    }
    let mut size = u64::from(size_first) & ((1u64 << (8 - size_len)) - 1);
    let mut all_ones = size == (1u64 << (8 - size_len)) - 1;
    for i in 1..size_len {
        let b = *bytes.get(at + id_len + i)?;
        all_ones &= b == 0xFF;
        size = (size << 8) | u64::from(b);
    }
    Some((id, id_len + size_len, if all_ones { None } else { Some(size) }))
}

/// Direct children of `bytes[start..end]`: (id, payload start, payload end).
fn ebml_children(bytes: &[u8], start: usize, end: usize) -> Vec<(u64, usize, usize)> {
    let mut out = Vec::new();
    let mut at = start;
    while at < end && out.len() < MAX_EBML_CHILDREN {
        let Some((id, header, size)) = ebml_header(bytes, at) else { break };
        let payload = at + header;
        let stop = match size {
            Some(size) => payload.saturating_add(size as usize).min(end),
            None => end,
        };
        out.push((id, payload, stop));
        if size.is_none() {
            break;
        }
        at = stop;
    }
    out
}

fn ebml_uint(bytes: &[u8], start: usize, end: usize) -> u64 {
    bytes[start..end.min(bytes.len())].iter().take(8).fold(0u64, |acc, b| (acc << 8) | u64::from(*b))
}

fn ebml_text(bytes: &[u8], start: usize, end: usize) -> String {
    String::from_utf8_lossy(&bytes[start..end.min(bytes.len())]).trim_end_matches('\0').to_string()
}

/// Reads one whole element whose header starts at `position` (absolute), in ≤ 1 MiB ranges.
async fn read_element(store: &mut ByteStore, position: u64, max: u64) -> Option<(u64, Vec<u8>)> {
    let head = store.read_exact(position, 16).await.ok()?;
    let (id, header, size) = ebml_header(&head, 0)?;
    let total = header as u64 + size?;
    if total > max {
        return None;
    }
    let mut bytes = Vec::with_capacity(total as usize);
    let mut at = position;
    while (bytes.len() as u64) < total {
        let want = (total - bytes.len() as u64).min(MAX_RANGE_BYTES as u64) as usize;
        bytes.extend(store.read_exact(at, want).await.ok()?);
        at += want as u64;
    }
    Some((id, bytes))
}

struct IndexTrack {
    number: u64,
    language: Option<String>,
}

fn index_tracks(bytes: &[u8], start: usize, end: usize) -> Vec<IndexTrack> {
    ebml_children(bytes, start, end)
        .into_iter()
        .filter(|(id, _, _)| *id == ID_TRACK_ENTRY)
        .filter_map(|(_, s, e)| {
            let fields = ebml_children(bytes, s, e);
            let get = |want: u64| fields.iter().find(|(id, _, _)| *id == want).map(|(_, a, b)| (*a, *b));
            let kind = get(ID_TRACK_TYPE).map(|(a, b)| ebml_uint(bytes, a, b))?;
            let codec = get(ID_CODEC_ID).map(|(a, b)| ebml_text(bytes, a, b)).unwrap_or_default();
            // Text subtitles (image formats such as PGS index a show and a clear frame per line).
            if kind != 0x11 || !codec.starts_with("S_TEXT") {
                return None;
            }
            let language = get(ID_LANGUAGE_BCP47).or_else(|| get(ID_LANGUAGE)).map(|(a, b)| ebml_text(bytes, a, b)).filter(|l| !l.is_empty() && l != "und");
            Some(IndexTrack { number: get(ID_TRACK_NUMBER).map(|(a, b)| ebml_uint(bytes, a, b))?, language })
        })
        .collect()
}

/// Subtitle timelines from the Cues index; `None` when the file has no usable index.
pub(super) async fn load(store: &mut ByteStore) -> Option<Vec<EmbeddedTrack>> {
    let initial = store.initial().to_vec();
    // EBML header, then the Segment.
    let (_, ebml_head, ebml_size) = ebml_header(&initial, 0)?;
    let segment_at = ebml_head + ebml_size? as usize;
    let (segment_id, segment_head, _) = ebml_header(&initial, segment_at)?;
    if segment_id != ID_SEGMENT {
        return None;
    }
    let data_start = (segment_at + segment_head) as u64;

    // Top-level elements in the probe, plus what the SeekHead points to.
    let mut positions: BTreeMap<u64, u64> = BTreeMap::new();
    let mut scale_ns = DEFAULT_MKV_SCALE_NS;
    let mut tracks: Vec<IndexTrack> = Vec::new();
    let mut at = data_start as usize;
    while at < initial.len() {
        let Some((id, header, Some(size))) = ebml_header(&initial, at) else { break };
        if id == ID_CLUSTER {
            break;
        }
        let (start, end) = (at + header, at + header + size as usize);
        if end > initial.len() {
            positions.entry(id).or_insert(at as u64 - data_start);
            break;
        }
        match id {
            ID_SEEKHEAD => {
                for (_, s, e) in ebml_children(&initial, start, end).into_iter().filter(|(id, _, _)| *id == ID_SEEK) {
                    let fields = ebml_children(&initial, s, e);
                    let seek_id = fields.iter().find(|(id, _, _)| *id == ID_SEEK_ID).map(|(_, a, b)| ebml_uint(&initial, *a, *b));
                    let seek_pos = fields.iter().find(|(id, _, _)| *id == ID_SEEK_POSITION).map(|(_, a, b)| ebml_uint(&initial, *a, *b));
                    if let (Some(i), Some(p)) = (seek_id, seek_pos) {
                        positions.entry(i).or_insert(p);
                    }
                }
            }
            ID_INFO => {
                if let Some((_, a, b)) = ebml_children(&initial, start, end).into_iter().find(|(id, _, _)| *id == ID_TIMECODE_SCALE) {
                    scale_ns = ebml_uint(&initial, a, b).max(1);
                }
            }
            ID_TRACKS => tracks = index_tracks(&initial, start, end),
            _ => {}
        }
        at = end;
    }
    if tracks.is_empty() {
        let pos = *positions.get(&ID_TRACKS)?;
        let (id, bytes) = read_element(store, data_start + pos, 2 * 1024 * 1024).await?;
        if id != ID_TRACKS {
            return None;
        }
        let (_, header, _) = ebml_header(&bytes, 0)?;
        tracks = index_tracks(&bytes, header, bytes.len());
    }
    if tracks.is_empty() {
        return Some(Vec::new());
    }
    let cues_pos = *positions.get(&ID_CUES)?;
    let (id, cues) = read_element(store, data_start + cues_pos, MAX_CUES_BYTES).await?;
    if id != ID_CUES {
        return None;
    }
    let (_, header, _) = ebml_header(&cues, 0)?;
    let mut by_track: BTreeMap<u64, Vec<SyncCue>> = BTreeMap::new();
    for (_, s, e) in ebml_children(&cues, header, cues.len()).into_iter().filter(|(id, _, _)| *id == ID_CUE_POINT) {
        let fields = ebml_children(&cues, s, e);
        let Some(time) = fields.iter().find(|(id, _, _)| *id == ID_CUE_TIME).map(|(_, a, b)| ebml_uint(&cues, *a, *b)) else { continue };
        for (_, ps, pe) in fields.iter().filter(|(id, _, _)| *id == ID_CUE_TRACK_POSITIONS) {
            let pos = ebml_children(&cues, *ps, *pe);
            let track = pos.iter().find(|(id, _, _)| *id == ID_CUE_TRACK).map(|(_, a, b)| ebml_uint(&cues, *a, *b));
            let Some(track) = track.filter(|t| tracks.iter().any(|x| x.number == *t)) else { continue };
            let duration = pos.iter().find(|(id, _, _)| *id == ID_CUE_DURATION).map(|(_, a, b)| ebml_uint(&cues, *a, *b));
            let to_ms = |ticks: u64| (u128::from(ticks) * u128::from(scale_ns) / 1_000_000) as i64;
            let start_ms = to_ms(time);
            let end_ms = duration.map(|d| start_ms + to_ms(d).clamp(1, 30_000)).unwrap_or(start_ms + LAST_CUE_DURATION_MS);
            by_track.entry(track).or_default().push(SyncCue { start_ms, end_ms, text: String::new() });
        }
    }
    Some(
        tracks
            .into_iter()
            .filter_map(|t| {
                let mut cues = by_track.remove(&t.number)?;
                cues.sort_by_key(|c| c.start_ms);
                cues.dedup_by_key(|c| c.start_ms);
                Some(EmbeddedTrack { id: format!("mkv:{}", t.number), language: t.language, kind: "sub".into(), cues })
            })
            .collect(),
    )
}

}

async fn load_mkv(store: &mut ByteStore) -> Result<Vec<EmbeddedTrack>, String> {
    // The index first (the original's path); scanning clusters only when there is none.
    if let Some(found) = mkv_index::load(store).await {
        if found.iter().any(|t| t.cues.len() >= 4) {
            return Ok(found);
        }
    }
    let initial = store.initial().to_vec();
    let document = parse_mkv_document(&initial);
    let mut tracks = document.tracks.clone();
    let mut blocks = document.blocks.clone();
    let mut scale_ns = document.timecode_scale_ns;
    scan_mkv_window(store, &initial, 0, &mut blocks).await;

    let mut position = initial.len() as u64;
    let mut windows = 0usize;
    while position < MAX_TOTAL_BYTES && windows < MAX_MKV_WINDOWS {
        let length = (MAX_RANGE_BYTES as u64).min(MAX_TOTAL_BYTES - position) as usize;
        let window = match store.read_exact(position, length).await {
            Ok(bytes) => bytes,
            Err(_) => break,
        };
        let fresh = parse_mkv_document(&window);
        if fresh.timecode_scale_ns != DEFAULT_MKV_SCALE_NS {
            scale_ns = fresh.timecode_scale_ns;
        }
        if tracks.is_empty() {
            tracks = fresh.tracks;
        }
        scan_mkv_window(store, &window, position, &mut blocks).await;
        position = position.saturating_add(length as u64);
        windows += 1;
        let probe = MkvDocumentInfo {
            timecode_scale_ns: scale_ns,
            tracks: tracks.clone(),
            blocks: blocks.clone(),
        };
        if !tracks.is_empty() && has_useful_span(&probe) {
            break;
        }
    }

    Ok(mkv_cues_from_blocks(scale_ns, &tracks, &blocks))
}

// ----------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn box_bytes(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
        out.extend_from_slice(typ);
        out.extend_from_slice(payload);
        out
    }

    fn full_box(typ: &[u8; 4], version: u8, payload: &[u8]) -> Vec<u8> {
        let mut body = vec![version, 0, 0, 0];
        body.extend_from_slice(payload);
        box_bytes(typ, &body)
    }

    fn language_code(text: &[u8; 3]) -> u16 {
        ((u16::from(text[0] & 0x1f) >> 0) << 10)
            | ((u16::from(text[1] & 0x1f)) << 5)
            | u16::from(text[2] & 0x1f)
    }

    fn mp4_fixture() -> Vec<u8> {
        let mut mdhd = Vec::new();
        mdhd.extend_from_slice(&0u32.to_be_bytes());
        mdhd.extend_from_slice(&0u32.to_be_bytes());
        mdhd.extend_from_slice(&1_000u32.to_be_bytes());
        mdhd.extend_from_slice(&4_000u32.to_be_bytes());
        mdhd.extend_from_slice(&language_code(b"eng").to_be_bytes());
        mdhd.extend_from_slice(&0u16.to_be_bytes());
        let mdhd = full_box(b"mdhd", 0, &mdhd);

        let mut hdlr = vec![0u8; 4]; // predefined
        hdlr.extend_from_slice(b"sbtl");
        hdlr.extend_from_slice(&[0u8; 12]);
        let hdlr = full_box(b"hdlr", 0, &hdlr);

        let mut stsd = Vec::new();
        stsd.extend_from_slice(&1u32.to_be_bytes());
        stsd.extend_from_slice(&box_bytes(b"tx3g", &[0u8; 16]));
        let stsd = full_box(b"stsd", 0, &stsd);

        let mut stts = Vec::new();
        stts.extend_from_slice(&1u32.to_be_bytes());
        stts.extend_from_slice(&2u32.to_be_bytes());
        stts.extend_from_slice(&2_000u32.to_be_bytes());
        let stts = full_box(b"stts", 0, &stts);

        let mut stsc = Vec::new();
        stsc.extend_from_slice(&1u32.to_be_bytes());
        stsc.extend_from_slice(&1u32.to_be_bytes());
        stsc.extend_from_slice(&2u32.to_be_bytes());
        stsc.extend_from_slice(&1u32.to_be_bytes());
        let stsc = full_box(b"stsc", 0, &stsc);

        let mut stsz = Vec::new();
        stsz.extend_from_slice(&0u32.to_be_bytes());
        stsz.extend_from_slice(&2u32.to_be_bytes());
        stsz.extend_from_slice(&8u32.to_be_bytes());
        stsz.extend_from_slice(&8u32.to_be_bytes());
        let stsz = full_box(b"stsz", 0, &stsz);

        let mut stco = Vec::new();
        stco.extend_from_slice(&1u32.to_be_bytes());
        stco.extend_from_slice(&1_024u32.to_be_bytes());
        let stco = full_box(b"stco", 0, &stco);

        let mut stbl = Vec::new();
        stbl.extend_from_slice(&stsd);
        stbl.extend_from_slice(&stts);
        stbl.extend_from_slice(&stsc);
        stbl.extend_from_slice(&stsz);
        stbl.extend_from_slice(&stco);
        let stbl = box_bytes(b"stbl", &stbl);
        let minf = box_bytes(b"minf", &stbl);
        let mut mdia = Vec::new();
        mdia.extend_from_slice(&mdhd);
        mdia.extend_from_slice(&hdlr);
        mdia.extend_from_slice(&minf);
        let mdia = box_bytes(b"mdia", &mdia);

        let mut tkhd = Vec::new();
        tkhd.extend_from_slice(&0u32.to_be_bytes());
        tkhd.extend_from_slice(&0u32.to_be_bytes());
        tkhd.extend_from_slice(&7u32.to_be_bytes());
        tkhd.extend_from_slice(&0u32.to_be_bytes());
        let tkhd = full_box(b"tkhd", 0, &tkhd);

        let mut trak = Vec::new();
        trak.extend_from_slice(&tkhd);
        trak.extend_from_slice(&mdia);
        let trak = box_bytes(b"trak", &trak);

        let mut mvhd = Vec::new();
        mvhd.extend_from_slice(&0u32.to_be_bytes());
        mvhd.extend_from_slice(&0u32.to_be_bytes());
        mvhd.extend_from_slice(&1_000u32.to_be_bytes());
        mvhd.extend_from_slice(&4_000u32.to_be_bytes());
        let mvhd = full_box(b"mvhd", 0, &mvhd);

        let mut moov = Vec::new();
        moov.extend_from_slice(&mvhd);
        moov.extend_from_slice(&trak);
        box_bytes(b"moov", &moov)
    }

    fn tx3g_sample(text: &str) -> Vec<u8> {
        let mut out = (text.len() as u16).to_be_bytes().to_vec();
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(text.as_bytes());
        out
    }

    fn vint(value: u64) -> Vec<u8> {
        let mut length = 1usize;
        while length < 8 && value >= (1u64 << (7 * length)) - 1 {
            length += 1;
        }
        let marker = 0x80u8 >> (length - 1);
        let value_bits = 8 - length;
        let mut out = vec![marker | ((value >> (8 * (length - 1))) as u8 & ((1u8 << value_bits) - 1))];
        for index in (1..length).rev() {
            out.push((value >> (8 * (index - 1))) as u8);
        }
        out
    }

    fn element(id: &[u8], payload: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend_from_slice(&vint(payload.len() as u64));
        out.extend_from_slice(payload);
        out
    }

    fn block(track: u64, relative_ms: i16, text: &[u8]) -> Vec<u8> {
        let mut payload = vint(track);
        payload.extend_from_slice(&relative_ms.to_be_bytes());
        payload.push(0);
        payload.extend_from_slice(text);
        element(&[0xA3], &payload)
    }

    fn mkv_fixture(with_info: bool, base_ticks: u64, blocks: &[Vec<u8>]) -> Vec<u8> {
        let mut track_entry = Vec::new();
        track_entry.extend_from_slice(&element(&[0xD7], &vint(2)));
        track_entry.extend_from_slice(&element(&[0x83], &vint(17)));
        track_entry.extend_from_slice(&element(&[0x86], b"S_TEXT/UTF8"));
        track_entry.extend_from_slice(&element(&[0x22, 0xB5, 0x9C], b"eng"));
        track_entry.extend_from_slice(&element(&[0x53, 0x6E], b"English"));
        let tracks = element(&[0x16, 0x54, 0xAE, 0x6B], &element(&[0xAE], &track_entry));

        let mut cluster = element(&[0xE7], &vint(base_ticks));
        for item in blocks {
            cluster.extend_from_slice(item);
        }
        let cluster = element(&[0x1F, 0x43, 0xB6, 0x75], &cluster);

        let mut segment = Vec::new();
        if with_info {
            let scale = element(&[0x2A, 0xD7, 0xB1], &vint(1_000_000));
            segment.extend_from_slice(&element(&[0x15, 0x49, 0xA9, 0x66], &scale));
        }
        segment.extend_from_slice(&tracks);
        segment.extend_from_slice(&cluster);
        element(&[0x18, 0x53, 0x80, 0x67], &segment)
    }

    #[test]
    fn mp4_detects_subtitle_track_and_sample_timing() {
        let tracks = parse_mp4_from_bytes(&mp4_fixture());
        assert_eq!(tracks.len(), 1, "expected one subtitle track");
        let track = &tracks[0];
        assert_eq!(track.track_id, 7);
        assert_eq!(track.handler, "sbtl");
        assert_eq!(track.codec, "tx3g");
        assert_eq!(track.language.as_deref(), Some("eng"));
        assert_eq!(track.timescale, 1_000);
        assert_eq!(track.samples.len(), 2);
        assert_eq!(track.samples[0].start_ms, 0);
        assert_eq!(track.samples[0].end_ms, 2_000);
        assert_eq!(track.samples[1].start_ms, 2_000);
        assert_eq!(track.samples[1].end_ms, 4_000);
        assert_eq!(track.samples[0].offset, 1_024);
        assert_eq!(track.samples[1].offset, 1_032);
    }

    #[test]
    fn mp4_tx3g_samples_produce_timed_cues() {
        let tracks = parse_mp4_from_bytes(&mp4_fixture());
        let payloads = vec![
            Some(tx3g_sample("Hello")),
            Some(tx3g_sample("")), // gap sample: empty text is not a cue
        ];
        let cues = mp4_cues_from_samples(&tracks[0], &payloads);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start_ms, 0);
        assert_eq!(cues[0].end_ms, 2_000);
        assert_eq!(cues[0].text, "Hello");
    }

    #[test]
    fn mp4_raw_payload_with_embedded_vtt_timing_uses_its_timestamps() {
        let cues = cues_for_text(
            "WEBVTT\n\n00:00:01.500 --> 00:00:03.500\nPayload text\n",
            0,
            2_000,
        );
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start_ms, 1_500);
        assert_eq!(cues[0].end_ms, 3_500);
        assert_eq!(cues[0].text, "Payload text");
    }

    #[test]
    fn mkv_detects_track_scale_and_block_timing() {
        let blocks = vec![block(2, 0, b"Hi"), block(2, 2_000, b"There")];
        let document = parse_mkv_document(&mkv_fixture(true, 1_000, &blocks));
        assert_eq!(document.timecode_scale_ns, 1_000_000);
        assert_eq!(document.tracks.len(), 1);
        assert_eq!(document.tracks[0].number, 2);
        assert_eq!(document.tracks[0].codec_id, "S_TEXT/UTF8");
        assert_eq!(document.tracks[0].language.as_deref(), Some("eng"));
        assert_eq!(document.blocks.len(), 2);
        assert_eq!(document.blocks[0].time_ticks, 1_000);
        assert_eq!(document.blocks[1].time_ticks, 3_000);

        let timelines = mkv_cues_from_blocks(
            document.timecode_scale_ns,
            &document.tracks,
            &document.blocks,
        );
        assert_eq!(timelines.len(), 1);
        assert_eq!(timelines[0].id, "mkv:2");
        assert_eq!(timelines[0].cues.len(), 2);
        assert_eq!(timelines[0].cues[0].start_ms, 1_000);
        assert_eq!(timelines[0].cues[0].end_ms, 3_000);
        assert_eq!(timelines[0].cues[0].text, "Hi");
        assert_eq!(timelines[0].cues[1].start_ms, 3_000);
        assert_eq!(timelines[0].cues[1].end_ms, 5_000);
        assert_eq!(timelines[0].cues[1].text, "There");
    }

    #[test]
    fn mkv_defaults_scale_and_honours_webvtt_payload_timing() {
        let payload = b"WEBVTT\n\n00:00:00.500 --> 00:00:02.500\nHello\n";
        let document = parse_mkv_document(&mkv_fixture(false, 5_000, &[block(2, 0, payload)]));
        assert_eq!(document.timecode_scale_ns, DEFAULT_MKV_SCALE_NS);
        let timelines = mkv_cues_from_blocks(
            document.timecode_scale_ns,
            &document.tracks,
            &document.blocks,
        );
        assert_eq!(timelines.len(), 1);
        assert_eq!(timelines[0].cues.len(), 1);
        assert_eq!(timelines[0].cues[0].start_ms, 500);
        assert_eq!(timelines[0].cues[0].end_ms, 2_500);
        assert_eq!(timelines[0].cues[0].text, "Hello");
    }

    #[test]
    fn malformed_bytes_never_panic() {
        assert!(parse_mp4_from_bytes(&[0, 0, 0, 12, b'm', b'o', b'o', b'v', 0, 0, 0]).is_empty());
        assert!(parse_mp4_from_bytes(&[]).is_empty());
        assert!(parse_mp4_from_bytes(&[0xff; 64]).is_empty());
        assert!(parse_mkv_from_bytes(&[0x18, 0x53, 0x80]).is_empty());
        let document = parse_mkv_document(&[0x1F, 0x43, 0xB6, 0x75, 0x8A, 0xA3, 0x81]);
        assert!(document.tracks.is_empty());
        assert!(document.blocks.is_empty());
    }

    #[test]
    fn range_store_serves_in_memory_bytes() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let bytes = vec![0u8; 32];
            let mut store = ByteStore::from_bytes(bytes);
            assert_eq!(store.read_exact(4, 8).await.unwrap(), vec![0u8; 8]);
            assert_eq!(store.read_cached(2, 4).await.unwrap(), vec![0u8; 4]);
            assert!(store.read_exact(1_000, 16).await.is_err());
        });
    }
}

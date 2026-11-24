//! Subtitle fonts (`SubtitleFonts`, `SubtitleFontResolver`): the original's built-in choices, every
//! font installed in Windows, and files the user imports (TTF/OTF up to 10 MB), copied to
//! `%APPDATA%\NuvioNextGen\subtitle-fonts` and given to mpv as `sub-fonts-dir`. Nothing is downloaded.

use nuvio_store::nextgen::AppStore;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[cfg(windows)]
use windows::core::{w, PWSTR};
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{EnumFontFamiliesExW, GetDC, ReleaseDC, DEFAULT_CHARSET, LOGFONTW, TEXTMETRICW};
#[cfg(windows)]
use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW};

/// `MaxSubtitleFontBytes`.
const MAX_BYTES: u64 = 10 * 1024 * 1024;
const DOC: &str = "subtitle-fonts";
const SYSTEM: &str = "system:";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Font {
    /// `custom-<uuid>`, as in the original.
    pub id: String,
    pub name: String,
    pub family: String,
    file: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontList {
    pub imported: Vec<Font>,
    pub system: Vec<String>,
}

fn dir(store: &AppStore) -> PathBuf {
    store.dir().join("subtitle-fonts")
}

pub fn imported(store: &AppStore) -> Vec<Font> {
    store.read::<Vec<Font>>(DOC).ok().flatten().unwrap_or_default()
}

pub fn list(store: &AppStore) -> FontList {
    FontList { imported: imported(store), system: system_families() }
}

/// What mpv gets for a font id: (family, folder of the imported fonts).
pub fn mpv_font(store: &AppStore, id: &str) -> (String, Option<String>) {
    let family = match id {
        "serif" => "serif".to_string(),
        "mono" => "monospace".to_string(),
        _ => match id.strip_prefix(SYSTEM).filter(|f| !f.trim().is_empty()) {
            Some(family) => family.to_string(),
            None => match imported(store).into_iter().find(|f| f.id == id) {
                Some(font) => return (font.family, Some(dir(store).to_string_lossy().into_owned())),
                None => "sans-serif".to_string(),
            },
        },
    };
    (family, None)
}

/// The family in the `name` table (typographic family, else family), as the original reads it.
/// `None`: not a single TrueType / OpenType font.
fn family_name(bytes: &[u8]) -> Option<String> {
    let u16_at = |at: usize| bytes.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]) as usize);
    let u32_at = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
    if !matches!(bytes.get(..4)?, [0, 1, 0, 0] | b"OTTO" | b"true") {
        return None;
    }
    let table = (0..u16_at(4)?).map(|i| 12 + i * 16).find(|&at| bytes.get(at..at + 4) == Some(b"name".as_slice()))?;
    let table = u32_at(table + 8)?;
    let strings = table + u16_at(table + 4)?;
    let records: Vec<usize> = (0..u16_at(table + 2)?).map(|i| table + 6 + i * 12).collect();
    let read = |wanted: usize| -> Option<String> {
        let mut found: Vec<usize> = records.iter().copied().filter(|&r| u16_at(r + 6) == Some(wanted)).collect();
        found.sort_by_key(|&r| u16_at(r) != Some(3)); // Windows names first
        found.into_iter().find_map(|record| {
            let platform = u16_at(record)?;
            let start = strings + u16_at(record + 10)?;
            let raw = bytes.get(start..start + u16_at(record + 8)?)?;
            let text = if platform == 3 || platform == 0 {
                String::from_utf16_lossy(&raw.chunks_exact(2).map(|p| u16::from_be_bytes([p[0], p[1]])).collect::<Vec<_>>())
            } else {
                raw.iter().map(|&b| b as char).collect()
            };
            let text: String = text.chars().filter(|c| !c.is_control()).collect();
            (!text.trim().is_empty()).then(|| text.trim().to_string())
        })
    };
    read(16).or_else(|| read(1))
}

/// Same file = same id, so importing twice keeps one copy.
fn font_id(bytes: &[u8]) -> String {
    let hash = |seed: u64| bytes.iter().fold(seed, |h, &b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    let hex = format!("{:016x}{:016x}", hash(0xcbf2_9ce4_8422_2325), hash(0x8422_2325_cbf2_9ce4));
    format!("custom-{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

pub fn import(store: &AppStore, path: &Path) -> Result<Font, String> {
    let size = std::fs::metadata(path).map_err(|_| "Não foi possível ler o arquivo.".to_string())?.len();
    if size > MAX_BYTES {
        return Err("A fonte passa de 10 MB.".into());
    }
    let bytes = std::fs::read(path).map_err(|_| "Não foi possível ler o arquivo.".to_string())?;
    let family = family_name(&bytes).ok_or("Este arquivo não é uma fonte TTF ou OTF válida.")?;
    let id = font_id(&bytes);
    let mut fonts = imported(store);
    if let Some(existing) = fonts.iter().find(|f| f.id == id) {
        return Ok(existing.clone());
    }
    let file = format!("{id}.{}", if bytes.starts_with(b"OTTO") { "otf" } else { "ttf" });
    let save = || -> std::io::Result<()> {
        std::fs::create_dir_all(dir(store))?;
        std::fs::write(dir(store).join(&file), &bytes)
    };
    save().map_err(|e| format!("Não foi possível guardar a fonte: {e}"))?;
    let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| family.clone());
    let font = Font { id, name, family, file };
    fonts.push(font.clone());
    store.write(DOC, &fonts).map_err(|e| e.to_string())?;
    Ok(font)
}

/// Removes Astra's copy (the user's own file is never touched).
pub fn remove(store: &AppStore, id: &str) -> Result<(), String> {
    let (gone, kept): (Vec<Font>, Vec<Font>) = imported(store).into_iter().partition(|f| f.id == id);
    store.write(DOC, &kept).map_err(|e| e.to_string())?;
    for font in gone {
        // Only a plain file name of the fonts folder.
        if Path::new(&font.file).file_name().is_some_and(|n| n.to_string_lossy() == font.file) {
            let _ = std::fs::remove_file(dir(store).join(&font.file)); // may be in use by mpv
        }
    }
    Ok(())
}

/// The bytes of an imported font, for the preview.
pub fn data(store: &AppStore, id: &str) -> Result<Vec<u8>, String> {
    let font = imported(store).into_iter().find(|f| f.id == id).ok_or("Fonte não encontrada.")?;
    let file = Path::new(&font.file).file_name().ok_or("Fonte não encontrada.")?;
    std::fs::read(dir(store).join(file)).map_err(|_| "Fonte não encontrada.".into())
}

#[cfg(windows)]
unsafe extern "system" fn on_family(font: *const LOGFONTW, _: *const TEXTMETRICW, _: u32, data: LPARAM) -> i32 {
    // SAFETY: GDI passes a valid LOGFONTW; `data` is the Vec given to EnumFontFamiliesExW.
    unsafe {
        let face = &(*font).lfFaceName;
        let len = face.iter().position(|&c| c == 0).unwrap_or(face.len());
        (*(data.0 as *mut Vec<String>)).push(String::from_utf16_lossy(&face[..len]));
    }
    1
}

/// Font families installed in Windows (vertical `@` variants left out).
#[cfg(windows)]
fn system_families() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    // SAFETY: the DC is released below; the callback runs inside the call, while `names` lives.
    unsafe {
        let dc = GetDC(None);
        let query = LOGFONTW { lfCharSet: DEFAULT_CHARSET, ..Default::default() };
        EnumFontFamiliesExW(dc, &query, Some(on_family), LPARAM(&mut names as *mut Vec<String> as isize), 0);
        ReleaseDC(None, dc);
    }
    names.retain(|n| !n.is_empty() && !n.starts_with('@'));
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

/// The Windows file dialog. `None` = cancelled.
#[cfg(windows)]
pub fn choose_file(owner: isize) -> Option<PathBuf> {
    let mut path = [0u16; 2048];
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: HWND(owner as *mut _),
        lpstrFilter: w!("Fontes (TTF, OTF)\0*.ttf;*.otf\0"),
        lpstrFile: PWSTR(path.as_mut_ptr()),
        nMaxFile: path.len() as u32,
        lpstrTitle: w!("Importar fonte para legendas"),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    // SAFETY: every pointer outlives the modal call.
    if !unsafe { GetOpenFileNameW(&mut dialog) }.as_bool() {
        return None;
    }
    let len = path.iter().position(|&c| c == 0)?;
    (len > 0).then(|| PathBuf::from(String::from_utf16_lossy(&path[..len])))
}

#[cfg(not(windows))]
fn system_families() -> Vec<String> {
    // Fontconfig is the Linux equivalent; keep the subtitle font list useful there too.
    std::process::Command::new("fc-list")
        .arg(":")
        .arg("family")
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .flat_map(|l| l.split(','))
                .map(|f| f.trim().to_string())
                .filter(|f| !f.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

#[cfg(not(windows))]
pub fn choose_file(_owner: isize) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn imports_a_windows_font_once_and_rejects_other_files() {
        let dir = std::env::temp_dir().join(format!("nuvio-fonts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = AppStore::open(&dir).unwrap();
        let arial = PathBuf::from(std::env::var_os("WINDIR").unwrap()).join("Fonts").join("arial.ttf");
        let font = import(&store, &arial).unwrap();
        assert_eq!(font.family, "Arial");
        assert_eq!(import(&store, &arial).unwrap(), font);
        assert_eq!(imported(&store).len(), 1);
        assert!(font.id.len() == 43 && font.id.starts_with("custom-"));
        assert_eq!(mpv_font(&store, &font.id).0, "Arial");
        assert!(mpv_font(&store, &font.id).1.is_some());
        assert_eq!(mpv_font(&store, "system:Verdana"), ("Verdana".to_string(), None));
        assert_eq!(mpv_font(&store, "default").0, "sans-serif");
        let junk = dir.join("junk.ttf");
        std::fs::write(&junk, b"not a font").unwrap();
        assert!(import(&store, &junk).is_err());
        assert!(system_families().iter().any(|f| f == "Arial"));
        remove(&store, &font.id).unwrap();
        assert!(imported(&store).is_empty() && data(&store, &font.id).is_err());
    }
}

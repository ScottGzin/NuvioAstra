//! Test helper: capture what the window shows on screen and measure the video area.
//!
//! Capturing from the screen DC records the DWM-composed result, i.e. the mpv video plus the
//! WebView2 overlay, which is what the user sees (a window DC would miss both). The pixel
//! statistics let the self-test fail when the video area is white or flat instead of a picture.

use std::path::Path;

#[cfg(windows)]
use windows::Win32::Foundation::{HWND, POINT, RECT};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    BitBlt, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

/// Pixel statistics of a region of the composed window.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionStats {
    /// Share of near-white pixels (all channels ≥ 245).
    pub white_fraction: f64,
    /// Share of near-black pixels (all channels ≤ 10).
    pub black_fraction: f64,
    /// Standard deviation of luminance (0–255): a real picture has texture, a flat fill has ~0.
    pub luma_stddev: f64,
    /// Mean absolute difference (0–255) between two captures 0.5 s apart: video being presented
    /// changes, a stuck or uncomposited surface does not.
    pub motion: f64,
}

impl RegionStats {
    /// Frames are being presented in the region: it changes over time and is not a uniform fill.
    /// (Brightness alone is not a criterion: test patterns and skies are legitimately bright.)
    pub fn looks_like_video(&self) -> bool {
        self.motion > 0.3 && self.white_fraction < 0.98 && self.black_fraction < 0.98
    }
}

struct Capture {
    width: i32,
    height: i32,
    stride: usize,
    /// Bottom-up BGR rows, as GetDIBits returns them for a positive height.
    pixels: Vec<u8>,
}

#[cfg(not(windows))]
fn capture_client(_window: &tauri::WebviewWindow) -> Result<Capture, String> {
    Err("captura de tela não disponível no Linux nesta fase".into())
}

#[cfg(windows)]
fn capture_client(window: &tauri::WebviewWindow) -> Result<Capture, String> {
    let hwnd = window.hwnd().map(|h| HWND(h.0)).map_err(|e| e.to_string())?;
    let mut client = RECT::default();
    let mut origin = POINT::default();
    // SAFETY: GDI calls on valid handles; every created object is released below.
    unsafe {
        GetClientRect(hwnd, &mut client).map_err(|e| e.to_string())?;
        let _ = ClientToScreen(hwnd, &mut origin);
        let (w, h) = (client.right - client.left, client.bottom - client.top);
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bitmap.into());
        let _ = BitBlt(mem, 0, 0, w, h, Some(screen), origin.x, origin.y, SRCCOPY);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: h,
                biPlanes: 1,
                biBitCount: 24,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let stride = ((w * 3 + 3) & !3) as usize;
        let mut pixels = vec![0u8; stride * h as usize];
        GetDIBits(mem, bitmap, 0, h as u32, Some(pixels.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        Ok(Capture { width: w, height: h, stride, pixels })
    }
}

fn save_bmp(c: &Capture, path: &Path) -> Result<(), String> {
    let mut file = Vec::with_capacity(54 + c.pixels.len());
    let size = (54 + c.pixels.len()) as u32;
    file.extend_from_slice(b"BM");
    file.extend_from_slice(&size.to_le_bytes());
    file.extend_from_slice(&0u32.to_le_bytes());
    file.extend_from_slice(&54u32.to_le_bytes());
    file.extend_from_slice(&40u32.to_le_bytes());
    file.extend_from_slice(&c.width.to_le_bytes());
    file.extend_from_slice(&c.height.to_le_bytes());
    file.extend_from_slice(&1u16.to_le_bytes());
    file.extend_from_slice(&24u16.to_le_bytes());
    file.extend_from_slice(&0u32.to_le_bytes());
    file.extend_from_slice(&(c.pixels.len() as u32).to_le_bytes());
    file.extend_from_slice(&[0u8; 16]);
    file.extend_from_slice(&c.pixels);
    std::fs::write(path.with_extension("bmp"), file).map_err(|e| e.to_string())
}

fn stats(c: &Capture, region: (i32, i32, i32, i32)) -> RegionStats {
    let (x0, y0) = (region.0.clamp(0, c.width), region.1.clamp(0, c.height));
    let (x1, y1) = ((region.0 + region.2).clamp(0, c.width), (region.1 + region.3).clamp(0, c.height));
    let (mut n, mut white, mut black, mut sum, mut sum_sq) = (0u64, 0u64, 0u64, 0f64, 0f64);
    // Every 4th pixel in both directions is plenty for these statistics.
    for y in (y0..y1).step_by(4) {
        let row = (c.height - 1 - y) as usize * c.stride; // bottom-up rows
        for x in (x0..x1).step_by(4) {
            let i = row + x as usize * 3;
            let (b, g, r) = (c.pixels[i] as f64, c.pixels[i + 1] as f64, c.pixels[i + 2] as f64);
            n += 1;
            if r >= 245.0 && g >= 245.0 && b >= 245.0 {
                white += 1;
            }
            if r <= 10.0 && g <= 10.0 && b <= 10.0 {
                black += 1;
            }
            let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            sum += luma;
            sum_sq += luma * luma;
        }
    }
    let n = n.max(1) as f64;
    let mean = sum / n;
    RegionStats {
        white_fraction: white as f64 / n,
        black_fraction: black as f64 / n,
        luma_stddev: (sum_sq / n - mean * mean).max(0.0).sqrt(),
        motion: 0.0,
    }
}

fn motion(a: &Capture, b: &Capture, region: (i32, i32, i32, i32)) -> f64 {
    if a.width != b.width || a.height != b.height {
        return 0.0;
    }
    let (x0, y0) = (region.0.clamp(0, a.width), region.1.clamp(0, a.height));
    let (x1, y1) = ((region.0 + region.2).clamp(0, a.width), (region.1 + region.3).clamp(0, a.height));
    let (mut n, mut total) = (0u64, 0f64);
    for y in (y0..y1).step_by(4) {
        let row = (a.height - 1 - y) as usize * a.stride;
        for x in (x0..x1).step_by(4) {
            let i = row + x as usize * 3;
            for c in 0..3 {
                total += (a.pixels[i + c] as f64 - b.pixels[i + c] as f64).abs();
            }
            n += 3;
        }
    }
    total / n.max(1) as f64
}

/// Saves the composed client area (`<path>.bmp`) and returns statistics of `region`
/// (physical client pixels; `None` = whole client area).
/// Two captures 0.5 s apart; the second is saved.
pub fn capture_and_measure(window: &tauri::WebviewWindow, path: &Path, region: Option<(i32, i32, i32, i32)>) -> Result<RegionStats, String> {
    let first = capture_client(window)?;
    std::thread::sleep(std::time::Duration::from_millis(500));
    let second = capture_client(window)?;
    save_bmp(&second, path)?;
    let region = region.unwrap_or((0, 0, second.width, second.height));
    let mut result = stats(&second, region);
    result.motion = motion(&first, &second, region);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(width: i32, height: i32, bgr: [u8; 3]) -> Capture {
        let stride = ((width * 3 + 3) & !3) as usize;
        let mut pixels = vec![0u8; stride * height as usize];
        for y in 0..height as usize {
            for x in 0..width as usize {
                pixels[y * stride + x * 3..y * stride + x * 3 + 3].copy_from_slice(&bgr);
            }
        }
        Capture { width, height, stride, pixels }
    }

    #[test]
    fn static_regions_are_not_video_and_changing_ones_are() {
        // The rendering bug: a stuck white/black surface does not change between captures.
        let white = flat(64, 64, [255, 255, 255]);
        let mut s = stats(&white, (0, 0, 64, 64));
        s.motion = motion(&white, &flat(64, 64, [255, 255, 255]), (0, 0, 64, 64));
        assert!(!s.looks_like_video());
        // A bright test pattern that changes between frames is video.
        let a = flat(64, 64, [230, 230, 230]);
        let mut b = flat(64, 64, [230, 230, 230]);
        for y in 0..64usize {
            for x in 0..16usize {
                let i = y * b.stride + x * 3;
                b.pixels[i..i + 3].copy_from_slice(&[90, 90, 90]);
            }
        }
        let mut s = stats(&b, (0, 0, 64, 64));
        s.motion = motion(&a, &b, (0, 0, 64, 64));
        assert!(s.looks_like_video());
        // A fully black frame is not accepted even if it changed size.
        assert_eq!(motion(&flat(8, 8, [0, 0, 0]), &flat(16, 16, [0, 0, 0]), (0, 0, 8, 8)), 0.0);
    }
}

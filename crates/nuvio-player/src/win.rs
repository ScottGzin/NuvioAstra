//! Win32 windows owned by the player: the video container (mpv's `wid`) and the
//! picture-in-picture window. Same model as `player_bridge.cpp`: mpv renders into a container
//! child window; PiP moves that container into a separate always-on-top window with `SetParent`,
//! so playback never restarts. That window is the app's (a transparent webview that draws the
//! controls over the video, like the main one); here it only gets the container and, while it has
//! it, keeps the video's aspect when resized (`pipWindowSubclassProc` of the bridge).
//!
//! All functions here must run on the thread that owns the host window (the UI thread).

use std::sync::OnceLock;
use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    FillRect, GetMonitorInfoW, GetStockObject, MonitorFromWindow, BLACK_BRUSH, HBRUSH, HDC, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

const CONTAINER_CLASS: PCWSTR = w!("NuvioNextGenVideoContainer");
const PIP_SUBCLASS: usize = 1;

/// Default PiP placement, from `DesktopPlayerPictureInPicture.computeDefaultBounds`.
pub const PIP_DEFAULT_WIDTH: i32 = 480;
pub const PIP_MIN_WIDTH: i32 = 320;
pub const PIP_MIN_HEIGHT: i32 = 180;
pub const PIP_SCREEN_MARGIN: i32 = 24;

static CLASSES: OnceLock<()> = OnceLock::new();

fn register_classes() {
    CLASSES.get_or_init(|| unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let background = HBRUSH(GetStockObject(BLACK_BRUSH).0);
        let container = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(container_proc),
            hInstance: instance.into(),
            hbrBackground: background,
            lpszClassName: CONTAINER_CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&container);
    });
}

unsafe extern "system" fn container_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // Let clicks fall through to the webview above in the main window.
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// What the subclass of a PiP window needs (owned by it).
struct PipState {
    container: isize,
    aspect: f64,
}

/// A size that keeps `aspect`, at least 320×180. `by_height`: the height is what the user drags.
fn pip_size(width: i32, height: i32, aspect: f64, by_height: bool) -> (i32, i32) {
    let min_height = PIP_MIN_HEIGHT.max((PIP_MIN_WIDTH as f64 / aspect).round() as i32);
    let min_width = (min_height as f64 * aspect).round() as i32;
    if by_height {
        let height = height.max(min_height);
        ((height as f64 * aspect).round() as i32, height)
    } else {
        let width = width.max(min_width);
        (width, (width as f64 / aspect).round() as i32)
    }
}

unsafe extern "system" fn pip_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, data: usize) -> LRESULT {
    // SAFETY: `data` is the `PipState` boxed in `enter_pip`, freed only after the subclass is removed.
    unsafe {
        let state = &*(data as *const PipState);
        match msg {
            WM_SIZING => {
                let rect = &mut *(lparam.0 as *mut RECT);
                let edge = wparam.0 as u32;
                let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
                let by_height = match edge {
                    WMSZ_TOP | WMSZ_BOTTOM => true,
                    WMSZ_LEFT | WMSZ_RIGHT => false,
                    _ => (width as f64) < height as f64 * state.aspect,
                };
                let (width, height) = pip_size(width, height, state.aspect, by_height);
                if matches!(edge, WMSZ_LEFT | WMSZ_TOPLEFT | WMSZ_BOTTOMLEFT) {
                    rect.left = rect.right - width;
                } else {
                    rect.right = rect.left + width;
                }
                if matches!(edge, WMSZ_TOP | WMSZ_TOPLEFT | WMSZ_TOPRIGHT) {
                    rect.top = rect.bottom - height;
                } else {
                    rect.bottom = rect.top + height;
                }
                LRESULT(1)
            }
            WM_ERASEBKGND => {
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                FillRect(HDC(wparam.0 as *mut _), &rect, HBRUSH(GetStockObject(BLACK_BRUSH).0));
                LRESULT(1)
            }
            WM_WINDOWPOSCHANGED => {
                let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                let container = HWND(state.container as *mut _);
                if GetParent(container).is_ok_and(|parent| parent == hwnd) {
                    fit_to_parent(container);
                }
                result
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}

/// Creates the video container as a child of `host`, at the bottom of the z-order (below the webview).
pub fn create_container(host: HWND) -> Result<HWND, String> {
    register_classes();
    unsafe {
        let mut rect = RECT::default();
        GetClientRect(host, &mut rect).map_err(|e| e.to_string())?;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CONTAINER_CLASS,
            w!(""),
            // No WS_CLIPSIBLINGS: the webview host (`WRY_WEBVIEW`) is a sibling above this window and
            // covers the whole client area. With WS_CLIPSIBLINGS the container's visible region (and
            // therefore mpv's video window inside it) would be empty and DWM would never show the video
            // swapchain (white/black areas instead of the picture). Verified with captures.
            WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN,
            0,
            0,
            rect.right.max(1),
            rect.bottom.max(1),
            Some(host),
            None,
            GetModuleHandleW(None).ok().map(Into::into),
            None,
        )
        .map_err(|e| format!("CreateWindowEx(container): {e}"))?;
        let _ = SetWindowPos(hwnd, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        Ok(hwnd)
    }
}

/// Resizes the container to fill its current parent's client area.
pub fn fit_to_parent(container: HWND) {
    unsafe {
        if let Ok(parent) = GetParent(container) {
            let mut rect = RECT::default();
            if GetClientRect(parent, &mut rect).is_ok() {
                let _ = SetWindowPos(container, None, 0, 0, rect.right, rect.bottom, SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
    }
}

pub fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    (rect.right.max(1), rect.bottom.max(1))
}

pub fn destroy(hwnd: HWND) {
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

/// Default PiP bounds for an aspect ratio (`desktopPipBoundsForAspectRatio` + bottom-right placement).
pub fn pip_default_bounds(work_area: RECT, aspect: f64) -> (i32, i32, i32, i32) {
    let width = PIP_DEFAULT_WIDTH.max(PIP_MIN_WIDTH).max((180.0 * aspect).ceil() as i32);
    let height = ((width as f64 / aspect).round() as i32).max(PIP_MIN_HEIGHT);
    (
        work_area.right - width - PIP_SCREEN_MARGIN,
        work_area.bottom - height - PIP_SCREEN_MARGIN,
        width,
        height,
    )
}

/// Places `pip` (an app window, not shown yet) at the bottom right of `host`'s monitor, moves
/// `container` to the bottom of it and starts keeping the aspect. Returns the state for `exit_pip`.
pub fn enter_pip(container: HWND, host: HWND, pip: HWND, aspect: f64) -> Result<usize, String> {
    let aspect = if aspect.is_finite() && aspect > 0.05 { aspect } else { 16.0 / 9.0 };
    unsafe {
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(MonitorFromWindow(host, MONITOR_DEFAULTTONEAREST), &mut info).as_bool() {
            return Err("GetMonitorInfo(pip)".into());
        }
        let work = info.rcWork;
        let (_, _, w, h) = pip_default_bounds(work, aspect);
        let dpi = GetDpiForWindow(host).max(96) as f64 / 96.0;
        let (w, h) = ((w as f64 * dpi).round() as i32, (h as f64 * dpi).round() as i32);
        let (x, y) = (work.right - w - PIP_SCREEN_MARGIN, work.bottom - h - PIP_SCREEN_MARGIN);
        SetWindowPos(pip, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE).map_err(|e| format!("SetWindowPos(pip): {e}"))?;
        SetParent(container, Some(pip)).map_err(|e| format!("SetParent(pip): {e}"))?;
        let _ = SetWindowPos(container, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        fit_to_parent(container);
        let state = Box::into_raw(Box::new(PipState { container: container.0 as isize, aspect })) as usize;
        // A failure only loses the aspect lock.
        let _ = SetWindowSubclass(pip, Some(pip_proc), PIP_SUBCLASS, state);
        Ok(state)
    }
}

/// Moves `container` back into `host`. The app destroys the PiP window afterwards (never before:
/// the video's window would go with it).
pub fn exit_pip(container: HWND, host: HWND, pip: HWND, state: usize) {
    unsafe {
        let _ = RemoveWindowSubclass(pip, Some(pip_proc), PIP_SUBCLASS);
        let _ = SetParent(container, Some(host));
        let _ = SetWindowPos(container, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        fit_to_parent(container);
        // SAFETY: allocated by `enter_pip`; the subclass that used it is gone.
        drop(Box::from_raw(state as *mut PipState));
    }
}

/// Number of windows of mpv's class anywhere under `root` (test helper).
pub fn count_mpv_windows(root: HWND) -> usize {
    unsafe extern "system" fn visit(child: HWND, count: LPARAM) -> BOOL {
        let mut class = [0u16; 32];
        let len = unsafe { GetClassNameW(child, &mut class) } as usize;
        if String::from_utf16_lossy(&class[..len]) == "mpv" {
            unsafe { *(count.0 as *mut usize) += 1 };
        }
        BOOL(1)
    }
    let mut count = 0usize;
    unsafe {
        let _ = EnumChildWindows(Some(root), Some(visit), LPARAM(&mut count as *mut usize as isize));
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pip_default_bounds_match_the_original_sizing() {
        let work = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
        // 16:9 → 480×270, bottom-right with a 24 px margin.
        assert_eq!(pip_default_bounds(work, 16.0 / 9.0), (1920 - 480 - 24, 1040 - 270 - 24, 480, 270));
        // Very wide video: width grows so the height reaches 180 (ceil(180 × ratio)).
        assert_eq!(pip_default_bounds(work, 4.0).2, 720);
        assert_eq!(pip_default_bounds(work, 4.0).3, 180);
        // Resizing keeps the aspect and the minimum.
        assert_eq!(pip_size(640, 999, 16.0 / 9.0, false), (640, 360));
        assert_eq!(pip_size(999, 450, 16.0 / 9.0, true), (800, 450));
        assert_eq!(pip_size(100, 50, 16.0 / 9.0, false), (320, 180));
        assert_eq!(pip_size(100, 50, 4.0, false), (720, 180));
    }
}

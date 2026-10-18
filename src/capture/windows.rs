//! Top-level window enumeration shared by the window-pick overlay and the
//! "Open window" tray submenu.

use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetWindowLongW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, IsIconic, IsWindowVisible, WS_CHILD, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};

#[derive(Clone)]
pub struct WindowEntry {
    pub hwnd: HWND,
    pub title: String,
    /// Visible frame bounds (no invisible resize borders), screen coords.
    pub rect: RECT,
    pub minimized: bool,
}

/// All capturable top-level windows in Z-order (topmost first), including
/// ones hidden behind others and minimized ones. `skip` is excluded.
pub fn list(skip: HWND) -> Vec<WindowEntry> {
    let mut ctx = (skip, Vec::<WindowEntry>::new());
    unsafe {
        EnumWindows(Some(cb), &mut ctx as *mut _ as LPARAM);
    }
    ctx.1
}

unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        let ctx = &mut *(lparam as *mut (HWND, Vec<WindowEntry>));
        if hwnd == ctx.0 || !is_capturable(hwnd) {
            return 1;
        }
        let rect = frame_bounds(hwnd);
        ctx.1.push(WindowEntry { hwnd, title: title(hwnd), rect, minimized: IsIconic(hwnd) != 0 });
        1
    }
}

pub fn title(hwnd: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(hwnd);
        if n <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; n as usize + 1];
        let got = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..got.max(0) as usize])
    }
}

fn class_is(hwnd: HWND, names: &[&str]) -> bool {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 0 {
        return false;
    }
    let cls = &buf[..n as usize];
    names.iter().any(|name| name.encode_utf16().eq(cls.iter().copied()))
}

pub fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked: u32 = 0;
    let hr = unsafe {
        DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED as u32, &mut cloaked as *mut _ as _, 4)
    };
    hr >= 0 && cloaked != 0
}

/// Visible, uncloaked, titled, non-tool top-level window that isn't part of
/// the shell (desktop, taskbar).
pub fn is_capturable(hwnd: HWND) -> bool {
    unsafe {
        if IsWindowVisible(hwnd) == 0 || is_cloaked(hwnd) {
            return false;
        }
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if style & WS_CHILD != 0 || ex & WS_EX_TOOLWINDOW != 0 {
            return false;
        }
        // Click-through overlays (game bars, GPU overlays, …).
        if ex & WS_EX_TRANSPARENT != 0 && ex & WS_EX_LAYERED != 0 {
            return false;
        }
        if GetWindowTextLengthW(hwnd) <= 0 {
            return false;
        }
        if class_is(hwnd, &["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"]) {
            return false;
        }
        if IsIconic(hwnd) == 0 {
            let r = frame_bounds(hwnd);
            if r.right - r.left <= 1 || r.bottom - r.top <= 1 {
                return false;
            }
        }
        true
    }
}

/// DWM extended frame bounds (what the user sees), falling back to
/// GetWindowRect.
pub fn frame_bounds(hwnd: HWND) -> RECT {
    unsafe {
        let mut r: RECT = core::mem::zeroed();
        let hr = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            &mut r as *mut _ as _,
            core::mem::size_of::<RECT>() as u32,
        );
        if hr < 0 || r.right <= r.left {
            GetWindowRect(hwnd, &mut r);
        }
        r
    }
}

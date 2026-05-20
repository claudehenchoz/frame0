use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GWL_EXSTYLE,
    GWL_STYLE, IsWindowVisible, WS_CHILD, WS_EX_TOOLWINDOW,
};

#[derive(Clone)]
pub struct WindowEntry {
    pub hwnd: HWND,
    pub title: String,
    pub rect: RECT,
}

pub fn list() -> Vec<WindowEntry> {
    let mut out: Vec<WindowEntry> = Vec::new();
    unsafe {
        EnumWindows(Some(cb), &mut out as *mut _ as LPARAM);
    }
    out
}

unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        if style & WS_CHILD != 0 {
            return 1;
        }
        let exstyle = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if exstyle & WS_EX_TOOLWINDOW != 0 {
            return 1;
        }
        let title_len = GetWindowTextLengthW(hwnd);
        if title_len <= 0 {
            return 1;
        }
        let mut buf = vec![0u16; (title_len + 1) as usize];
        let written = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        let title = String::from_utf16_lossy(&buf[..written as usize]);
        let mut rect: RECT = core::mem::zeroed();
        GetWindowRect(hwnd, &mut rect);
        (&mut *(lparam as *mut Vec<WindowEntry>)).push(WindowEntry { hwnd, title, rect });
        1
    }
}

pub mod clipboard;
pub mod com;
pub mod dialog;
pub mod dpi;
pub mod hotkey;
pub mod tray;
pub mod window;

use windows_sys::Win32::Foundation::HWND;

pub fn wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

pub const HWND_NULL: HWND = 0;

use crate::error::{Error, Result};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
    VK_DOWN, VK_END, VK_F1, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_F10, VK_F11,
    VK_F12, VK_HOME, VK_INSERT, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RIGHT, VK_SNAPSHOT, VK_SPACE, VK_UP,
};

pub const HOTKEY_ID_CAPTURE: i32 = 1;

#[derive(Clone, Copy)]
pub struct HotkeySpec {
    pub mods: u32, // bitfield of MOD_*
    pub vk: u32,
}

impl HotkeySpec {
    pub fn default_capture() -> Self {
        Self {
            mods: (MOD_CONTROL | MOD_WIN | MOD_NOREPEAT) as u32,
            vk: VK_NEXT as u32, // PgDn
        }
    }
}

/// Parse a hotkey string like "Ctrl+Win+PgDn" into a HotkeySpec.
pub fn parse(s: &str) -> Option<HotkeySpec> {
    let mut mods: u32 = MOD_NOREPEAT as u32;
    let mut vk: Option<u32> = None;
    for raw in s.split('+') {
        let part = raw.trim();
        let lower = part.to_ascii_lowercase();
        match lower.as_str() {
            "ctrl" | "control" => mods |= MOD_CONTROL as u32,
            "alt" => mods |= MOD_ALT as u32,
            "shift" => mods |= MOD_SHIFT as u32,
            "win" | "windows" | "super" | "meta" => mods |= MOD_WIN as u32,
            _ => {
                vk = key_from_name(&lower);
                if vk.is_none() && part.len() == 1 {
                    let c = part.as_bytes()[0].to_ascii_uppercase();
                    if c.is_ascii_alphanumeric() {
                        vk = Some(c as u32);
                    }
                }
            }
        }
    }
    Some(HotkeySpec { mods, vk: vk? })
}

fn key_from_name(s: &str) -> Option<u32> {
    Some(match s {
        "pgdn" | "pagedown" | "next" => VK_NEXT as u32,
        "pgup" | "pageup" | "prior" => VK_PRIOR as u32,
        "home" => VK_HOME as u32,
        "end" => VK_END as u32,
        "insert" | "ins" => VK_INSERT as u32,
        "space" => VK_SPACE as u32,
        "left" => VK_LEFT as u32,
        "right" => VK_RIGHT as u32,
        "up" => VK_UP as u32,
        "down" => VK_DOWN as u32,
        "printscreen" | "prtsc" | "snapshot" => VK_SNAPSHOT as u32,
        "f1" => VK_F1 as u32,
        "f2" => VK_F2 as u32,
        "f3" => VK_F3 as u32,
        "f4" => VK_F4 as u32,
        "f5" => VK_F5 as u32,
        "f6" => VK_F6 as u32,
        "f7" => VK_F7 as u32,
        "f8" => VK_F8 as u32,
        "f9" => VK_F9 as u32,
        "f10" => VK_F10 as u32,
        "f11" => VK_F11 as u32,
        "f12" => VK_F12 as u32,
        _ => return None,
    })
}

pub fn register(hwnd: HWND, id: i32, spec: HotkeySpec) -> Result<()> {
    unsafe {
        if RegisterHotKey(hwnd, id, spec.mods, spec.vk) == 0 {
            return Err(Error::last_win32());
        }
    }
    Ok(())
}

pub fn unregister(hwnd: HWND, id: i32) {
    unsafe {
        UnregisterHotKey(hwnd, id);
    }
}

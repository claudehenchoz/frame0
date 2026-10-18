use crate::error::{Error, Result};
use crate::win::wide;
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CheckMenuRadioItem, CreatePopupMenu, DestroyMenu, GetCursorPos, HICON, HMENU,
    IDI_APPLICATION, LoadIconW, MF_BYCOMMAND, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR,
    MF_STRING, PostMessageW, RegisterWindowMessageW, SetForegroundWindow, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL, WM_RBUTTONUP,
};

pub const WM_TRAY: u32 = 0x8000 + 1; // WM_APP + 1
pub const TRAY_UID: u32 = 1;

pub struct Tray {
    pub hwnd: HWND,
    nid: NOTIFYICONDATAW,
    pub taskbar_created_msg: u32,
}

fn copy_w(dst: &mut [u16], s: &str) {
    let w = wide(s);
    let n = w.len().min(dst.len());
    dst[..n].copy_from_slice(&w[..n]);
    dst[dst.len() - 1] = 0;
}

impl Tray {
    pub fn install(hwnd: HWND, tooltip: &str) -> Result<Self> {
        unsafe {
            let hicon: HICON = LoadIconW(0, IDI_APPLICATION);
            let mut nid: NOTIFYICONDATAW = core::mem::zeroed();
            nid.cbSize = core::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = TRAY_UID;
            nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            nid.uCallbackMessage = WM_TRAY;
            nid.hIcon = hicon;
            copy_w(&mut nid.szTip, tooltip);

            if Shell_NotifyIconW(NIM_ADD, &nid) == 0 {
                return Err(Error::msg("Shell_NotifyIcon NIM_ADD failed"));
            }

            let tb_name = wide("TaskbarCreated");
            let taskbar_created_msg = RegisterWindowMessageW(tb_name.as_ptr());

            Ok(Self { hwnd, nid, taskbar_created_msg })
        }
    }

    pub fn re_add(&self) {
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &self.nid);
        }
    }

    pub fn modify_tip(&self, tip: &str) {
        let mut nid = self.nid;
        nid.uFlags = NIF_TIP;
        copy_w(&mut nid.szTip, tip);
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    /// Show a notification balloon / toast from the tray icon.
    pub fn balloon(&self, title: &str, text: &str, warning: bool) {
        let mut nid = self.nid;
        nid.uFlags = NIF_INFO;
        copy_w(&mut nid.szInfoTitle, title);
        copy_w(&mut nid.szInfo, text);
        nid.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO };
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    pub fn remove(&self) {
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &self.nid);
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
    }
}

/// Owned popup menu with a small builder API. Submenus are owned by their
/// parent once attached (DestroyMenu is recursive).
pub struct Menu(pub HMENU);

impl Menu {
    pub fn new() -> Self {
        Menu(unsafe { CreatePopupMenu() })
    }

    pub fn item(&self, id: u16, text: &str) -> &Self {
        self.item_flags(id, text, true, false)
    }

    pub fn item_flags(&self, id: u16, text: &str, enabled: bool, checked: bool) -> &Self {
        let mut f = MF_STRING;
        if !enabled {
            f |= MF_GRAYED;
        }
        if checked {
            f |= MF_CHECKED;
        }
        let t = wide(text);
        unsafe { AppendMenuW(self.0, f, id as usize, t.as_ptr()) };
        self
    }

    pub fn sep(&self) -> &Self {
        unsafe { AppendMenuW(self.0, MF_SEPARATOR, 0, core::ptr::null()) };
        self
    }

    pub fn sub(&self, text: &str, sub: Menu, enabled: bool) -> &Self {
        let mut f = MF_STRING | MF_POPUP;
        if !enabled {
            f |= MF_GRAYED;
        }
        let t = wide(text);
        let h = sub.0;
        core::mem::forget(sub);
        unsafe { AppendMenuW(self.0, f, h as usize, t.as_ptr()) };
        self
    }

    /// Radio-check `checked` within the id range [first, last].
    pub fn radio(&self, first: u16, last: u16, checked: u16) -> &Self {
        unsafe {
            CheckMenuRadioItem(self.0, first as u32, last as u32, checked as u32, MF_BYCOMMAND);
        }
        self
    }

    /// Show at the cursor; returns the chosen command id or 0.
    pub fn track(&self, hwnd: HWND) -> u16 {
        unsafe {
            let mut pt = core::mem::zeroed();
            GetCursorPos(&mut pt);
            SetForegroundWindow(hwnd);
            let cmd = TrackPopupMenu(
                self.0,
                TPM_RIGHTBUTTON | TPM_RETURNCMD,
                pt.x,
                pt.y,
                0,
                hwnd,
                core::ptr::null(),
            );
            // KB135788: make the menu dismiss correctly next time.
            PostMessageW(hwnd, WM_NULL, 0, 0);
            cmd as u16
        }
    }
}

impl Drop for Menu {
    fn drop(&mut self) {
        unsafe { DestroyMenu(self.0) };
    }
}

/// Escape `&` so menu labels show it literally, and truncate long titles.
pub fn menu_label(s: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max_chars * 2));
    for (n, c) in s.chars().enumerate() {
        if n == max_chars {
            out.push('…');
            break;
        }
        if c == '&' {
            out.push('&');
        }
        out.push(c);
    }
    out
}

/// Convert the LPARAM from a WM_TRAY message into the underlying mouse message.
pub fn tray_event(lparam: LPARAM) -> u32 {
    (lparam as u32) & 0xFFFF
}

#[inline]
pub fn is_right_click(lparam: LPARAM) -> bool {
    tray_event(lparam) == WM_RBUTTONUP
}

#[inline]
pub fn is_double_click(lparam: LPARAM) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONDBLCLK;
    tray_event(lparam) == WM_LBUTTONDBLCLK
}

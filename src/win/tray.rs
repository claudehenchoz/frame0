use crate::error::{Error, Result};
use crate::win::wide;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, HICON, HMENU, IDI_APPLICATION,
    LoadIconW, MF_SEPARATOR, MF_STRING, RegisterWindowMessageW, SetForegroundWindow,
    TPM_RIGHTBUTTON, TrackPopupMenu, WM_RBUTTONUP,
};

pub const WM_TRAY: u32 = 0x8000 + 1; // WM_APP + 1
pub const TRAY_UID: u32 = 1;

pub const CMD_TAKE: u16 = 100;
pub const CMD_EDIT_FROM_FILE: u16 = 101;
pub const CMD_REPEAT_LAST: u16 = 102;
pub const CMD_EXIT: u16 = 103;

pub struct Tray {
    pub hwnd: HWND,
    nid: NOTIFYICONDATAW,
    pub taskbar_created_msg: u32,
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

            let tip = wide(tooltip);
            let copy_len = tip.len().min(nid.szTip.len());
            nid.szTip[..copy_len].copy_from_slice(&tip[..copy_len]);

            if Shell_NotifyIconW(NIM_ADD, &nid) == 0 {
                return Err(Error::msg("Shell_NotifyIcon NIM_ADD failed"));
            }

            let tb_name = wide("TaskbarCreated");
            let taskbar_created_msg = RegisterWindowMessageW(tb_name.as_ptr());

            Ok(Self {
                hwnd,
                nid,
                taskbar_created_msg,
            })
        }
    }

    pub fn re_add(&self) {
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &self.nid);
        }
    }

    pub fn modify_tip(&mut self, tip: &str) {
        let tipw = wide(tip);
        let copy_len = tipw.len().min(self.nid.szTip.len());
        self.nid.szTip[..copy_len].copy_from_slice(&tipw[..copy_len]);
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &self.nid);
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

/// Show the tray right-click context menu at the current cursor position.
/// Returns the selected command id, or 0 if nothing was selected.
pub unsafe fn show_context_menu(hwnd: HWND) -> u16 {
    unsafe {
        let menu: HMENU = CreatePopupMenu();
        let s_take = wide("Take screenshot\tCtrl+Win+PgDn");
        let s_edit = wide("Edit screenshot from file…");
        let s_repeat = wide("Repeat last capture");
        let s_exit = wide("Exit");
        AppendMenuW(menu, MF_STRING, CMD_TAKE as usize, s_take.as_ptr());
        AppendMenuW(menu, MF_STRING, CMD_EDIT_FROM_FILE as usize, s_edit.as_ptr());
        AppendMenuW(menu, MF_STRING, CMD_REPEAT_LAST as usize, s_repeat.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, core::ptr::null());
        AppendMenuW(menu, MF_STRING, CMD_EXIT as usize, s_exit.as_ptr());

        let mut pt = core::mem::zeroed();
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);

        let cmd = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | windows_sys::Win32::UI::WindowsAndMessaging::TPM_RETURNCMD,
            pt.x,
            pt.y,
            0,
            hwnd,
            core::ptr::null(),
        );
        DestroyMenu(menu);
        cmd as u16
    }
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

// Silence unused-import warning when these aren't used elsewhere in this module.
#[allow(dead_code)]
const _USED: (WPARAM, LRESULT) = (0, 0);

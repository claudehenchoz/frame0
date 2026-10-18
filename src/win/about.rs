// Frame Zero - a tiny Windows screenshot and annotation tool
// Copyright (C) 2026 Claude Henchoz
// SPDX-License-Identifier: GPL-3.0-or-later
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Help → About dialog, built from an in-memory dialog template (no .rc
//! dialog resource). The source link is a SysLink, which needs Common
//! Controls v6 (enabled by `res/frame0.manifest`).

use crate::win::wide;
use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{CreateFontIndirectW, DeleteObject, GetObjectW, LOGFONTW};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{ICC_LINK_CLASS, INITCOMMONCONTROLSEX, InitCommonControlsEx, NMHDR};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DLGTEMPLATE, DestroyIcon, DialogBoxIndirectParamW, EndDialog, GetClientRect, GetDlgItem,
    IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, STM_SETICON, SW_SHOWNORMAL, SendMessageW,
    SetForegroundWindow, WM_COMMAND, WM_DESTROY, WM_GETFONT, WM_INITDIALOG, WM_NOTIFY, WM_SETFONT,
};

const ID_ICON: u16 = 200;
const ID_TITLE: u16 = 201;
const ID_LINK: u16 = 202;
const SOURCE_URL: &str = "https://github.com/claudehenchoz/frame0";
const NM_CLICK: u32 = -2i32 as u32;
const NM_RETURN: u32 = -4i32 as u32;

/// Show the modal About box. `owner` may be 0 (tray), in which case the
/// dialog centres on the screen and takes the foreground itself.
pub fn show(owner: HWND) {
    let template = build_template();
    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: core::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LINK_CLASS,
        };
        InitCommonControlsEx(&icc);
        DialogBoxIndirectParamW(
            GetModuleHandleW(core::ptr::null()),
            template.as_ptr() as *const DLGTEMPLATE,
            owner,
            Some(dlg_proc),
            0,
        );
    }
}

/// DLGTEMPLATE + DLGITEMTEMPLATEs, returned as u32s so the buffer is
/// DWORD-aligned as the dialog manager requires.
fn build_template() -> Vec<u32> {
    let mut buf: Vec<u8> = Vec::new();

    fn u16_(buf: &mut Vec<u8>, v: u16) {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u32_(buf: &mut Vec<u8>, v: u32) {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    fn wstr(buf: &mut Vec<u8>, s: &str) {
        for c in s.encode_utf16() {
            u16_(buf, c);
        }
        u16_(buf, 0);
    }
    // One DLGITEMTEMPLATE. `class` is either a predefined atom or a class name.
    fn item(buf: &mut Vec<u8>, style: u32, rect: [i16; 4], id: u16, class: Result<u16, &str>, text: &str) {
        while buf.len() % 4 != 0 {
            buf.push(0);
        }
        u32_(buf, 0x4000_0000 | 0x1000_0000 | style); // WS_CHILD | WS_VISIBLE
        u32_(buf, 0);
        for v in rect {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        u16_(buf, id);
        match class {
            Ok(atom) => {
                u16_(buf, 0xFFFF);
                u16_(buf, atom);
            }
            Err(name) => wstr(buf, name),
        }
        wstr(buf, text);
        u16_(buf, 0); // no creation data
    }

    // WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_CENTER | DS_MODALFRAME | DS_SETFONT
    u32_(&mut buf, 0x8000_0000 | 0x00C0_0000 | 0x0008_0000 | 0x0800 | 0x0080 | 0x0040);
    u32_(&mut buf, 0); // exstyle
    u16_(&mut buf, 7); // cdit
    for v in [0i16, 0, 230, 104] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    u16_(&mut buf, 0); // no menu
    u16_(&mut buf, 0); // default dialog class
    wstr(&mut buf, "About Frame Zero");
    u16_(&mut buf, 9);
    wstr(&mut buf, "Segoe UI");

    const STATIC: Result<u16, &str> = Ok(0x0082);
    const BUTTON: Result<u16, &str> = Ok(0x0080);
    // SS_ICON | SS_CENTERIMAGE; the icon is loaded at the control's pixel size.
    item(&mut buf, 0x0003 | 0x0200, [10, 10, 34, 34], ID_ICON, STATIC, "");
    item(&mut buf, 0, [54, 10, 166, 14], ID_TITLE, STATIC, "Frame Zero");
    let mut version = String::from("Version ");
    version.push_str(env!("CARGO_PKG_VERSION"));
    item(&mut buf, 0, [54, 26, 166, 10], 0xFFFF, STATIC, &version);
    item(&mut buf, 0, [54, 40, 166, 10], 0xFFFF, STATIC, "Copyright \u{00A9} 2026 Claude Henchoz");
    item(&mut buf, 0, [54, 52, 166, 18], 0xFFFF, STATIC, "Licensed under the GNU General Public License, version 3 or later.");
    let mut link = String::from("Source: <a href=\"");
    link.push_str(SOURCE_URL);
    link.push_str("\">");
    link.push_str(SOURCE_URL.trim_start_matches("https://"));
    link.push_str("</a>");
    item(&mut buf, 0x0001_0000, [54, 72, 166, 10], ID_LINK, Err("SysLink"), &link); // WS_TABSTOP
    item(&mut buf, 0x0001_0000 | 0x01, [170, 84, 50, 14], 1, BUTTON, "OK"); // BS_DEFPUSHBUTTON, IDOK

    while buf.len() % 4 != 0 {
        buf.push(0);
    }
    buf.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

unsafe extern "system" fn dlg_proc(hdlg: HWND, msg: u32, w: WPARAM, l: LPARAM) -> isize {
    unsafe {
        match msg {
            WM_INITDIALOG => {
                // App icon at the icon control's actual (DPI-scaled) size.
                let ctrl = GetDlgItem(hdlg, ID_ICON as i32);
                let mut rc: RECT = core::mem::zeroed();
                GetClientRect(ctrl, &mut rc);
                let size = (rc.right - rc.left).min(rc.bottom - rc.top);
                let icon = LoadImageW(GetModuleHandleW(core::ptr::null()), 1 as _, IMAGE_ICON, size, size, LR_DEFAULTCOLOR);
                if icon != 0 {
                    SendMessageW(ctrl, STM_SETICON, icon as WPARAM, 0);
                }
                // Title in a larger bold variant of the dialog font.
                let base = SendMessageW(hdlg, WM_GETFONT, 0, 0);
                let mut lf: LOGFONTW = core::mem::zeroed();
                if GetObjectW(base, core::mem::size_of::<LOGFONTW>() as i32, &mut lf as *mut _ as *mut _) != 0 {
                    lf.lfWeight = 700;
                    lf.lfHeight = lf.lfHeight * 3 / 2;
                    SendMessageW(GetDlgItem(hdlg, ID_TITLE as i32), WM_SETFONT, CreateFontIndirectW(&lf) as WPARAM, 0);
                }
                // Opened from the tray there's no owner to inherit the foreground.
                SetForegroundWindow(hdlg);
                // Focus OK rather than the link (first tab stop); return 0 so it sticks.
                SetFocus(GetDlgItem(hdlg, 1));
                0
            }
            WM_NOTIFY => {
                let hdr = &*(l as *const NMHDR);
                if hdr.idFrom == ID_LINK as usize && (hdr.code == NM_CLICK || hdr.code == NM_RETURN) {
                    ShellExecuteW(hdlg, wide("open").as_ptr(), wide(SOURCE_URL).as_ptr(), core::ptr::null(), core::ptr::null(), SW_SHOWNORMAL);
                    return 1;
                }
                0
            }
            WM_COMMAND => {
                let id = w & 0xFFFF;
                if id == 1 || id == 2 {
                    EndDialog(hdlg, 0);
                    1
                } else {
                    0
                }
            }
            WM_DESTROY => {
                // Free the icon and title font created in WM_INITDIALOG.
                let old = SendMessageW(GetDlgItem(hdlg, ID_ICON as i32), STM_SETICON, 0, 0);
                if old != 0 {
                    DestroyIcon(old);
                }
                let font = SendMessageW(GetDlgItem(hdlg, ID_TITLE as i32), WM_GETFONT, 0, 0);
                if font != 0 {
                    DeleteObject(font);
                }
                0
            }
            _ => 0,
        }
    }
}

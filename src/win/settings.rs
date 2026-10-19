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

//! Settings dialog: default folders for projects (Ctrl+S) and exports
//! (Ctrl+E). Same plain-Win32 construction as `prompt.rs`.

use crate::win::prompt::draw_button;
use crate::win::wide;
use std::cell::RefCell;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    COLOR_WINDOW, CreateFontIndirectW, DeleteObject, GetMonitorInfoW, GetSysColor,
    GetSysColorBrush, HDC, HFONT, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    SetBkColor,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::DRAWITEMSTRUCT;
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, SystemParametersInfoForDpi};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BS_OWNERDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, ES_AUTOHSCROLL, GetCursorPos,
    GetWindowTextLengthW, GetWindowTextW, IDC_ARROW, IDCANCEL, IDOK, LoadCursorW,
    NONCLIENTMETRICSW, RegisterClassExW, SPI_GETNONCLIENTMETRICS, SW_SHOW, SendMessageW,
    SetForegroundWindow, SetWindowTextW, ShowWindow, WM_CLOSE, WM_COMMAND, WM_CTLCOLORSTATIC,
    WM_DRAWITEM, WM_SETFONT, WNDCLASSEXW, WS_BORDER, WS_CAPTION, WS_CHILD, WS_EX_DLGMODALFRAME,
    WS_EX_TOPMOST, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};

const CLASS_NAME: &str = "frame0.settings";
const DM_GETDEFID: u32 = 0x0400;
const ID_EDIT0: usize = 1000;
const ID_BROWSE0: usize = 1100;
const ROWS: [&str; 2] = ["Projects folder", "Exports folder"];

struct State {
    hwnd: HWND,
    edits: [HWND; 2],
    result: Option<[String; 2]>,
    done: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Edit the default project / export folders. Returns them on OK.
pub fn show(owner: HWND, project: &str, export: &str) -> Option<(String, String)> {
    if STATE.with(|s| s.borrow().is_some()) {
        return None;
    }
    unsafe {
        register_class();
        let dpi = GetDpiForSystem().max(96);
        let px = |v: i32| v * dpi as i32 / 96;
        let (margin, label_w, edit_w, row_h, gap, btn_w, btn_h) =
            (px(14), px(110), px(340), px(24), px(10), px(84), px(28));
        let client_w = margin * 2 + label_w + edit_w + px(8) + btn_w;
        let hint_y = margin + 2 * (row_h + gap);
        let client_h = hint_y + row_h + px(14) + btn_h + margin;

        let ex = WS_EX_DLGMODALFRAME | WS_EX_TOPMOST;
        let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
        let mut wr = RECT { left: 0, top: 0, right: client_w, bottom: client_h };
        AdjustWindowRectExForDpi(&mut wr, style, 0, ex, dpi);
        let (ww, wh) = (wr.right - wr.left, wr.bottom - wr.top);

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let mut mi: MONITORINFO = core::mem::zeroed();
        mi.cbSize = core::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST), &mut mi);
        let w = mi.rcWork;
        let (x, y) = (w.left + (w.right - w.left - ww) / 2, w.top + (w.bottom - w.top - wh) / 2);

        let hinst = GetModuleHandleW(core::ptr::null());
        let hwnd = CreateWindowExW(
            ex, wide(CLASS_NAME).as_ptr(), wide("Frame Zero settings").as_ptr(), style, x, y, ww, wh,
            owner, 0, hinst, core::ptr::null(),
        );
        if hwnd == 0 {
            return None;
        }

        let mut ncm: NONCLIENTMETRICSW = core::mem::zeroed();
        ncm.cbSize = core::mem::size_of::<NONCLIENTMETRICSW>() as u32;
        SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, ncm.cbSize, &mut ncm as *mut _ as _, 0, dpi);
        let font: HFONT = CreateFontIndirectW(&ncm.lfMessageFont);

        let child = |class: &str, text: &str, style: u32, id: usize, x: i32, y: i32, w: i32, h: i32| {
            let c = CreateWindowExW(
                0, wide(class).as_ptr(), wide(text).as_ptr(), WS_CHILD | WS_VISIBLE | style,
                x, y, w, h, hwnd, id as _, hinst, core::ptr::null(),
            );
            SendMessageW(c, WM_SETFONT, font as WPARAM, 0);
            c
        };

        let mut edits = [0; 2];
        for (i, (label, init)) in ROWS.iter().zip([project, export]).enumerate() {
            let y = margin + i as i32 * (row_h + gap);
            child("STATIC", label, 0x0200 /* SS_CENTERIMAGE */, 0, margin, y, label_w - px(8), row_h);
            edits[i] = child(
                "EDIT", init, WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
                ID_EDIT0 + i, margin + label_w, y, edit_w, row_h,
            );
            child("BUTTON", "Browse…", WS_TABSTOP | BS_OWNERDRAW as u32, ID_BROWSE0 + i,
                margin + label_w + edit_w + px(8), y - px(2), btn_w, row_h + px(4));
        }
        child("STATIC", "Ctrl+S / Ctrl+E save straight into these folders. Leave empty to always ask.",
            0, 0, margin, hint_y, client_w - 2 * margin, row_h);
        let by = client_h - margin - btn_h;
        child("BUTTON", "OK", WS_TABSTOP | BS_OWNERDRAW as u32, IDOK as usize,
            client_w - margin - 2 * btn_w - px(8), by, btn_w, btn_h);
        child("BUTTON", "Cancel", WS_TABSTOP | BS_OWNERDRAW as u32, IDCANCEL as usize,
            client_w - margin - btn_w, by, btn_w, btn_h);

        STATE.with(|s| *s.borrow_mut() = Some(State { hwnd, edits, result: None, done: false }));

        if owner != 0 {
            EnableWindow(owner, 0);
        }
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus(edits[0]);

        crate::win::window::modal_loop(hwnd, &|| STATE.with(|s| s.borrow().as_ref().map_or(true, |s| s.done)));

        if owner != 0 {
            EnableWindow(owner, 1);
        }
        DestroyWindow(hwnd);
        DeleteObject(font as _);
        STATE.with(|s| s.borrow_mut().take()).and_then(|s| s.result).map(|[p, e]| (p, e))
    }
}

unsafe fn register_class() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let cname = wide(CLASS_NAME);
        let wc = WNDCLASSEXW {
            cbSize: core::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: GetModuleHandleW(core::ptr::null()),
            hIcon: 0,
            hCursor: LoadCursorW(0, IDC_ARROW),
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            lpszMenuName: core::ptr::null(),
            lpszClassName: cname.as_ptr(),
            hIconSm: 0,
        };
        RegisterClassExW(&wc);
    });
}

fn window_text(hwnd: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut buf = vec![0u16; n + 1];
        let got = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
        String::from_utf16_lossy(&buf[..got]).trim().to_string()
    }
}

fn finish(ok: bool) {
    STATE.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            if ok {
                st.result = Some(st.edits.map(window_text));
            }
            st.done = true;
        }
    });
}

/// Folder picker for row `i`, starting at its current path.
fn browse(i: usize) {
    let Some((hwnd, edit)) = STATE.with(|s| s.borrow().as_ref().map(|s| (s.hwnd, s.edits[i]))) else { return };
    if let Some(path) = crate::win::dialog::pick_folder(hwnd, &window_text(edit), ROWS[i]) {
        unsafe { SetWindowTextW(edit, wide(&path).as_ptr()) };
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                match (w & 0xFFFF) as usize {
                    id if id == IDOK as usize => finish(true),
                    id if id == IDCANCEL as usize => finish(false),
                    id if (ID_BROWSE0..ID_BROWSE0 + ROWS.len()).contains(&id) => browse(id - ID_BROWSE0),
                    _ => {}
                }
                0
            }
            DM_GETDEFID => (IDOK as isize) | (0x534B << 16), // DC_HASDEFID
            WM_CLOSE => {
                finish(false);
                0
            }
            WM_CTLCOLORSTATIC => {
                SetBkColor(w as HDC, GetSysColor(COLOR_WINDOW));
                GetSysColorBrush(COLOR_WINDOW)
            }
            WM_DRAWITEM => {
                draw_button(&*(l as *const DRAWITEMSTRUCT));
                1
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

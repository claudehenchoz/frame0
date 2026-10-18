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

//! Fullscreen capture overlays (region drag, window pick). Both show a
//! frozen snapshot of the virtual screen, dimmed, with the candidate area
//! shown undimmed. Shared GDI plumbing lives here.

pub mod region;
pub mod window_pick;

use crate::gfx::bitmap::Bitmap;
use crate::win::wide;
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleDC,
    CreateDIBSection, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DIB_RGB_COLORS,
    DT_CALCRECT, DT_NOPREFIX, DT_SINGLELINE, DeleteDC, DeleteObject, DrawTextW, FW_NORMAL,
    FillRect, HBITMAP, HDC, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, VARIABLE_PITCH,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, IDC_CROSS, LoadCursorW, RegisterClassExW, SW_SHOW, SetForegroundWindow,
    ShowWindow, WNDCLASSEXW, WNDPROC, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

pub const ACCENT: u32 = 0x0066FF00; // COLORREF (BGR): green

struct Dib {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
}

impl Dib {
    unsafe fn new(w: i32, h: i32) -> Dib {
        unsafe {
            let mut bi: BITMAPINFO = core::mem::zeroed();
            bi.bmiHeader.biSize = core::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w;
            bi.bmiHeader.biHeight = -h;
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB as u32;
            let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
            let bmp = CreateDIBSection(0, &bi, DIB_RGB_COLORS, &mut bits, 0, 0);
            let dc = CreateCompatibleDC(0);
            let old = SelectObject(dc, bmp as _);
            Dib { dc, bmp, old, bits: bits as *mut u8 }
        }
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bmp as _);
            DeleteDC(self.dc);
        }
    }
}

/// Three same-sized DIBs: the clear snapshot, a dimmed copy, and the back
/// buffer each frame is composed into before one BitBlt to the window.
pub struct Backdrop {
    clear: Dib,
    dim: Dib,
    back: Dib,
    pub w: i32,
    pub h: i32,
    pub font: HFONT,
    pub scale: i32, // DPI / 96 * 100
}

impl Backdrop {
    pub fn new(bg: &Bitmap) -> Self {
        let (w, h) = (bg.width as i32, bg.height as i32);
        unsafe {
            let clear = Dib::new(w, h);
            let dim = Dib::new(w, h);
            let back = Dib::new(w, h);
            let n = bg.bgra.len();
            if !clear.bits.is_null() && !dim.bits.is_null() {
                core::ptr::copy_nonoverlapping(bg.bgra.as_ptr(), clear.bits, n);
                let d = core::slice::from_raw_parts_mut(dim.bits, n);
                for (o, s) in d.chunks_exact_mut(4).zip(bg.bgra.chunks_exact(4)) {
                    o[0] = ((s[0] as u32 * 115) >> 8) as u8;
                    o[1] = ((s[1] as u32 * 115) >> 8) as u8;
                    o[2] = ((s[2] as u32 * 115) >> 8) as u8;
                    o[3] = 0xFF;
                }
            }
            let scale = (GetDpiForSystem().max(96) * 100 / 96) as i32;
            let font = CreateFontW(
                -(14 * scale / 100), 0, 0, 0, FW_NORMAL as i32, 0, 0, 0,
                DEFAULT_CHARSET as u32, OUT_DEFAULT_PRECIS as u32, CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32, VARIABLE_PITCH as u32, wide("Segoe UI").as_ptr(),
            );
            Backdrop { clear, dim, back, w, h, font, scale }
        }
    }

    pub fn px(&self, v: i32) -> i32 {
        v * self.scale / 100
    }
    pub fn clear_dc(&self) -> HDC {
        self.clear.dc
    }
    pub fn back_dc(&self) -> HDC {
        self.back.dc
    }

    /// Start a frame: dimmed snapshot into the back buffer.
    pub unsafe fn begin(&self) {
        unsafe { blit(self.back.dc, 0, 0, self.w, self.h, self.dim.dc, 0, 0) }
    }

    /// Reveal (undim) a rect of the snapshot.
    pub unsafe fn reveal(&self, x: i32, y: i32, w: i32, h: i32) {
        unsafe { blit(self.back.dc, x, y, w, h, self.clear.dc, x, y) }
    }

    /// Present the composed frame.
    pub unsafe fn present(&self, hdc: HDC) {
        unsafe { blit(hdc, 0, 0, self.w, self.h, self.back.dc, 0, 0) }
    }

    /// Text in a dark pill. (x, y) is the top-left; returns the pill size.
    pub unsafe fn label(&self, x: i32, y: i32, text: &str) -> (i32, i32) {
        unsafe {
            let dc = self.back.dc;
            let old = SelectObject(dc, self.font as _);
            let t = wide(text);
            let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            DrawTextW(dc, t.as_ptr(), -1, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
            let (pw, ph) = (r.right + self.px(12), r.bottom + self.px(6));
            let bg = RECT { left: x, top: y, right: x + pw, bottom: y + ph };
            let br = CreateSolidBrush(0x00202020);
            FillRect(dc, &bg, br);
            DeleteObject(br as _);
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, 0x00FFFFFF);
            let mut tr = RECT { left: x + self.px(6), top: y + self.px(3), right: x + pw, bottom: y + ph };
            DrawTextW(dc, t.as_ptr(), -1, &mut tr, DT_SINGLELINE | DT_NOPREFIX);
            SelectObject(dc, old);
            (pw, ph)
        }
    }

    pub unsafe fn outline(&self, x: i32, y: i32, w: i32, h: i32, width: i32, color: u32) {
        unsafe { outline(self.back.dc, x, y, w, h, width, color) }
    }
}

impl Drop for Backdrop {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.font as _) };
    }
}

pub unsafe fn blit(dst: HDC, x: i32, y: i32, w: i32, h: i32, src: HDC, sx: i32, sy: i32) {
    use windows_sys::Win32::Graphics::Gdi::{BitBlt, SRCCOPY};
    unsafe {
        BitBlt(dst, x, y, w, h, src, sx, sy, SRCCOPY);
    }
}

/// Hollow rectangle of `width` px drawn just outside (x, y, w, h).
pub unsafe fn outline(dc: HDC, x: i32, y: i32, w: i32, h: i32, width: i32, color: u32) {
    unsafe {
        let br = CreateSolidBrush(color);
        let k = width;
        for r in [
            RECT { left: x - k, top: y - k, right: x + w + k, bottom: y },
            RECT { left: x - k, top: y + h, right: x + w + k, bottom: y + h + k },
            RECT { left: x - k, top: y, right: x, bottom: y + h },
            RECT { left: x + w, top: y, right: x + w + k, bottom: y + h },
        ] {
            FillRect(dc, &r, br);
        }
        DeleteObject(br as _);
    }
}

/// Register (once per class name) and create a topmost popup covering the
/// virtual screen; brings it to the foreground with keyboard focus.
pub unsafe fn create_window(class: &str, wndproc: WNDPROC, virt: (i32, i32, i32, i32)) -> HWND {
    unsafe {
        let cname = wide(class);
        let hinst = GetModuleHandleW(core::ptr::null());
        let wc = WNDCLASSEXW {
            cbSize: core::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: wndproc,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: 0,
            hCursor: LoadCursorW(0, IDC_CROSS),
            hbrBackground: 0,
            lpszMenuName: core::ptr::null(),
            lpszClassName: cname.as_ptr(),
            hIconSm: 0,
        };
        RegisterClassExW(&wc); // fails harmlessly if already registered
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            cname.as_ptr(),
            cname.as_ptr(),
            WS_POPUP,
            virt.0,
            virt.1,
            virt.2,
            virt.3,
            0,
            0,
            hinst,
            core::ptr::null(),
        );
        hwnd
    }
}

pub unsafe fn show(hwnd: HWND) {
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
    }
}

/// Cursor position in client coordinates of an overlay at `origin`.
pub fn cursor_client(origin: (i32, i32)) -> (i32, i32) {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt) };
    (pt.x - origin.0, pt.y - origin.1)
}

pub fn lparam_xy(l: isize) -> (i32, i32) {
    ((l & 0xFFFF) as i16 as i32, ((l >> 16) & 0xFFFF) as i16 as i32)
}

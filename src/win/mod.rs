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

pub mod about;
pub mod clipboard;
pub mod com;
pub mod dialog;
pub mod dpi;
pub mod hotkey;
pub mod prompt;
pub mod tray;
pub mod window;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, HICON, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, SM_CXICON, SM_CXSMICON,
};

pub fn wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

pub const HWND_NULL: HWND = 0;

/// The embedded app icon (`res/frame0.ico`, resource id 1) at the system's
/// small (title bar / tray) or large icon size. Shared; never destroyed.
pub fn app_icon(small: bool) -> HICON {
    unsafe {
        let n = GetSystemMetrics(if small { SM_CXSMICON } else { SM_CXICON });
        LoadImageW(GetModuleHandleW(core::ptr::null()), 1 as _, IMAGE_ICON, n, n, LR_DEFAULTCOLOR)
    }
}

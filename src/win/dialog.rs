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

use crate::error::Result;
use crate::win::wide;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Shell::IFileDialog;
use windows::core::PCWSTR;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};

pub fn open_file(hwnd: HWND, title: &str, filter: &str) -> Result<Option<String>> {
    run_dialog(hwnd, title, filter, false, true)
}

pub fn save_file(hwnd: HWND, title: &str, filter: &str, default_name: &str) -> Result<Option<String>> {
    run_dialog_save(hwnd, title, filter, default_name)
}

fn run_dialog(hwnd: HWND, title: &str, filter: &str, _save: bool, open: bool) -> Result<Option<String>> {
    let title_w = wide(title);
    let filter_w = build_filter(filter);
    let mut buf = [0u16; 1024];
    unsafe {
        let mut ofn: OPENFILENAMEW = core::mem::zeroed();
        ofn.lStructSize = core::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = hwnd;
        ofn.lpstrFilter = filter_w.as_ptr();
        ofn.nFilterIndex = 1;
        ofn.lpstrFile = buf.as_mut_ptr();
        ofn.nMaxFile = buf.len() as u32;
        ofn.lpstrTitle = title_w.as_ptr();
        ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;
        let ok = if open {
            GetOpenFileNameW(&mut ofn) != 0
        } else {
            GetSaveFileNameW(&mut ofn) != 0
        };
        if !ok {
            return Ok(None);
        }
    }
    Ok(Some(wide_to_string(&buf)))
}

/// Explorer folder picker starting at `start` (if it exists).
pub fn pick_folder(owner: HWND, start: &str, title: &str) -> Option<String> {
    use windows::Win32::UI::Shell::{FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog};
    unsafe {
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        dlg.SetOptions(dlg.GetOptions().ok()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        dlg.SetTitle(PCWSTR(wide(title).as_ptr())).ok()?;
        set_folder(&dlg, start);
        show(&dlg, owner)
    }
}

/// Save dialog. `default` is a file name, or a full path whose folder the
/// dialog opens in (IFileDialog::SetFolder; GetSaveFileNameW would reopen the
/// last-used folder instead).
fn run_dialog_save(hwnd: HWND, title: &str, filter: &str, default: &str) -> Result<Option<String>> {
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST, FileSaveDialog, IFileSaveDialog};
    let segs: Vec<Vec<u16>> = filter.split('|').map(wide).collect();
    let specs: Vec<COMDLG_FILTERSPEC> = segs
        .chunks_exact(2)
        .map(|p| COMDLG_FILTERSPEC { pszName: PCWSTR(p[0].as_ptr()), pszSpec: PCWSTR(p[1].as_ptr()) })
        .collect();
    let (dir, name) = match default.rfind('\\') {
        Some(i) => (&default[..i], &default[i + 1..]),
        None => ("", default),
    };
    unsafe {
        let dlg: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
        dlg.SetOptions(dlg.GetOptions()? | FOS_OVERWRITEPROMPT | FOS_PATHMUSTEXIST)?;
        dlg.SetTitle(PCWSTR(wide(title).as_ptr()))?;
        dlg.SetFileTypes(&specs)?;
        dlg.SetFileName(PCWSTR(wide(name).as_ptr()))?;
        set_folder(&dlg, dir);
        Ok(show(&dlg, hwnd))
    }
}

unsafe fn set_folder(dlg: &IFileDialog, dir: &str) {
    use windows::Win32::UI::Shell::{IShellItem, SHCreateItemFromParsingName};
    // A folder that doesn't exist yet opens at its nearest existing parent.
    let mut dir = dir;
    while !dir.is_empty() && !std::path::Path::new(dir).is_dir() {
        dir = dir.rfind('\\').map_or("", |i| &dir[..i]);
    }
    if dir.is_empty() {
        return;
    }
    unsafe {
        if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(wide(dir).as_ptr()), None) {
            let _ = dlg.SetFolder(&item);
        }
    }
}

/// Run the dialog; `None` on cancel (which comes back as an error HRESULT).
unsafe fn show(dlg: &IFileDialog, owner: HWND) -> Option<String> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::SIGDN_FILESYSPATH;
    unsafe {
        dlg.Show(Some(windows::Win32::Foundation::HWND(owner as _))).ok()?;
        let p = dlg.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = String::from_utf16_lossy(p.as_wide());
        CoTaskMemFree(Some(p.0 as _));
        Some(s)
    }
}

fn build_filter(filter: &str) -> Vec<u16> {
    // Encode filter strings like "PNG\0*.png\0JPG\0*.jpg\0\0"
    let mut out = Vec::new();
    for seg in filter.split('|') {
        out.extend(seg.encode_utf16());
        out.push(0);
    }
    out.push(0);
    out
}

fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

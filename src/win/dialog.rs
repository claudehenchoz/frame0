use crate::error::Result;
use crate::win::wide;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OFN_FILEMUSTEXIST, OFN_OVERWRITEPROMPT,
    OFN_PATHMUSTEXIST, OPENFILENAMEW,
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

fn run_dialog_save(hwnd: HWND, title: &str, filter: &str, default_name: &str) -> Result<Option<String>> {
    let title_w = wide(title);
    let filter_w = build_filter(filter);
    let default_w = wide(default_name);
    let mut buf = [0u16; 1024];
    let copy_n = default_w.len().min(buf.len());
    buf[..copy_n].copy_from_slice(&default_w[..copy_n]);
    unsafe {
        let mut ofn: OPENFILENAMEW = core::mem::zeroed();
        ofn.lStructSize = core::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = hwnd;
        ofn.lpstrFilter = filter_w.as_ptr();
        ofn.nFilterIndex = 1;
        ofn.lpstrFile = buf.as_mut_ptr();
        ofn.nMaxFile = buf.len() as u32;
        ofn.lpstrTitle = title_w.as_ptr();
        ofn.Flags = OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST;
        if GetSaveFileNameW(&mut ofn) == 0 {
            return Ok(None);
        }
    }
    Ok(Some(wide_to_string(&buf)))
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

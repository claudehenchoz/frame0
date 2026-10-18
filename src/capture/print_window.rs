use crate::error::{Error, Result};
use crate::gfx::bitmap::Bitmap;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject,
};
use windows_sys::Win32::Storage::Xps::PrintWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, IsIconic, IsWindow, SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE, ShowWindow,
};

const PW_RENDERFULLCONTENT: u32 = 0x00000002;

/// Capture a window's content even when it is covered by other windows.
/// Minimized windows are restored without activation, captured and
/// re-minimized. The result is cropped to the DWM frame bounds so the
/// invisible resize borders don't show up as black bars.
pub fn capture(hwnd: HWND) -> Result<Bitmap> {
    unsafe {
        if IsWindow(hwnd) == 0 {
            return Err(Error::msg("window is gone"));
        }
        let was_min = IsIconic(hwnd) != 0;
        if was_min {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            // Give the restore animation and the app's repaint a moment.
            crate::win::window::sleep_ms(350);
        }
        let r = grab(hwnd);
        if was_min {
            ShowWindow(hwnd, SW_SHOWMINNOACTIVE);
        }
        r
    }
}

unsafe fn grab(hwnd: HWND) -> Result<Bitmap> {
    unsafe {
        let mut rect: RECT = core::mem::zeroed();
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return Err(Error::msg("GetWindowRect failed"));
        }
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        if w <= 0 || h <= 0 {
            return Err(Error::msg("zero-size window"));
        }
        let screen_dc = GetDC(0);
        let mem_dc = CreateCompatibleDC(screen_dc);
        let bmp = CreateCompatibleBitmap(screen_dc, w, h);
        let old = SelectObject(mem_dc, bmp as _);
        let ok = PrintWindow(hwnd, mem_dc, PW_RENDERFULLCONTENT) != 0;
        SelectObject(mem_dc, old);
        let full = if ok { Some(super::bitblt::read_dib(mem_dc, bmp, w, h)) } else { None };
        DeleteObject(bmp as _);
        DeleteDC(mem_dc);
        ReleaseDC(0, screen_dc);
        let full = full.ok_or(Error::msg("PrintWindow failed"))?;

        let fb = super::windows::frame_bounds(hwnd);
        let vis = (fb.left, fb.top, fb.right - fb.left, fb.bottom - fb.top);
        Ok(super::bitblt::crop(&full, (rect.left, rect.top), vis).unwrap_or(full))
    }
}

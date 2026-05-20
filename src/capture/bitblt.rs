use crate::error::{Error, Result};
use crate::gfx::bitmap::Bitmap;
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC,
    GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{CURSORINFO, CURSOR_SHOWING, GetCursorInfo};

pub fn virtual_screen(include_cursor: bool) -> Result<Bitmap> {
    let (x, y, w, h) = super::monitors::virtual_screen_bounds();
    region(x, y, w, h, include_cursor)
}

pub fn region(x: i32, y: i32, w: i32, h: i32, include_cursor: bool) -> Result<Bitmap> {
    if w <= 0 || h <= 0 {
        return Err(Error::msg("invalid region"));
    }
    unsafe {
        let screen_dc = GetDC(0);
        if screen_dc == 0 {
            return Err(Error::msg("GetDC failed"));
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc == 0 {
            ReleaseDC(0, screen_dc);
            return Err(Error::msg("CreateCompatibleDC failed"));
        }
        let bmp = CreateCompatibleBitmap(screen_dc, w, h);
        let old = SelectObject(mem_dc, bmp as _);

        BitBlt(mem_dc, 0, 0, w, h, screen_dc, x, y, SRCCOPY | CAPTUREBLT);

        if include_cursor {
            draw_cursor_into_dc(mem_dc, x, y);
        }

        // Now extract BGRA pixels via GetDIBits.
        let mut bi: BITMAPINFO = core::mem::zeroed();
        bi.bmiHeader.biSize = core::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h; // top-down
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB as u32;
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        GetDIBits(
            mem_dc,
            bmp,
            0,
            h as u32,
            bgra.as_mut_ptr() as _,
            &mut bi,
            DIB_RGB_COLORS,
        );
        // Force alpha to 0xFF on screen captures (BitBlt zeroes alpha channel from GDI DCs).
        for px in bgra.chunks_exact_mut(4) {
            px[3] = 0xFF;
        }

        SelectObject(mem_dc, old);
        DeleteObject(bmp as _);
        DeleteDC(mem_dc);
        ReleaseDC(0, screen_dc);

        Ok(Bitmap {
            width: w as u32,
            height: h as u32,
            bgra,
        })
    }
}

unsafe fn draw_cursor_into_dc(
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    offset_x: i32,
    offset_y: i32,
) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DrawIconEx, GetIconInfo, ICONINFO};
    unsafe {
        let mut info: CURSORINFO = core::mem::zeroed();
        info.cbSize = core::mem::size_of::<CURSORINFO>() as u32;
        if GetCursorInfo(&mut info) == 0 || (info.flags & CURSOR_SHOWING) == 0 {
            return;
        }
        let mut ii: ICONINFO = core::mem::zeroed();
        if GetIconInfo(info.hCursor as _, &mut ii) == 0 {
            return;
        }
        let cursor_x = info.ptScreenPos.x - offset_x - ii.xHotspot as i32;
        let cursor_y = info.ptScreenPos.y - offset_y - ii.yHotspot as i32;
        DrawIconEx(
            dc,
            cursor_x,
            cursor_y,
            info.hCursor as _,
            0,
            0,
            0,
            0,
            DI_NORMAL,
        );
        if ii.hbmMask != 0 {
            DeleteObject(ii.hbmMask as _);
        }
        if ii.hbmColor != 0 {
            DeleteObject(ii.hbmColor as _);
        }
    }
}

#[allow(dead_code)]
fn _silence_unused() {
    let _ = CreateDIBSection;
    let _: POINT = unsafe { core::mem::zeroed() };
}

use crate::error::{Error, Result};
use crate::gfx::bitmap::Bitmap;
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC,
    GetDIBits, HBITMAP, HDC, ReleaseDC, SRCCOPY, SelectObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CURSOR_SHOWING, CURSORINFO, DI_NORMAL, DrawIconEx, GetCursorInfo, GetIconInfo, HCURSOR,
    ICONINFO,
};

pub fn virtual_screen(include_cursor: bool) -> Result<Bitmap> {
    let (x, y, w, h) = super::monitors::virtual_screen_bounds();
    region(x, y, w, h, include_cursor)
}

/// Live BitBlt of a screen rectangle.
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
        let bmp = CreateCompatibleBitmap(screen_dc, w, h);
        if mem_dc == 0 || bmp == 0 {
            if bmp != 0 {
                DeleteObject(bmp as _);
            }
            if mem_dc != 0 {
                DeleteDC(mem_dc);
            }
            ReleaseDC(0, screen_dc);
            return Err(Error::msg("CreateCompatibleBitmap failed"));
        }
        let old = SelectObject(mem_dc, bmp as _);
        BitBlt(mem_dc, 0, 0, w, h, screen_dc, x, y, SRCCOPY | CAPTUREBLT);
        if include_cursor {
            if let Some(c) = snap_cursor() {
                draw_cursor_dc(mem_dc, &c, x, y);
            }
        }
        SelectObject(mem_dc, old);
        let out = read_dib(mem_dc, bmp, w, h);
        DeleteObject(bmp as _);
        DeleteDC(mem_dc);
        ReleaseDC(0, screen_dc);
        Ok(out)
    }
}

/// Pull the pixels of a (deselected) DDB as top-down BGRA with opaque alpha.
pub(crate) unsafe fn read_dib(dc: HDC, bmp: HBITMAP, w: i32, h: i32) -> Bitmap {
    unsafe {
        let mut bi: BITMAPINFO = core::mem::zeroed();
        bi.bmiHeader.biSize = core::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h; // top-down
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB as u32;
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        GetDIBits(dc, bmp, 0, h as u32, bgra.as_mut_ptr() as _, &mut bi, DIB_RGB_COLORS);
        // GDI leaves alpha at zero.
        for px in bgra.chunks_exact_mut(4) {
            px[3] = 0xFF;
        }
        Bitmap { width: w as u32, height: h as u32, bgra }
    }
}

/// Copy `rect` (screen coords) out of a snapshot whose top-left is at
/// `origin` (screen coords). The rect is clipped to the snapshot.
pub fn crop(src: &Bitmap, origin: (i32, i32), rect: super::ScreenRect) -> Option<Bitmap> {
    let x0 = (rect.0 - origin.0).max(0);
    let y0 = (rect.1 - origin.1).max(0);
    let x1 = (rect.0 + rect.2 - origin.0).min(src.width as i32);
    let y1 = (rect.1 + rect.3 - origin.1).min(src.height as i32);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let stride = src.width as usize * 4;
    let mut bgra = Vec::with_capacity(w * h * 4);
    for y in y0 as usize..y1 as usize {
        let off = y * stride + x0 as usize * 4;
        bgra.extend_from_slice(&src.bgra[off..off + w * 4]);
    }
    Some(Bitmap { width: w as u32, height: h as u32, bgra })
}

/// Cursor state remembered at snapshot time, so it can be composited onto a
/// crop taken later from a frozen snapshot.
#[derive(Clone, Copy)]
pub struct CursorSnap {
    cursor: HCURSOR,
    x: i32, // top-left of the cursor image, screen coords
    y: i32,
}

pub fn snap_cursor() -> Option<CursorSnap> {
    unsafe {
        let mut info: CURSORINFO = core::mem::zeroed();
        info.cbSize = core::mem::size_of::<CURSORINFO>() as u32;
        if GetCursorInfo(&mut info) == 0 || (info.flags & CURSOR_SHOWING) == 0 {
            return None;
        }
        let mut ii: ICONINFO = core::mem::zeroed();
        if GetIconInfo(info.hCursor as _, &mut ii) == 0 {
            return None;
        }
        if ii.hbmMask != 0 {
            DeleteObject(ii.hbmMask as _);
        }
        if ii.hbmColor != 0 {
            DeleteObject(ii.hbmColor as _);
        }
        Some(CursorSnap {
            cursor: info.hCursor,
            x: info.ptScreenPos.x - ii.xHotspot as i32,
            y: info.ptScreenPos.y - ii.yHotspot as i32,
        })
    }
}

unsafe fn draw_cursor_dc(dc: HDC, c: &CursorSnap, origin_x: i32, origin_y: i32) {
    unsafe {
        DrawIconEx(dc, c.x - origin_x, c.y - origin_y, c.cursor as _, 0, 0, 0, 0, DI_NORMAL);
    }
}

/// Composite a remembered cursor onto `bmp`, whose top-left is at `origin`
/// in screen coordinates.
pub fn draw_cursor(bmp: &mut Bitmap, origin: (i32, i32), c: &CursorSnap) {
    let (w, h) = (bmp.width as i32, bmp.height as i32);
    // Skip if the cursor (≤ 256 px) can't touch the bitmap.
    if c.x - origin.0 >= w || c.y - origin.1 >= h || c.x - origin.0 < -256 || c.y - origin.1 < -256 {
        return;
    }
    unsafe {
        let mut bi: BITMAPINFO = core::mem::zeroed();
        bi.bmiHeader.biSize = core::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB as u32;
        let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
        let dib = CreateDIBSection(0, &bi, DIB_RGB_COLORS, &mut bits, 0, 0);
        if dib == 0 || bits.is_null() {
            return;
        }
        let dc = CreateCompatibleDC(0);
        let old = SelectObject(dc, dib as _);
        core::ptr::copy_nonoverlapping(bmp.bgra.as_ptr(), bits as *mut u8, bmp.bgra.len());
        draw_cursor_dc(dc, c, origin.0, origin.1);
        windows_sys::Win32::Graphics::Gdi::GdiFlush();
        let px = core::slice::from_raw_parts(bits as *const u8, bmp.bgra.len());
        for (d, s) in bmp.bgra.chunks_exact_mut(4).zip(px.chunks_exact(4)) {
            d[0] = s[0];
            d[1] = s[1];
            d[2] = s[2];
        }
        SelectObject(dc, old);
        DeleteDC(dc);
        DeleteObject(dib as _);
    }
}

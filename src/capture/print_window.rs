use crate::error::{Error, Result};
use crate::gfx::bitmap::Bitmap;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject,
};
use windows_sys::Win32::Storage::Xps::PrintWindow;
const PW_RENDERFULLCONTENT: u32 = 0x00000002;
use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;

pub fn capture(hwnd: HWND) -> Result<Bitmap> {
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

        if PrintWindow(hwnd, mem_dc, PW_RENDERFULLCONTENT) == 0 {
            SelectObject(mem_dc, old);
            DeleteObject(bmp as _);
            DeleteDC(mem_dc);
            ReleaseDC(0, screen_dc);
            return Err(Error::msg("PrintWindow failed"));
        }

        let mut bi: BITMAPINFO = core::mem::zeroed();
        bi.bmiHeader.biSize = core::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h;
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

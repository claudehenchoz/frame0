//! GDI+ thin wrappers for PNG/JPG encode/decode.
//!
//! v1: file-based encode/decode only. In-memory PNG encoding for clipboard is
//! deferred to v1.5 (will require IStream COM plumbing).

use crate::error::{Error, Result};
use crate::gfx::bitmap::Bitmap;
use crate::win::wide;
use windows_sys::Win32::Graphics::GdiPlus::{
    GdipBitmapLockBits, GdipBitmapUnlockBits, GdipCreateBitmapFromFile, GdipCreateBitmapFromScan0,
    GdipDisposeImage, GdipGetImageEncoders, GdipGetImageEncodersSize, GdipGetImageHeight,
    GdipGetImageWidth, GdipSaveImageToFile, GdiplusStartup, GdiplusStartupInput, GdiplusShutdown,
    GpBitmap, GpImage, ImageCodecInfo, ImageLockModeRead, BitmapData,
};
// PixelFormat32bppARGB constant value per GDI+ headers
const PIXEL_FORMAT_32_ARGB_VAL: i32 = 2498570; // PixelFormat32bppARGB, 0x0026200A

pub fn startup() -> Result<usize> {
    let mut token: usize = 0;
    let input = GdiplusStartupInput {
        GdiplusVersion: 1,
        DebugEventCallback: 0,
        SuppressBackgroundThread: 0,
        SuppressExternalCodecs: 0,
    };
    unsafe {
        let status = GdiplusStartup(&mut token, &input, core::ptr::null_mut());
        if status != 0 {
            return Err(Error::msg("GdiplusStartup failed"));
        }
    }
    Ok(token)
}

pub fn shutdown(token: usize) {
    unsafe {
        GdiplusShutdown(token);
    }
}

const PIXEL_FORMAT_32_ARGB: i32 = PIXEL_FORMAT_32_ARGB_VAL;

fn make_gdip_bitmap(bm: &Bitmap) -> Result<*mut GpBitmap> {
    let mut out: *mut GpBitmap = core::ptr::null_mut();
    let stride = (bm.width * 4) as i32;
    unsafe {
        let status = GdipCreateBitmapFromScan0(
            bm.width as i32,
            bm.height as i32,
            stride,
            PIXEL_FORMAT_32_ARGB,
            bm.bgra.as_ptr() as *mut u8,
            &mut out,
        );
        if status != 0 {
            return Err(Error::msg("GdipCreateBitmapFromScan0 failed"));
        }
    }
    Ok(out)
}

fn dispose(p: *mut GpBitmap) {
    unsafe {
        GdipDisposeImage(p as *mut GpImage);
    }
}

fn find_encoder_clsid(mime: &str) -> Result<windows_sys::core::GUID> {
    let mut num: u32 = 0;
    let mut size: u32 = 0;
    unsafe {
        if GdipGetImageEncodersSize(&mut num, &mut size) != 0 || num == 0 || size == 0 {
            return Err(Error::msg("GdipGetImageEncodersSize failed"));
        }
        let mut buf = vec![0u8; size as usize];
        if GdipGetImageEncoders(num, size, buf.as_mut_ptr() as *mut ImageCodecInfo) != 0 {
            return Err(Error::msg("GdipGetImageEncoders failed"));
        }
        let codecs = core::slice::from_raw_parts(buf.as_ptr() as *const ImageCodecInfo, num as usize);
        for c in codecs {
            let mt_ptr = c.MimeType;
            if mt_ptr.is_null() {
                continue;
            }
            let mut len = 0;
            while *mt_ptr.add(len) != 0 {
                len += 1;
            }
            let mt_slice = core::slice::from_raw_parts(mt_ptr, len);
            let mt = String::from_utf16_lossy(mt_slice);
            if mt.eq_ignore_ascii_case(mime) {
                return Ok(c.Clsid);
            }
        }
    }
    Err(Error::msg("encoder not found"))
}

pub fn save_png(bm: &Bitmap, path: &str) -> Result<()> {
    save_image(bm, path, "image/png")
}
pub fn save_jpg(bm: &Bitmap, path: &str, _quality: u8) -> Result<()> {
    save_image(bm, path, "image/jpeg")
}

fn save_image(bm: &Bitmap, path: &str, mime: &str) -> Result<()> {
    let clsid = find_encoder_clsid(mime)?;
    let gp = make_gdip_bitmap(bm)?;
    let pathw = wide(path);
    unsafe {
        let status = GdipSaveImageToFile(
            gp as *mut GpImage,
            pathw.as_ptr(),
            &clsid,
            core::ptr::null(),
        );
        dispose(gp);
        if status != 0 {
            return Err(Error::msg("GdipSaveImageToFile failed"));
        }
    }
    Ok(())
}

/// Encode PNG via temp file (v1 path; in-memory IStream-based path lands in v1.5).
pub fn encode_png_bytes(bm: &Bitmap) -> Result<Vec<u8>> {
    let tmp = temp_path("png");
    save_png(bm, &tmp)?;
    let bytes = std::fs::read(&tmp).map_err(|_| Error::msg("read tmp failed"))?;
    let _ = std::fs::remove_file(&tmp);
    Ok(bytes)
}

fn temp_path(ext: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::var("TEMP").unwrap_or_else(|_| ".".into());
    format!("{}\\frame-tmp-{}.{}", dir, ts, ext)
}

pub fn decode_file(path: &str) -> Result<Bitmap> {
    let pathw = wide(path);
    unsafe {
        let mut bmp: *mut GpBitmap = core::ptr::null_mut();
        if GdipCreateBitmapFromFile(pathw.as_ptr(), &mut bmp) != 0 || bmp.is_null() {
            return Err(Error::msg("GdipCreateBitmapFromFile failed"));
        }
        let r = read_pixels(bmp);
        dispose(bmp);
        r
    }
}

/// Decode via temp file. v1 simplification.
pub fn decode_bytes(bytes: &[u8]) -> Result<Bitmap> {
    let tmp = temp_path("png");
    std::fs::write(&tmp, bytes).map_err(|_| Error::msg("tmp write failed"))?;
    let r = decode_file(&tmp);
    let _ = std::fs::remove_file(&tmp);
    r
}

unsafe fn read_pixels(gp: *mut GpBitmap) -> Result<Bitmap> {
    unsafe {
        let mut w: u32 = 0;
        let mut h: u32 = 0;
        GdipGetImageWidth(gp as *mut GpImage, &mut w);
        GdipGetImageHeight(gp as *mut GpImage, &mut h);
        if w == 0 || h == 0 {
            return Err(Error::msg("zero-size image"));
        }
        let mut data: BitmapData = core::mem::zeroed();
        let rect = windows_sys::Win32::Graphics::GdiPlus::Rect {
            X: 0,
            Y: 0,
            Width: w as i32,
            Height: h as i32,
        };
        let status = GdipBitmapLockBits(
            gp,
            &rect,
            ImageLockModeRead as u32,
            PIXEL_FORMAT_32_ARGB,
            &mut data,
        );
        if status != 0 {
            return Err(Error::msg("GdipBitmapLockBits failed"));
        }
        let stride = data.Stride.unsigned_abs() as usize;
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        for y in 0..h as usize {
            let src_row = (data.Scan0 as *const u8).add(y * stride);
            let dst_row = bgra.as_mut_ptr().add(y * (w as usize) * 4);
            core::ptr::copy_nonoverlapping(src_row, dst_row, (w * 4) as usize);
        }
        GdipBitmapUnlockBits(gp, &mut data);
        Ok(Bitmap { width: w, height: h, bgra })
    }
}

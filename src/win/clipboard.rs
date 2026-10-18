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

use crate::error::{Error, Result};
use crate::gfx::bitmap::Bitmap;
use crate::win::wide;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{BI_BITFIELDS, BITMAPV5HEADER};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GHND, GlobalAlloc, GlobalLock, GlobalUnlock};
unsafe extern "system" {
    fn GlobalFree(hmem: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
}
use windows_sys::Win32::System::Ole::CF_DIB;
use windows_sys::Win32::System::Ole::CF_DIBV5;

pub fn write_image(hwnd: HWND, bmp: &Bitmap, png_bytes: &[u8]) -> Result<()> {
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            return Err(Error::msg("OpenClipboard failed"));
        }
        EmptyClipboard();

        let dibv5 = encode_dibv5(bmp);
        set_global(CF_DIBV5 as u32, &dibv5);

        let dib = encode_dib(bmp);
        set_global(CF_DIB as u32, &dib);

        let png_name = wide("PNG");
        let cf_png = RegisterClipboardFormatW(png_name.as_ptr());
        if cf_png != 0 {
            set_global(cf_png, png_bytes);
        }

        CloseClipboard();
    }
    Ok(())
}

unsafe fn set_global(format: u32, data: &[u8]) {
    unsafe {
        let h = GlobalAlloc(GHND, data.len());
        if h.is_null() {
            return;
        }
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            GlobalFree(h);
            return;
        }
        core::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        GlobalUnlock(h);
        if SetClipboardData(format, h as _) == 0 {
            GlobalFree(h);
        }
    }
}

fn encode_dibv5(bmp: &Bitmap) -> Vec<u8> {
    let header_size = core::mem::size_of::<BITMAPV5HEADER>();
    let pixel_size = bmp.row_stride() * bmp.height as usize;
    let mut out = vec![0u8; header_size + pixel_size];
    unsafe {
        let h: *mut BITMAPV5HEADER = out.as_mut_ptr() as _;
        (*h).bV5Size = header_size as u32;
        (*h).bV5Width = bmp.width as i32;
        (*h).bV5Height = -(bmp.height as i32);
        (*h).bV5Planes = 1;
        (*h).bV5BitCount = 32;
        (*h).bV5Compression = BI_BITFIELDS as u32;
        (*h).bV5RedMask = 0x00FF0000;
        (*h).bV5GreenMask = 0x0000FF00;
        (*h).bV5BlueMask = 0x000000FF;
        (*h).bV5AlphaMask = 0xFF000000;
        (*h).bV5SizeImage = pixel_size as u32;
        (*h).bV5CSType = u32::from_be_bytes(*b"sRGB");
    }
    out[header_size..].copy_from_slice(&bmp.bgra);
    out
}

fn encode_dib(bmp: &Bitmap) -> Vec<u8> {
    use windows_sys::Win32::Graphics::Gdi::BITMAPINFOHEADER;
    let header_size = core::mem::size_of::<BITMAPINFOHEADER>();
    let pixel_size = bmp.row_stride() * bmp.height as usize;
    let mut out = vec![0u8; header_size + pixel_size];
    unsafe {
        let h: *mut BITMAPINFOHEADER = out.as_mut_ptr() as _;
        (*h).biSize = header_size as u32;
        (*h).biWidth = bmp.width as i32;
        (*h).biHeight = -(bmp.height as i32);
        (*h).biPlanes = 1;
        (*h).biBitCount = 32;
        (*h).biCompression = 0;
        (*h).biSizeImage = pixel_size as u32;
    }
    out[header_size..].copy_from_slice(&bmp.bgra);
    out
}

pub fn read_image(hwnd: HWND) -> Option<Bitmap> {
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            return None;
        }
        let png_name = wide("PNG");
        let cf_png = RegisterClipboardFormatW(png_name.as_ptr());
        let mut result: Option<Bitmap> = None;

        if cf_png != 0 {
            let h = GetClipboardData(cf_png);
            if h != 0 {
                let p = GlobalLock(h as _) as *const u8;
                if !p.is_null() {
                    let len = windows_sys::Win32::System::Memory::GlobalSize(h as _);
                    let bytes = core::slice::from_raw_parts(p, len);
                    if let Ok(bmp) = crate::gfx::gdiplus::decode_bytes(bytes) {
                        result = Some(bmp);
                    }
                    GlobalUnlock(h as _);
                }
            }
        }

        if result.is_none() {
            let h = GetClipboardData(CF_DIB as u32);
            if h != 0 {
                let p = GlobalLock(h as _) as *const u8;
                if !p.is_null() {
                    let len = windows_sys::Win32::System::Memory::GlobalSize(h as _);
                    let bytes = core::slice::from_raw_parts(p, len);
                    result = decode_dib(bytes);
                    GlobalUnlock(h as _);
                }
            }
        }

        CloseClipboard();
        result
    }
}

fn decode_dib(bytes: &[u8]) -> Option<Bitmap> {
    use windows_sys::Win32::Graphics::Gdi::BITMAPINFOHEADER;
    if bytes.len() < core::mem::size_of::<BITMAPINFOHEADER>() {
        return None;
    }
    unsafe {
        let h: *const BITMAPINFOHEADER = bytes.as_ptr() as _;
        let w = (*h).biWidth as u32;
        let raw_h = (*h).biHeight;
        let h_abs = raw_h.unsigned_abs();
        let top_down = raw_h < 0;
        let bc = (*h).biBitCount;
        if bc != 32 && bc != 24 {
            return None;
        }
        let header_size = (*h).biSize as usize;
        let stride = ((w * bc as u32 / 8 + 3) & !3) as usize;
        let pix_start = header_size;
        if bytes.len() < pix_start + stride * h_abs as usize {
            return None;
        }
        let mut bgra = vec![0u8; (w * h_abs * 4) as usize];
        for row in 0..h_abs {
            let src_row = if top_down { row } else { h_abs - 1 - row } as usize;
            let src = &bytes[pix_start + src_row * stride..];
            let dst = &mut bgra[(row * w * 4) as usize..(row * w * 4 + w * 4) as usize];
            if bc == 32 {
                dst.copy_from_slice(&src[..(w * 4) as usize]);
            } else {
                for x in 0..w as usize {
                    dst[x * 4] = src[x * 3];
                    dst[x * 4 + 1] = src[x * 3 + 1];
                    dst[x * 4 + 2] = src[x * 3 + 2];
                    dst[x * 4 + 3] = 0xFF;
                }
            }
        }
        Some(Bitmap {
            width: w,
            height: h_abs,
            bgra,
        })
    }
}

pub mod project;

use crate::editor::Document;
use crate::error::Result;

pub fn export_png(doc: &Document, path: &str) -> Result<()> {
    let bmp = crate::editor::flatten::flatten(doc)?;
    crate::gfx::gdiplus::save_png(&bmp, path)
}

pub fn export_jpg(doc: &Document, path: &str) -> Result<()> {
    let bmp = crate::editor::flatten::flatten(doc)?;
    crate::gfx::gdiplus::save_jpg(&bmp, path, 90)
}

pub fn copy_to_clipboard(hwnd: windows_sys::Win32::Foundation::HWND, doc: &Document) -> Result<()> {
    let bmp = crate::editor::flatten::flatten(doc)?;
    let png = crate::gfx::gdiplus::encode_png_bytes(&bmp)?;
    crate::win::clipboard::write_image(hwnd, &bmp, &png)
}

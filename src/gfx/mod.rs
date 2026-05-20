pub mod bitmap;
pub mod color;
pub mod d2d;
pub mod dwrite;
pub mod gdiplus;

use crate::error::Result;

/// Lives on `App`; created lazily after tray init on a background thread.
pub struct Gfx {
    pub d2d: d2d::D2D,
    pub dwrite: dwrite::DWrite,
    pub gdiplus_token: usize,
}

impl Gfx {
    pub fn create() -> Result<Self> {
        let gdiplus_token = gdiplus::startup()?;
        let d2d = d2d::D2D::create()?;
        let dwrite = dwrite::DWrite::create()?;
        // Prime the font collection (slow first call).
        unsafe {
            let mut collection: Option<
                windows::Win32::Graphics::DirectWrite::IDWriteFontCollection,
            > = None;
            let _ = dwrite
                .factory
                .GetSystemFontCollection(&mut collection, false);
        }
        Ok(Self {
            d2d,
            dwrite,
            gdiplus_token,
        })
    }
}

impl Drop for Gfx {
    fn drop(&mut self) {
        gdiplus::shutdown(self.gdiplus_token);
    }
}

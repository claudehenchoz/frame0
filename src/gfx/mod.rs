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

pub mod bitmap;
pub mod color;
pub mod d2d;
pub mod dwrite;
pub mod gdiplus;
pub mod prewarm;

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

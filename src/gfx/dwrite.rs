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

//! DirectWrite factory + cached text formats.

use crate::error::{Error, Result};
use std::cell::RefCell;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::core::PCWSTR;

pub struct DWrite {
    pub factory: IDWriteFactory,
    cache: RefCell<Vec<CacheEntry>>,
}

struct CacheEntry {
    family: String,
    size: f32,
    bold: bool,
    centered: bool,
    fmt: IDWriteTextFormat,
}

fn to_wstr(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

impl DWrite {
    pub fn create() -> Result<Self> {
        let factory: IDWriteFactory =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        Ok(Self {
            factory,
            cache: RefCell::new(Vec::new()),
        })
    }

    /// Get (or create + cache) a text format for the given family/size/style.
    pub fn text_format(
        &self,
        family: &str,
        size: f32,
        bold: bool,
        centered: bool,
    ) -> Result<IDWriteTextFormat> {
        {
            let cache = self.cache.borrow();
            for e in cache.iter() {
                if e.family == family
                    && (e.size - size).abs() < 0.01
                    && e.bold == bold
                    && e.centered == centered
                {
                    return Ok(e.fmt.clone());
                }
            }
        }
        let fam_w = to_wstr(family);
        let locale_w = to_wstr("en-US");
        let weight = if bold {
            DWRITE_FONT_WEIGHT_BOLD
        } else {
            DWRITE_FONT_WEIGHT_NORMAL
        };
        let fmt = unsafe {
            self.factory
                .CreateTextFormat(
                    PCWSTR(fam_w.as_ptr()),
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    PCWSTR(locale_w.as_ptr()),
                )
                ?
        };
        if centered {
            unsafe {
                let _ = fmt.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
                let _ = fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            }
        }
        self.cache.borrow_mut().push(CacheEntry {
            family: family.to_string(),
            size,
            bold,
            centered,
            fmt: fmt.clone(),
        });
        Ok(fmt)
    }
}

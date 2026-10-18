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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const RED: Color = Color::rgb(0xE5, 0x3E, 0x3E);
    pub const BLUE: Color = Color::rgb(0x2C, 0x7B, 0xE5);
    pub const GREEN: Color = Color::rgb(0x2E, 0xA8, 0x4F);
    pub const YELLOW: Color = Color::rgb(0xFF, 0xD4, 0x3B);
    pub const ORANGE: Color = Color::rgb(0xF0, 0x8C, 0x00);
    pub const PURPLE: Color = Color::rgb(0x8E, 0x44, 0xEF);
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
    pub const TRANSPARENT: Color = Color { r: 0, g: 0, b: 0, a: 0 };

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 0xFF }
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub fn from_colorref(c: u32) -> Self {
        Color::rgb((c & 0xFF) as u8, ((c >> 8) & 0xFF) as u8, ((c >> 16) & 0xFF) as u8)
    }

    pub fn as_colorref(&self) -> u32 {
        (self.r as u32) | ((self.g as u32) << 8) | ((self.b as u32) << 16)
    }

    // UI theme colours (dark modern palette)
    pub const SURFACE: Color = Color::rgb(0x1C, 0x1E, 0x20);
    pub const ELEVATED: Color = Color::rgb(0x25, 0x25, 0x28);
    pub const HOVER: Color = Color::rgb(0x2E, 0x2E, 0x32);
    pub const ACTIVE: Color = Color::rgb(0x3A, 0x3D, 0x42);
    pub const BORDER: Color = Color::rgb(0x3A, 0x3A, 0x3D);
    pub const ICON_IDLE: Color = Color::rgb(0x8A, 0x8A, 0x8D);
    pub const ICON_ACTIVE: Color = Color::rgb(0xE4, 0xE4, 0xE5);
    pub const ACCENT: Color = Color::rgb(0x4A, 0x80, 0xD0);
    pub const SWATCH_RING: Color = Color::rgb(0x4A, 0x4A, 0x4D);
    pub const TEXT_LABEL: Color = Color::rgb(0x9A, 0x9A, 0x9D);
    pub const TEXT_VALUE: Color = Color::rgb(0xE4, 0xE4, 0xE5);
}
